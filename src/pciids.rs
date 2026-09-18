//! pci.ids lookup with a product-name fallback table.
//!
//! Grammar: `#` comments; vendor `^([0-9a-f]{4})  (.+)$`; device one tab deeper; subsystem
//! (double tab) ignored. Only vendor 8086 is ever looked up (discovery keys on the xe driver).

use crate::paths::Roots;
use std::path::Path;

const FALLBACK_PATHS: [&str; 3] = [
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/usr/share/pci.ids",
];

/// Fallback product names, spelled out.
const BATTLEMAGE_IDS: [u16; 14] = [
    0xe202, 0xe209, 0xe20b, 0xe20c, 0xe20d, 0xe210, 0xe211, 0xe212, 0xe215, 0xe216, 0xe220, 0xe221,
    0xe222, 0xe223,
];
// `dg2_desc` only; ATS-M (56c0–56c2) is its own table in and is not DG2-named.
pub(crate) const DG2_IDS: [u16; 25] = [
    0x5690, 0x5691, 0x5692, 0x5693, 0x5694, 0x5695, 0x5696, 0x5697, 0x56a0, 0x56a1, 0x56a2, 0x56a3,
    0x56a4, 0x56a5, 0x56a6, 0x56b0, 0x56b1, 0x56b2, 0x56b3, 0x56ba, 0x56bb, 0x56bc, 0x56bd, 0x56be,
    0x56bf,
];

/// Device name for a PCI id: file lookup per the seams, then the fallback table.
pub fn device_name(roots: &Roots, vendor: u16, device: u16) -> String {
    if vendor == 0x8086 {
        // `XE_GMI_PCIIDS=""` disables lookup entirely; unset searches the system files.
        let from_file = match &roots.pciids {
            Some(p) if !p.as_os_str().is_empty() => lookup_file(p, vendor, device),
            Some(_) => None,
            None => FALLBACK_PATHS
                .iter()
                .find_map(|p| lookup_file(Path::new(p), vendor, device)),
        };
        if let Some(name) = from_file {
            return name;
        }
        return fallback_name(device);
    }
    format!("GPU [{vendor:04x}:{device:04x}]")
}

/// One pci.ids file, `None` if unreadable or the id is absent.
pub fn lookup_file(path: &Path, vendor: u16, device: u16) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let want_vendor = format!("{vendor:04x}");
    let want_device = format!("{device:04x}");
    let mut in_vendor = false;
    for line in content.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('\t') {
            if !in_vendor || rest.starts_with('\t') {
                continue; // subsystem lines and anything outside a vendor block
            }
            if let Some((id, name)) = split_id_name(rest) {
                if id == want_device {
                    return Some(name.to_string());
                }
            }
        } else {
            in_vendor = match split_id_name(line) {
                Some((id, _)) => id == want_vendor,
                None => false, // `C 0x03` class lines and similar end the block
            };
        }
    }
    None
}

/// `xxxx  Name` → (`xxxx`, `Name`), requiring the exact two-space separator of pci.ids.
fn split_id_name(s: &str) -> Option<(&str, &str)> {
    let (id, sep) = s.split_at(4);
    if !id.bytes().all(|b| b.is_ascii_hexdigit()) || !sep.starts_with("  ") {
        return None;
    }
    let name = &s[6..];
    if name.is_empty() {
        return None;
    }
    Some((id, name))
}

/// Only IDs verified against real hardware are named; anything else degrades to the table
/// family or a bare `Intel GPU [8086:<id>]`.
pub fn fallback_name(device: u16) -> String {
    match device {
        0xe20b => "Battlemage G21 [Arc B580]".into(),
        0xe20c => "Battlemage G21 [Arc B570]".into(),
        0xe222 => "Battlemage G31 [Arc Pro B65]".into(),
        0x56a0 => "DG2 [Arc A770]".into(),
        d if BATTLEMAGE_IDS.contains(&d) => {
            let family = if (0xe220..=0xe223).contains(&d) {
                "G31"
            } else {
                "G21"
            };
            format!("Battlemage {family} [8086:{d:04x}]")
        }
        d if DG2_IDS.contains(&d) => format!("DG2 [8086:{d:04x}]"),
        d => format!("Intel GPU [8086:{d:04x}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mini_and_missing() {
        let mini = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/pci.ids.mini");
        assert_eq!(
            lookup_file(&mini, 0x8086, 0xe222).as_deref(),
            Some("Battlemage G31 [Arc Pro B65]")
        );
        assert_eq!(lookup_file(&mini, 0x8086, 0xe999), None);
        assert_eq!(
            lookup_file(Path::new("/nonexistent/pci.ids"), 0x8086, 0xe222),
            None
        );
        // Names must come from the 8086 block, not the 10de one.
        assert_eq!(
            lookup_file(&mini, 0x10de, 0xffff).as_deref(),
            Some("Placeholder Device (synthetic fixture)")
        );
    }

    #[test]
    fn fallback_table_is_exact() {
        assert_eq!(fallback_name(0xe20b), "Battlemage G21 [Arc B580]");
        assert_eq!(fallback_name(0xe222), "Battlemage G31 [Arc Pro B65]");
        assert_eq!(fallback_name(0xe220), "Battlemage G31 [8086:e220]");
        assert_eq!(fallback_name(0xe216), "Battlemage G21 [8086:e216]");
        assert_eq!(fallback_name(0x56a0), "DG2 [Arc A770]");
        assert_eq!(fallback_name(0x56b1), "DG2 [8086:56b1]");
        assert_eq!(fallback_name(0xe999), "Intel GPU [8086:e999]");
    }
}
