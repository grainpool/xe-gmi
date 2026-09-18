# Changelog

All notable changes to xe-gmi are documented here. The format follows Keep a Changelog; versions
follow SemVer. JSON output is versioned separately (`schema_version`, currently 1).

## [0.2.2] — 2026-09-18

### Added
- Read-only PCIe power fields `pci.power_state`, `pci.runtime_status`, `pci.d3cold_allowed`,
  `pci.aspm.policy`, `pci.aspm.l1`; `pcie` gains Power and ASPM lines, `doctor` gains the matching
  capability lines. xe-gmi still never writes runtime-PM files (docs/hardware-safety.md).
- `pcie`/`info` resolve the link **via the root port**: the Arc endpoint's own `current/max_link_*`
  nodes carry the Intel KB 000094587 artifact (gen1 x1 on a Gen5 x16 card) and no longer override
  the root port's trained view; when only the endpoint nodes exist they are annotated as such.
- `doctor` scans the kernel log for the driver's `Runtime PM usage count underflow!` records and
  lists them per device, so a wedged power state is visible before a polling loop hammers it.
- Fixture `b65-g31-k7.1-d3cold` (+ `-t1`): a card parked in D3cold with bus-sentinel hwmon reads,
  the KB 000094587 artifact, the lying `idle_status` mirror and kmsg underflow records; golden and
  behavioural tests against it.

### Changed
- `idle.status` is derived: `clock.act == 0` decides for a parked GT because `gtidle/idle_status`
  keeps mirroring `gt-c0` after the park (proven on hardware).
- Powered-down sentinel reads are gated: while `power/runtime_status` reads `error`, `255` °C,
  fan 0 and sentinel power answer as N/A with the runtime-state reason, never as measurements.

### Fixed
- `src/kabi`: an `EINVAL` from a DRM query is no longer always called "not supported by this
  kernel" — when the device's runtime-PM status read `error` at open time the message names both
  readings (the same errno means an ioctl refused during a broken power state); a replay file that
  exists but cannot be read is reported as such instead of "not supported".
- `src/kabi`: a microcontroller that answers the firmware query with an all-zero version is
  reported N/A ("this microcontroller is not loaded") instead of version `0.0.0`.

## [0.2.1] — 2026-09-17

### Fixed
- `src/kabi`: a `drm-ras` nlctrl family lookup answered with `NLMSG_ERROR` and a raw negative code
  on kernels where the family exists but xe registers no nodes; feeding that code to
  `Errno::from_raw_os_error` tripped the `linux_raw` backend's encoded-errno assertion and aborted
  every command that probes a device. The raw code is now mapped directly, so `ras` lists N/A per
  device with exit 0 there, and exit 5 stays reserved for kernels where the family itself is absent
  (`docs/compat.md` corrected accordingly).
- `src/kabi`: the `drm-ras` family is resolved through the nlctrl id 16.
- `recover`: now fails when the driver does not rebind, instead of proceeding as if the device had
  come back.

## [0.2.0] — unreleased

### Added
- `src/kabi`: the crate's unsafe boundary — DRM render-node queries, the `drm-ras` generic-netlink
  client and the uevent reader (read-only, replayable; see `docs/kabi.md`). Every other module
  stays free of `unsafe`.
- `firmware`: GuC and HuC versions (DRM query, kernel 6.9+).
- `topology --hardware`: engine, GT and EU topology from the kernel; `info` gains the kernel
  allocator's memory numbers (authoritative total/used, CPU-visible, min page size) and VA-bits /
  min-alignment lines; `doctor` gains kernel memory, firmware and RAS lines.
- `ras` / `ras --clear`: RAS error counters over the `drm-ras` netlink family (root; xe nodes are
  7.2+ Battlemage); fields `ras.correctable`, `ras.uncorrectable`.
- `events`: live kernel uevent stream for xe devices (wedged with recovery methods, bind/unbind,
  devcoredump); `--json` prints one object per line.
- `cgroup set PATH --vram-max SIZE|max`: the dmem limit of one cgroup (write verified, clamps
  reported, not persisted).
- `sriov enable/disable` and `sriov vf N set|stop`: VF provisioning via `sriov_admin` (bulk profile
  via `vf all`); `stop` interrupts the VF's user and requires `--force`. Never persisted.
- `recover`: preconditions-checked device recovery (crash save → unbind → flr/bus reset or rebind
  → rebind → re-apply persistence); refuses on unadvertised methods, enabled VFs or a busy bus.

