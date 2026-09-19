# Capability ledger

What a GPU management CLI can offer, and what xe-gmi 0.2.2 does on Battlemage/DG2 under the xe driver.

The complete surface a GPU management CLI can offer, classified against the current release.
Statuses: **mapped** (implemented), **degraded** (implemented with a documented difference in
meaning or scope), **N/A-interface** (the kernel exposes no interface; nothing xe-gmi can do
without kernel changes), **N/A-scope** (an interface exists but the current release deliberately
leaves it out; listed with the path so a later release can add it), **impossible-without-kernel-work**
(the only route is debugfs/ioctl/firmware and is excluded by this product's safety rules).

## Identity and bus

| Capability | Status | Detail |
|---|---|---|
| Device enumeration, index, PCI address | mapped | `list`, `index`, `pci.address` |
| Product name | mapped | pci.ids lookup + fallback table |
| Vendor/device/subsystem/revision IDs | mapped | `pci.*` |
| PCIe generation and width, current and max | mapped | `pci.link.*`; since 0.2.2 resolved via the root port when the endpoint carries the KB 000094587 artifact |
| Kernel/driver version | mapped | `driver.kernel`; the xe module carries no separate version |
| Firmware versions (GuC/HuC/GSC) | mapped (0.2.0, GuC/HuC) | `firmware`, `firmware.guc/huc` from the DRM `UC_FW_VERSION` query (6.9+); since 0.2.2 a microcontroller answering with an all-zero version reads N/A ("not loaded") instead of `0.0.0`; GSC stays debugfs-only, no sysfs interface |
| VBIOS/OPROM version, board serial, UUID, part number | N/A-interface | nothing in sysfs |
| NUMA node / CPU affinity | mapped (0.1.2) | `topology`, `pci.numa_node`, `pci.local_cpus` |
| Topology between GPUs, peer links | mapped (0.1.2, affinity only) | file-derived PIX/PHB/NODE/SYS matrix; P2P capability probing stays excluded (it would submit GPU work); no fabric on these products |
| Display connectors, attached displays, modes | mapped read (0.1.2) | `info` Connectors, `display.*` from `/sys/class/drm/card<N>-*`; mode setting stays out of scope |

## Telemetry

| Capability | Status | Detail |
|---|---|---|
| Power draw | degraded | average over the sampling window from `energy*_input`; no instantaneous reading exists (`power*_input` unimplemented in xe) |
| Energy consumed | mapped | `energy.card`, `energy.pkg` |
| Package voltage | mapped | `voltage.pkg` when `in1_input` exists |
| GPU temperature (+ limits) | mapped | `temp.pkg[.max/.crit/.emergency]` (7.0 for limits) |
| Memory temperature (+ limits) | mapped | `temp.vram[.crit/.emergency]` |
| Additional sensors (mctrl, pcie, per-channel VRAM) | mapped | `info` Thermal, 7.0+ |
| Fan speed RPM | mapped | `fan.rpm`, read-only, 6.16+ |
| Fan speed percent | N/A-interface | no PWM/max-RPM attribute |
| GPU utilization | degraded | GT C0 residency (`gtidle`), i.e. "GT awake" time, not engine occupancy; per GT |
| Engine utilization (render/compute/video/enhance/copy) | degraded | sum over *visible* DRM clients from fdinfo cycles; complete only as root; 6.11+ |
| Memory-bandwidth utilization | N/A-interface | |
| Encoder/decoder sessions | N/A-interface | per-client `vcs`/`vecs` utilization is the nearest data |
| Throttle status and reasons | mapped | `throttle.*` |
| Performance state | degraded | no P-state notion; `clock.act`/`clock.cur` and `idle.status` carry the information; `idle.status` is derived from `clock.act == 0` since 0.2.2 (the `gtidle/idle_status` mirror lies for a parked GT) |
| Device power state, runtime-PM status, D3cold policy, ASPM | mapped (0.2.2) | `pci.power_state`, `pci.runtime_status`, `pci.d3cold_allowed`, `pci.aspm.policy`, `pci.aspm.l1`; read-only, never written (docs/hardware-safety.md); `runtime_status=error` names the driver's underflow wedge and gates the bus-sentinel reads |
| VRAM total | mapped (0.2.0) | kernel allocator view via the DRM `MEM_REGIONS` query (`info` memory block, `memory.total`); fdinfo-text fallback stays documented in the status legend |
| VRAM used (device-level) | mapped (0.2.0) | `memory.used` from the `MEM_REGIONS` query through the `src/kabi` unsafe boundary (docs/kabi.md); kernel < 7.0 reports 0 without CAP_PERFMON and xe-gmi says so instead of lying |
| VRAM per process | mapped | `processes` |
| Memory clock | N/A-interface | `tile*/memory/freq0` exists only on PVC |
| GT clocks (actual, requested, min, max, hardware range, efficient) | mapped | `clock.*`, per GT |
| ECC status / error counts / retired pages | N/A-interface | Arc Pro cards have ECC memory but xe exposes no ECC attribute |
| Hang events, crash dumps | mapped (0.1.2) | `crash list|show|save|release` (devcoredump); `doctor` shows pending dumps and wedged kernel-log lines (root); since 0.2.0 `events` consumes the kernel's wedged uevent live. The kernel keeps no reset or hang counters |
| RAS error counters | mapped (0.2.0, root) | `ras`, `ras --clear`, `ras.correctable/uncorrectable` over the `drm-ras` netlink family; the core is 7.1 but xe nodes exist only on 7.2+ Battlemage — N/A names the kernel floor |
| Device recovery after a wedge | mapped (0.2.0) | `recover` (crash save, unbind, flr/bus reset or rebind, re-apply persistence) behind the preconditions in docs/hardware-safety.md; since 0.2.1 `recover` fails when the xe driver does not rebind |

## Control

| Capability | Status | Detail |
|---|---|---|
| Power limit (sustained PL1) | mapped | where firmware enables it (DG2; Battlemage when PL1 is on) |
| Power limit (burst PL2) | mapped | 6.17+; the only writable limit on the B65 |
| Power limit default / reset | mapped | recorded boot default; `rated_max` on DG2; driver-clamp sentinel on Battlemage |
| Power limit minimum / maximum bounds | degraded | the kernel exposes no min; the max is the firmware default discovered by the clamp |
| Power limit window (Tau) | mapped (0.1.2) | `set power-window`; firmware rounding reported, persisted and restored like the limits |
| Critical (I1) limit | N/A-scope | writable in the kernel; deliberately read-only (safety) |
| Lock/limit GT clocks | mapped | `set clocks --min --max`, per GT or all |
| Reset clocks | mapped | recorded boot default, else hardware range with the Battlemage 1200 MHz rule |
| Memory clocks | N/A-interface | |
| Application/default clocks, auto-boost | N/A-interface | no such concept in xe; `power_profile` is the nearest |
| Power profile (base / power_saving) | mapped | 6.18+ |
| Fan control | impossible-without-kernel-work | firmware-controlled; no interface |
| Compute mode | N/A-interface | no such concept in xe |
| Partitioning: SR-IOV virtual functions and their profiles | read/write (0.2.0) | `sriov status` plus `sriov enable/disable`, `sriov vf N set` and `vf N stop --force` (never persisted; VF stop is disruptive); `all` provisions the bulk profile |
| Device reset | excluded by safety rules | PCI `reset` exists, but xe has no PCI error handlers, so a reset only makes sense inside a driver unbind/bind sequence; xe-gmi does not offer it |
| Persistence of controls across reboot | mapped | udev rule + state file |
| Runtime power management (D3cold threshold) | N/A-scope | `vram_d3cold_threshold` is writable; read-only in 0.1.1 |
| Driver load parameters (`force_probe`) | N/A-scope | `doctor` explains; never written |

## Reporting

| Capability | Status | Detail |
|---|---|---|
| One-screen status table | mapped | `status`, `-u` |
| Full report | mapped | `info`, sections |
| Scriptable field query, CSV | mapped | `query`, `fields` |
| JSON | mapped | `--json`, schema v1 |
| XML | N/A-scope | JSON instead |
| Per-process table, loop | mapped | `processes -u` |
| Device monitor loop | mapped | `query -u`, `status -u` |
| Shell completions, man page | mapped | |

## Known upstream gaps worth reporting to intel-xe (facts for docs/abi-notes.md)

1. No sysfs attribute for VRAM size or device-level VRAM usage (only the ioctl query).
2. No instantaneous power reading (`power*_input`).
3. `power*_rated_max` invisible on mailbox platforms, so the firmware default is discoverable only by
   the clamp side effect.
4. No fan control or fan maximum.
5. No ECC status although the Pro cards ship ECC memory.
6. Engine utilization needs PTRACE_MODE_READ on every process (no device-level counters).

## Release-by-release surface

| Capability | 0.1.1 | 0.1.2 | 0.2.0 | 0.2.2 | Interface / note |
|---|---|---|---|---|---|
| NUMA node, local CPUs, IOMMU group, PCI path | — | Full | Full | same | PCI sysfs |
| Multi-GPU affinity matrix | — | Full (PIX/PHB/NODE/SYS, own definitions) | Full | same | PCI ancestry + NUMA |
| P2P capability probing | — | Excluded | Excluded | Excluded | would submit GPU work |
| AER error counters (endpoint, root port) | — | Full where exposed / N/A otherwise | same | same | `aer_dev_*`, `aer_rootport_*` |
| Crash dumps: list/show/save/release | — | Full | Full | same | devcoredump |
| Survivability mode/info | — | Read (root) | Read | same | xe sysfs 6.15+ |
| Wedged detection | — | Kernel log (root) | + live events | + runtime-PM underflow scan in `doctor` | kmsg; uevent WEDGED |
| Power-limit averaging windows | Read | Read/Write, persisted | same | same | hwmon `*_interval` |
| fdinfo memory: all stats × all regions | Partial | Full (JSON) | Full | same | fdinfo |
| Process grouping (client/cgroup/user), `pmon` | — | Full | Full | same | fdinfo + procfs |
| cgroup dmem view | — | Full | Full | same | cgroup v2 6.14+ |
| cgroup dmem limits | — | — | Write (`cgroup set`) | same | `dmem.max` |
| SR-IOV state and profiles | — | Read | Read/Write (enable/disable/vf set/stop) | same | PCI + `sriov_admin` 7.0/7.1 |
| Connectors | — | Read | Read | same | DRM sysfs |
| Support bundle | — | Full (directory, redacted) | Full | same | — |
| Provenance per field | — | Full | Full | same | — |
| Kernel memory regions (authoritative used/total, small-BAR) | — | — | Full (render node) | same | DRM query 1 |
| Engine/GT/EU topology | — | — | Full | same | DRM queries 0/3/5 |
| GuC/HuC versions | — | — | Full (6.9+) | honesty on EINVAL and unloaded microcontrollers | DRM query 7 |
| Device configuration (VA bits, alignment) | — | — | Full | same | DRM query 2 |
| RAS counters, clear | — | — | Full on 7.2+ Battlemage (root) | same | DRM RAS netlink |
| Event stream | — | — | Full (root) | same | uevent netlink |
| Device recovery (`recover`) | — | — | Full (flr/bus/rebind, preconditions) | fails when the driver does not rebind | PCI reset, driver bind |
| Link state resolved via the root port | — | — | — | Full | endpoint's own `current/max_link_*` carry the KB 000094587 artifact and are annotated, not trusted |
| Device power state, runtime-PM status, D3cold policy, ASPM | — | — | — | Full (read-only) | `pci.power_state`, `pci.runtime_status`, `pci.d3cold_allowed`, `pci.aspm.*` |
| Derived `idle.status`, powered-down sentinel gating | — | — | — | Full | `clock.act == 0` decides for a parked GT; `255` °C / fan 0 / sentinel power answer N/A while `runtime_status` reads `error` |
| Firmware flashing, fan control, ECC mode, OA/EU-stall, `gpu_health`, Xe configfs | Excluded | Excluded | Excluded | Excluded | reasons above and in docs/compat.md |
