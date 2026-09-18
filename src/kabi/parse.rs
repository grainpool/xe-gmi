//! Byte parsers for every DRM device-query reply (plain, safe code, unit-tested against the
//! fixture `.bin` files under `fixtures/synthetic/b65-g31-k7.1/kabi/drm-query/`).
//!
//! All layouts are little-endian with explicit offsets, mirroring `include/uapi/drm/xe_drm.h`;
//! no `#[repr(C)]` structs cross a boundary here.

use super::{Config, Engine, GtInfo, MemRegion, TopologyMask, UcFw};

pub const Q_ENGINES: u32 = 0;
pub const Q_MEM_REGIONS: u32 = 1;
pub const Q_CONFIG: u32 = 2;
pub const Q_GT_LIST: u32 = 3;
pub const Q_GT_TOPOLOGY: u32 = 5;
pub const Q_UC_FW_VERSION: u32 = 7;

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap_or([0; 2]))
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]))
}

fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}

fn ok(off: usize, need: usize, len: usize) -> bool {
    off + need <= len
}

pub fn mem_regions(buf: &[u8]) -> Vec<MemRegion> {
    if !ok(0, 8, buf.len()) {
        return Vec::new();
    }
    let num = u32le(buf, 0) as usize;
    let mut out = Vec::new();
    for i in 0..num {
        let r = 8 + i * 88;
        if !ok(r, 40, buf.len()) {
            break;
        }
        out.push(MemRegion {
            class: u16le(buf, r),
            instance: u16le(buf, r + 2),
            min_page_size: u32le(buf, r + 4),
            total: u64le(buf, r + 8),
            used: u64le(buf, r + 16),
            cpu_visible: u64le(buf, r + 24),
            cpu_visible_used: u64le(buf, r + 32),
        });
    }
    out
}

pub fn engines(buf: &[u8]) -> Vec<Engine> {
    if !ok(0, 8, buf.len()) {
        return Vec::new();
    }
    let num = u32le(buf, 0) as usize;
    let mut out = Vec::new();
    for i in 0..num {
        let r = 8 + i * 32;
        if !ok(r, 8, buf.len()) {
            break;
        }
        let class = u16le(buf, r);
        if class == 5 {
            continue; // vm-bind is a UAPI scheduling construct, not a physical engine
        }
        out.push(Engine {
            class,
            instance: u16le(buf, r + 2),
            gt_id: u16le(buf, r + 4),
        });
    }
    out
}

pub fn gt_list(buf: &[u8]) -> Vec<GtInfo> {
    if !ok(0, 8, buf.len()) {
        return Vec::new();
    }
    let num = u32le(buf, 0) as usize;
    let mut out = Vec::new();
    for i in 0..num {
        let r = 8 + i * 96;
        if !ok(r, 40, buf.len()) {
            break;
        }
        out.push(GtInfo {
            kind: u16le(buf, r),
            tile: u16le(buf, r + 2),
            gt: u16le(buf, r + 4),
            reference_clock: u32le(buf, r + 12),
            near_mem: u64le(buf, r + 16),
            far_mem: u64le(buf, r + 24),
            ip: (u16le(buf, r + 32), u16le(buf, r + 34), u16le(buf, r + 36)),
        });
    }
    out
}

pub fn gt_topology(buf: &[u8]) -> Vec<TopologyMask> {
    let mut out = Vec::new();
    let mut r = 0usize;
    while ok(r, 8, buf.len()) {
        let n = u32le(buf, r + 4) as usize;
        if !ok(r + 8, n, buf.len()) {
            break;
        }
        out.push(TopologyMask {
            gt: u16le(buf, r),
            kind: u16le(buf, r + 2),
            mask: buf[r + 8..r + 8 + n].to_vec(),
        });
        r += 8 + n;
    }
    out
}

pub fn uc_fw(buf: &[u8]) -> UcFw {
    if !ok(0, 20, buf.len()) {
        return UcFw {
            uc_type: 0,
            branch: 0,
            major: 0,
            minor: 0,
            patch: 0,
        };
    }
    UcFw {
        uc_type: u16le(buf, 0),
        branch: u32le(buf, 4),
        major: u32le(buf, 8),
        minor: u32le(buf, 12),
        patch: u32le(buf, 16),
    }
}