### Changed
- `#![forbid(unsafe_code)]` became `#![deny(unsafe_code)]` with one `#![allow(unsafe_code)]` file
  (`src/kabi/ioctl.rs`); a unit test enforces that no other file contains the token.
- New dependency: `rustix` (default-features off; `std`, `fs`, `net`) — exactly four direct crates.
- `memory.used` prefers the kernel's accounting (CAP_PERFMON gap on < 7.0 kernels is reported,
  not papered over); `cgroup show` label column widened to the documented 25-wide rule.

### Security
- Allowlist additions only (`src/write.rs`): PCI reset pair, `sriov_admin` attributes,
  driver `bind`/`unbind` (recover steps) and cgroup `dmem.{max,min,low}`; each guarded by its own
  exact path shape and unit test.

## [0.1.2] — unreleased

### Added
- `topology`: NUMA node, local CPUs, IOMMU group, PCI path (host bridge, root port) and SR-IOV per
  device, plus a PIX/PHB/NODE/SYS affinity matrix when there is more than one xe device.
- `pcie`: link state and AER error statistics, endpoint and root port, counted separately.
- `crash list|show|save|release`: pending GPU crash dumps (devcoredump); `release` is the one new
  dump write (any write releases a dump; xe-gmi writes `1`).
- `cgroups` and `cgroup show`: GPU memory accounting per cgroup via the cgroup v2 dmem controller.
- `sriov status`: PF state and per-VF provisioning from `sriov_admin` (read-only in 0.1.2).
- `processes --group-by client|cgroup|user` and `pmon`; processes gain full fdinfo memory
  (all five statistics per region), pids, uid and cgroup attribution.
- `set power-window`: the power-limit averaging window (ms or s), reported when the firmware rounds
  the value; persisted and restored by `reset` like the limits themselves.
- `doctor --bundle DIR [--no-redact]`: a directory of redacted reports (text, JSON, kmsg excerpt).
- Provenance on every field: `fields` gains SOURCE/ACCESS columns, JSON gains
  source/scope/access/quality, `-v` annotates explanations; unavailable entries name their source.
- `info --section topology|pcie|connectors|sriov|crash`; `info` and `doctor` gain topology, PCIe
  health, connectors, SR-IOV, crash dump and wedge-kernel-log lines; `get power-limit` shows windows.
- Fields: `pci.numa_node`, `pci.local_cpus`, `pci.iommu_group`, `pci.root_port`, `pci.aer.*`,
  `sriov.total_vfs`, `sriov.num_vfs`, `display.*`, `crash.pending`, `health.survivability`,
  `memory.used.source`.

### Changed
- Test goldens and JSON/golden fixtures follow the 0.1.2 output; scalar JSON arrays render inline.

## [0.1.1] — unreleased

### Fixed
- `verify/run-verify.sh`: the post-restore check read the wrong `power_profile` token and could
  report a false mismatch (or pass vacuously); it now reads the bracketed selection.

### Added
- `docs/hardware-safety.md`: everything xe-gmi writes, everything it never touches, and how to undo.
- `scripts/bootstrap-toolchain.sh`: private rustup toolchains for `scripts/ci-local.sh`.
- README section on testing against your own card with `scripts/capture-fixtures.sh`.
- Unit tests for name truncation, error-to-exit-code mapping, the persistence state file, the udev
  rule executable policy, kernel version parsing, the write allowlist (fixture walk) and readback
  classification.

### Changed
- Internal: one allowlisted sysfs write path instead of two.

## [0.1.0] — 2026-09-15

### Added
- `status` table (default), `info` report with sections, `list`, `fields`, `query` (CSV/JSON, loop
  mode), `processes` (per-client engine utilization and resident VRAM), `get power-limit|clocks|power-profile`.
- `set power-limit` (PL1/PL2, card/pkg channels, units W/mW/uW, driver-clamp reporting),
  `set clocks` (per GT or all, validated against the hardware range), `set power-profile`.
- `reset power-limit|clocks|power-profile|all` restoring recorded boot defaults.
- `persist install|remove|show|apply`: udev bind rule + `/etc/xe-gmi/persist.conf`, sticky `set`.
- `doctor`: kernel/driver/device diagnosis with per-device capability summary; exit 3 when no xe device is usable.
- `--json` (schema v1) for every command, shell completions, man page.
- Static musl builds for x86_64 and aarch64; MSRV 1.75 (Ubuntu 24.04 system toolchain).
