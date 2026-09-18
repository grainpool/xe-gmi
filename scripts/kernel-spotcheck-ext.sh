#!/usr/bin/env bash
# Spot-check the kernel facts the 0.1.2 features rely on against the tagged sources (raw files, no clone).
# Prints the lines the docs cite; compare by eye. Needs access to raw.githubusercontent.com.
set -euo pipefail
R=https://raw.githubusercontent.com/torvalds/linux
f() { curl -sf "$R/$1/$2"; }
echo "== sriov_admin ABI (v7.2)"; f v7.2 Documentation/ABI/testing/sysfs-driver-intel-xe-sriov | grep -nE '^What:'
echo "== AER sysfs attributes (v7.2)"; f v7.2 drivers/pci/pcie/aer.c | grep -nE 'aer_stats_(dev|rootport)_attr\(aer_'
echo "== dmem files (v7.2)"; f v7.2 Documentation/admin-guide/cgroup-v2.rst | grep -nE 'dmem\.(max|min|low|capacity|current)'
echo "== devcoredump data attribute: any write releases (v7.2)"; f v7.2 drivers/base/devcoredump.c | grep -nE '__BIN_ATTR\(data|devcd_data_write'
echo "== xe coredump header lines (v7.2)"; f v7.2 drivers/gpu/drm/xe/xe_devcoredump.c | grep -nE 'Reason: |Snapshot time'
echo "== survivability attributes (v7.2)"; f v7.2 drivers/gpu/drm/xe/xe_survivability_mode.c | grep -nE 'DEVICE_ATTR|survivability_(mode|info)'
echo "== xe has no PCI error handlers (expect none)"; f v7.2 drivers/gpu/drm/xe/xe_pci.c | grep -nE 'reset_prepare|reset_done|err_handler' || echo "(none)"
