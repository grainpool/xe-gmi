# Kernel interface notes

xe-gmi reads and writes the sysfs and procfs files documented here. Every statement names the
kernel version it was verified on (sources at tags v6.8–v7.2, plus the hardware run below).

## Paths

Everything hangs off the PCI device directory (`/sys/devices/pci…/DDDD:BB:DD.F/`, found through
`/sys/class/drm/card*` → `device`, kept only when `uevent` says `DRIVER=xe`):

| Area | Path | xe-gmi uses it for |
|---|---|---|
| hwmon | `hwmon/hwmon<K>/` (with `name` == `xe`) | power, energy, temperature, fan |
| GT clocks | `tile<T>/gt<G>/freq0/` | clock report and control |
| Throttle | `tile<T>/gt<G>/freq0/throttle/` | throttle status and reasons |
| Idle | `tile<T>/gt<G>/gtidle/` | utilization, idle status |
| VRAM BAR | `resource` | VRAM total (see below) |
| PCI link | `current_link_speed`, `max_link_speed`, `current_link_width`, `max_link_width` | `pci.link.*` |
| Process stats | `/proc/<pid>/fdinfo/<fd>` (+ `/proc/<pid>/comm`, `/proc/uptime`) | `processes` |

`gt<G>` uses the **global** GT id, not per-tile numbering: on Battlemage tile0 carries gt0
(render/compute) and gt1 (media); a second tile would continue with gt2, gt3. The GT kind comes
from `gtidle/name` (`gt0-rc` render, `gt1-mc` media). Verified 6.8–v7.2.

## Units

| Domain | Unit in sysfs | Note |
|---|---|---|
| Power | **µW** | `power1_*`, `power2_*` |
| Energy | **µJ** | `energy1_input`, `energy2_input`, 64-bit accumulated, driver-corrected wrap |
| Temperature | **millidegree C** | all `temp*` |
| Voltage / current | mV / mA | `in*_input`, `curr*_crit` |
| Windows | ms | `*_interval` |
| Frequencies | MHz | all `freq0/` files |
| Idle residency | ms | `idle_residency_ms`, 64-bit, wrap-corrected |

hwmon channel 1 is labelled `card`, channel 2 `pkg` (`power<N>_label`, `energy<N>_label`, …).
`xe` never creates `temp1` (the "card" channel); package temperature is `temp2_input`, VRAM is
`temp3_input`, first seen in **6.15**.

## Power limits and the driver clamp

| Attribute | Meaning | First kernel |
|---|---|---|
| `power1_max` / `power2_max` | PL1 sustained limit (channel = card / pkg), visible only when firmware has PL1 enabled; reading `0` means disabled | 6.8 (pkg 6.10) |
| `power1_cap` / `power2_cap` | PL2 burst limit | **6.17** (the ABI document says 6.15; the code landed in 6.17) |
| `power<N>_max_interval` / `_cap_interval` | PL1 / PL2 window (Tau), ms | 6.8 / 6.17 |
| `power1_rated_max` | default TDP — **never visible on mailbox platforms (Battlemage)**, present on DG2 when the power SKU is valid | 6.8 |
| `power1_crit` / `curr1_crit` | critical (I1) limit — read-only in xe-gmi | 6.8 |

Writing a limit (verified in `xe_hwmon_power_max_write` at v7.2):

1. Writing `0` attempts to *disable* the limit; hardware that keeps it enabled returns
   `EOPNOTSUPP` (xe-gmi exit 5).
2. Above the register's representable maximum the driver clamps to it (a `drm_info` line lands in
   dmesg).
3. **Mailbox platforms** (all Battlemage since 6.16, identified by PCI device id — the flag itself
   is not exposed to userspace) additionally clamp silently to the **firmware default seen at
   boot**. The write still succeeds; **only the readback reveals the clamp**. Values below the
   default are accepted as-is. This is why `set power-limit 250W` on a 200 W B65 prints
   `set to 200.00 W (requested 250.00 W; clamped by the driver to the firmware maximum)`.
4. DG2 (MMIO model) clamps in hardware to the SKU min/max; reads show the clamped value. xe-gmi's
   reset therefore writes `power<N>_rated_max` instead of the sentinel below.

