# Persistence

Kernel control values (power limits, frequency bounds, power profile) are volatile: the driver
restores firmware defaults at every probe — boot, module reload. xe-gmi persists them with one udev
rule that re-applies the saved settings whenever the `xe` driver binds a PCI device
(verified against kernel v6.8–v7.2 sources; the bind ordering was verified on 7.1.13 hardware).

## What is installed

`sudo xe-gmi persist install` writes:

| File | Default path | Purpose |
|---|---|---|
| udev rule | `/etc/udev/rules.d/90-xe-gmi-persist.rules` | runs `xe-gmi persist apply --udev-devpath=%p` on `ACTION=="bind"`, `SUBSYSTEM=="pci"`, `ENV{DRIVER}=="xe"` |
| state file | `/etc/xe-gmi/persist.conf` | the saved settings (format below) |
| apply log | `/var/lib/xe-gmi/persist.log` | one line per applied setting, per run |

The rule fires on **bind**, which the kernel emits after `probe()` returned — every hwmon and
GT attribute exists by then. The program runs as root inside udev's sandbox and finishes in
milliseconds. `install` reloads udev rules (`udevadm control --reload-rules`) but never triggers a
bind event, so nothing changes until the next boot or driver reload. The rule's program path must
be under `/usr/` or `/opt/` (home directories may not be mounted at boot);
`persist install --exe PATH` overrides it.

After installing, every successful `sudo xe-gmi set …` records its value in the state file
(`persistence: recorded in …`), and every `sudo xe-gmi reset …` forgets the keys it reset
(`persistence: removed from …`). `--no-persist` records nothing; `--persist` when the rule is not
installed exits 5 without touching the control.

## The state file

```
# xe-gmi persist.conf format 1. Managed by `xe-gmi persist`; one setting per line: <pci> <key> <raw value>
format=1
0000:e3:00.0 pl2.card 150000000
0000:e3:00.0 gt0.max_freq 2000
0000:e3:00.0 gt1.power_profile power_saving
```

| Key | Attribute | Value |
|---|---|---|
| `pl1.card` / `pl1.pkg` | `power1_max` / `power2_max` | µW |
| `pl2.card` / `pl2.pkg` | `power1_cap` / `power2_cap` | µW |
| `gt<G>.min_freq` / `gt<G>.max_freq` | `tile*/gt<G>/freq0/{min,max}_freq` | MHz |
| `gt<G>.power_profile` | `tile*/gt<G>/freq0/power_profile` | `base` \| `power_saving` |

The file is rewritten atomically (temp file + rename, 0644).

## Hand-editing

You can add or edit lines by hand — same three-column shape, `key` from the table above. `apply`
skips lines it cannot honour and says why: unknown device (`skipped (device not present)`), an
attribute the current kernel does not expose (`skipped (attribute not exposed)`), or a clock value
outside the GT's `[rpn_freq, rp0_freq]` range. Comment lines and unknown keys are preserved
untouched through upserts and removals. Every apply run appends to
`/var/lib/xe-gmi/persist.log`; `xe-gmi persist show` prints the entries and the last run's results.

```
$ xe-gmi persist show
Rule file            : /etc/udev/rules.d/90-xe-gmi-persist.rules (installed)
State file           : /etc/xe-gmi/persist.conf (3 entries)
  0000:e3:00.0 pl2.card 150000000
  0000:e3:00.0 gt0.max_freq 2000
  0000:e3:00.0 gt1.power_profile power_saving
Last apply           : 2026-09-14T10:22:31Z (3 ok, 0 skipped, 0 failed)
  2026-09-14T10:22:31Z 0000:e3:00.0 pl2.card 150000000 ok
  2026-09-14T10:22:31Z 0000:e3:00.0 gt0.max_freq 2000 ok
  2026-09-14T10:22:31Z 0000:e3:00.0 gt1.power_profile power_saving ok
```

## Removing

```
sudo xe-gmi persist remove            # deletes the rule, reloads udev; state file and log stay
sudo xe-gmi persist remove --purge    # also deletes /etc/xe-gmi/persist.conf
```

Removing persistence does not change current control values; use `sudo xe-gmi reset all` for that.
