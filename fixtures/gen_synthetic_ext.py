#!/usr/bin/env python3
"""Extends the synthetic fixture trees for the 0.1.2 / 0.2.0 features. Run AFTER gen_synthetic.py:

    python3 fixtures/gen_synthetic.py && python3 fixtures/gen_synthetic_ext.py

Adds to existing trees (without changing any file the 0.1.x goldens read):
  - PCI topology files (numa_node, local_cpulist, iommu_group), reset/reset_method
  - SR-IOV PF files and the xe `sriov_admin` tree (7 VFs, unprovisioned) on the B65
  - a cgroup v2 tree with the dmem controller (sys/fs/cgroup) and /proc/<pid>/cgroup files
  - connectors with status/enabled/modes/dpms; the devcoredump class (no dumps)
  - a second process sharing DRM client 7 (pid 4301) on the B65
  - kabi replay files (DRM query responses as raw bytes, RAS/uevent replays) under <tree>/kabi/
Creates the new tree `srv-2gpu-k7.2`: four B65-class GPUs behind different host bridges / root ports /
a switch on kernel 7.2, with AER counters on GPU A, two enabled VFs on GPU A, a pending crash dump on
GPU B, and kabi replays including RAS counters.
"""
import os, struct, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gen_synthetic import (OUT, w, ln, d, pci_device, drm_nodes, hwmon, temp_channel, gt, proc_common,
                           proc_pid, xe_fdinfo, OS_RELEASE)

# ------------------------------------------------------------------ helpers
def dev_dir(root, addr):
    """Return the sys-relative device directory for an address created by pci_device()."""
    for dirpath, dirs, files in os.walk(os.path.join(root, "sys/devices")):
        if os.path.basename(dirpath) == addr and "vendor" in files:
            return os.path.relpath(dirpath, root)
    raise KeyError(addr)

def topology_files(root, dev, numa, cpulist, iommu):
    w(root, f"{dev}/numa_node", str(numa))
    w(root, f"{dev}/local_cpulist", cpulist)
    w(root, f"{dev}/reset_method", "flr bus")
    w(root, f"{dev}/reset", "")
    d(root, f"sys/kernel/iommu_groups/{iommu}/devices")
    depth = dev.count("/")  # sys/devices/pci.../addr -> number of ../ needed to reach sys/
    ln(root, f"{dev}/iommu_group", "../" * depth + f"kernel/iommu_groups/{iommu}")
    ln(root, f"sys/kernel/iommu_groups/{iommu}/devices/{os.path.basename(dev)}", "../../../../" + dev[len("sys/"):])

def sriov_pf(root, dev, total, num, vfs=None, provisioned=None):
    """vfs: list of VF addresses (enabled ones); provisioned: dict vfN -> (quota, quantum, preempt, prio)."""
    w(root, f"{dev}/sriov_totalvfs", str(total))
    w(root, f"{dev}/sriov_numvfs", str(num))
    w(root, f"{dev}/sriov_drivers_autoprobe", "1")
    adm = f"{dev}/sriov_admin"
    d(root, f"{adm}/pf/profile")
    w(root, f"{adm}/pf/profile/exec_quantum_ms", "0")
    w(root, f"{adm}/pf/profile/preempt_timeout_us", "0")
    w(root, f"{adm}/pf/profile/sched_priority", "[low] normal high")
    ln(root, f"{adm}/pf/device", "../..")
    d(root, f"{adm}/.bulk_profile")
    for a in ("exec_quantum_ms", "preempt_timeout_us", "sched_priority", "vram_quota"):
        w(root, f"{adm}/.bulk_profile/{a}", "")
    for n in range(1, total + 1):
        p = f"{adm}/vf{n}/profile"
        q, eq, pt, pr = (provisioned or {}).get(f"vf{n}", (0, 0, 0, "low"))
        w(root, f"{p}/vram_quota", str(q))
        w(root, f"{p}/exec_quantum_ms", str(eq))
        w(root, f"{p}/preempt_timeout_us", str(pt))
        w(root, f"{p}/sched_priority", " ".join(f"[{x}]" if x == pr else x for x in ("low", "normal", "high")))
        w(root, f"{adm}/vf{n}/stop", "")
        if vfs and n <= len(vfs):
            ln(root, f"{adm}/vf{n}/device", "../../../" + vfs[n - 1])
    for i, vaddr in enumerate(vfs or []):
        ln(root, f"{dev}/virtfn{i}", "../" + vaddr)