pub fn config(buf: &[u8]) -> Config {
    let mut c = Config {
        device_id: 0,
        revision: 0,
        flags: 0,
        min_alignment: 0,
        va_bits: 0,
        max_prio: 0,
    };
    if !ok(0, 8, buf.len()) {
        return c;
    }
    let num = (u32le(buf, 0) as usize).min(5);
    let param = |i: usize| -> u64 {
        let at = 8 + i * 8;
        if i < num && ok(at, 8, buf.len()) {
            u64le(buf, at)
        } else {
            0
        }
    };
    let rev_and_dev = param(0);
    c.device_id = (rev_and_dev & 0xFFFF) as u16;
    c.revision = ((rev_and_dev >> 16) & 0xFF) as u8;
    c.flags = param(1);
    c.min_alignment = param(2);
    c.va_bits = param(3);
    c.max_prio = param(4);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/synthetic/b65-g31-k7.1/kabi/drm-query/0000:e3:00.0")
            .join(name);
        std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
    }

    #[test]
    fn kabi_parse_mem_regions() {
        let r = mem_regions(&fixture("mem_regions.bin"));
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].class, r[0].instance), (0, 0)); // sysmem
        assert_eq!(r[1].class, 1); // vram
        assert_eq!(r[1].total, 34359738368);
        assert_eq!(r[1].used, 9412 * 1024 * 1024);
        assert_eq!(r[1].min_page_size, 65536);
        assert_eq!(r[1].cpu_visible, 34359738368);
        assert_eq!(r[1].cpu_visible_used, 9412 * 1024 * 1024);
    }

    #[test]
    fn kabi_parse_engines() {
        let e = engines(&fixture("engines.bin"));
        assert_eq!(e.len(), 10);
        assert_eq!((e[0].class, e[0].instance, e[0].gt_id), (0, 0, 0)); // rcs0 on gt0
        assert!(e.iter().all(|x| x.class != 5)); // no vm-bind
        assert_eq!((e[9].class, e[9].instance, e[9].gt_id), (3, 0, 1)); // vecs0 on gt1
    }

    #[test]
    fn kabi_parse_gt_list() {
        let g = gt_list(&fixture("gt_list.bin"));
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].kind, g[0].gt), (0, 0));
        assert_eq!(g[0].reference_clock, 19200000);
        assert_eq!(g[0].ip, (20, 1, 0));
        assert_eq!((g[1].kind, g[1].gt), (1, 1));
        assert_eq!(g[1].ip, (13, 0, 0));
        assert_eq!(g[0].near_mem, 0x2);
        assert_eq!(g[0].far_mem, 0x1);
    }

    #[test]
    fn kabi_parse_gt_topology() {
        let t = gt_topology(&fixture("gt_topology.bin"));
        assert_eq!(t.len(), 5);
        assert_eq!((t[0].gt, t[0].kind), (0, 1)); // geometry DSS
        assert_eq!(super::super::popcount(&t[0].mask), 20);
        assert_eq!(t[1].kind, 2);
        assert_eq!(super::super::popcount(&t[1].mask), 20);
        assert_eq!(t[2].kind, 3);
        assert_eq!(super::super::popcount(&t[2].mask), 16);
        assert_eq!(super::super::popcount(&t[3].mask), 8);
        assert_eq!(super::super::popcount(&t[4].mask), 8);
    }

    #[test]
    fn kabi_parse_uc_fw() {
        let g = uc_fw(&fixture("uc_fw_version_0.bin"));
        assert_eq!((g.branch, g.major, g.minor, g.patch), (0, 70, 44, 1));
        let h = uc_fw(&fixture("uc_fw_version_1.bin"));
        assert_eq!((h.uc_type, h.major, h.minor, h.patch), (1, 9, 4, 13));
    }

    #[test]
    fn kabi_parse_config() {
        let c = config(&fixture("config.bin"));
        assert_eq!(c.device_id, 0xe222);
        assert_eq!(c.revision, 1);
        assert_eq!(c.flags & 1, 1);
        assert_eq!(c.min_alignment, 65536);
        assert_eq!(c.va_bits, 48);
        assert_eq!(c.max_prio, 2);
    }
}
