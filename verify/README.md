# Hardware verification (manual, requires root)

`run-verify.sh` exercises every control path of xe-gmi on the real card, under sudo, with the
failsafes listed below: one PCI device only, values never beyond firmware defaults, everything
snapshotted beforehand and restorable afterwards. Run it yourself when you want proof the control
paths work on your hardware.

## Run

```
cp verify/verify.conf.example verify/verify.conf     # edit PCI= and XE_GMI= if needed
cargo build --release
sudo verify/run-verify.sh
```

Duration: about a minute. Output: `verify/results/<UTC timestamp>/`.

## What it does, in order

1. Refuses to start unless `PCI` is bound to `xe` and the binary runs. Records kernel, distro,
   binary hash, boot id, uptime, dmesg.
2. Snapshots every writable attribute (power limits, per-GT min/max, power profile) to
   `snapshot.txt` and writes `restore.sh`. A `trap` restores and re-verifies the snapshot on exit,
   including on error or Ctrl-C.
3. Read-only commands as root, including a `query` over every field the binary lists, and a check
   that no output ever mentions the other GPUs.
4. Power limit: lower by `POWER_DELTA_W` (never below half the boot value) → readback → `reset`
   → readback equals boot value; boot-default record checked; clamp probe (`boot + 50 W` must read
   back as the boot value with a "clamped" message); `set power-limit 0` must be refused before any
   write.
5. Clocks, per GT: lower `max_freq` by `CLOCK_DELTA_MHZ` → readback → `reset clocks` → boot values;
   an out-of-range request must be refused before writing.
6. Power profile (6.18+): switch, verify, reset.
7. Persistence: install rule + state (copies the binary to `PERSIST_EXE` if that path is empty),
   sticky `set`, `persist apply --udev-devpath` exactly as udev calls it, log check, `reset`,
   `persist remove --purge` (unless `KEEP_PERSIST=1`).
8. Unprivileged runs as `$SUDO_USER`: `status` works, `set` exits 4 with the sudo hint and changes
   nothing.
9. Sampling sanity: power draw and VRAM total are plausible numbers.

Every step leaves `NN-name.cmd/.stdout/.stderr/.exit/.ms/.before/.after`; `results.md` is the
human-readable summary with PASS/FAIL/SKIP and expected-vs-observed per step; `results.json` is the same for
tools; `dmesg-delta.txt` holds kernel messages emitted during the run; `observed-clocks.txt` and
`observed-sample.txt` hold the measured boot values the docs need.

## If something goes wrong

* The script aborts on suspicious dmesg lines (GPU hang/reset) and restores immediately.
* `sudo verify/results/<ts>/restore.sh` re-applies the snapshot at any later time.
* `sudo xe-gmi reset all` restores the recorded boot defaults; a reboot restores everything.
* Nothing the script does survives a reboot unless `KEEP_PERSIST=1` was set.

## Reviewing results

`verify/results/<ts>/results.md` lists every step with expected vs observed values; each step also
has `.cmd/.stdout/.stderr/.exit/.ms/.before/.after` files. For a FAIL, compare those against the
control messages in the README and the kernel interface notes in `docs/abi-notes.md` — the kernel
is the arbiter. The boot values in `snapshot.txt` are the ground truth to record under "Observed on
hardware" in `docs/abi-notes.md`; `restore.sh` puts everything back.

## Extended feature verification — `run-verify-ext.sh`

`run-verify-ext.sh` exercises the 0.2.0 surface (kernel memory/firmware/hardware-topology reads,
events, cgroup limits, SR-IOV, device recovery) plus the read paths of the half-A features, and
includes unprivileged steps so a regression that only bites non-root users cannot pass. Flags:
`--yes` (no pauses), `--pci <busid>` (pick the card; prefer one no display is attached to),
`--force-recover` (run the recovery sequence even with DRM clients visible — the guarded refusal
is recorded first either way). It writes `verify/results-<date>.md` and `verify/events-<date>.log`
in a single file each; these are **local hardware evidence by design** — gitignored, never
committed, the repository ships the harness and not someone's machine.

Run it from a virtual terminal (Ctrl-Alt-F2): `sudo verify/run-verify-ext.sh --yes --pci <busid>`.
On a desktop machine the run itself can outlive your session — enabling an SR-IOV VF hotplugs a
render node some compositors crash on — so on remote or desktop rigs use the transient system-unit
recipe in the script's header; on SELinux-enforcing distributions a system unit cannot execute
scripts from `$HOME`, which is why the recipe `install`s copies under `/usr/local` first (remove
them afterwards; the recipe explains the reasoning end to end).
