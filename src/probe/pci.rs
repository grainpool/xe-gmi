//! PCI identity, link, VRAM.

use crate::avail::{Avail, Reason};
use crate::sysfs;
use std::path::Path;

#[derive(Debug)]
pub struct PciIds {
    pub vendor: u16,
    pub device: u16,
    pub subsystem: Avail<(u16, u16)>,
    pub revision: Avail<u8>,
    pub class: Avail<u32>,
}

#[derive(Debug, Clone)]
pub struct PcieLink {
    pub gen_cur: Option<u8>,
    pub gen_max: Option<u8>,
    pub width_cur: Option<u32>,
    pub width_max: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VramSource {
    Bar,
    Table,
}

#[derive(Debug)]
pub struct VramTotal {
    pub bytes: u64,
    pub source: VramSource,
}

/// sysfs identity files are `0x`-prefixed hex, so they do not go through `read_u64`.
pub fn parse_hex_u64(raw: &str) -> Option<u64> {
    let s = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    u64::from_str_radix(s, 16).ok()
}

fn read_hex_file(p: &Path) -> Avail<u64> {
    match sysfs::read_string(p) {
        Avail::Value(s) => match parse_hex_u64(&s) {
            Some(v) => Avail::Value(v),
            None => Avail::NotAvailable(Reason::Unparseable(p.to_path_buf(), s)),
        },
        Avail::NotAvailable(r) => Avail::NotAvailable(r),
    }
}

fn read_hex<T: TryFrom<u64> + Default>(p: &Path) -> Avail<T> {
    read_hex_file(p).map(|v| T::try_from(v).unwrap_or_default())
}

/// vendor + device are required for a device to be usable; the rest degrade to N/A.
pub fn read_pci_ids(dev_dir: &Path) -> Option<PciIds> {
    let vendor = read_hex::<u16>(&dev_dir.join("vendor")).value()?.to_owned();
    let device = read_hex::<u16>(&dev_dir.join("device")).value()?.to_owned();
    let sub_v = read_hex::<u16>(&dev_dir.join("subsystem_vendor"));
    let sub_d = read_hex::<u16>(&dev_dir.join("subsystem_device"));
    let subsystem = match (sub_v, sub_d) {
        (Avail::Value(v), Avail::Value(d)) => Avail::Value((v, d)),
        (Avail::NotAvailable(r), _) | (_, Avail::NotAvailable(r)) => Avail::NotAvailable(r),
    };
    Some(PciIds {
        vendor,
        device,
        subsystem,
        revision: read_hex::<u8>(&dev_dir.join("revision")),
        class: read_hex::<u32>(&dev_dir.join("class")),
    })
}

fn link_gen_of_file(p: &Path) -> Option<u8> {
    sysfs::read_string(p)
        .value()
        .and_then(|s| link_gen_mapping(s))
}

fn width_of_file(p: &Path) -> Option<u32> {
    sysfs::read_u64(p)
        .value()
        .and_then(|v| u32::try_from(*v).ok())
}

pub fn read_link(dev_dir: &Path) -> Avail<PcieLink> {
    let link = PcieLink {
        gen_cur: link_gen_of_file(&dev_dir.join("current_link_speed")),
        gen_max: link_gen_of_file(&dev_dir.join("max_link_speed")),
        width_cur: width_of_file(&dev_dir.join("current_link_width")),
        width_max: width_of_file(&dev_dir.join("max_link_width")),
    };
    if link.gen_cur.is_none()
        && link.gen_max.is_none()
        && link.width_cur.is_none()
        && link.width_max.is_none()
    {
        return Avail::NotAvailable(Reason::Missing(dev_dir.join("current_link_speed")));
    }
    Avail::Value(link)
}

/// Which PCI node a reported link was read from. Intel Arc cards carry an internal PCIe
/// switch; the nodes near the GPU always report Gen1 x1 regardless of the trained link
/// (Intel KB 000094587), so the root port's view wins whenever it is available there.
pub const LINK_VIA_ENDPOINT: &str = "endpoint";
pub const LINK_VIA_ROOT_PORT: &str = "root port";

/// The always-Gen1-x1 endpoint reading from Intel KB 000094587 (current and maximum both
/// pinned to gen1 x1 — a real negotiated link never reports a maximum of gen1 x1 on a GPU).
pub fn is_gen1_artifact(l: &PcieLink) -> bool {
    l.gen_cur == Some(1) && l.gen_max == Some(1) && l.width_cur == Some(1) && l.width_max == Some(1)
}

/// Prefer the root port's link when the endpoint carries the Arc internal-switch artifact
/// (or exposes nothing at all); otherwise keep the endpoint's view of the same link.
pub fn resolve_link(
    endpoint: Avail<PcieLink>,
    root_port: Option<Avail<PcieLink>>,
) -> (Avail<PcieLink>, &'static str) {
    if let Some(Avail::Value(rp)) = &root_port {
        let take_rp = match &endpoint {
            Avail::NotAvailable(_) => true,
            Avail::Value(ep) => is_gen1_artifact(ep),
        };
        if take_rp {
            return (Avail::Value(rp.clone()), LINK_VIA_ROOT_PORT);
        }
    }
    (endpoint, LINK_VIA_ENDPOINT)
}

/// PCI power management and ASPM state, all plain sysfs reads (`pci-sysfs`/`sysfs`):
/// the device's power state, its runtime-PM status, whether D3cold is allowed, the
/// system ASPM policy, and the negotiated L1 state at each end of the link.
#[derive(Debug)]
pub struct PciPower {
    /// `power_state`: D0 / D3hot / D3cold.
    pub state: Avail<String>,
    /// `power/runtime_status`: active / suspended / resuming / suspending / error / no,
    pub runtime_status: Avail<String>,
    /// `power/d3cold_allowed`: 1 = the kernel may cut slot power in D3cold.
    pub d3cold_allowed: Avail<u64>,
    /// `link/l1_aspm` on the device (`enabled`/`disabled`); absent without an upstream bridge.
    pub l1_endpoint: Avail<String>,
    /// `link/l1_aspm` on the root port.
    pub l1_root_port: Avail<String>,
    /// The bracketed selection in `/sys/module/pcie_aspm/parameters/policy`, system-wide.
    pub policy: Avail<String>,
}

pub fn read_pci_power(dev_dir: &Path, rp_dir: Option<&Path>, policy: &Avail<String>) -> PciPower {
    PciPower {
        state: sysfs::read_string(&dev_dir.join("power_state")),
        runtime_status: sysfs::read_string(&dev_dir.join("power/runtime_status")),
        d3cold_allowed: sysfs::read_u64(&dev_dir.join("power/d3cold_allowed")),
        l1_endpoint: sysfs::read_string(&dev_dir.join("link/l1_aspm")),
        l1_root_port: match rp_dir {
            Some(d) => sysfs::read_string(&d.join("link/l1_aspm")),
            None => Avail::NotAvailable(Reason::NotSupported(
                "no root port between the host bridge and the device",
            )),
        },
        policy: policy.clone(),
    }
}

/// `2.5 GT/s PCIe` → 1 … `64.0 GT/s PCIe` → 6; `Unknown speed` (or anything unknown) → None.
pub fn link_gen_mapping(s: &str) -> Option<u8> {
    let value: f64 = s.split_whitespace().next()?.parse().ok()?;
    match value {
        v if (v - 2.5).abs() < 0.01 => Some(1),
        v if (v - 5.0).abs() < 0.01 => Some(2),
        v if (v - 8.0).abs() < 0.01 => Some(3),
        v if (v - 16.0).abs() < 0.01 => Some(4),
        v if (v - 32.0).abs() < 0.01 => Some(5),
        v if (v - 64.0).abs() < 0.01 => Some(6),
        _ => None,
    }
}

/// `resource`: 17 lines `0x<start> 0x<end> 0x<flags>`. The largest prefetchable memory BAR
/// (flags bit 9 = memory, bit 13 = prefetchable) is accepted as VRAM when ≥ 1 GiB, else the
/// SKU table, else N/A.
pub fn parse_resource_vram(text: &str, device_id: u16) -> Avail<VramTotal> {
    const GIB: u64 = 1 << 30;
    let mut best: Option<u64> = None;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(start), Some(end), Some(flags)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let (Some(start), Some(end), Some(flags)) = (
            parse_hex_u64(start),
            parse_hex_u64(end),
            parse_hex_u64(flags),
        ) else {
            continue;
        };
        if flags & 0x200 == 0 || flags & 0x2000 == 0 || end < start {
            continue;
        }
        let size = end - start + 1;
        if best.map_or(true, |b| size > b) {
            best = Some(size);
        }
    }
    match best {
        Some(b) if b >= GIB => Avail::Value(VramTotal {
            bytes: b,
            source: VramSource::Bar,
        }),
        _ => match table_vram(device_id) {
            Some(bytes) => Avail::Value(VramTotal {
                bytes,
                source: VramSource::Table,
            }),
            None => Avail::NotAvailable(Reason::NotSupported(
                "VRAM total: small BAR (resize failed) and no SKU table entry",
            )),
        },
    }
}

