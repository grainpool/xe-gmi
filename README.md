# xe-gmi

Command-line management for Intel GPUs on the open-source `xe` kernel driver (Arc Pro B-series,
Arc B-series "Battlemage", and Arc A-series when bound to xe): status, full reports, scriptable
queries, per-process usage, power-limit and clock control, and boot persistence of those controls.
One static binary, no daemon, no libraries; it talks to the kernel through `/sys` and `/proc` only.

```
$ xe-gmi
xe-gmi 0.2.2 | driver xe | kernel 7.1.4-200.fc43.x86_64 | 2026-09-14T10:22:31Z
+-----+------------------------------+--------------+----------+------------------+----------------+
| Idx | Device                       | Bus Id       | Temp C   | Power W          | Memory MiB     |
| GT  | Clock MHz                    | Profile      | Util %   | Throttle         | Fan RPM        |
+=====+==============================+==============+==========+==================+================+
| 0   | Battlemage G31 [Arc Pro B65] | 0000:e3:00.0 | 41 / 38  | 12.40 / 200.00   | 1088 / 32768   |
| gt0 | 1200 / 2850                  | base         | 5.0      | none             | 1200           |
| gt1 | 300 / 2400                   | base         | 0.0      | none             |                |
+-----+------------------------------+--------------+----------+------------------+----------------+
Temp = pkg / vram; Power = draw / limit; Memory = used / total; Clock = cur / max.
Memory used = resident VRAM summed over visible DRM clients (run as root to see every user).
```

## Install

* Release tarball (static, x86_64 or aarch64): download from
  [the GitHub releases page](https://github.com/grainpool/xe-gmi/releases), verify
  `SHA256SUMS`, then `sudo install -m 0755 xe-gmi /usr/local/bin/xe-gmi`. The tarball also contains
  the man page and shell completions.
* From crates.io: `cargo install --locked xe-gmi`.
* From source with the distribution's Rust (Ubuntu 24.04's `cargo 1.75` included):
  `cargo build --release --locked` — `Cargo.lock` is kept in the version-3 format for that reason.

## Quick start

```
xe-gmi                                  # status table, every xe device
xe-gmi -u 10                            # refresh every 10 s
xe-gmi info --section power,clocks      # full report, selected sections
xe-gmi query --fields=temp.pkg,power.draw,memory.used --no-header -u 1
xe-gmi processes --sort util            # per-process engine utilization and resident VRAM
xe-gmi fields                           # every query field with units and availability here
sudo xe-gmi set power-limit 150W        # written, read back, clamps reported
sudo xe-gmi set clocks --max 2000       # every GT; add --gt N for one
sudo xe-gmi set power-profile power-saving
sudo xe-gmi reset all                   # back to the recorded boot defaults
sudo xe-gmi persist install             # re-apply settings at every driver bind (boot)
xe-gmi doctor                           # why isn't my card showing up?
xe-gmi --json info                      # JSON, schema version 1
xe-gmi topology                         # NUMA, CPUs, IOMMU groups, PCI paths, GPU affinity
xe-gmi pcie                             # link state and AER error statistics (endpoint, root port)
xe-gmi crash list                       # pending GPU crash dumps (devcoredump)
xe-gmi crash show 1                     # print a crash dump
xe-gmi cgroups                          # GPU memory per cgroup (dmem controller)
xe-gmi sriov status                     # SR-IOV PF state and per-VF provisioning
xe-gmi processes --group-by cgroup      # aggregate by client, cgroup or user
xe-gmi pmon -u 2                        # processes refreshing every 2 s
sudo xe-gmi set power-window 28s        # power-limit averaging window (read back, rounding reported)
sudo xe-gmi crash release 1             # release a crash dump (frees the kernel's copy)
xe-gmi doctor --bundle /tmp/xe-report   # redacted diagnostic bundle (directory)
xe-gmi firmware                         # GuC / HuC versions (DRM query, kernel 6.9+)
xe-gmi topology --hardware              # engine, GT and EU topology from the kernel
sudo xe-gmi ras                         # RAS error counters (kernel 7.2+, root); --clear resets
sudo xe-gmi events                      # live wedged/bind/unbind/devcoredump uevents (root)
sudo xe-gmi cgroup set /system.slice/llama.service --vram-max 8G
sudo xe-gmi sriov enable 4              # writes sriov_numvfs (never persisted)
sudo xe-gmi sriov vf 1 set --vram 8G --priority normal
sudo xe-gmi sriov vf 1 stop --force     # interrupts the VF's user
xe-gmi recover --dry-run                # print the recovery plan, refuse if preconditions fail
sudo xe-gmi recover --force             # crash save, unbind, reset (flr), rebind, re-apply
```

## Provenance

Every query field carries its provenance: where the value comes from (table, hwmon, sysfs, fdinfo,
computed, ...), its scope (card, GT, visible clients, ...), the access that was needed and the
quality of the number (authoritative, derived, partial, visible-only). `xe-gmi fields` prints the
SOURCE and ACCESS columns; `xe-gmi -v info` appends the provenance to N/A explanations and to the
memory `Used` line, and every `unavailable` entry in JSON output names its source. Values computed
from sampling (utilization, power draw) are labelled as derived, and client-visible totals
(`memory.used`, `processes.count`) are labelled `partial` for non-root users.

