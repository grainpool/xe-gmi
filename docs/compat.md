# Compatibility



## Feature availability by kernel

| Feature | First kernel | Notes |
|---|---|---|
| Device discovery, clocks, throttle reasons, idle residency, per-process VRAM | 6.8 | first kernel with xe |
| Battlemage G21 (B580/B570) | 6.11 (force_probe), 6.12 (automatic) | |
| Engine utilization per process (`drm-cycles`) | 6.11 | |
| Achievable frequency `clock.rpa` | 6.14 | |
| Temperatures (pkg, vram) | 6.15 | |
| Writable power limit on Battlemage (PL1 when firmware enables it), fan speed, 1200 MHz default GT0 minimum | 6.16 | |
| PL2 burst limit (`power.limit.pl2`), Arc Pro B65/B70 (`e220`–`e223`) | 6.17 | |
| Power profile (`base` / `power_saving`) | 6.18 | |
| Aggregate throttle `reasons` attribute | 6.19 | derived from `reason_*` on older kernels |
| Temperature limits (max/crit/emergency), memory-controller/PCIe/per-channel VRAM sensors | 7.0 | |
| Crash dumps (devcoredump), PCI placement (NUMA, IOMMU, root port), AER counters | 6.8 | AER files exist only where the kernel owns AER; firmware-first platforms report `N/A`, which is normal |
| Power-limit averaging windows (`power*_max_interval`, `power*_cap_interval`) | 6.12 | present on every platform xe-gmi has tested; writable since 0.1.2, the firmware rounds to a representable value |
| cgroup v2 dmem controller, xe VRAM region registration (`cgroups`) | 6.14 | |
| Survivability sysfs attributes, wedged uevents (`health.survivability`, `doctor`) | 6.15 | the kernel-log wedge scan reads `/dev/kmsg` (root) |
| `sriov_admin` scheduling profiles (`sriov status`) | 7.0 | `vram_quota` and `stop` from 7.1; absent on kernels before 7.0 shows defaults |
| `sriov enable/disable`, `vf set/stop` | 7.0 / 7.1 | enabling VFs needs 7.0; `vram_quota` and `stop` need 7.1; never persisted |
| `firmware` versions (DRM `UC_FW_VERSION` query, render node) | 6.9 | `N/A (kernel 6.9+)` otherwise; the query runs through the kabi module (docs/kabi.md) |
| Kernel memory accounting in `info`/`query` (DRM `MEM_REGIONS` query) | 6.8 | unprivileged `used` from 7.0: before that the kernel reports 0 without CAP_PERFMON and `memory.used` says so instead of falling back silently |
| `topology --hardware`, VA bits and min alignment (DRM queries 0/2/3/5) | 6.8 | render node access is the only permission floor |
| `ras` counters and clear (`drm-ras` netlink family) | 7.1 core, 7.2 xe nodes | Battlemage nodes appear with 7.2; on a kernel whose core carries the family but no xe nodes (e.g. Fedora 7.1.13) `ras` lists N/A per device; exit 5 is reserved for kernels where the family itself is absent |
| `events` (live uevent stream incl. wedged) | 6.15 | binding the kernel multicast group needs root (`sudo xe-gmi events`) |
| `recover` (unbind, `reset_method` + `reset`, bind) | 6.8 | methods advertised by the device's `reset_method`; see docs/hardware-safety.md |

AER and RAS availability: AER files exist only where the kernel owns AER (firmware-first platforms
report `N/A`, which is normal). RAS needs the `drm-ras` netlink family **and** registered xe nodes
(7.2+ Battlemage); `ras`, `ras.correctable/uncorrectable` and the `doctor` line all degrade to N/A
that names the kernel version. Both stay fixture-only until a rig runs the matching kernel.

## Distributions (September 2026)

| Distro | Kernel | B65 | Battlemage power limit |
|---|---|---|---|
| Ubuntu 24.04 GA | 6.8 | no | – |
| Ubuntu 24.04.2 HWE | 6.11 | no | – |
| Ubuntu 24.04.3 HWE | 6.14 | no | – |
| Ubuntu 24.04.4 HWE | 6.17 | yes | PL2 |
| Ubuntu 24.04.5 HWE, Ubuntu 26.04 LTS | 7.0 | yes | PL2 (PL1 where enabled) |
| Fedora 43 (updates) | 7.1 | yes | PL2 |
| Fedora 44 | 6.19+ | yes | PL2 |
| Debian 13 | 6.12 (backports 7.1) | backports only | backports only |

## Toolchain

MSRV 1.75.0 (Ubuntu 24.04's cargo builds it with `--locked`); static musl binaries for x86_64 and aarch64.

## Measured on real hardware

From a read-only capture of the test machine taken with `scripts/capture-fixtures.sh` on
2026-09-14 (captures are personal snapshots and are not distributed with the repository; make your
own to run `tests/cli_captured.rs`). PCI topology `0000:e3:00.0`, `8086:e222`:

| Fact | Value |
|---|---|
| Distro / kernel | Fedora 43, `7.1.13-100.fc43.x86_64` |
| xe module state | `live`, `/lib/modules/7.1.13-100.fc43.x86_64/kernel/drivers/gpu/drm/xe/xe.ko.xz` |
| Product | `Battlemage G31 [Arc Pro B65]`, `card1` + `renderD128` |
| GTs | `tile0/gt0` (render, gtidle `gt0-rc`) and `tile0/gt1` (media, `gt1-mc`) |
| Clock boot values | gt0 min 1200 / max 2400 (rpn 400, rp0 2400 — **not** the 2850 of the synthetic B65 sample); gt1 min 400 / max 1500 (rpn 400, rp0 1500) |
| hwmon | `hwmon6` (`name=xe`): `power1_cap` 200000000 (PL2 only — no `power1_max` at all), `power1_crit` 400000000, `power1_cap_interval` 15, `energy1_input` + `energy1_label=card` |
| PCIe link | `2.5 GT/s PCIe` ×1 current **and** max in the capture — the Intel KB 000094587 endpoint artifact, not a trained-down link; since 0.2.2 xe-gmi reports the root port's trained gen5 ×16 view (`via root port`) and annotates the endpoint's own nodes |

`tests/cli_captured.rs` checks the same facts against whatever capture is present locally.

Confirmed by a manual hardware run (`sudo verify/run-verify.sh`, results
`20260915T003907Z`, same machine): boot clocks, `power1_cap`-only hwmon, silent driver clamp
(boot+50 W request stayed at the boot value), immediate `power_profile` switching, refused
out-of-range writes, persistence round trip via the real udev rule, and exit 4 + `sudo` hint for
unprivileged `set`. Two NVIDIA cards stay invisible to every command except `doctor` (selecting
one by PCI address exits 3). Full details: `docs/abi-notes.md` "Observed on hardware".