def connectors(root, card, conns):
    """conns: list of (name, status, enabled, modes list, dpms)."""
    for name, status, enabled, modes, dpms in conns:
        c = f"sys/class/drm/card{card}-{name}"
        d(root, c)
        w(root, f"{c}/status", status)
        w(root, f"{c}/enabled", enabled)
        w(root, f"{c}/modes", "\n".join(modes) if modes else "")
        w(root, f"{c}/dpms", dpms)

def cgroup_tree(root, capacity, groups):
    """capacity: {region: bytes}; groups: {path: {'current': {region: bytes}, 'max': {region: 'max'|bytes}, 'procs': [pids]}}"""
    cg = "sys/fs/cgroup"
    d(root, cg)
    w(root, f"{cg}/dmem.capacity", "\n".join(f"{r} {b}" for r, b in capacity.items()))
    total = {}
    for path, g in groups.items():
        p = f"{cg}/{path.strip('/')}"
        d(root, p)
        w(root, f"{p}/dmem.current", "\n".join(f"{r} {b}" for r, b in g["current"].items()))
        for k in ("max", "min", "low"):
            vals = g.get(k, {r: ("max" if k == "max" else 0) for r in capacity})
            w(root, f"{p}/dmem.{k}", "\n".join(f"{r} {v}" for r, v in vals.items()))
        w(root, f"{p}/cgroup.procs", "\n".join(str(x) for x in g.get("procs", [])))
        w(root, f"{p}/cgroup.controllers", "cpu memory dmem")
        for r, b in g["current"].items():
            total[r] = total.get(r, 0) + b
        for pid in g.get("procs", []):
            w(root, f"proc/{pid}/cgroup", f"0::/{path.strip('/')}")
    w(root, f"{cg}/dmem.current", "\n".join(f"{r} {b}" for r, b in total.items()))
    w(root, f"{cg}/cgroup.controllers", "cpu memory dmem")
    w(root, f"{cg}/cgroup.subtree_control", "cpu memory dmem")

def devcoredump_class(root, dumps):
    """dumps: list of (id, failing_dev_sysrel, text)."""
    d(root, "sys/class/devcoredump")
    w(root, "sys/class/devcoredump/disabled", "0")
    for i, dev, text in dumps:
        c = f"sys/class/devcoredump/devcd{i}"
        d(root, c)
        w(root, f"{c}/data", text)
        ln(root, f"{c}/failing_device", "../../../" + dev[len("sys/"):])
        depth = dev.count("/")
        ln(root, f"{dev}/devcoredump", "../" * depth + f"class/devcoredump/devcd{i}")

# ------------------------------------------------------------------ kabi replay (raw DRM query bytes)
def mem_regions_bin(regions):
    """regions: list of (mem_class, instance, min_page_size, total, used, cpu_visible, cpu_visible_used)"""
    out = struct.pack("<II", len(regions), 0)
    for r in regions:
        out += struct.pack("<HHIQQQQ", *r) + b"\0" * 48
    return out

def engines_bin(engines):
    """engines: list of (class, instance, gt_id)"""
    out = struct.pack("<II", len(engines), 0)
    for c, i, g in engines:
        out += struct.pack("<HHHH", c, i, g, 0) + b"\0" * 24
    return out

def gt_list_bin(gts):
    """gts: list of (type, tile, gt, refclk, near_mask, far_mask, ipmaj, ipmin, iprev)"""
    out = struct.pack("<II", len(gts), 0)
    for t, tile, g, clk, near, far, a, b, c in gts:
        out += struct.pack("<HHH", t, tile, g) + b"\0" * 6 + struct.pack("<IQQHHHH", clk, near, far, a, b, c, 0) + b"\0" * 56
    return out

