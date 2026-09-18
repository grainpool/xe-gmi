//! PCI placement: NUMA, CPUs, IOMMU group, sysfs path, root port, AER, SR-IOV, reset methods
//! (spec/23 §1–2). Every read degrades to `N/A`; absence of AER files is normal.

use crate::avail::{Avail, Reason};
use crate::sysfs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Vf {
    pub addr: String,
    pub driver: Avail<String>,
}

#[derive(Debug)]
pub struct SriovPf {
    pub total: Avail<u64>,
    pub num: Avail<u64>,
    pub autoprobe: Avail<u64>,
    pub vfs: Vec<Vf>,
}

/// One named counter line of an `aer_dev_*` file.
#[derive(Debug)]
pub struct NamedCounter {
    pub name: String,
    pub count: u64,
    /// 0 = correctable, 1 = non-fatal, 2 = fatal (file the line came from).
    pub kind: u8,
}

#[derive(Debug)]
pub struct Aer {
    pub correctable: Avail<u64>,
    pub nonfatal: Avail<u64>,
    pub fatal: Avail<u64>,
    /// Non-zero named counters, correctable first, then non-fatal, then fatal, file order.
    pub named: Vec<NamedCounter>,
}

#[derive(Debug)]
pub struct AerTotals {
    pub correctable: Avail<u64>,
    pub nonfatal: Avail<u64>,
    pub fatal: Avail<u64>,
}

#[derive(Debug)]
pub struct Placement {
    pub numa: Avail<i64>,
    pub local_cpus: Avail<String>,
    pub iommu_group: Avail<String>,
    /// Components after `…/devices/`: host bridge, bridges, the device. Never empty.
    pub path: Vec<String>,
    pub host_bridge: Avail<String>,
    pub root_port: Avail<String>,
    /// sysfs directory of `root_port` when one was identified (link and ASPM reads live there).
    pub root_port_dir: Option<PathBuf>,
    pub reset_methods: Avail<String>,
    pub sriov: Option<SriovPf>,
    /// For a VF: the PF address (from `physfn`).
    pub vf_of: Avail<String>,
    pub aer: Option<Aer>,
    pub root_port_aer: Option<AerTotals>,
}

fn na<T>() -> Avail<T> {
    Avail::NotAvailable(Reason::Missing(PathBuf::new()))
}

impl Default for Placement {
    fn default() -> Self {
        Placement {
            numa: na(),
            local_cpus: na(),
            iommu_group: na(),
            path: Vec::new(),
            host_bridge: na(),
            root_port: na(),
            root_port_dir: None,
            reset_methods: na(),
            sriov: None,
            vf_of: na(),
            aer: None,
            root_port_aer: None,
        }
    }
}

/// `…/devices/pci0000:16/0000:16:01.0/0000:17:00.0` → `["pci0000:16", "0000:16:01.0", "0000:17:00.0"]`.
/// A path without a `devices` ancestor yields the address alone (older flat captures).
pub fn path_components(dev_dir: &Path, pci: &str) -> Vec<String> {
    let s = dev_dir.to_string_lossy().into_owned();
    if let Some(rest) = s.rsplit_once("/devices/") {
        let comps: Vec<String> = rest
            .1
            .split('/')
            .filter(|c| !c.is_empty())
            .map(|c| c.to_string())
            .collect();
        if !comps.is_empty() {
            return comps;
        }
    }
    vec![pci.to_string()]
}

/// `name count` pairs with `TOTAL_ERR_*` pulled out as the total (spec/23 §2).
pub fn parse_aer_file(text: &str) -> (Vec<(String, u64)>, Option<u64>) {
    let mut pairs = Vec::new();
    let mut total = None;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(count)) = (it.next(), it.next()) else {
            continue;
        };
        let Ok(count) = count.parse::<u64>() else {
            continue;
        };
        if name.starts_with("TOTAL_ERR_") {
            total = Some(count);
        } else {
            pairs.push((name.to_string(), count));
        }
    }
    (pairs, total)
}