`recover` is the one multi-step control operation: it saves and releases any pending
crash dump, unbinds the driver, performs a PCI reset (flr by default) and rebinds. It refuses
unless the method is advertised, no VFs are enabled, and (without `--force`) no clients or active
connectors are on the device; a `bus` reset additionally requires a quiet bus. xe has no PCI error
handlers, so a wedged device may not survive the reset — read `docs/hardware-safety.md` before
using it on a production machine.

## Privileges

Everything that reads works as any user. `processes` and the `memory.used` / engine-utilization
figures cover the DRM clients you are allowed to inspect (your own processes; all of them as root).
`set`, `reset` and `persist` write to sysfs and therefore need root; run them under `sudo`. On a
permission error xe-gmi prints the exact `sudo` command line to retry (exit code 4).

## What can be controlled, and how safely

xe-gmi writes exactly five kinds of kernel attributes: the sustained/burst power limits
(`power*_max`, `power*_cap`), their averaging windows (`power*_max_interval`, `power*_cap_interval`),
GT frequency bounds (`min_freq`, `max_freq`), the firmware power profile (`power_profile`), and the
`data` file of a crash dump when you ask `crash release` to free it. The first four are volatile
requests to firmware that already enforces its own limits: values above the firmware default are
clamped by the driver (xe-gmi tells you), clocks
outside the hardware range are refused before anything is written, and a reboot or
`xe-gmi reset all` restores the defaults. Critical limits, PCI topology, firmware, debugfs and other
vendors' GPUs are never touched. See `docs/capability-ledger.md` for everything the kernel does and
does not expose.

## Persistence

`sudo xe-gmi persist install` installs one udev rule (`/etc/udev/rules.d/90-xe-gmi-persist.rules`)
that runs `xe-gmi persist apply` whenever the xe driver binds a device, plus a state file
(`/etc/xe-gmi/persist.conf`). After that every `set` is remembered and every `reset` forgets; use
`--no-persist` to change a value for this boot only. `xe-gmi persist show` displays the state and
the last apply log; `sudo xe-gmi persist remove --purge` removes everything. Details: `docs/persistence.md`.

## Kernel and distribution support

| Distro / kernel | Arc Pro B65 (`8086:e222`) | Arc B580/B570 | Notes |
|---|---|---|---|
| Ubuntu 24.04 GA (6.8) | not recognised — `doctor` explains | not recognised | install `linux-generic-hwe-24.04` |
| Ubuntu 24.04 HWE 6.14 | not recognised | works; no temperatures, no writable power limit | |
| Ubuntu 24.04 HWE 6.17 / 7.0, Ubuntu 26.04 (7.0) | full | full | |
| Fedora 43 (7.1), Fedora 44 | full | full | |
| Debian 13 (6.12) | needs `linux-image-amd64` from trixie-backports | works; reduced sensors | |
| Arc A-series (DG2) | on xe only with `xe.force_probe=<id> i915.force_probe=!<id>` (every kernel through 7.2) | | |

Feature availability by kernel version is listed in `docs/compat.md`; `xe-gmi doctor` reports what
this machine offers.

## Troubleshooting

* **`no xe device found`** — run `xe-gmi doctor`: it lists every GPU on the bus, which driver holds it,
  and what to do (newer kernel, `force_probe`, load the module).
* **Values print `N/A`** — the kernel does not expose that datum on this card/kernel. `xe-gmi -v info`
  prints the reason next to every N/A.
* **Power draw or utilization is `N/A` on the first line of a loop** — rate fields need two samples;
  from the second frame on they are populated. Single-shot commands wait one sampling window
  (`--sample-ms`, default 1000 ms).
* **`set to 200.00 W (requested 250.00 W; clamped …)`** — the driver clamps to the firmware default
  on Battlemage; the reported value is what the card enforces.
* **`permission denied`** — rerun with the printed `sudo …` line.

## JSON

Add `--json` to any command. The schema (`schema/xe-gmi-v1.schema.json`) is versioned by
`schema_version`; changes within version 1 are additive. See `docs/json-schema.md`.

## Building and testing

`cargo test` runs the suite against synthetic sysfs/procfs trees in `fixtures/`; no hardware or root
is needed. `scripts/ci-local.sh` replicates CI (MSRV 1.75 and stable, clippy, fmt, static musl build,
publish dry-run); it needs the `stable` and `1.75.0` rustup toolchains, which
`scripts/bootstrap-toolchain.sh` installs privately under `.toolchains/`.

### Testing against your own card

1. `scripts/capture-fixtures.sh` takes a read-only snapshot of your machine's `/sys` and `/proc`
   (xe devices in full; other GPUs contribute identity only) into `fixtures/captured/<host>-<kernel>/`.
   No root needed; root additionally captures other users' GPU processes. Captures contain your
   hostname, kernel release and GPU process names, so they are gitignored — never commit one.
2. `cargo test --test cli_captured` runs every read-only command against the capture (it skips when
   no capture exists, which is how public CI runs).
3. To exercise the control paths (power limits, clocks, profile, persistence) on real hardware,
   read `verify/README.md` and run `sudo verify/run-verify.sh`; it snapshots and restores every value
   it touches. `docs/hardware-safety.md` lists what the tool will and will not write.

## License and attribution

Copyright © 2026 Grainpool Holdings LLC (grainpoolholdings.com). Licensed under either of Apache
License 2.0 or MIT license at your option. Kernel interface facts were verified against the Linux
sources (`drivers/gpu/drm/xe`, tags v6.8–v7.2).