def topology_bin(masks):
    """masks: list of (gt_id, type, bytes)"""
    out = b""
    for g, t, m in masks:
        out += struct.pack("<HHI", g, t, len(m)) + m
    return out

def uc_fw_bin(uc_type, branch, major, minor, patch):
    return struct.pack("<HHIIIII", uc_type, 0, branch, major, minor, patch, 0) + b"\0" * 8

def config_bin(device, rev, flags, min_align, va_bits, max_prio):
    info = [(rev << 16) | device, flags, min_align, va_bits, max_prio]
    return struct.pack("<II", len(info), 0) + b"".join(struct.pack("<Q", x) for x in info)

def kabi_replay(root, addrs, used_bytes, guc=(0, 70, 44, 1), huc=(0, 9, 4, 13), ras=None, uevents=None):
    GIB = 1024 ** 3
    for addr in addrs:
        q = f"kabi/drm-query/{addr}"
        d(root, q)
        wb = lambda name, data: open(os.path.join(root, q, name), "wb").write(data)
        wb("mem_regions.bin", mem_regions_bin([
            (0, 0, 4096, 64 * GIB, 0, 64 * GIB, 0),                               # sysmem
            (1, 0, 65536, 32 * GIB, used_bytes, 32 * GIB, used_bytes),            # vram0, full BAR
        ]))
        wb("engines.bin", engines_bin([(0, 0, 0), (4, 0, 0), (4, 1, 0), (4, 2, 0), (4, 3, 0), (1, 0, 0), (1, 1, 0),
                                       (2, 0, 1), (2, 1, 1), (3, 0, 1)]))
        wb("gt_list.bin", gt_list_bin([(0, 0, 0, 19200000, 0x2, 0x1, 20, 1, 0), (1, 0, 1, 19200000, 0x2, 0x1, 13, 0, 0)]))
        geometry = bytes([0xff, 0xff, 0x0f, 0x00])   # 20 DSS
        compute = bytes([0xff, 0xff, 0x0f, 0x00])
        l3 = bytes([0xff, 0xff, 0x00, 0x00])          # 16 banks
        eu = bytes([0xff])                            # 8 EUs per DSS
        simd16 = bytes([0xff])
        wb("gt_topology.bin", topology_bin([(0, 1, geometry), (0, 2, compute), (0, 3, l3), (0, 4, eu), (0, 5, simd16)]))
        wb("uc_fw_version_0.bin", uc_fw_bin(0, *guc))
        wb("uc_fw_version_1.bin", uc_fw_bin(1, *huc))
        wb("config.bin", config_bin(0xe222, 1, 0x1, 65536, 48, 2))
    if ras is not None:
        d(root, "kabi/ras")
        # nodes.txt: node-id device-name node-name node-type   (one per line, as the LIST_NODES dump would return)
        lines = []
        for addr, nodes in ras.items():
            for nid, nname, counters in nodes:
                lines.append(f"{nid} {addr} {nname} 1")
                w(root, f"kabi/ras/counters-{nid}.txt", "\n".join(f"{eid} {ename} {val}" for eid, ename, val in counters))
        w(root, "kabi/ras/nodes.txt", "\n".join(lines))
    if uevents is not None:
        # one uevent per line; fields separated by '|' where the kernel uses NUL
        w(root, "kabi/uevents.txt", "\n".join("|".join(ev) for ev in uevents))

