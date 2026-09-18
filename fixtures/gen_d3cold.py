#!/usr/bin/env python3
"""Creates the tree `b65-g31-k7.1-d3cold` (run after gen_synthetic.py + gen_synthetic_ext.py).

One Arc Pro B65 (Battlemage G31) behind host bridge 16 / root port 16:01.0, captured in the
state the real rig reaches with `pcie_aspm.policy=powersupersave`:

  - the endpoint carries the Intel KB 000094587 artifact (current and max pinned to gen1 x1)
    while the root port reports the trained link (gen5 x16); L1 ASPM is enabled on the root
    port, the endpoint attribute reads disabled while powered down
  - power_state D3cold, d3cold_allowed 1, runtime_status `error` (the driver's runtime PM
    usage-count underflow bug parks the device in a state where every read answers sentinels)
  - hwmon reads the powered-down sentinels: 255 C on every temperature, fan 0
  - gt0 `act_freq` is 0 (RC6) while `gtidle/idle_status` still mirrors gt-c0 (proven on the
    healthy rig: the file lies after the park)
  - kmsg.txt carries `Runtime PM usage count underflow!` records for the device
  - the HuC answers the query with a zero version (microcontroller not loaded)
"""
import os, shutil, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gen_synthetic import OUT, w, ln, d, pci_device, gt, proc_common, temp_channel
from gen_synthetic_ext import topology_files, kabi_replay

def tree():
    root = os.path.join(OUT, "b65-g31-k7.1-d3cold")
    proc_common(root, "7.1.13-100.fc43.x86_64", distro="fedora")
    d(root, "sys/module/xe"); w(root, "sys/module/xe/initstate", "live")
    # system ASPM policy: powersupersave selected (the rig's powersave->D3cold recipe)
    w(root, "sys/module/pcie_aspm/parameters/policy", "default performance powersave [powersupersave]")

    rp = "sys/devices/pci0000:16/0000:16:01.0"
    d(root, rp)
    for k, v in {"vendor": "0x8086", "device": "0x7ab7", "class": "0x060400",
                 "numa_node": "0", "power_state": "D0unconstrained"}.items():
        w(root, f"{rp}/{k}", v)
    for k, v in {"current_link_speed": "32.0 GT/s PCIe", "current_link_width": "16",
                 "max_link_speed": "32.0 GT/s PCIe", "max_link_width": "16"}.items():
        w(root, f"{rp}/{k}", v)
    w(root, f"{rp}/link/l1_aspm", "enabled")
    ln(root, f"sys/bus/pci/devices/0000:16:01.0", "../../../" + rp[len("sys/"):])

    GIB = 1024 ** 3
    # endpoint: the KB 000094587 artifact (gen1 x1, current AND max)
    dev = pci_device(root, "0000:17:00.0", 0x8086, 0xe222, 0x8086, 0x1100, 0x01, 0x030000,
                     32 * GIB, link=("2.5 GT/s PCIe", "1", "2.5 GT/s PCIe", "1"), driver="xe")
    target = f"{rp}/0000:17:00.0"
    os.rename(os.path.join(root, dev), os.path.join(root, target))
    os.rmdir(os.path.join(root, "sys/devices/pci0000:17"))
    dev = target
    depth = dev.count("/")
    ln(root, f"{dev}/driver", "../" * (depth - 1) + "bus/pci/drivers/xe")
    ln(root, f"sys/bus/pci/drivers/xe/0000:17:00.0", "../../../../" + dev[len("sys/"):])
    ln(root, f"sys/bus/pci/devices/0000:17:00.0", "../../../" + dev[len("sys/"):])
    w(root, f"{dev}/link/l1_aspm", "disabled")

    # the powered-down device: D3cold with the runtime-PM state wedged in `error`
    w(root, f"{dev}/power_state", "D3cold")
    w(root, f"{dev}/power/runtime_status", "error")
    w(root, f"{dev}/power/d3cold_allowed", "1")

    d(root, f"{dev}/drm/card0"); d(root, f"{dev}/drm/renderD128")
    w(root, f"{dev}/drm/card0/dev", "226:0"); w(root, f"{dev}/drm/renderD128/dev", "226:128")
    w(root, f"{dev}/drm/card0/uevent", "MAJOR=226\nMINOR=0\nDEVNAME=dri/card0\nDEVTYPE=drm_minor")
    ln(root, f"{dev}/drm/card0/device", "../../../" + "0000:17:00.0")
    ln(root, f"{dev}/drm/renderD128/device", "../../../" + "0000:17:00.0")
    ln(root, f"sys/class/drm/card0", "../../" + dev[len("sys/"):] + "/drm/card0")
    ln(root, f"sys/class/drm/renderD128", "../../" + dev[len("sys/"):] + "/drm/renderD128")

    # hwmon: everything is the powered-down sentinel; limits stay real (config reads)
    files = {"power1_label": "card", "power1_cap": 150000000, "power1_cap_interval": 15,
             "power1_crit": 400000000, "energy1_label": "card", "energy1_input": 500_000_000_000,
             "fan1_label": "fan1", "fan1_input": 0}
    temp_channel(files, 2, "pkg", 255000, crit=100000, emerg=105000, mx=95000)
    temp_channel(files, 3, "vram", 255000, crit=95000, emerg=100000)
    hwdir = f"{dev}/hwmon/hwmon7"
    d(root, hwdir); w(root, f"{hwdir}/name", "xe")
    for k, v in files.items():
        w(root, f"{hwdir}/{k}", str(v))
    ln(root, f"sys/class/hwmon/hwmon7", "../../" + dev[len("sys/"):] + "/hwmon/hwmon7")

    gt(root, dev, 0, 0, "render", dict(act=0, cur=0, min=300, max=2850, rp0=2850, rpe=300,
                                       rpn=300, rpa=2850), 120000, profile="base")
    # the rig-proven artifact: parked GT (act 0) whose mirror file still says gt-c0
    w(root, f"{dev}/tile0/gt0/gtidle/idle_status", "gt-c0")

    topology_files(root, dev, 0, "0-15", 16)
    kabi_replay(root, ["0000:17:00.0"], 2 * GIB, guc=(0, 70, 44, 1), huc=(0, 0, 0, 0))

    w(root, "kmsg.txt", "\n".join([
        "6,1200,1024000,-;xe 0000:17:00.0: [drm] Found 0000:17:00.0 to be a Battlemage GPU",
        "4,1201,1030000,-;xe 0000:17:00.0: Runtime PM usage count underflow!",
        "4,1202,1040240,-;xe 0000:17:00.0: Runtime PM usage count underflow!",
        "",
    ]))

    # -t1: the second sample of a watch window; idle residency advanced a full window, so the
    # parked GT reads 0 % utilization (and the energy delta stays frozen -> gated anyway)
    t1 = os.path.join(OUT, "b65-g31-k7.1-d3cold-t1")
    # symlinks=True is mandatory: the tree carries relative symlink cycles
    # (device/driver -> drivers/xe/<device> -> back to the device); dereferencing
    # them makes the copy recurse without bound.
    shutil.copytree(root, t1, symlinks=True)
    w(t1, "sys/devices/pci0000:16/0000:16:01.0/0000:17:00.0/tile0/gt0/gtidle/idle_residency_ms",
      "122000")
    return root

if __name__ == "__main__":
    r = tree()
    n = sum(len(f) for _, _, f in os.walk(r))
    print(f"b65-g31-k7.1-d3cold: {n} files under {r}")