Because the firmware default is invisible on mailbox platforms, `reset power-limit` writes the
sentinel `2000000000` µW, reads the clamped result back, and reports it as the firmware default
(6.16+ behaviour, verified v7.2).

## Clocks and the Battlemage 1200 MHz default

`tile<T>/gt<G>/freq0/`: `act_freq` (0 while the GT is in C6), `cur_freq` (SLPC request),
`rpn_freq`/`rpe_freq`/`rp0_freq` (hardware min / efficient / max) since 6.8; `rpa_freq`
("achievable") since **6.14**; `min_freq`/`max_freq` RW since 6.8; `power_profile` RW since
**6.18** (reads `base    [power_saving]` with the selected token bracketed; write the bare token).

Writing `min_freq`/`max_freq` outside `[rpn_freq, rp0_freq]` returns **EINVAL**; the driver does
**not** enforce `min ≤ max` (SLPC resolves it), so xe-gmi validates the pair before writing
anything.

Boot defaults set by GuC PC init (`pc_adjust_freq_bounds`, v7.2): `max_freq` = RP0; `min_freq` =
RPn on client parts but **raised to 1200 MHz on the Battlemage graphics GT** (`BMG_MIN_FREQ`,
6.16+; G21 media GT too under a workaround). A boot record of the actual values is therefore part
of `set`/`reset` — on the captured B65 (7.1.13) gt0 boots at min 1200 / max 2400
while `rpn_freq` is 400. `power_profile` boots to `base`. All values are volatile: driver probe
restores them.

## Throttle, idle and utilization

`freq0/throttle/status` is `1` while any reason bit is set; the individual `reason_pl1`,
`reason_pl2`, `reason_pl4`, `reason_thermal`, `reason_prochot`, `reason_ratl`,
`reason_vr_thermalert`, `reason_vr_tdc` files (6.8) carry the bits. A joined `reasons` file
(`none` or space-separated names) exists from **6.19**; xe-gmi reads it when present and otherwise
derives the list from the `reason_*` files in canonical order. Crescent Island's extra reason
files are parsed generically (6.19+; not a product target here).

Utilization is GT **activity**, not engine saturation:
`100 × (1 − Δidle_residency_ms / Δt_ms)` from `gtidle/idle_residency_ms` (6.8+), clamped to
[0, 100]. It needs two samples — every rate prints N/A on the first frame of a loop, single-shot
commands wait one sampling window.

## VRAM