# ------------------------------------------------------------------ extend existing trees
def extend_b65(root, t1=False):
    dev = dev_dir(root, "0000:e3:00.0")
    topology_files(root, dev, numa=0, cpulist="0-31", iommu=42)
    sriov_pf(root, dev, total=7, num=0)
    connectors(root, 1, [("DP-1", "disconnected", "disabled", [], "Off"),
                         ("HDMI-A-1", "connected", "enabled", ["3840x2160", "2560x1440", "1920x1080"], "On")])
    devcoredump_class(root, [])
    # a second process sharing DRM client 7 (same client id on the same device -> one client, two pids)
    T = 12_000_000 if t1 else 10_000_000
    proc_pid(root, 4301, "Xwayland-helper", uid=1000, fds=[(11, "/dev/dri/renderD128",
             xe_fdinfo("0000:e3:00.0", 7,
                       {"vram0": dict(total="64 MiB", shared="64 MiB", active="0", resident="64 MiB", purgeable="0")},
                       {"rcs": (110_000 if t1 else 100_000, T, 1), "ccs": (0, T, 1), "bcs": (0, T, 1), "vcs": (0, T, 2), "vecs": (0, T, 2)}))])
    GIB = 1024 ** 3
    cgroup_tree(root, {"drm/0000:e3:00.0/vram0": 32 * GIB},
                {"system.slice/llama.service": {"current": {"drm/0000:e3:00.0/vram0": 1 * GIB},
                                                "max": {"drm/0000:e3:00.0/vram0": 20 * GIB}, "procs": [4242]},
                 "user.slice/user-1000.slice/session-2.scope": {"current": {"drm/0000:e3:00.0/vram0": 64 * 1024 ** 2},
                                                                "procs": [4300, 4301]},
                 "init.scope": {"current": {"drm/0000:e3:00.0/vram0": 0}, "procs": [5000, 6100]}})
    kabi_replay(root, ["0000:e3:00.0"], used_bytes=9412 * 1024 ** 2)
    if not t1:  # the recover suite runs against the primary tree; bind/unbind must exist
        for wf in ("bind", "unbind"):
            wp = os.path.join(root, "sys/bus/pci/drivers/xe", wf)
            open(wp, "w").close()
            os.chmod(wp, 0o600)  # real sysfs is 0200; fixtures need owner-readable cp

def extend_simple(root, addr, numa=0, cpulist="0-15", iommu=7, total_vfs=0):
    dev = dev_dir(root, addr)
    topology_files(root, dev, numa, cpulist, iommu)
    if total_vfs:
        sriov_pf(root, dev, total_vfs, 0)
    devcoredump_class(root, [])

# ------------------------------------------------------------------ the server tree
DUMP_TEXT = """**** Xe Device Coredump ****
Reason: GuC timeout on gt0 (exec queue 12)
kernel: 7.2.1-300.fc44.x86_64
module: xe
Snapshot time: 1789400000.123456789
Process: llama-server [pid 8100]
**** GT #0 ****
GuC firmware: 70.44.1
""" + "\n".join(f"reg 0x{i:04x} 0x{i * 7:08x}" for i in range(64))

