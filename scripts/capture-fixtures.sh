#!/usr/bin/env bash
# Read-only snapshot of the xe device(s) on this machine into fixtures/captured/<host>-<kernel>/.
# No root needed (root adds /proc fdinfo of other users and dmesg). Never writes to /sys or /proc.
# Copies only allowlisted files (see below); other GPUs contribute only vendor/device/class/driver
# for exclusion tests. Result is a tree usable through the XE_GMI_*_ROOT seams.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="${1:-$(hostname -s)-$(uname -r)}"
OUT="$ROOT/fixtures/captured/$NAME"
rm -rf "$OUT"; mkdir -p "$OUT/sys/class/drm" "$OUT/sys/bus/pci/devices" "$OUT/sys/bus/pci/drivers" "$OUT/proc/sys/kernel/random" "$OUT/etc" "$OUT/lib/modules"
rd() { timeout 2 cat "$1" 2>/dev/null || true; }   # some sysfs files block or are not readable
copy_file() { local src="$1" dst="$OUT/sys${1#/sys}"; mkdir -p "$(dirname "$dst")"; rd "$src" > "$dst"; }
copy_link() { local src="$1" dst="$OUT/sys${1#/sys}"; mkdir -p "$(dirname "$dst")"; ln -sfn "$(readlink "$src")" "$dst"; }

echo "capturing into $OUT"
cp /etc/os-release "$OUT/etc/os-release" 2>/dev/null || true
rd /proc/sys/kernel/osrelease > "$OUT/proc/sys/kernel/osrelease"
rd /proc/sys/kernel/random/boot_id > "$OUT/proc/sys/kernel/random/boot_id"
rd /proc/uptime > "$OUT/proc/uptime"
mkdir -p "$OUT/proc/self"; grep -E '^(Name|State|Pid|Uid|Gid):' /proc/self/status > "$OUT/proc/self/status"
for m in xe i915 nvidia; do [ -d /sys/module/$m ] && { mkdir -p "$OUT/sys/module/$m"; rd /sys/module/$m/initstate > "$OUT/sys/module/$m/initstate"; [ -d /sys/module/$m/parameters ] && { mkdir -p "$OUT/sys/module/$m/parameters"; rd /sys/module/$m/parameters/force_probe > "$OUT/sys/module/$m/parameters/force_probe"; }; }; done
K="$(uname -r)"; for f in /lib/modules/$K/kernel/drivers/gpu/drm/xe/xe.ko*; do [ -e "$f" ] && { mkdir -p "$OUT/lib/modules/$K/kernel/drivers/gpu/drm/xe"; : > "$OUT/lib/modules/$K/kernel/drivers/gpu/drm/xe/$(basename "$f")"; }; done