No kernel tag ever exposed a VRAM size attribute, so xe-gmi derives the total from the BAR: `xe`
resizes the VRAM BAR (BAR 2) to the full size at probe when the platform allows ("Small BAR
device" in dmesg when not). The largest prefetchable memory BAR in `resource` is taken as the
total when it is ≥ 1 GiB (source `bar`), otherwise a SKU table is used (source `table`: `e20b`
12 GiB, `e20c` 10 GiB, `e222` 32 GiB, `56a0` 16 GiB). Otherwise N/A. `tile*/memory/freq0/` exists
only on PVC, so memory clocks are always N/A on Battlemage/DG2. `vram_d3cold_threshold` is shown
read-only in `info -v`. Verified 6.8–v7.2.

## Process statistics (fdinfo)

`/proc/<pid>/fdinfo/<fd>` for fds whose link target starts with `/dev/dri/` and whose content says
`drm-driver: xe`. Client identity is (`drm-pdev`, `drm-client-id`). Memory stats since 6.8:
`drm-resident-vram0/vram1` (bytes, printed as `N`, `N KiB` or `N MiB` when evenly divisible —
there is no `GiB` unit, 1 GiB prints as `1024 MiB`). Engine utilization since **6.11**:
`100 × Δdrm-cycles-<class> / Δdrm-total-cycles-<class> / drm-engine-capacity-<class>`
(class ∈ rcs, ccs, bcs, vcs, vecs) — both counters live in the GPU clock domain, so no wall-clock
term is involved. Reading another user's fdinfo requires `PTRACE_MODE_READ_FSCREDS`: as root all
clients are visible, as a user only your own processes (this is why the tables say "run as root to
see every user"). A missing `drm-total-cycles-<class>` makes that class N/A on older kernels.

## PCI link

`current_link_speed`/`max_link_speed` contain e.g. `16.0 GT/s PCIe` (or `Unknown speed`);
`*_{current,max}_link_width` are integers. **Both current values drop while the link is in a
low-power state** — the captured B65 shows `2.5 GT/s` ×1 on a Gen5 x16 card; xe-gmi reports what
it reads.

## Write error codes you may see

| errno | Meaning | xe-gmi exit |
|---|---|---|
| EACCES / EPERM | not root | 4 (the `sudo …` retry line is printed) |
| EINVAL | frequency outside `[rpn, rp0]`, bad token, window too large | 6 |
| EOPNOTSUPP | writing `0` to a limit that cannot be disabled | 5 |
| ENOENT | attribute vanished (unbind/reload mid-run) | 5 |

## Things the kernel does not expose

Fan control or PWM (fan RPM is read-only, 6.16+), fan maximum, ECC status (despite Pro cards' ECC
memory), device-level VRAM used (ioctl only), instantaneous power (`power*_input` unimplemented —
draw is the energy delta over the sampling window), VRAM frequency on non-PVC, UUID/serial, an
engine-busy sysfs counter, a GPU-reset trigger (debugfs only, deliberately out of scope), and
`power*_rated_max` on Battlemage. `docs/capability-ledger.md` lists the consequences.

## Observed on hardware

Verified 2026-09-15 on the test machine (`0000:e3:00.0`, `8086:e222` Arc Pro B65, kernel
`7.1.13-100.fc43.x86_64`, `xe` module `live`) via `sudo verify/run-verify.sh`: results
`20260915T010500Z` — **69/69 checks pass**. (The first run, `20260915T003907Z`, showed one FAIL
from the script's own power_profile extractor — it compared the first token of the file instead
of the bracketed selection; the script now reads the bracketed token, as the kernel interface section above documents, and the step's
own `.after` capture had already shown the write working: `base    [power_saving]`.)

Boot defaults the driver set (snapshot, before any write):

| Attribute | Observed | Consistent with |
|---|---|---|
| `hwmon6/power1_cap` | 200000000 (PL2 = 200.00 W) | firmware default; `power1_max` **never appeared** — the mailbox SKU exposes no PL1 |
| `power1_crit` / `power1_cap_interval` | 400000000 / 15 ms | reference values |
| gt0 (`tile0/gt0`) | `min_freq` 1200, `max_freq` 2400; rpn 400, rpe 400, rpa 2400, rp0 2400 | `BMG_MIN_FREQ` 1200 on the graphics GT (not rpn 400); `max = rp0` |
| gt1 (`tile0/gt1`, media) | `min_freq` 400, `max_freq` 1500; rpn/rpe 400, rpa/rp0 1500 | media GT keeps `min = rpn` (the 1200 raise is graphics-GT-only on B65) |
| `power_profile` (both GTs) | `[base]    power_saving` | boots to `base` |

Control-path behaviour: `set power-limit 150W` wrote and read back 150000000; a clamp probe
(boot + 50 W) returned success but the readback **stayed at the boot value 200000000** — the
silent mailbox clamp documented above, confirmed; `reset` restored 200000000. `writing 0`
was refused with exit 2 before touching the attribute. Clocks: `--max` above rp0 refused before
writing; round trips and per-GT resets restored exactly 1200/2400 and 400/1500.
`set power-profile power-saving` took effect immediately (readback `[power_saving]` selected),
reset restored `[base]`. **dmesg stayed completely empty across every write** — the clamp is
silent on this platform, readback is the only signal, as documented above.

Telemetry at idle: `draw` 49.96–50.46 W over the sampling window, `util` 0.0, `act_freq` 0
(GT in C6) with `cur_freq` 1200/400, `temp` 41–44 °C, `fan1_input` 1085 RPM, VRAM
`used` 0 MiB. VRAM total read as **57344 MiB with source `bar`** — the platform resized the BAR
to a 56 GiB aperture, larger than the 32 GiB of the SKU table; xe-gmi reports the BAR when it is
≥ 1 GiB, which is exactly what happened (source `bar`, not `table`).