def tree_srv():
    root = os.path.join(OUT, "srv-2gpu-k7.2")
    proc_common(root, "7.2.1-300.fc44.x86_64", distro="fedora")
    w(root, "etc/os-release", OS_RELEASE["fedora"].replace("43", "44"))
    d(root, "sys/module/xe"); w(root, "sys/module/xe/initstate", "live")
    GIB = 1024 ** 3
    # host bridge 16: root ports 16:01.0 (GPU A at 17:00.0, AER, 2 VFs) and 16:03.0 (GPU D at 19:00.0); NUMA 0
    # host bridge 64: root port 64:01.0 -> switch upstream 65:00.0 -> downstream 66:04.0 (GPU B 67:00.0), 66:08.0 (GPU C 68:00.0); NUMA 1
    def bridge(path_parent, addr, numa, root_port=False, aer=None):
        p = f"{path_parent}/{addr}"
        d(root, p)
        w(root, f"{p}/vendor", "0x8086"); w(root, f"{p}/device", "0x7ab8" if root_port else "0x1030")
        w(root, f"{p}/class", "0x060400"); w(root, f"{p}/numa_node", str(numa))
        ln(root, f"sys/bus/pci/devices/{addr}", "../../../" + p[len("sys/"):])
        if aer:
            cor, nf, fat = aer
            w(root, f"{p}/aer_rootport_total_err_cor", str(cor))
            w(root, f"{p}/aer_rootport_total_err_nonfatal", str(nf))
            w(root, f"{p}/aer_rootport_total_err_fatal", str(fat))
        return p
    rp1 = bridge("sys/devices/pci0000:16", "0000:16:01.0", 0, root_port=True, aer=(7, 0, 0))
    rp3 = bridge("sys/devices/pci0000:16", "0000:16:03.0", 0, root_port=True)
    rp64 = bridge("sys/devices/pci0000:64", "0000:64:01.0", 1, root_port=True)
    up = bridge(rp64, "0000:65:00.0", 1)
    dn4 = bridge(up, "0000:66:04.0", 1)
    dn8 = bridge(up, "0000:66:08.0", 1)

    def gpu(parent, addr, idx, card, render, hw, numa, cpulist, iommu):
        # pci_device() places the device under sys/devices/pci<dom:bus>; move it under the bridge chain instead
        dom_bus = "pci" + addr[:7]
        dev = pci_device(root, addr, 0x8086, 0xe222, 0x8086, 0x1100, 0x01, 0x030000, 32 * GIB,
                         link=("32.0 GT/s PCIe", "16", "32.0 GT/s PCIe", "16"), driver="xe")
        target = f"{parent}/{addr}"
        os.rename(os.path.join(root, dev), os.path.join(root, target))
        os.rmdir(os.path.join(root, f"sys/devices/{dom_bus}"))
        dev = target
        depth = dev.count("/")
        ln(root, f"{dev}/driver", "../" * (depth - 1) + "bus/pci/drivers/xe")
        ln(root, f"sys/bus/pci/drivers/xe/{addr}", "../../../../" + dev[len("sys/"):])
        for wf in ("bind", "unbind"):
            wp = os.path.join(root, "sys/bus/pci/drivers/xe", wf)
            open(wp, "w").close()
            os.chmod(wp, 0o600)  # real sysfs is 0200; fixtures need owner-readable cp
        ln(root, f"sys/bus/pci/devices/{addr}", "../../../" + dev[len("sys/"):])
        # drm nodes (drm_nodes() assumes the flat layout; recreate its links for the deep path)
        d(root, f"{dev}/drm/card{card}"); d(root, f"{dev}/drm/renderD{render}")
        w(root, f"{dev}/drm/card{card}/dev", f"226:{card}"); w(root, f"{dev}/drm/renderD{render}/dev", f"226:{render}")
        w(root, f"{dev}/drm/card{card}/uevent", f"MAJOR=226\nMINOR={card}\nDEVNAME=dri/card{card}\nDEVTYPE=drm_minor")
        ln(root, f"{dev}/drm/card{card}/device", "../../../" + addr)
        ln(root, f"{dev}/drm/renderD{render}/device", "../../../" + addr)
        ln(root, f"sys/class/drm/card{card}", "../../" + dev[len("sys/"):] + f"/drm/card{card}")
        ln(root, f"sys/class/drm/renderD{render}", "../../" + dev[len("sys/"):] + f"/drm/renderD{render}")
        files = {"power1_label": "card", "power1_cap": 200000000, "power1_cap_interval": 15, "power1_crit": 400000000,
                 "energy1_label": "card", "energy1_input": 500_000_000_000 + idx}
        temp_channel(files, 2, "pkg", 45000 + idx * 1000, crit=100000, emerg=105000, mx=95000)
        temp_channel(files, 3, "vram", 40000, crit=95000, emerg=100000)
        # hwmon() builds the class link from the flat layout; write it by hand here
        hwdir = f"{dev}/hwmon/hwmon{hw}"
        d(root, hwdir); w(root, f"{hwdir}/name", "xe")
        for k, v in files.items():
            w(root, f"{hwdir}/{k}", str(v))
        ln(root, f"sys/class/hwmon/hwmon{hw}", "../../" + dev[len("sys/"):] + f"/hwmon/hwmon{hw}")
        gt(root, dev, 0, 0, "render", dict(act=1200, cur=1200, min=1200, max=2400, rp0=2400, rpe=400, rpn=400, rpa=2400), 1000, profile="base")
        gt(root, dev, 0, 1, "media", dict(act=0, cur=400, min=400, max=1500, rp0=1500, rpe=400, rpn=400, rpa=1500), 1000, profile="base")
        topology_files(root, dev, numa, cpulist, iommu)
        return dev
    A = gpu(rp1, "0000:17:00.0", 0, 0, 128, 10, 0, "0-31", 30)
    D = gpu(rp3, "0000:19:00.0", 1, 1, 129, 11, 0, "0-31", 31)
    B = gpu(dn4, "0000:67:00.0", 2, 2, 130, 12, 1, "32-63", 32)
    C = gpu(dn8, "0000:68:00.0", 3, 3, 131, 13, 1, "32-63", 33)
    # AER on GPU A's endpoint
    w(root, f"{A}/aer_dev_correctable", "RxErr 2\nBadTLP 0\nBadDLLP 0\nRollover 0\nTimeout 3\nNonFatalErr 0\nCorrIntErr 0\nHeaderOF 0\nTOTAL_ERR_COR 5")
    w(root, f"{A}/aer_dev_fatal", "Undefined 0\nDLP 0\nSDES 0\nTLP 0\nFCP 0\nCmpltTO 0\nCmpltAbrt 0\nUnxCmplt 0\nRxOF 0\nMalfTLP 0\nECRC 0\nUnsupReq 0\nACSViol 0\nUncorrIntErr 0\nBlockedTLP 0\nAtomicOpBlocked 0\nTLPBlockedErr 0\nPoisonTLPBlocked 0\nTOTAL_ERR_FATAL 0")
    w(root, f"{A}/aer_dev_nonfatal", "Undefined 0\nDLP 0\nSDES 0\nTLP 0\nFCP 0\nCmpltTO 0\nCmpltAbrt 0\nUnxCmplt 0\nRxOF 0\nMalfTLP 0\nECRC 0\nUnsupReq 1\nACSViol 0\nUncorrIntErr 0\nBlockedTLP 0\nAtomicOpBlocked 0\nTLPBlockedErr 0\nPoisonTLPBlocked 0\nTOTAL_ERR_NONFATAL 1")
    # two VFs enabled on GPU A, bound to vfio-pci, provisioned quotas
    for i, vaddr in enumerate(["0000:17:00.1", "0000:17:00.2"]):
        vp = f"{rp1}/{vaddr}"
        d(root, vp)
        w(root, f"{vp}/vendor", "0x8086"); w(root, f"{vp}/device", "0xe222"); w(root, f"{vp}/class", "0x030000"); w(root, f"{vp}/numa_node", "0")
        d(root, "sys/bus/pci/drivers/vfio-pci")
        ln(root, f"{vp}/driver", "../../../../bus/pci/drivers/vfio-pci")
        ln(root, f"{vp}/physfn", "../0000:17:00.0")
        ln(root, f"sys/bus/pci/devices/{vaddr}", "../../../" + vp[len("sys/"):])
    sriov_pf(root, A, total=7, num=2, vfs=["0000:17:00.1", "0000:17:00.2"],
             provisioned={"vf1": (4 * GIB, 10, 20000, "normal"), "vf2": (4 * GIB, 10, 20000, "normal")})
    for g_ in (B, C, D):
        sriov_pf(root, g_, total=7, num=0)
    connectors(root, 0, [("DP-1", "disconnected", "disabled", [], "Off")])
    devcoredump_class(root, [(1, B, DUMP_TEXT)])
    cgroup_tree(root, {f"drm/{a}/vram0": 32 * GIB for a in ("0000:17:00.0", "0000:19:00.0", "0000:67:00.0", "0000:68:00.0")},
                {"system.slice/llama.service": {"current": {"drm/0000:17:00.0/vram0": 8 * GIB, "drm/0000:19:00.0/vram0": 0,
                                                            "drm/0000:67:00.0/vram0": 0, "drm/0000:68:00.0/vram0": 0},
                                                "max": {"drm/0000:17:00.0/vram0": 16 * GIB, "drm/0000:19:00.0/vram0": "max",
                                                        "drm/0000:67:00.0/vram0": "max", "drm/0000:68:00.0/vram0": "max"},
                                                "procs": [8100]}})
    proc_pid(root, 8100, "llama-server", uid=0, fds=[(9, "/dev/dri/renderD128", xe_fdinfo("0000:17:00.0", 3,
             {"vram0": dict(total="8192 MiB", shared="0", active="4096 MiB", resident="8192 MiB", purgeable="0")},
             {"rcs": (0, 10_000_000, 1), "ccs": (2_000_000, 10_000_000, 4), "bcs": (0, 10_000_000, 2), "vcs": (0, 10_000_000, 2), "vecs": (0, 10_000_000, 1)}))])
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t0\t0\t0\t0\n")
    # /dev/kmsg replay (seam XE_GMI_KMSG): records in the kernel's "prio,seq,ts,-;message" form
    w(root, "kmsg.txt", "6,1200,1789399000000000,-;xe 0000:67:00.0: [drm] GT0: GuC timeout on exec queue 12\n"
                        "3,1201,1789400000123456,-;xe 0000:67:00.0: [drm] *ERROR* device wedged, needs recovery\n"
                        "6,1202,1789400000200000,-;xe 0000:67:00.0: [drm] Xe device coredump has been created\n")
    kabi_replay(root, ["0000:17:00.0", "0000:19:00.0", "0000:67:00.0", "0000:68:00.0"], used_bytes=8 * GIB,
                ras={"0000:17:00.0": [(1, "correctable-errors", [(1, "core-compute", 0), (2, "soc-internal", 3)]),
                                      (2, "uncorrectable-errors", [(1, "core-compute", 0), (2, "soc-internal", 0)])],
                     "0000:67:00.0": [(3, "correctable-errors", [(1, "core-compute", 1), (2, "soc-internal", 0)]),
                                      (4, "uncorrectable-errors", [(1, "core-compute", 1), (2, "soc-internal", 0)])]},
                uevents=[["change@/devices/pci0000:64/0000:64:01.0/0000:65:00.0/0000:66:04.0/0000:67:00.0/drm/card2",
                          "ACTION=change", "DEVPATH=/devices/pci0000:64/0000:64:01.0/0000:65:00.0/0000:66:04.0/0000:67:00.0/drm/card2",
                          "SUBSYSTEM=drm", "WEDGED=rebind,bus-reset", "SEQNUM=4001"],
                         ["add@/devices/virtual/devcoredump/devcd1", "ACTION=add", "DEVPATH=/devices/virtual/devcoredump/devcd1",
                          "SUBSYSTEM=devcoredump", "SEQNUM=4002"],
                         ["unbind@/devices/pci0000:64/0000:64:01.0/0000:65:00.0/0000:66:04.0/0000:67:00.0", "ACTION=unbind",
                          "DEVPATH=/devices/pci0000:64/0000:64:01.0/0000:65:00.0/0000:66:04.0/0000:67:00.0", "SUBSYSTEM=pci",
                          "DRIVER=xe", "PCI_SLOT_NAME=0000:67:00.0", "SEQNUM=4003"],
                         ["bind@/devices/pci0000:64/0000:64:01.0/0000:65:00.0/0000:66:04.0/0000:67:00.0", "ACTION=bind",
                          "DEVPATH=/devices/pci0000:64/0000:64:01.0/0000:65:00.0/0000:66:04.0/0000:67:00.0", "SUBSYSTEM=pci",
                          "DRIVER=xe", "PCI_SLOT_NAME=0000:67:00.0", "SEQNUM=4004"]])

if __name__ == "__main__":
    for name in ("b65-g31-k7.1", "b65-g31-k7.1-t1"):
        extend_b65(os.path.join(OUT, name), t1=name.endswith("-t1"))
    extend_simple(os.path.join(OUT, "b580-g21-k6.14"), "0000:03:00.0", total_vfs=0)
    extend_simple(os.path.join(OUT, "dg2-a770-k6.12-forced"), "0000:03:00.0")
    extend_simple(os.path.join(OUT, "bmg-both-pl1-pl2-k7.0"), "0000:03:00.0", total_vfs=7)
    extend_simple(os.path.join(OUT, "multi-tile-2x2-k7.1"), "0000:17:00.0", total_vfs=7)
    tree_srv()
    n = sum(len(f) for _, _, f in os.walk(OUT))
    print(f"extended; {n} files under {OUT}")