# --- every display-class PCI device: identity only ---
for d in /sys/bus/pci/devices/*; do
  cls="$(rd "$d/class")"; case "$cls" in 0x03*) ;; *) continue;; esac
  real="$(readlink -f "$d")"; addr="$(basename "$d")"
  mkdir -p "$OUT/sys${real#/sys}"
  for f in vendor device subsystem_vendor subsystem_device revision class; do copy_file "$real/$f"; done
  # Privacy: a bystander (non-Intel) GPU contributes only "another vendor's GPU is on this
  # bus". Its device and subsystem ids are replaced by the synthetic placeholder (0xffff) —
  # the capture must never record the product identity of hardware the tool does not manage.
  if [ "$(rd "$real/vendor")" != "0x8086" ]; then o="$OUT/sys${real#/sys}"
    rd "$real/vendor" > "$o/subsystem_vendor"; echo 0xffff > "$o/device"; echo 0xffff > "$o/subsystem_device"; fi
  ln -sfn "$(readlink "$d")" "$OUT/sys/bus/pci/devices/$addr"
  if [ -L "$real/driver" ]; then drv="$(basename "$(readlink "$real/driver")")"; copy_link "$real/driver"; mkdir -p "$OUT/sys/bus/pci/drivers/$drv"; ln -sfn "$(readlink "/sys/bus/pci/drivers/$drv/$addr" 2>/dev/null || echo "../../../../devices/${real#/sys/devices/}")" "$OUT/sys/bus/pci/drivers/$drv/$addr"; fi
  drv="$(basename "$(readlink "$real/driver" 2>/dev/null || echo none)")"
  [ "$drv" = xe ] || continue          # everything below is xe-only
  echo "  xe device $addr"
  for f in current_link_speed current_link_width max_link_speed max_link_width resource numa_node local_cpulist enable modalias uevent vram_d3cold_threshold; do [ -e "$real/$f" ] && copy_file "$real/$f"; done
  mkdir -p "$OUT/sys${real#/sys}/power"; for f in runtime_status control; do copy_file "$real/power/$f"; done
  for h in "$real"/hwmon/hwmon*; do [ -d "$h" ] || continue; for f in "$h"/*; do case "$(basename "$f")" in name|uevent|power*|energy*|temp*|fan*|curr*|in*) copy_file "$f";; esac; done; done
  for g in "$real"/tile*/gt*; do [ -d "$g" ] || continue
    for f in "$g"/freq0/* "$g"/freq0/throttle/* "$g"/gtidle/*; do [ -f "$f" ] && copy_file "$f"; done; done
  for t in "$real"/tile*/memory/freq0/*; do [ -f "$t" ] && copy_file "$t"; done
  for n in "$real"/drm/card* "$real"/drm/renderD*; do [ -d "$n" ] || continue; nb="$(basename "$n")"
    copy_file "$n/dev"; [ -f "$n/uevent" ] && copy_file "$n/uevent"; copy_link "$n/device"
    ln -sfn "$(readlink "/sys/class/drm/$nb")" "$OUT/sys/class/drm/$nb"; done
done
# --- processes using xe nodes (own user unless root) ---
for p in /proc/[0-9]*; do pid="$(basename "$p")"
  for fd in "$p"/fd/*; do t="$(readlink "$fd" 2>/dev/null || true)"; case "$t" in /dev/dri/*) ;; *) continue;; esac
    info="$(rd "$p/fdinfo/$(basename "$fd")")"; echo "$info" | grep -q '^drm-driver:.*xe' || continue
    mkdir -p "$OUT/proc/$pid/fd" "$OUT/proc/$pid/fdinfo"; rd "$p/comm" > "$OUT/proc/$pid/comm"
    grep -E '^(Name|State|Pid|Uid|Gid):' "$p/status" > "$OUT/proc/$pid/status" 2>/dev/null || true
    ln -sfn "$t" "$OUT/proc/$pid/fd/$(basename "$fd")"; printf '%s\n' "$info" > "$OUT/proc/$pid/fdinfo/$(basename "$fd")"
  done
done
# --- 0.1.2: placement, SR-IOV, AER, survivability, connectors, crash-dump metadata, cgroups ---
XE_ADDRS="$(for d in /sys/bus/pci/drivers/xe/0000:*; do [ -e "$d" ] && basename "$d"; done)"
PIDS="$(ls "$OUT/proc" | grep -E '^[0-9]+$' || true)"
for addr in $XE_ADDRS; do real="$(readlink -f "/sys/bus/pci/devices/$addr")"; o="$OUT/sys${real#/sys}"
  # bridge identity along the ancestry (class/vendor/device/numa_node, root-port AER totals)
  p=/sys/devices; for comp in $(echo "${real#/sys/devices/}" | tr '/' ' '); do p="$p/$comp"; [ "$p" = "$real" ] && break
    for f in class vendor device numa_node aer_rootport_total_err_cor aer_rootport_total_err_nonfatal aer_rootport_total_err_fatal; do [ -e "$p/$f" ] && copy_file "$p/$f"; done
    [ -e "$p/vendor" ] && ln -sfn "../../../${p#/sys/}" "$OUT/sys/bus/pci/devices/$comp"; done
  for f in reset_method sriov_totalvfs sriov_numvfs sriov_drivers_autoprobe aer_dev_correctable aer_dev_nonfatal aer_dev_fatal survivability_mode survivability_info; do [ -e "$real/$f" ] && copy_file "$real/$f"; done
  [ -e "$real/reset" ] && : > "$o/reset"
  if [ -L "$real/iommu_group" ]; then g="$(basename "$(readlink -f "$real/iommu_group")")"; mkdir -p "$OUT/sys/kernel/iommu_groups/$g/devices"
    depth="$(echo "${real#/sys/}" | tr -cd '/' | wc -c)"; ln -sfn "$(printf '../%.0s' $(seq 1 "$depth"))kernel/iommu_groups/$g" "$o/iommu_group"; fi
  for v in "$real"/virtfn*; do [ -L "$v" ] && copy_link "$v"; done
  if [ -d "$real/sriov_admin" ]; then
    find "$real/sriov_admin" -type f \( -name exec_quantum_ms -o -name preempt_timeout_us -o -name sched_priority -o -name vram_quota \) 2>/dev/null | while read -r f; do copy_file "$f"; done
    for st in "$real"/sriov_admin/vf*/stop; do [ -e "$st" ] && { mkdir -p "$(dirname "$o/${st#$real/}")"; : > "$o/${st#$real/}"; }; done
    for l in "$real"/sriov_admin/*/device; do [ -L "$l" ] && copy_link "$l"; done
  fi
  [ -L "$real/devcoredump" ] && copy_link "$real/devcoredump"
  for c in "$real"/drm/card*; do [ -d "$c" ] || continue; card="$(basename "$c")"
    for conn in /sys/class/drm/"$card"-*; do [ -d "$conn" ] || continue; for f in status enabled modes dpms; do [ -e "$conn/$f" ] && copy_file "$conn/$f"; done; done; done
done
# crash-dump class: metadata only, never the dump itself
mkdir -p "$OUT/sys/class/devcoredump"; rd /sys/class/devcoredump/disabled > "$OUT/sys/class/devcoredump/disabled" 2>/dev/null || true
for d in /sys/class/devcoredump/devcd*; do [ -d "$d" ] || continue; mkdir -p "$OUT/sys/class/devcoredump/$(basename "$d")"
  copy_link "$d/failing_device"; printf '**** Xe Device Coredump ****\nReason: (not captured)\n' > "$OUT/sys/class/devcoredump/$(basename "$d")/data"; done
# cgroup v2 dmem tree: dmem.* files, cgroup.procs restricted to captured pids, plus /proc/<pid>/cgroup
if [ -e /sys/fs/cgroup/dmem.capacity ]; then cg=/sys/fs/cgroup; mkdir -p "$OUT/sys/fs/cgroup"
  rd $cg/dmem.capacity > "$OUT/sys/fs/cgroup/dmem.capacity"; rd $cg/dmem.current > "$OUT/sys/fs/cgroup/dmem.current" 2>/dev/null || true
  find $cg -mindepth 1 -maxdepth 8 -type d 2>/dev/null | while read -r d; do [ -e "$d/dmem.current" ] || continue
    if grep -qvE ' 0$' "$d/dmem.current" 2>/dev/null || grep -qvE ' max$' "$d/dmem.max" 2>/dev/null; then o="$OUT/sys/fs/cgroup${d#$cg}"; mkdir -p "$o"
      for f in dmem.current dmem.max dmem.min dmem.low; do [ -e "$d/$f" ] && rd "$d/$f" > "$o/$f"; done
      : > "$o/cgroup.procs"; for pid in $PIDS; do grep -qx "$pid" "$d/cgroup.procs" 2>/dev/null && echo "$pid" >> "$o/cgroup.procs"; done; fi; done
  for pid in $PIDS; do [ -r "/proc/$pid/cgroup" ] && rd "/proc/$pid/cgroup" > "$OUT/proc/$pid/cgroup"; done
fi
# --- context (read-only; provenance for docs and fixture review) ---
lspci -nn 2>/dev/null | grep -iE 'vga|3d|display' > "$OUT/lspci-nn.txt" || true
{ echo "captured_utc=$(date -u +%FT%TZ)"; echo "kernel=$K"; echo "uid=$(id -u)"; echo "hostname=$(hostname -s)"; } > "$OUT/CAPTURE.txt"
[ "$(id -u)" = 0 ] && { dmesg 2>/dev/null | grep -iE '\bxe\b|\[drm\]' > "$OUT/raw-dmesg.txt" || true; }
find "$OUT" -type f | wc -l | xargs echo "files captured:"
echo "done: $OUT  (cargo test --test cli_captured now runs against it)"
