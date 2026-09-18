#!/usr/bin/env python3
"""Generate the synthetic sysfs/procfs fixture trees under fixtures/synthetic/.

Single source of truth for every number the golden tests assert on.
Re-run after editing; commit the regenerated trees together with this file.

Each tree is a self-contained root containing `sys/`, `proc/` and (where relevant)
`lib/modules/`. Tests point XE_GMI_SYSFS_ROOT at `<tree>/sys`, XE_GMI_PROCFS_ROOT at
`<tree>/proc`, XE_GMI_MODULES_ROOT at `<tree>/lib/modules`, XE_GMI_ETC_ROOT at `<tree>/etc`.
A `<tree>-t1` sibling is the same system one sampling interval later (delta seams).
"""
import os, shutil, sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "synthetic")

def w(root, rel, content):
    p = os.path.join(root, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        f.write(content if content.endswith("\n") or content == "" else content + "\n")

def ln(root, rel, target):
    p = os.path.join(root, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    if os.path.lexists(p):
        os.remove(p)
    os.symlink(target, p)

def d(root, rel):
    os.makedirs(os.path.join(root, rel), exist_ok=True)

# ---------------------------------------------------------------- PCI device
def pci_device(root, addr, vendor, device, subv, subd, rev, class_, bar2_bytes,
               link=("16.0 GT/s PCIe", "16", "32.0 GT/s PCIe", "16"), driver=None,
               bar0_bytes=16 * 1024 * 1024):
    """Create sys/devices/pci0000:XX/<addr> and sys/bus/pci/devices/<addr> symlink."""
    dom_bus = "pci" + addr[:7]  # pci0000:e3 (synthetic: device sits directly on a root bus)
    dev = f"sys/devices/{dom_bus}/{addr}"
    d(root, dev)
    w(root, f"{dev}/vendor", f"0x{vendor:04x}")
    w(root, f"{dev}/device", f"0x{device:04x}")
    w(root, f"{dev}/subsystem_vendor", f"0x{subv:04x}")
    w(root, f"{dev}/subsystem_device", f"0x{subd:04x}")
    w(root, f"{dev}/revision", f"0x{rev:02x}")
    w(root, f"{dev}/class", f"0x{class_:06x}")
    w(root, f"{dev}/current_link_speed", link[0])
    w(root, f"{dev}/current_link_width", link[1])
    w(root, f"{dev}/max_link_speed", link[2])
    w(root, f"{dev}/max_link_width", link[3])
    w(root, f"{dev}/numa_node", "-1")
    w(root, f"{dev}/enable", "1")
    w(root, f"{dev}/modalias", f"pci:v{vendor:08X}d{device:08X}sv{subv:08X}sd{subd:08X}bc03sc00i00")
    w(root, f"{dev}/uevent", f"DRIVER={driver}\nPCI_CLASS={class_:X}\nPCI_ID={vendor:04X}:{device:04X}\n"
             f"PCI_SUBSYS_ID={subv:04X}:{subd:04X}\nPCI_SLOT_NAME={addr}\nMODALIAS=pci:v{vendor:08X}d{device:08X}sv{subv:08X}sd{subd:08X}bc03sc00i00"
             if driver else f"PCI_CLASS={class_:X}\nPCI_ID={vendor:04X}:{device:04X}\nPCI_SUBSYS_ID={subv:04X}:{subd:04X}\nPCI_SLOT_NAME={addr}")
    # resource file: 17 lines "start end flags" (6 BARs, ROM, bridge/reserved zeros)
    lines = []
    bar0_start = 0xf4000000
    lines.append(f"0x{bar0_start:016x} 0x{bar0_start + bar0_bytes - 1:016x} 0x{0x140204:016x}")
    lines.append("0x0000000000000000 0x0000000000000000 0x0000000000000000")
    if bar2_bytes:
        s = 0x4000000000
        lines.append(f"0x{s:016x} 0x{s + bar2_bytes - 1:016x} 0x{0x14220c:016x}")
    else:
        lines.append("0x0000000000000000 0x0000000000000000 0x0000000000000000")
    while len(lines) < 17:
        lines.append("0x0000000000000000 0x0000000000000000 0x0000000000000000")
    w(root, f"{dev}/resource", "\n".join(lines))
    d(root, f"{dev}/power")
    w(root, f"{dev}/power/runtime_status", "active")
    w(root, f"{dev}/power/control", "auto")
    if driver:
        d(root, f"sys/bus/pci/drivers/{driver}")
        ln(root, f"{dev}/driver", f"../../../bus/pci/drivers/{driver}")
        ln(root, f"sys/bus/pci/drivers/{driver}/{addr}", f"../../../../devices/{dom_bus}/{addr}")
    ln(root, f"sys/bus/pci/devices/{addr}", f"../../../devices/{dom_bus}/{addr}")
    return dev

def drm_nodes(root, dev, addr, card, render, driver_name):
    dom_bus = "pci" + addr[:7]
    d(root, f"{dev}/drm/card{card}")
    d(root, f"{dev}/drm/renderD{render}")
    w(root, f"{dev}/drm/card{card}/dev", f"226:{card}")
    w(root, f"{dev}/drm/renderD{render}/dev", f"226:{render}")
    w(root, f"{dev}/drm/card{card}/uevent", f"MAJOR=226\nMINOR={card}\nDEVNAME=dri/card{card}\nDEVTYPE=drm_minor")
    ln(root, f"{dev}/drm/card{card}/device", "../../../" + addr)
    ln(root, f"{dev}/drm/renderD{render}/device", "../../../" + addr)
    ln(root, f"sys/class/drm/card{card}", f"../../devices/{dom_bus}/{addr}/drm/card{card}")
    ln(root, f"sys/class/drm/renderD{render}", f"../../devices/{dom_bus}/{addr}/drm/renderD{render}")
    # a connector entry, to make sure discovery ignores card1-DP-1 style names
    d(root, f"sys/class/drm/card{card}-DP-1")
    w(root, f"sys/class/drm/card{card}-DP-1/status", "disconnected")

# ---------------------------------------------------------------- hwmon
def hwmon(root, dev, idx, files):
    hw = f"{dev}/hwmon/hwmon{idx}"
    d(root, hw)
    w(root, f"{hw}/name", "xe")
    w(root, f"{hw}/uevent", f"HWMON_NAME=xe\nOF_FULLNAME=")
    for k, v in files.items():
        w(root, f"{hw}/{k}", str(v))
    ln(root, f"sys/class/hwmon/hwmon{idx}", f"../../devices/pci{dev.split('/')[2][3:]}/{dev.split('/')[3]}/hwmon/hwmon{idx}")
    return hw

def temp_channel(files, n, label, inp, crit=None, emerg=None, mx=None):
    files[f"temp{n}_label"] = label
    files[f"temp{n}_input"] = inp
    if crit is not None: files[f"temp{n}_crit"] = crit
    if emerg is not None: files[f"temp{n}_emergency"] = emerg
    if mx is not None: files[f"temp{n}_max"] = mx

# ---------------------------------------------------------------- GT
THROTTLE_REASONS = ["pl1", "pl2", "pl4", "thermal", "prochot", "ratl", "vr_thermalert", "vr_tdc"]

def gt(root, dev, tile, gtid, kind, freqs, idle_ms, profile=None, rpa=True, reasons_attr=True,
       throttle=None):
    """freqs: dict(act, cur, min, max, rp0, rpe, rpn, rpa). throttle: set of active reason names."""
    base = f"{dev}/tile{tile}/gt{gtid}"
    f0 = f"{base}/freq0"
    d(root, f0)
    for k in ("act_freq", "cur_freq", "min_freq", "max_freq", "rp0_freq", "rpe_freq", "rpn_freq"):
        w(root, f"{f0}/{k}", str(freqs[k[:-5]]))
    if rpa:
        w(root, f"{f0}/rpa_freq", str(freqs["rpa"]))
    if profile is not None:
        w(root, f"{f0}/power_profile", "[base]    power_saving" if profile == "base" else "base    [power_saving]")
    th = f"{f0}/throttle"
    d(root, th)
    active = set(throttle or ())
    w(root, f"{th}/status", "1" if active else "0")
    for r in THROTTLE_REASONS:
        w(root, f"{th}/reason_{r}", "1" if r in active else "0")
    if reasons_attr:
        w(root, f"{th}/reasons", " ".join(r for r in THROTTLE_REASONS if r in active) or "none")
    gi = f"{base}/gtidle"
    d(root, gi)
    w(root, f"{gi}/name", f"gt{gtid}-{'mc' if kind == 'media' else 'rc'}")
    w(root, f"{gi}/idle_status", "gt-c6" if freqs["act"] == 0 else "gt-c0")
    w(root, f"{gi}/idle_residency_ms", str(idle_ms))

# ---------------------------------------------------------------- proc
OS_RELEASE = {
    "fedora": 'NAME="Fedora Linux"\nVERSION="43 (Workstation Edition)"\nID=fedora\nVERSION_ID=43\nPRETTY_NAME="Fedora Linux 43 (Workstation Edition)"\n',
    "ubuntu": 'PRETTY_NAME="Ubuntu 24.04.5 LTS"\nNAME="Ubuntu"\nVERSION_ID="24.04"\nVERSION="24.04.5 LTS (Noble Numbat)"\nID=ubuntu\nID_LIKE=debian\n',
    "debian": 'PRETTY_NAME="Debian GNU/Linux 13 (trixie)"\nNAME="Debian GNU/Linux"\nVERSION_ID="13"\nVERSION="13 (trixie)"\nID=debian\n',
    "arch": 'NAME="Arch Linux"\nPRETTY_NAME="Arch Linux"\nID=arch\n',
}

def proc_common(root, osrelease, boot_id="a1b2c3d4-0000-4000-8000-000000000001", distro="fedora"):
    w(root, "etc/os-release", OS_RELEASE[distro])
    w(root, "proc/sys/kernel/osrelease", osrelease)
    w(root, "proc/sys/kernel/random/boot_id", boot_id)
    w(root, "proc/uptime", "12345.67 98765.43")

def proc_pid(root, pid, comm, state="S (sleeping)", uid=1000, fds=None):
    """fds: list of (fd, link_target, fdinfo_text_or_None)"""
    p = f"proc/{pid}"
    d(root, p)
    w(root, f"{p}/comm", comm)
    w(root, f"{p}/status", f"Name:\t{comm}\nState:\t{state}\nPid:\t{pid}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nGid:\t{uid}\t{uid}\t{uid}\t{uid}\n")
    d(root, f"{p}/fd")
    d(root, f"{p}/fdinfo")
    for fd, target, info in (fds or []):
        ln(root, f"{p}/fd/{fd}", target)
        w(root, f"{p}/fdinfo/{fd}", f"pos:\t0\nflags:\t02100002\nmnt_id:\t28\nino:\t1234\n" + (info or ""))

def xe_fdinfo(pdev, client_id, mem, cycles):
    """mem: dict region -> dict(total, shared, active, resident, purgeable) strings (as the kernel prints them)
       cycles: dict class -> (cycles, total_cycles, capacity)"""
    out = [f"drm-driver:\txe", f"drm-client-id:\t{client_id}", f"drm-pdev:\t{pdev}"]
    for region in ("system", "gtt", "vram0", "stolen"):
        m = mem.get(region, {})
        for stat in ("total", "shared", "active", "resident", "purgeable"):
            out.append(f"drm-{stat}-{region}:\t{m.get(stat, '0')}")
    for cls in ("rcs", "bcs", "vcs", "vecs", "ccs"):
        c, t, cap = cycles.get(cls, (0, 0, 1))
        out.append(f"drm-cycles-{cls}:\t{c}")
        out.append(f"drm-total-cycles-{cls}:\t{t}")
        out.append(f"drm-engine-capacity-{cls}:\t{cap}")
    return "\n".join(out) + "\n"

# ---------------------------------------------------------------- NVIDIA cards (must be ignored)
# Synthetic placeholder ids: the fixture only needs "a GPU on another vendor's driver exists".
def nvidia_cards(root, first_card=0, first_render=129):
    a = pci_device(root, "0000:01:00.0", 0x10de, 0xffff, 0x10de, 0xffff, 0xa1, 0x030000, 0, driver="nvidia")
    drm_nodes(root, a, "0000:01:00.0", first_card, first_render, "nvidia")
    b = pci_device(root, "0000:81:00.0", 0x10de, 0xffff, 0x10de, 0xffff, 0xa1, 0x030000, 0, driver="nvidia")
    drm_nodes(root, b, "0000:81:00.0", first_card + 2, first_render + 1, "nvidia")
    # deliberately NO hwmon, NO tile dirs for these; the tool must never look.
    d(root, "sys/module/nvidia")
    w(root, "sys/module/nvidia/initstate", "live")

# ================================================================ TREES
def b65_common(root, t1=False, boot_id=None):
    """Battlemage G31 (Arc Pro B65, 8086:e222) on kernel 7.1 — the primary rig snapshot."""
    proc_common(root, "7.1.4-200.fc43.x86_64", boot_id or "a1b2c3d4-0000-4000-8000-000000000001")
    d(root, "sys/module/xe"); w(root, "sys/module/xe/initstate", "live")
    w(root, "sys/module/xe/srcversion", "0123456789ABCDEF00000000")
    d(root, "sys/module/xe/parameters"); w(root, "sys/module/xe/parameters/force_probe", "")
    nvidia_cards(root)
    dev = pci_device(root, "0000:e3:00.0", 0x8086, 0xe222, 0x8086, 0x1100, 0x01, 0x030000, 32 * 1024**3, driver="xe")
    drm_nodes(root, dev, "0000:e3:00.0", 1, 128, "xe")
    # hwmon — kernel 7.1 BMG: PL2 cap only on card channel, energy card+pkg, temps, fan
    E1, E2 = 123_456_789_012, 98_765_432_100
    if t1:  # +2000 ms: card +24.8 J (12.40 W), pkg +20.0 J (10.00 W)
        E1 += 24_800_000; E2 += 20_000_000
    files = {
        "power1_label": "card", "power1_cap": 200000000, "power1_cap_interval": 15, "power1_crit": 400000000,
        "energy1_label": "card", "energy1_input": E1,
        "energy2_label": "pkg", "energy2_input": E2,
        "fan1_input": 1200,
    }
    temp_channel(files, 2, "pkg", 41000, crit=100000, emerg=105000, mx=95000)
    temp_channel(files, 3, "vram", 38000, crit=95000, emerg=100000)
    temp_channel(files, 4, "mctrl", 40000, crit=100000, emerg=105000)
    temp_channel(files, 5, "pcie", 39000, crit=100000, emerg=105000)
    for i in range(8):
        temp_channel(files, 6 + i, f"vram_ch_{i}", 37000 + (i % 3) * 1000, crit=95000, emerg=100000)
    hwmon(root, dev, 7, files)
    # GTs: gt0 render (boot default min 1200 on BMG), gt1 media
    idle0 = 1_234_567 + (1900 if t1 else 0)   # +1900 idle of 2000 ms => 5.0 % active
    idle1 = 2_345_678 + (2000 if t1 else 0)   # 0.0 % active
    gt(root, dev, 0, 0, "render", dict(act=1200, cur=1200, min=1200, max=2850, rp0=2850, rpe=1800, rpn=300, rpa=2400), idle0, profile="base")
    gt(root, dev, 0, 1, "media",  dict(act=0,    cur=300,  min=300,  max=2400, rp0=2400, rpe=1500, rpn=300, rpa=2100), idle1, profile="base")
    w(root, f"{dev}/vram_d3cold_threshold", "300")
    # processes
    T = 12_000_000 if t1 else 10_000_000  # drm-total-cycles advances by 2_000_000 per interval
    proc_pid(root, 4242, "llama-server", fds=[
        (27, "/dev/dri/renderD128", xe_fdinfo("0000:e3:00.0", 42,
            {"vram0": dict(total="1024 MiB", shared="0", active="512 MiB", resident="1024 MiB", purgeable="0"),
             "gtt": dict(total="192 KiB", shared="0", active="0", resident="192 KiB", purgeable="0")},
            {"rcs": (1_400_000 if t1 else 1_000_000, T, 1), "ccs": (5_500_000 if t1 else 5_000_000, T, 1),
             "bcs": (0, T, 1), "vcs": (0, T, 2), "vecs": (0, T, 2)})),
        (3, "/dev/null", None),
    ])
    proc_pid(root, 4300, "Xwayland", fds=[
        (15, "/dev/dri/card1", xe_fdinfo("0000:e3:00.0", 7,
            {"vram0": dict(total="64 MiB", shared="64 MiB", active="0", resident="64 MiB", purgeable="0")},
            {"rcs": (110_000 if t1 else 100_000, T, 1), "ccs": (0, T, 1), "bcs": (0, T, 1), "vcs": (0, T, 2), "vecs": (0, T, 2)})),
        (16, "/dev/dri/renderD128", "drm-driver:\txe\ndrm-client-id:\t7\ndrm-pdev:\t0000:e3:00.0\n"),  # same client id => same client, must not double count
    ])
    proc_pid(root, 5000, "chrome", state="Z (zombie)", fds=[])          # zombie: no fds, must be skipped silently
    proc_pid(root, 6100, "cuda-app", fds=[(9, "/dev/nvidia0", None), (10, "/dev/dri/renderD129",
             "drm-driver:\tnvidia-drm\ndrm-client-id:\t3\ndrm-pdev:\t0000:01:00.0\n")])   # foreign GPU: ignore
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t0\t0\t0\t0\n")

def tree_b65(): 
    root = os.path.join(OUT, "b65-g31-k7.1"); b65_common(root)
    root = os.path.join(OUT, "b65-g31-k7.1-t1"); b65_common(root, t1=True)

def tree_b580_k614():
    """Battlemage G21 (Arc B580, 8086:e20b) on kernel 6.14: crit-only power, no temps, no fan, no profile."""
    root = os.path.join(OUT, "b580-g21-k6.14")
    proc_common(root, "6.14.0-32-generic", distro="ubuntu")
    d(root, "sys/module/xe"); w(root, "sys/module/xe/initstate", "live")
    dev = pci_device(root, "0000:03:00.0", 0x8086, 0xe20b, 0x8086, 0x1100, 0x01, 0x030000, 12 * 1024**3,
                     link=("16.0 GT/s PCIe", "8", "16.0 GT/s PCIe", "8"), driver="xe")
    drm_nodes(root, dev, "0000:03:00.0", 0, 128, "xe")
    hwmon(root, dev, 3, {"power1_label": "card", "power1_crit": 190000000,
                         "energy1_label": "card", "energy1_input": 55_000_000_000,
                         "energy2_label": "pkg", "energy2_input": 44_000_000_000})
    gt(root, dev, 0, 0, "render", dict(act=0, cur=300, min=300, max=2850, rp0=2850, rpe=1800, rpn=300, rpa=2400), 999_000, reasons_attr=False)
    gt(root, dev, 0, 1, "media",  dict(act=0, cur=300, min=300, max=2400, rp0=2400, rpe=1500, rpn=300, rpa=2100), 999_000, reasons_attr=False)
    w(root, f"{dev}/vram_d3cold_threshold", "300")
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t1000\t1000\t1000\t1000\n")

def tree_dg2_k612():
    """DG2 (Arc A770, 8086:56a0) on kernel 6.12 bound to xe via force_probe: PL1 max + rated_max + crit, no temps, 1 GT, no rpa."""
    root = os.path.join(OUT, "dg2-a770-k6.12-forced")
    proc_common(root, "6.12.43+deb13-amd64", distro="debian")
    d(root, "sys/module/xe"); w(root, "sys/module/xe/initstate", "live")
    d(root, "sys/module/xe/parameters"); w(root, "sys/module/xe/parameters/force_probe", "56a0")
    dev = pci_device(root, "0000:03:00.0", 0x8086, 0x56a0, 0x8086, 0x1020, 0x08, 0x030000, 16 * 1024**3,
                     link=("16.0 GT/s PCIe", "16", "16.0 GT/s PCIe", "16"), driver="xe")
    drm_nodes(root, dev, "0000:03:00.0", 0, 128, "xe")
    hwmon(root, dev, 2, {"power1_label": "card", "power1_max": 190000000, "power1_rated_max": 190000000,
                         "power1_max_interval": 28000, "power1_crit": 260000000,
                         "energy1_label": "card", "energy1_input": 77_000_000_000,
                         "in1_label": "pkg", "in1_input": 1120})
    gt(root, dev, 0, 0, "render", dict(act=0, cur=300, min=300, max=2400, rp0=2400, rpe=1650, rpn=300, rpa=0), 500_000, rpa=False, reasons_attr=False)
    w(root, f"{dev}/vram_d3cold_threshold", "300")
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t0\t0\t0\t0\n")

def tree_both_pl():
    """Battlemage G21 (8086:e20b) on kernel 7.0 with firmware enabling BOTH PL1 (max) and PL2 (cap), on card and pkg."""
    root = os.path.join(OUT, "bmg-both-pl1-pl2-k7.0")
    proc_common(root, "7.0.9-arch1-1", distro="arch")
    d(root, "sys/module/xe"); w(root, "sys/module/xe/initstate", "live")
    dev = pci_device(root, "0000:03:00.0", 0x8086, 0xe20b, 0x8086, 0x1100, 0x01, 0x030000, 12 * 1024**3,
                     link=("16.0 GT/s PCIe", "8", "16.0 GT/s PCIe", "8"), driver="xe")
    drm_nodes(root, dev, "0000:03:00.0", 0, 128, "xe")
    files = {"power1_label": "card", "power1_max": 190000000, "power1_max_interval": 28000,
             "power1_cap": 250000000, "power1_cap_interval": 15, "power1_crit": 300000000,
             "power2_label": "pkg", "power2_max": 150000000, "power2_max_interval": 28000,
             "power2_cap": 200000000, "power2_cap_interval": 15,
             "energy1_label": "card", "energy1_input": 10_000_000_000, "energy2_label": "pkg", "energy2_input": 9_000_000_000,
             "fan1_input": 900}
    temp_channel(files, 2, "pkg", 45000, crit=100000, emerg=105000, mx=95000)
    temp_channel(files, 3, "vram", 42000, crit=95000, emerg=100000)
    hwmon(root, dev, 5, files)
    gt(root, dev, 0, 0, "render", dict(act=1200, cur=1200, min=1200, max=2850, rp0=2850, rpe=1800, rpn=300, rpa=2400), 100_000, profile="power_saving", throttle={"pl1", "thermal"})
    gt(root, dev, 0, 1, "media",  dict(act=0, cur=300, min=300, max=2400, rp0=2400, rpe=1500, rpn=300, rpa=2100), 100_000, profile="base")
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t0\t0\t0\t0\n")

def tree_multi_tile():
    """Synthetic 2-tile x 2-GT device (8086:e222 as placeholder) on kernel 7.1 — exercises all-GT writes and gt numbering."""
    root = os.path.join(OUT, "multi-tile-2x2-k7.1")
    proc_common(root, "7.1.4-200.fc43.x86_64")
    d(root, "sys/module/xe"); w(root, "sys/module/xe/initstate", "live")
    dev = pci_device(root, "0000:17:00.0", 0x8086, 0xe222, 0x8086, 0x1100, 0x01, 0x030000, 64 * 1024**3, driver="xe")
    drm_nodes(root, dev, "0000:17:00.0", 0, 128, "xe")
    files = {"power1_label": "card", "power1_cap": 300000000, "power1_cap_interval": 15, "power1_crit": 450000000,
             "energy1_label": "card", "energy1_input": 1_000_000_000}
    temp_channel(files, 2, "pkg", 50000, crit=100000, emerg=105000, mx=95000)
    hwmon(root, dev, 9, files)
    gt(root, dev, 0, 0, "render", dict(act=1200, cur=1200, min=1200, max=2850, rp0=2850, rpe=1800, rpn=300, rpa=2400), 1, profile="base")
    gt(root, dev, 0, 1, "media",  dict(act=0, cur=300, min=300, max=2400, rp0=2400, rpe=1500, rpn=300, rpa=2100), 1, profile="base")
    gt(root, dev, 1, 2, "render", dict(act=1200, cur=1200, min=1200, max=2850, rp0=2850, rpe=1800, rpn=300, rpa=2400), 1, profile="base")
    gt(root, dev, 1, 3, "media",  dict(act=0, cur=300, min=300, max=2400, rp0=2400, rpe=1500, rpn=300, rpa=2100), 1, profile="base")
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t0\t0\t0\t0\n")

def tree_no_xe_ubuntu68():
    """Ubuntu 24.04 GA kernel 6.8: B65 present but unbound (ID unknown to xe), DG2 bound to i915, two NVIDIA cards."""
    root = os.path.join(OUT, "no-xe-ubuntu-6.8")
    proc_common(root, "6.8.0-79-generic", distro="ubuntu")
    nvidia_cards(root, first_card=0, first_render=128)
    dg2 = pci_device(root, "0000:03:00.0", 0x8086, 0x56a0, 0x8086, 0x1020, 0x08, 0x030000, 16 * 1024**3, driver="i915")
    drm_nodes(root, dg2, "0000:03:00.0", 1, 130, "i915")
    pci_device(root, "0000:e3:00.0", 0x8086, 0xe222, 0x8086, 0x1100, 0x01, 0x030000, 256 * 1024**2, driver=None)
    d(root, "sys/module/i915"); w(root, "sys/module/i915/initstate", "live")
    # xe module is available for this kernel but not loaded (no sys/module/xe)
    w(root, "lib/modules/6.8.0-79-generic/kernel/drivers/gpu/drm/xe/xe.ko.zst", "")
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t1000\t1000\t1000\t1000\n")

def tree_no_gpu():
    root = os.path.join(OUT, "no-gpu")
    proc_common(root, "7.0.0-27-generic", distro="ubuntu")
    d(root, "sys/class/drm"); d(root, "sys/bus/pci/devices")
    w(root, "lib/modules/7.0.0-27-generic/kernel/drivers/gpu/drm/xe/xe.ko.zst", "")
    w(root, "proc/self/status", "Name:\txe-gmi\nState:\tR (running)\nPid:\t1\nUid:\t1000\t1000\t1000\t1000\n")

if __name__ == "__main__":
    if os.path.isdir(OUT):
        shutil.rmtree(OUT)
    for fn in (tree_b65, tree_b580_k614, tree_dg2_k612, tree_both_pl, tree_multi_tile, tree_no_xe_ubuntu68, tree_no_gpu):
        fn()
    n = sum(len(f) for _, _, f in os.walk(OUT))
    print(f"generated {n} files under {OUT}")
