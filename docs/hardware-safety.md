# Hardware safety

xe-gmi writes to exactly five kinds of kernel attributes and nothing else. This page lists them,
what the kernel does with them, everything the tool deliberately never touches, and how to undo any
change.

## What xe-gmi writes, and why it cannot damage the card

| Attribute | Effect | Persistence | Guard in the kernel |
|---|---|---|---|
| `hwmon/hwmon*/power{1,2}_max`, `power{1,2}_cap` | sustained (PL1) / burst (PL2) power limit | volatile; the firmware default returns at every driver probe | clamped to the firmware default on Battlemage (mailbox) or to the SKU range on DG2; `0` may be refused |
| `tile*/gt*/freq0/min_freq`, `max_freq` | GT frequency bounds | volatile | `EINVAL` outside `[rpn_freq, rp0_freq]` |
| `tile*/gt*/freq0/power_profile` | firmware power profile | volatile | only `base` / `power_saving` are accepted |
| `hwmon/hwmon*/power{1,2}_max_interval`, `power{1,2}_cap_interval` | power-limit averaging window (ms) | volatile; persisted like the limits | `EINVAL` above the register maximum (~28 s); the firmware rounds to a representable value, which xe-gmi reports |
| `class/devcoredump/devcd*/data` (write) | releases one GPU crash dump | not applicable (kernel state) | any write releases the dump; xe-gmi writes `1`, the only content it ever sends there |

These are runtime requests to firmware that already enforces its own limits. The worst outcome of
a wrong value is a slower or throttled GPU until `xe-gmi reset all`, a driver reload, or a reboot.
There is no path from these files to firmware images, fuses, VBIOS or PCI configuration.

The allowlist is the only place these paths are spelled out (`src/write.rs`): `allowed()` covers
the device tree above plus `reset`, `reset_method`, `sriov_numvfs` and the `sriov_admin/` attributes
(0.2.0); `allowed_system()` covers exactly `bus/pci/drivers/xe/bind` and `unbind` (recover only);
the coredump release and the cgroup `dmem.{max,min,low}` files are gated by their own exact path
shapes. Control writes go through `write_verified` (write, read back, report rounding); genuinely
write-only attributes (`reset`, `stop`, `.bulk_profile/*`, `bind`, `unbind`, coredump `data`) go
through `write_fire` — same allowlist, same error mapping, no readback. Unit tests walk a fixture
device tree and assert that exactly the exposed writable files pass, and that the system and cgroup
helpers reject anything outside their shapes.

| Attribute | Effect | Persistence | Guard in the kernel |
|---|---|---|---|
| `sriov_numvfs` | enable/disable the VFs of a PF | volatile (never persisted) | the kernel refuses while VFs are in use; xe-gmi refuses to enable over enabled VFs |
| `sriov_admin/vf*/profile/*`, `sriov_admin/.bulk_profile/*` | VF scheduling provisioning | volatile (never persisted) | kernel validates ranges and the profile state |
| `sriov_admin/vf*/stop` | stops one VF's context | volatile | requires `--force`: this interrupts the VF's user |
| `reset_method` + `reset` | selects and performs the PCI function-level (or bus) reset inside `recover` | not applicable | the kernel only accepts advertised methods; a bus reset needs a quiet bus (checked first) |
| `bus/pci/drivers/xe/bind`, `unbind` | driver detach/attach, **only as steps of `recover`** | not applicable | kernel driver core |
| `cgroup/<path>/dmem.max` (and `min`/`low` shapes) | VRAM limit of one cgroup | volatile (never persisted) | dmem controller; the kernel clamps |

0.2.0 also *reads* kernel interfaces other CLIs touch with ioctls: DRM queries go through the one
allow-`unsafe` module (`src/kabi`, docs/kabi.md); they never write the render node.

## What xe-gmi never does

* No firmware operations of any kind.
* No PCI topology operations: it never writes `remove`, `rescan`, `enable`, `reset_subordinate`,
  `resource*_resize` or `driver_override`. The only PCI writes are the selected device's own
  `reset`/`reset_method` (never `reset_subordinate`) and the xe driver's own `bind`/`unbind` — and
  only as preconditions-checked steps of `recover`.
* Nothing under `/sys/kernel/debug` (xe's debugfs can force wedged mode and GT resets).
* No module operations: no `modprobe`, no `force_probe` changes, no writes under
  `/sys/module/*/parameters`. `doctor` explains `force_probe`; it never sets it.
* No runtime-PM writes: `power/control`, `power/autosuspend_delay_ms`, `vram_d3cold_threshold`.
* No writes to `power*_crit` or `curr*_crit`.
* Never writes `0` to a power limit.
* Never writes a clock outside `[rpn_freq, rp0_freq]` (checked before writing, then the kernel
  checks again).
* Never opens anything belonging to another GPU: no `/dev/nvidia*`, no DRM node of a non-xe card,
  no hwmon of another vendor. The only contact with other GPUs is reading `vendor`, `device`,
  `class` and the `driver` symlink name under `/sys/bus/pci/devices/*`, for exclusion and for
  `doctor`'s listing.

## Device recovery (`recover`)

`recover` is the one multi-step operation: save any pending crash dump to
`/var/lib/xe-gmi/crash/` (0600) and release it, unbind xe, reset (flr or bus), rebind, and
re-apply persisted settings. Before touching anything it prints a plan and refuses:

* hard (refused even with `--force`): the method is not advertised in `reset_method`; VFs are
  enabled (disable them first); the `bus` method with other functions on the bus — a bus reset
  would hit them;
* soft (need `--force`): visible DRM clients on the device, enabled connectors, a pending crash
  dump without `--no-save-crash`.

A failed step stops the sequence and says which; there is deliberately no automatic rebind after a
failed reset — the message tells you to finish with `recover --method rebind`. Note that xe has no
PCI error handlers: a device stuck wedged may not survive to rebind, and a bus reset affects every
function on that bus. That is why `--method bus` is never the default and why the plan is printed
even with `--dry-run`.

## SR-IOV and cgroups

`sriov enable/disable`, `sriov vf N set` and `sriov vf N stop --force` are never persisted: VF
provisioning and `stop` are disruptive (stop interrupts the VF's user, which is why `--force` is
required), and re-applying them at boot against a different VF layout would be worse than silence.
`cgroup set` writes `dmem.max` for the selected device's region only (other regions' lines are
preserved); the dmem controller clamps, and xe-gmi reports the read-back value when the kernel
rounds. None of these touch `sriov_drivers_autoprobe` or the VF bind state.

## Undoing changes

* `sudo xe-gmi reset all` restores the boot defaults recorded before the first control write of
  the current boot (`/var/lib/xe-gmi/boot/<boot_id>/<pci>.defaults`).
* If persistence was installed, `sudo xe-gmi persist remove --purge` removes the udev rule and the
  state file; by hand: delete `/etc/udev/rules.d/90-xe-gmi-persist.rules` and `/etc/xe-gmi/persist.conf`.
* After a `verify/run-verify.sh` run, `sudo verify/results/<timestamp>/restore.sh` re-applies the
  values snapshotted before that run, without needing the xe-gmi binary.
* A reboot restores everything: nothing xe-gmi does survives a driver probe unless the persistence
  rule re-applies it, and that rule only ever writes the same four attribute families.
