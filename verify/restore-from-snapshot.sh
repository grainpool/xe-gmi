#!/usr/bin/env bash
# Standalone restore: writes the values recorded in a snapshot.txt (see run-verify.sh) back to sysfs.
# Usage: sudo verify/restore-from-snapshot.sh verify/results/<ts>/snapshot.txt
# Only the four writable attribute families are touched; clocks are bounds-checked against
# rpn_freq/rp0_freq; 0 is never written. The kernel does not enforce min_freq <= max_freq, so the
# order of the two clock writes does not matter for acceptance.
set -u
snap="${1:?snapshot.txt path}"
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
rc=0
while IFS=' ' read -r path value; do
  [ -n "$path" ] || continue
  case "$path" in \#*) continue;; esac
  base="$(basename "$path")"
  case "$base" in
    power[12]_max|power[12]_cap|min_freq|max_freq|power_profile) ;;
    *) continue;;
  esac
  if [ "$value" = "0" ] || [ -z "$value" ]; then echo "SKIP $path: refusing to write '$value'" >&2; continue; fi
  if [ "$base" = min_freq ] || [ "$base" = max_freq ]; then
    d="$(dirname "$path")"; rpn="$(cat "$d/rpn_freq")"; rp0="$(cat "$d/rp0_freq")"
    if [ "$value" -lt "$rpn" ] || [ "$value" -gt "$rp0" ]; then echo "SKIP $path: $value outside [$rpn,$rp0]" >&2; continue; fi
  fi
  if [ "$base" = power_profile ]; then value="$(echo "$value" | tr -d '[]' | awk '{print $1}')"; fi
  if printf '%s\n' "$value" > "$path" 2>/dev/null; then
    echo "restored $path = $value (reads $(tr -d '\n' < "$path"))"
  else
    echo "FAILED $path = $value" >&2; rc=1
  fi
done < "$snap"
exit $rc
