#!/usr/bin/env bash
# Capture real DRM device-query responses into a kabi replay directory (spec/28) for the given xe
# devices: DIR/drm-query/<pci>/{mem_regions,engines,gt_list,gt_topology,config}.bin, uc_fw_version_{0,1}.bin.
# Read-only (query ioctls only). Needs read/write access to the render node (render group or root).
set -euo pipefail
DIR="${1:?usage: capture-kabi.sh DIR PCI...}"; shift
for addr in "$@"; do
  node="$(ls -d /sys/bus/pci/devices/"$addr"/drm/renderD* 2>/dev/null | head -1 | xargs -r basename)"
  [ -n "$node" ] || { echo "$addr: no render node"; continue; }
  mkdir -p "$DIR/drm-query/$addr"
  python3 - "/dev/dri/$node" "$DIR/drm-query/$addr" <<'PY'
import sys, os, fcntl, struct, ctypes
node, out = sys.argv[1], sys.argv[2]
REQ = 0xC0286440
fd = os.open(node, os.O_RDWR | os.O_CLOEXEC)
def query(qid, prefill=b""):
    arg = bytearray(struct.pack("<QIIQQQ", 0, qid, 0, 0, 0, 0))
    fcntl.ioctl(fd, REQ, arg, True)
    size = struct.unpack_from("<I", arg, 12)[0]
    buf = ctypes.create_string_buffer(size)
    ctypes.memmove(buf, prefill, min(len(prefill), size))
    arg = bytearray(struct.pack("<QIIQQQ", 0, qid, size, ctypes.addressof(buf), 0, 0))
    fcntl.ioctl(fd, REQ, arg, True)
    return buf.raw
names = {0: "engines", 1: "mem_regions", 2: "config", 3: "gt_list", 5: "gt_topology"}
for qid, name in names.items():
    try:
        open(os.path.join(out, name + ".bin"), "wb").write(query(qid)); print(f"{name}: ok")
    except OSError as e:
        print(f"{name}: {e.strerror}")
for uc in (0, 1):
    try:
        open(os.path.join(out, f"uc_fw_version_{uc}.bin"), "wb").write(query(7, struct.pack("<H", uc))); print(f"uc_fw_version_{uc}: ok")
    except OSError as e:
        print(f"uc_fw_version_{uc}: {e.strerror}")
PY
done
