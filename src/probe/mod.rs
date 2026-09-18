//! Device discovery and selection.

pub mod gt;
pub mod hwmon;
pub mod pci;
pub mod placement;

use crate::avail::Avail;
use crate::error::{Error, Result};
use crate::paths::Roots;
use crate::pciids;
use crate::sysfs;
use gt::Gt;
pub use hwmon::{Hwmon, TempSensor};
pub use pci::{PciIds, PcieLink, VramSource, VramTotal};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Device {
    pub index: usize,
    pub pci: String,
    pub dev_dir: PathBuf,
    pub card: String,
    pub render: Option<String>,
    pub ids: PciIds,
    pub name: String,
    pub link: Avail<PcieLink>,
    pub vram_total: Avail<VramTotal>,
    pub hwmon: Option<Hwmon>,
    pub gts: Vec<Gt>,
    pub d3cold_threshold_mib: Avail<u64>,
    pub placement: placement::Placement,
    /// Device queries through the kernel interface (N/A without a render node; everything else
    /// keeps working). See `src/kabi`.
    pub kabi: crate::kabi::Kabi,
}

fn digit_suffix(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

fn tail_component(p: &Path) -> Option<String> {
    p.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Enumerate `class/drm/card<N>` bound to the xe driver, resolve each PCI device, sort by PCI
/// address string order and index from 0. Other vendors are keyed out
/// by the driver symlink and never opened further.
pub fn discover(roots: &Roots) -> Vec<Device> {
    let drm = roots.sysfs.join("class/drm");
    let render_nodes = sysfs::list_dir(&drm, |n| digit_suffix(n, "renderD"));
    let mut devs: Vec<Device> = Vec::new();
    for card in sysfs::list_dir(&drm, |n| digit_suffix(n, "card")) {
        let card_name = match tail_component(&card) {
            Some(n) => n,
            None => continue,
        };
        let dev_dir = match sysfs::canonical(&card.join("device")).value() {
            Some(d) => d.clone(),
            None => continue,
        };
        if sysfs::link_basename(&dev_dir.join("driver"))
            .value()
            .map(|d| d != "xe")
            .unwrap_or(true)
        {
            continue; // any driver other than xe (or none): never opened further
        }
        let pci = match tail_component(&dev_dir) {
            Some(p) if p.contains(':') => p, // canonical PCI dir ends with `DDDD:BB:DD.F`
            _ => continue,
        };
        if devs.iter().any(|d| d.pci == pci) {
            continue; // a second card of the same PCI device
        }
        let render = render_nodes
            .iter()
            .find(|r| {
                sysfs::canonical(&r.join("device"))
                    .value()
                    .is_some_and(|p| *p == *dev_dir)
            })
            .and_then(|p| tail_component(p));
        let Some(ids) = pci::read_pci_ids(&dev_dir) else {
            continue;
        };
        let name = pciids::device_name(roots, ids.vendor, ids.device);
        devs.push(Device {
            index: 0,
            link: pci::read_link(&dev_dir),
            vram_total: pci::vram_total(&dev_dir, ids.device),
            hwmon: hwmon::discover_hwmon(&dev_dir),
            gts: gt::discover_gts(&dev_dir),
            d3cold_threshold_mib: sysfs::read_u64(&dev_dir.join("vram_d3cold_threshold")),
            placement: placement::Placement::read(&dev_dir, &pci),
            kabi: crate::kabi::Kabi::read(roots, &pci, &card_name, render.as_deref()),
            pci,
            dev_dir,
            card: card_name,
            render,
            name,
            ids,
        });
    }
    devs.sort_by(|a, b| a.pci.cmp(&b.pci));
    for (i, d) in devs.iter_mut().enumerate() {
        d.index = i;
    }
    devs
}

/// `-i SEL`: index, full PCI address, or `BB:DD.F` with domain `0000` assumed.
pub fn normalize_address(sel: &str) -> String {
    if sel.split(':').count() == 2 {
        format!("0000:{sel}")
    } else {
        sel.to_string()
    }
}

fn selector_matches(sel: &str, index: usize, pci: &str) -> bool {
    match sel.parse::<usize>() {
        Ok(i) => i == index,
        Err(_) => normalize_address(sel) == pci,
    }
}

/// Device selection for read commands. A selector that matches nothing, or names a
/// PCI device that exists but is not xe-bound, exits 3.
pub fn select<'a>(devs: &'a [Device], sel: Option<&str>) -> Result<Vec<&'a Device>> {
    if devs.is_empty() {
        return Err(Error::NoDevice(
            "no xe device found (run: xe-gmi doctor)".into(),
        ));
    }
    let Some(sel) = sel else {
        return Ok(devs.iter().collect());
    };
    if let Some(d) = devs.iter().find(|d| selector_matches(sel, d.index, &d.pci)) {
        return Ok(vec![d]);
    }
    // A real PCI device with a different (or no) driver gets its own message.
    let addr = normalize_address(sel);
    let dev_path = Roots::from_env().sysfs.join("bus/pci/devices").join(&addr);
    if dev_path.exists() || dev_path.symlink_metadata().is_ok() {
        let what = match sysfs::link_basename(&dev_path.join("driver")).value() {
            Some(name) => format!("driver {name}"),
            None => "no driver".into(),
        };
        return Err(Error::NoDevice(format!(
            "{addr} is not an xe device ({what})"
        )));
    }
    Err(Error::NoDevice(format!(
        "no xe device matches \"{sel}\" (devices: {})",
        devs.iter()
            .map(|d| d.pci.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Convenience for control commands: exactly one device, or the contract's messages.
pub fn select_one<'a>(devs: &'a [Device], sel: Option<&str>) -> Result<&'a Device> {
    if devs.is_empty() {
        return Err(Error::NoDevice(
            "no xe device found (run: xe-gmi doctor)".into(),
        ));
    }
    match select(devs, sel)?.as_slice() {
        [d] => Ok(d),
        _ => Err(Error::Usage(
            "more than one xe device; select one with -i".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_forms() {
        let (index, pci) = (0usize, "0000:e3:00.0");
        assert!(selector_matches("0", index, pci));
        assert!(selector_matches("0000:e3:00.0", index, pci));
        assert!(selector_matches("e3:00.0", index, pci));
        assert!(!selector_matches("1", index, pci));
        assert!(!selector_matches("zz", index, pci));
    }
}