fn read_aer(dev_dir: &Path) -> Option<Aer> {
    let mut named: Vec<NamedCounter> = Vec::new();
    let mut totals: [Avail<u64>; 3] = [
        Avail::NotAvailable(Reason::Missing(dev_dir.join("aer_dev_correctable"))),
        Avail::NotAvailable(Reason::Missing(dev_dir.join("aer_dev_nonfatal"))),
        Avail::NotAvailable(Reason::Missing(dev_dir.join("aer_dev_fatal"))),
    ];
    let mut any = false;
    for (kind, file) in [
        (0u8, "aer_dev_correctable"),
        (1, "aer_dev_nonfatal"),
        (2, "aer_dev_fatal"),
    ] {
        if let Some(text) = sysfs::read_string(&dev_dir.join(file)).value() {
            any = true;
            let (pairs, total) = parse_aer_file(text);
            if let Some(t) = total {
                totals[kind as usize] = Avail::Value(t);
            }
            for (name, count) in pairs {
                if count != 0 {
                    named.push(NamedCounter { name, count, kind });
                }
            }
        }
    }
    if !any {
        return None;
    }
    Some(Aer {
        correctable: totals[0].clone(),
        nonfatal: totals[1].clone(),
        fatal: totals[2].clone(),
        named,
    })
}

fn is_bridge(dir: &Path) -> bool {
    sysfs::read_string(&dir.join("class"))
        .value()
        .map(|c| c.starts_with("0x0604"))
        .unwrap_or(false)
}

impl Placement {
    pub fn read(dev_dir: &Path, pci: &str) -> Placement {
        let path = path_components(dev_dir, pci);
        let host_bridge = path
            .first()
            .filter(|c| c.starts_with("pci"))
            .map(|c| Avail::Value(c.clone()))
            .unwrap_or_else(|| {
                Avail::NotAvailable(Reason::NotSupported("no ancestry in the device path"))
            });
        // root port: the component right after the host bridge, only when it is a bridge.
        let mut root_port: Avail<String> = Avail::NotAvailable(Reason::NotSupported(
            "no root port between the host bridge and the device",
        ));
        let mut rp_dir: Option<PathBuf> = None;
        if path.len() >= 3 {
            let mut dir = dev_dir.to_path_buf();
            for _ in 1..path.len() - 1 {
                dir = dir.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            }
            if is_bridge(&dir) {
                root_port = Avail::Value(path[1].clone());
                rp_dir = Some(dir);
            }
        }
        let sriov = if dev_dir.join("sriov_totalvfs").exists() {
            let mut vfs = Vec::new();
            let mut entries: Vec<(u32, PathBuf)> = sysfs::list_dir(dev_dir, |n| {
                n.strip_prefix("virtfn")
                    .is_some_and(|s| s.parse::<u32>().is_ok())
            })
            .into_iter()
            .filter_map(|p| {
                let n = p.file_name()?.to_string_lossy().into_owned();
                let idx = n.strip_prefix("virtfn")?.parse::<u32>().ok()?;
                Some((idx, p))
            })
            .collect();
            entries.sort_by_key(|(i, _)| *i);
            for (_, link) in entries {
                let vf_dir = sysfs::canonical(&link);
                if let Some(vd) = vf_dir.value() {
                    let addr = vd
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    vfs.push(Vf {
                        addr,
                        driver: sysfs::link_basename(&vd.join("driver")),
                    });
                }
            }
            Some(SriovPf {
                total: sysfs::read_u64(&dev_dir.join("sriov_totalvfs")),
                num: sysfs::read_u64(&dev_dir.join("sriov_numvfs")),
                autoprobe: sysfs::read_u64(&dev_dir.join("sriov_drivers_autoprobe")),
                vfs,
            })
        } else {
            None
        };
        let vf_of = match sysfs::link_basename(&dev_dir.join("physfn")).value() {
            Some(pf) => Avail::Value(pf.clone()),
            None => Avail::NotAvailable(Reason::Missing(dev_dir.join("physfn"))),
        };
        let root_port_aer = rp_dir.as_ref().and_then(|rd| {
            let cor = sysfs::read_u64(&rd.join("aer_rootport_total_err_cor"));
            let nonfatal = sysfs::read_u64(&rd.join("aer_rootport_total_err_nonfatal"));
            let fatal = sysfs::read_u64(&rd.join("aer_rootport_total_err_fatal"));
            if cor.value().is_some() || nonfatal.value().is_some() || fatal.value().is_some() {
                Some(AerTotals {
                    correctable: cor,
                    nonfatal,
                    fatal,
                })
            } else {
                None
            }
        });
        Placement {
            numa: sysfs::read_i64(&dev_dir.join("numa_node")),
            local_cpus: sysfs::read_string(&dev_dir.join("local_cpulist")),
            iommu_group: sysfs::link_basename(&dev_dir.join("iommu_group")),
            path,
            host_bridge,
            root_port,
            root_port_dir: rp_dir,
            reset_methods: sysfs::read_string(&dev_dir.join("reset_method")),
            sriov,
            vf_of,
            aer: read_aer(dev_dir),
            root_port_aer,
        }
    }