/// Verified SKU sizes.
fn table_vram(device_id: u16) -> Option<u64> {
    const GIB: u64 = 1 << 30;
    match device_id {
        0xe20b => Some(12 * GIB),
        0xe20c => Some(10 * GIB),
        0xe222 => Some(32 * GIB),
        0x56a0 => Some(16 * GIB),
        _ => None,
    }
}

pub fn vram_total(dev_dir: &Path, device_id: u16) -> Avail<VramTotal> {
    let path = dev_dir.join("resource");
    match sysfs::read_string(&path) {
        Avail::Value(text) => parse_resource_vram(&text, device_id),
        Avail::NotAvailable(r) => Avail::NotAvailable(r),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        link_gen_mapping as gen, parse_hex_u64, parse_resource_vram as parse_resource, VramSource,
    };

    const B65_RESOURCE: &str = "\
0x00000000f4000000 0x00000000f4ffffff 0x0000000000140204
0x0000000000000000 0x0000000000000000 0x0000000000000000
0x0000004000000000 0x00000047ffffffff 0x000000000014220c
0x0000000000000000 0x0000000000000000 0x0000000000000000";

    #[test]
    fn parse_resource_vram() {
        let v = parse_resource(B65_RESOURCE, 0xe222);
        let Some(t) = v.value() else {
            panic!("expected a value")
        };
        assert_eq!(t.bytes, 34_359_738_368, "32 GiB BAR at line index 2");
        assert_eq!(t.source, VramSource::Bar);
        // 256 MiB prefetchable BAR (< 1 GiB) → SKU table fallback for e222.
        let small = "0x00000000f4000000 0x00000000f5ffffff 0x000000000014220c";
        let v = parse_resource(small, 0xe222);
        let Some(t) = v.value() else {
            panic!("expected a value")
        };
        assert_eq!(t.bytes, 32 * (1 << 30));
        assert_eq!(t.source, VramSource::Table);
        // Unknown ID with only a small BAR → N/A.
        assert!(parse_resource(small, 0xe999).value().is_none());
        // A large non-prefetchable BAR must not count as VRAM.
        let np = "0x0000004000000000 0x00000047ffffffff 0x0000000000140200";
        assert!(parse_resource(np, 0xe999).value().is_none());
    }

    #[test]
    fn link_gen_mapping() {
        assert_eq!(gen("2.5 GT/s PCIe"), Some(1));
        assert_eq!(gen("5.0 GT/s PCIe"), Some(2));
        assert_eq!(gen("8.0 GT/s PCIe"), Some(3));
        assert_eq!(gen("16.0 GT/s PCIe"), Some(4));
        assert_eq!(gen("32.0 GT/s PCIe"), Some(5));
        assert_eq!(gen("64.0 GT/s PCIe"), Some(6));
        assert_eq!(gen("Unknown speed"), None);
    }

    #[test]
    fn hex_identity_parses() {
        assert_eq!(parse_hex_u64("0x8086"), Some(0x8086));
        assert_eq!(parse_hex_u64("0x030000"), Some(0x030000));
        assert_eq!(parse_hex_u64("0xe222"), Some(0xe222));
    }

    fn link(cur: u8, max: u8, wc: u32, wm: u32) -> crate::avail::Avail<super::PcieLink> {
        crate::avail::Avail::Value(super::PcieLink {
            gen_cur: Some(cur),
            gen_max: Some(max),
            width_cur: Some(wc),
            width_max: Some(wm),
        })
    }

    #[test]
    fn root_port_wins_for_the_gen1_artifact_only() {
        use super::{resolve_link, LINK_VIA_ENDPOINT, LINK_VIA_ROOT_PORT};
        let artifact = link(1, 1, 1, 1);
        let rp = link(5, 5, 16, 16);
        let (l, via) = resolve_link(artifact.clone(), Some(rp.clone()));
        assert_eq!(via, LINK_VIA_ROOT_PORT);
        assert_eq!(l.value().unwrap().gen_cur, Some(5));
        // A real endpoint reading is kept even when the root port also reports.
        let healthy = link(4, 5, 16, 16);
        let (_, via) = resolve_link(healthy, Some(rp.clone()));
        assert_eq!(via, LINK_VIA_ENDPOINT);
        // No root port at all: endpoint as before.
        let (_, via) = resolve_link(artifact.clone(), None);
        assert_eq!(via, LINK_VIA_ENDPOINT);
        // Endpoint exposes nothing but the root port does: the root port answers.
        let missing = crate::avail::Avail::NotAvailable(crate::avail::Reason::Missing(
            std::path::PathBuf::from("current_link_speed"),
        ));
        let (l, via) = resolve_link(missing, Some(rp));
        assert_eq!(via, LINK_VIA_ROOT_PORT);
        assert_eq!(l.value().unwrap().width_cur, Some(16));
        // Root port present but itself reporting nothing usable: endpoint stays.
        let rp_missing = crate::avail::Avail::NotAvailable(crate::avail::Reason::Missing(
            std::path::PathBuf::from("current_link_speed"),
        ));
        let (_, via) = resolve_link(artifact, Some(rp_missing));
        assert_eq!(via, LINK_VIA_ENDPOINT);
    }
}
