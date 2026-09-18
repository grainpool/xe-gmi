# The kernel interface boundary (`src/kabi`)

xe-gmi reads four things sysfs does not expose: authoritative memory-region accounting, engine/GT
topology, device configuration and GuC/HuC versions (DRM ioctls on the render node), RAS error
counters (a generic-netlink family), and live device events (the uevent multicast group). All of
it lives in one module, and it is the only part of the crate allowed to say `unsafe`.

## Policy

The crate root (`src/main.rs`) is `#![deny(unsafe_code)]`; exactly one file, `src/kabi/ioctl.rs`, carries
`#![allow(unsafe_code)]` for the single `ioctl` implementation. A unit test
(`tests/unsafe_boundary.rs`) fails the build if the token appears in any other file under `src/`.
The dependency set is unchanged except `rustix` (default features off; `std`, `fs`, `net`): the
ioctls and sockets are built from `rustix::net` and one hand-written `Ioctl` impl, no C or bindgen.

## The query protocol

`DRM_IOCTL_XE_DEVICE_QUERY` (opcode `0xC028_6440`) takes a `drm_xe_device_query` whose `size == 0`
first call reports the payload size; the second call fills a buffer of that size. The parsers in
`src/kabi/parse.rs` decode little-endian structs (`MEM_REGIONS`, `ENGINES`, `GT_LIST`,
`GT_TOPOLOGY`, `CONFIG`, `UC_FW_VERSION`) exactly as documented in spec/28 and unit-tested against
committed byte fixtures. Queries run against the card's render node; read access to that node is
the only permission floor (which is also how the tools these numbers are quoted from work).

## Netlink families

* `drm-ras` (generic netlink, resolved via `CTRL_CMD_GETFAMILY`): node enumeration, per-error
  counters and clearing. Frames are built by plain functions in `src/kabi/genl.rs` and the reply
  parsers are unit-tested against byte-level fixtures — the socket itself is `rustix::net`, no
  unsafe. Absent before kernel 7.1 (and without xe nodes before 7.2 Battlemage) the commands exit
  5 with the kernel floor in the message rather than failing opaquely.
* `NETLINK_KOBJECT_UEVENT` group 1 for `events`; binding the group is privileged, so without root
  the command exits 4 with the `sudo` hint.

## The replay seam (how tests and fixtures work)

`XE_GMI_KABI_REPLAY=DIR` switches the backend from real sockets to files:

```
DIR/drm-query/<pci>/{mem_regions,engines,gt_list,gt_topology,config}.bin
DIR/drm-query/<pci>/uc_fw_version_{0,1}.bin
DIR/ras/nodes.txt                 id pci node-name
DIR/ras/counters-<id>.txt         error-id name value
DIR/uevents.txt                   action@devpath|KEY=VAL|…   (| where the kernel puts NUL)
```

Every test golden that shows kernel numbers runs on fixture replay bytes; no test ever opens the
machine's render node or netlink sockets (the fixture gate refuses the real backend even if the
machine happens to have those interfaces). `--clear` against a replay directory appends the
cleared counters to `ras/clear.log` instead of talking to the kernel.

## Capturing real bytes

```
scripts/capture-kabi.sh DIR 0000:e3:00.0 [more pci...]
```

needs render-node access (render group or root), is read-only (query ioctls only) and writes the
`drm-query/` layout above; the RAS and uevent files follow the formats in spec/28 and are usually
captured by hand (`ras` on 7.2+ hardware, `events > uevents.txt` with `|` for NUL). Point
`XE_GMI_KABI_REPLAY` at the directory and the same parsers, tests and goldens consume it.