    pub fn numa_display(&self) -> Option<String> {
        self.numa
            .value()
            .filter(|n| **n >= 0)
            .map(|n| n.to_string())
    }

    /// Affinity relation between two xe devices (spec/22): PIX > PHB > NODE > SYS.
    pub fn relation(a: &Placement, b: &Placement) -> &'static str {
        if let (Some(x), Some(y)) = (a.root_port.value(), b.root_port.value()) {
            if x == y {
                return "PIX";
            }
        }
        if let (Some(x), Some(y)) = (a.host_bridge.value(), b.host_bridge.value()) {
            if x == y {
                return "PHB";
            }
        }
        match (a.numa.value(), b.numa.value()) {
            (Some(x), Some(y)) if *x >= 0 && *y >= 0 && x != y => "SYS",
            _ => "NODE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(numa: i64, rp: Option<&str>, hb: Option<&str>) -> Placement {
        Placement {
            numa: Avail::Value(numa),
            root_port: match rp {
                Some(r) => Avail::Value(r.into()),
                None => Avail::NotAvailable(Reason::Missing(PathBuf::from("x"))),
            },
            host_bridge: match hb {
                Some(h) => Avail::Value(h.into()),
                None => Avail::NotAvailable(Reason::Missing(PathBuf::from("x"))),
            },
            ..Default::default()
        }
    }
    #[test]
    fn affinity_classification() {
        let a = p(0, Some("0000:16:01.0"), Some("pci0000:16"));
        let same_rp = p(0, Some("0000:16:01.0"), Some("pci0000:16"));
        let other_rp = p(0, Some("0000:16:03.0"), Some("pci0000:16"));
        let other_hb = p(0, Some("0000:64:01.0"), Some("pci0000:64"));
        let far_hb = p(1, Some("0000:96:02.0"), Some("pci0000:94"));
        assert_eq!(Placement::relation(&a, &same_rp), "PIX");
        assert_eq!(Placement::relation(&a, &other_rp), "PHB");
        assert_eq!(Placement::relation(&a, &other_hb), "NODE"); // different HB, equal NUMA
        assert_eq!(Placement::relation(&a, &far_hb), "SYS"); // different NUMA
                                                             // unknown root ports, same host bridge still PHB
        let flat_a = p(0, None, Some("pci0000:e3"));
        let flat_b = p(0, None, Some("pci0000:e3"));
        assert_eq!(Placement::relation(&flat_a, &flat_b), "PHB");
        // unknown NUMA (-1) on another host bridge → NODE
        let unknown = p(-1, None, Some("pci0000:94"));
        assert_eq!(Placement::relation(&flat_a, &unknown), "NODE");
        // no host bridge info at all → NODE unless NUMA differs
        let stray = p(0, None, None);
        assert_eq!(Placement::relation(&flat_a, &stray), "NODE");
    }

    #[test]
    fn aer_parse_named_and_totals() {
        let text = "RxErr 2\nBadTLP 0\nTimeout 3\nTOTAL_ERR_COR 5\n";
        let (pairs, total) = parse_aer_file(text);
        assert_eq!(total, Some(5));
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], ("RxErr".into(), 2));
        assert_eq!(pairs[1], ("BadTLP".into(), 0));
        let (_, missing_total) = parse_aer_file("RxErr 1\n");
        assert_eq!(missing_total, None);
        let (garbage, _) = parse_aer_file("nonsense\nRxErr notanumber\n");
        assert!(garbage.is_empty());
    }

    #[test]
    fn path_components_split_at_devices() {
        let deep = Path::new("/sys/devices/pci0000:16/0000:16:01.0/0000:17:00.0");
        assert_eq!(
            path_components(deep, "0000:17:00.0"),
            ["pci0000:16", "0000:16:01.0", "0000:17:00.0"]
        );
        let flat = Path::new("/sys/bus/pci/devices/0000:03:00.0");
        assert_eq!(path_components(flat, "0000:03:00.0"), ["0000:03:00.0"]);
    }
}
