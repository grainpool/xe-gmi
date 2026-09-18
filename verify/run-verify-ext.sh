#!/usr/bin/env bash
# Hardware verification for the 0.1.2 / 0.2.0 features. Human-run, on the rig, as root:
#   sudo verify/run-verify-ext.sh [--half-a] [--yes] [--force-recover] [--pci 0000:e3:00.0]
# --half-a         only the file-based checks (after the 0.1.2 tag)
# --yes            do not ask before the disruptive steps (VF enable, device recovery, cgroup subtree change)
# --force-recover  run the recover phase with --force even while DRM clients are visible (the plan's
#                  refusal is still recorded first). On a machine with a desktop session this is the
#                  only way to exercise the sequence; the FLR will kill the clients that hold the card.
# DESKTOP MACHINES: creating an SR-IOV VF hotplugs a render node GNOME may choke on (observed with
# mutter on 7.1.13: gnome-shell crash, GUI session restart, remote session drops) — and the FLR step
# kills whatever holds the card. Run as a transient SYSTEM unit so the run outlives the session:
#   sudo mkdir -p /usr/local/verify
#   sudo install -m0755 verify/run-verify-ext.sh /usr/local/bin/xe-gmi-verify.sh
#   sudo install -m0755 target/release/xe-gmi  /usr/local/bin/xe-gmi-verify-bin
#   sudo systemd-run --unit=xe-gmi-verify --collect \
#     --property=Environment=XE_GMI_BIN=/usr/local/bin/xe-gmi-verify-bin \
#     /usr/local/bin/xe-gmi-verify.sh --yes [--force-recover] --pci <busid>
# then watch /usr/local/verify/results-<date>.md (the script derives paths from its own location).
# The /usr/local copy is required: a systemd unit runs in the SELinux init_t domain, which is denied
# executing scripts under $HOME. NEVER background sudo with bound redirects
# (`sudo cmd </dev/null >/dev/null 2>&1 &`) — it starves the password prompt into echoing plaintext.
# Afterwards: sudo rm -rf /usr/local/verify /usr/local/bin/xe-gmi-verify*
# Writes verify/results-<date>.md. Every step records PASS/FAIL/SKIP with the command output, so the
# agent can act on the file without the human interpreting it. Touches only the selected xe device,
# a scratch cgroup, and (if needed) the dmem entry in /sys/fs/cgroup/cgroup.subtree_control.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
XE="${XE_GMI_BIN:-$ROOT/target/release/xe-gmi}"; [ -x "$XE" ] || XE="$(command -v xe-gmi || true)"
[ -x "$XE" ] || { echo "xe-gmi binary not found (build with cargo build --release)"; exit 1; }
[ "$(id -u)" -eq 0 ] || { echo "run as root"; exit 1; }
HALF_A=0; YES=0; FORCE_RECOVER=0; PCI=""
while [ $# -gt 0 ]; do case "$1" in --half-a) HALF_A=1;; --yes) YES=1;; --force-recover) FORCE_RECOVER=1;; --pci) PCI="$2"; shift;; *) echo "unknown arg $1"; exit 2;; esac; shift; done
[ -n "$PCI" ] || PCI="$(for d in /sys/bus/pci/drivers/xe/0000:*; do basename "$d"; done | head -1)"
[ -n "$PCI" ] && [ -e "/sys/bus/pci/devices/$PCI" ] || { echo "no xe device"; exit 1; }
OUT="$ROOT/verify/results-$(date -u +%Y-%m-%d).md"; : > "$OUT"
log() { printf '%s\n' "$*" >> "$OUT"; }
step() { # name, command...
  local name="$1"; shift; log ""; log "## $name"; log '```'; log "\$ $*"
  local o; o="$("$@" 2>&1)"; local rc=$?; log "$o"; log "(exit $rc)"; log '```'
  if [ $rc -eq 0 ]; then log "RESULT: PASS"; else log "RESULT: FAIL"; fi; return $rc
}
skip() { log ""; log "## $1"; log "RESULT: SKIP — $2"; }
confirm() { [ $YES -eq 1 ] && return 0; read -r -p "$1 [y/N] " a; [ "$a" = y ]; }
DEV="$(readlink -f "/sys/bus/pci/devices/$PCI")"
log "# xe-gmi verification $(date -u +%FT%TZ)"; log "kernel $(uname -r); device $PCI; xe-gmi $("$XE" --version 2>&1)"
IDX="$("$XE" list --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print([x["index"] for x in d["devices"] if x["pci_address"]==sys.argv[1]][0])' "$PCI" 2>/dev/null || echo 0)"
X="$XE -i $IDX"

# ---------- Half A ----------
step "topology" $X topology
step "topology --json" $X topology --json
step "pcie" $X pcie
step "crash list" $X crash list
step "cgroups" $X cgroups
step "sriov status" $X sriov status
step "processes --group-by client" $X processes --group-by client
step "processes --group-by cgroup" $X processes --group-by cgroup
step "fields provenance" $X fields
step "info (all sections)" $X info
step "doctor" $X doctor
HW="$(ls -d "$DEV"/hwmon/hwmon* | head -1)"
WIN=""; for f in power1_cap_interval power1_max_interval; do [ -e "$HW/$f" ] && { WIN="$HW/$f"; break; }; done
if [ -n "$WIN" ]; then
  ORIG="$(cat "$WIN")"
  step "power window: set 20 ms" $X set power-window 20ms --no-persist && step "power window: read back" cat "$WIN"
  step "power window: restore $ORIG ms" $X set power-window "${ORIG}ms" --no-persist
else skip "power window" "no *_interval attribute"; fi
BUNDLE="$(mktemp -d)"; step "doctor --bundle" $X doctor --bundle "$BUNDLE/b" && { step "bundle: no process names" bash -c "! grep -rE 'llama|Xwayland|$(hostname)' '$BUNDLE/b'"; }
rm -rf "$BUNDLE"
[ $HALF_A -eq 1 ] && { echo "results in $OUT"; exit 0; }

# ---------- Half B ----------
step "firmware" $X firmware
step "topology --hardware" $X topology --hardware
step "memory sources" $X query --fields memory.total,memory.used,memory.total.source,memory.used.source,memory.cpu_visible.total
if [ "$(uname -r | cut -d. -f1-2 | tr -d .)" -ge 72 ]; then step "ras" $X ras; else skip "ras" "kernel $(uname -r) < 7.2 (drm-ras family absent)"; fi
# cgroup set/unset on a scratch cgroup
CG=/sys/fs/cgroup; if [ -e $CG/dmem.capacity ]; then
  ADDED=0; grep -qw dmem $CG/cgroup.subtree_control || { confirm "enable dmem in $CG/cgroup.subtree_control for the test?" && echo +dmem > $CG/cgroup.subtree_control && ADDED=1; }
  mkdir -p $CG/xe-gmi-verify
  step "cgroup set 4G" $X cgroup set /xe-gmi-verify --vram-max 4G && step "cgroup show" $X cgroup show /xe-gmi-verify && step "cgroup set max" $X cgroup set /xe-gmi-verify --vram-max max
  rmdir $CG/xe-gmi-verify; [ $ADDED -eq 1 ] && echo -dmem > $CG/cgroup.subtree_control
else skip "cgroup set" "no dmem controller"; fi
# SR-IOV enable 1 / disable
if [ -e "$DEV/sriov_totalvfs" ] && [ "$(cat "$DEV/sriov_numvfs")" = 0 ]; then
  if confirm "enable 1 VF on $PCI and disable it again?"; then
    step "sriov vf 1 set (quantum 10ms)" $X sriov vf 1 set --quantum 10ms
    step "sriov enable 1" $X sriov enable 1 && step "virtfn0 present" test -L "$DEV/virtfn0" && step "sriov status (1 VF)" $X sriov status
    step "sriov disable" $X sriov disable
  else skip "sriov enable/disable" "declined"; fi
else skip "sriov enable/disable" "no PF capability or VFs already enabled"; fi
# recovery: needs no clients and no active connector (or --force-recover to override)
CLIENTS="$($X processes --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["processes"]))')"
ACTIVE="$(for c in /sys/class/drm/card*-*; do [ -e "$c/enabled" ] && [ "$(cat "$c/enabled")" = enabled ] && echo "$c"; done)"
step "recover --dry-run" $X recover --dry-run \
  || log "RESULT: NOTE — a refusal (exit 5) here is the correct behavior while DRM clients are visible"
RUN_RECOVER=0
if [ "$CLIENTS" = 0 ] && [ -z "$ACTIVE" ] && confirm "recover $PCI (rebind, then FLR)? No DRM clients and no active display detected."; then
  RUN_RECOVER=1
fi
if [ $FORCE_RECOVER -eq 1 ]; then RUN_RECOVER=1; FORCE="--force"; log ""; log "NOTE: --force-recover: running the sequence with $FORCE despite clients=$CLIENTS active='${ACTIVE}'"; else FORCE=""; fi
if [ $RUN_RECOVER -eq 1 ]; then
  "$XE" events > "$ROOT/verify/events-$(date -u +%Y-%m-%d).log" 2>&1 & EV=$!
  sleep 1
  step "recover --method rebind $FORCE" $X recover --method rebind $FORCE && step "status after rebind" $X status
  step "recover (flr) $FORCE" $X recover $FORCE && step "status after flr" $X status && step "persist show" $X persist show
  sleep 1; kill $EV 2>/dev/null; step "events captured" cat "$ROOT/verify/events-$(date -u +%Y-%m-%d).log"
else skip "recover" "clients=$CLIENTS active connectors='$ACTIVE' or declined"; fi
echo "results in $OUT"
