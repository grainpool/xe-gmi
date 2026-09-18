//! `doctor`: kernel, module, PCI, and capability diagnosis.

use crate::avail::Avail;
use crate::format::fields::mib;
use crate::paths::Roots;
use crate::pciids::{self, DG2_IDS};
use crate::persist;
use crate::probe::{self, Device};
use crate::proc_scan;
use crate::sysfs;
use std::path::PathBuf;

pub enum ModuleState {
    Loaded,
    AvailableNotLoaded,
    NotBuilt,
    Unknown,
}

pub struct Doctor {
    pub kernel: String,
    pub distro: Option<String>,
    pub id: Option<String>,
    pub version_id: Option<String>,
    pub module_state: ModuleState,
    pub module_path: Option<PathBuf>,
    pub initstate: String,
    pub gpus: Vec<GpuRow>,
    pub usable: Vec<Device>,
}

pub struct GpuRow {
    pub pci: String,
    pub vendor: u16,
    pub device: u16,
    pub driver: Option<String>,
    pub verdict: String,
    pub first_kernel: Option<&'static str>,
}

/// Battlemage first-tag table. `e215` existed in 6.15–6.16 only.
const BMG_FIRST: &[(&[u16], &str)] = &[
    (&[0xe202, 0xe20b, 0xe20c, 0xe20d, 0xe212], "6.11"),
    (&[0xe215, 0xe210, 0xe211, 0xe216], "6.15"),
    (&[0xe209], "6.17"),
    (&[0xe220, 0xe221, 0xe222, 0xe223], "6.17"),
];

fn bmg_first_kernel(device: u16) -> Option<&'static str> {
    BMG_FIRST
        .iter()
        .find(|(ids, _)| ids.contains(&device))
        .map(|(_, k)| *k)
}

fn kernel_maj_min(osrelease: &str) -> (u32, u32) {
    let mut it = osrelease.split('.');
    let maj = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let min = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (maj, min)
}

fn first_lt(first: &str, kernel: &str) -> bool {
    let (fm, fn_) = kernel_maj_min(first);
    let (km, kn) = kernel_maj_min(kernel);
    (km, kn) < (fm, fn_)
}

fn distro_hint(d: &Doctor, first: &str) -> String {
    match (d.id.as_deref(), d.version_id.as_deref()) {
        (Some("ubuntu"), Some("24.04")) => {
            "Ubuntu 24.04: sudo apt install linux-generic-hwe-24.04".into()
        }
        (Some("ubuntu"), _) => format!("Ubuntu: install a kernel >= {first}"),
        (Some("debian"), Some("13")) => {
            "Debian 13: install linux-image-amd64 from trixie-backports".into()
        }
        (Some("fedora"), _) => "Fedora: sudo dnf upgrade kernel".into(),
        _ => format!("install a kernel >= {first}"),
    }
}

pub fn collect(roots: &Roots) -> Doctor {
    let kernel = sysfs::read_string(&roots.procfs.join("sys/kernel/osrelease"))
        .value()
        .cloned()
        .unwrap_or_else(|| "unknown".into());
    let osr = sysfs::read_string(&roots.etc.join("os-release"))
        .value()
        .cloned();
    let field = |k: &str| {
        osr.as_ref().and_then(|t| {
            t.lines()
                .find(|l| l.starts_with(k))
                .and_then(|l| l.split_once('='))
                .map(|(_, v)| v.trim_matches('"').to_string())
        })
    };
    let distro = field("PRETTY_NAME");
    let id = field("ID");
    let version_id = field("VERSION_ID");

    let (module_state, module_path, initstate) =
        match sysfs::read_string(&roots.sysfs.join("module/xe/initstate"))
            .value()
            .cloned()
        {
            Some(state) => (ModuleState::Loaded, None, state),
            None => {
                let dir = roots
                    .modules
                    .join(&kernel)
                    .join("kernel/drivers/gpu/drm/xe");
                let ko = std::fs::read_dir(&dir)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                            p.file_name()
                                .map(|n| n.to_string_lossy().starts_with("xe.ko"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(None);
                match ko {
                    Some(p) => {
                        // Display as the production path (/lib/modules/...) regardless of the seam.
                        let rel = p.strip_prefix(&roots.modules).unwrap_or(&p);
                        let shown = PathBuf::from("/lib/modules").join(rel);
                        (ModuleState::AvailableNotLoaded, Some(shown), String::new())
                    }
                    None if dir.parent().is_some_and(|d| d.exists()) => {
                        (ModuleState::NotBuilt, None, String::new())
                    }
                    None => (ModuleState::Unknown, None, String::new()),
                }
            }
        };

    let usable = probe::discover(roots);

    let mut gpus = Vec::new();
    let base = roots.sysfs.join("bus/pci/devices");
    let mut addrs: Vec<String> = sysfs::list_dir(&base, |_| true)
        .into_iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    addrs.sort();
    for addr in addrs {
        let dir = base.join(&addr);
        let Some(class) = sysfs::read_string(&dir.join("class")).value().cloned() else {
            continue;
        };
        if !class.starts_with("0x03") {
            continue; // not a display controller
        }
        let parse_hex = |f: &str| {
            sysfs::read_string(&dir.join(f))
                .value()
                .and_then(|s| crate::probe::pci::parse_hex_u64(s))
                .unwrap_or(0) as u16
        };
        let (vendor, device) = (parse_hex("vendor"), parse_hex("device"));
        let driver = sysfs::link_basename(&dir.join("driver")).value().cloned();
        let (verdict, first_kernel) = verdict_for(
            roots,
            &kernel,
            &addr,
            &{
                Doctor {
                    kernel: kernel.clone(),
                    distro: distro.clone(),
                    id: id.clone(),
                    version_id: version_id.clone(),
                    module_state: ModuleState::Unknown,
                    module_path: None,
                    initstate: String::new(),
                    gpus: Vec::new(),
                    usable: Vec::new(),
                }
            },
            vendor,
            device,
            driver.as_deref(),
            &usable,
        );
        gpus.push(GpuRow {
            pci: addr,
            vendor,
            device,
            driver,
            verdict,
            first_kernel,
        });
    }
    Doctor {
        kernel,
        distro,
        id,
        version_id,
        module_state,
        module_path,
        initstate,
        gpus,
        usable,
    }
}

#[allow(clippy::too_many_arguments)]
fn verdict_for(
    roots: &Roots,
    kernel: &str,
    addr: &str,
    d: &Doctor,
    vendor: u16,
    device: u16,
    driver: Option<&str>,
    usable: &[Device],
) -> (String, Option<&'static str>) {
    if vendor != 0x8086 {
        return ("ignored (not an Intel GPU)".into(), None);
    }
    let name = pciids::device_name(roots, vendor, device);
    let dg2 = DG2_IDS.contains(&device) || [0x56c0u16, 0x56c1, 0x56c2].contains(&device);
    let first = bmg_first_kernel(device);
    if driver == Some("xe") {
        match usable.iter().find(|x| x.pci == addr) {
            Some(dev) => {
                let hw = dev.hwmon.as_ref().map(|h| h.index);
                let nodes = match (&dev.render, hw) {
                    (Some(r), Some(k)) => format!("usable ({}, {}, hwmon{})", dev.card, r, k),
                    (None, Some(k)) => format!("usable ({}, hwmon{})", dev.card, k),
                    _ => format!("usable ({})", dev.card),
                };
                return (format!("Intel {name}: {nodes}"), None);
            }
            None => return (format!("Intel {name}: bound to xe but not usable"), None),
        }
    }
    match driver {
        Some("i915") if dg2 => (
            format!("Intel {name}: xe needs xe.force_probe={device:x} and i915.force_probe=!{device:x} on every kernel through 7.2"),
            None,
        ),
        Some("i915") => (format!("Intel {name}: bound to i915; xe has no support for this device"), None),
        None => {
            if let Some(first) = first {
                if first_lt(first, kernel) {
                    (
                        format!(
                            "Intel {name}: PCI ID first supported by xe in kernel {first}; this kernel is {}.{}. {}",
                            kernel_maj_min(kernel).0, kernel_maj_min(kernel).1, distro_hint(d, first)
                        ),
                        Some(first),
                    )
                } else {
                    (
                        format!("Intel {name}: supported by xe since {first} but no driver is bound; check that the xe module is loaded (modprobe xe) and dmesg for probe errors"),
                        Some(first),
                    )
                }
            } else if dg2 {
                (
                    format!("Intel {name}: xe needs xe.force_probe={device:x} and i915.force_probe=!{device:x} on every kernel through 7.2"),
                    None,
                )
            } else {
                (format!("Intel GPU [8086:{device:04x}]: not in xe-gmi's ID table; if xe supports it, it binds automatically"), None)
            }
        }
        Some(other) => (format!("Intel {name}: bound to {other}; not an xe device"), None),
    }
}

fn cap_lines(roots: &Roots, dev: &Device, scan: &proc_scan::Scan) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let h = dev.hwmon.as_ref();
    let pl = match h {
        Some(h) => {
            let (card, pkg) = (&h.card, &h.pkg);
            match card.pl1.value() {
                Some(_) => "pl1 card writable (power1_max)".to_string(),
                None => match card.pl2.value() {
                    Some(_) => "pl2 card writable (power1_cap); pl1 not exposed".to_string(),
                    None => match pkg.pl1.value().or(pkg.pl2.value()) {
                        Some(_) => {
                            let kind = if pkg.pl1.value().is_some() { "pl1" } else { "pl2" };
                            let attr = if pkg.pl1.value().is_some() { "power2_max" } else { "power2_cap" };
                            format!("{kind} pkg writable ({attr})")
                        }
                        None => "not writable: power1_max/power1_cap not exposed (mailbox limits need kernel 6.16+, PL2 cap 6.17+)".to_string(),
                    },
                },
            }
        }
        None => "not writable: hwmon not exposed (kernel 6.15+)".to_string(),
    };
    out.push(("power limit", pl));
    let crit = h
        .and_then(|h| h.card.crit.value())
        .map(|uw| format!("{:.2} W (read-only in xe-gmi)", *uw as f64 / 1e6))
        .unwrap_or_else(|| "not exposed".into());
    out.push(("critical limit", crit));
    let energy = match h {
        Some(h) => match (h.energy_card.value(), h.energy_pkg.value()) {
            (Some(_), Some(_)) => "card, pkg",
            (Some(_), None) => "card",
            _ => "none",
        },
        None => "none",
    };
    out.push(("energy counters", energy.to_string()));
    let temps = match h {
        Some(h) if !h.temps.is_empty() => {
            let mut labels =
                collapse_runs(&h.temps.iter().map(|t| t.label.clone()).collect::<Vec<_>>());
            if h.temps
                .iter()
                .any(|t| t.crit_mc.is_some() || t.emergency_mc.is_some())
            {
                labels.push_str(" (with crit/emergency limits)");
            }
            labels
        }
        _ => "none exposed (kernel 6.15+)".into(),
    };
    out.push(("temperatures", temps));
    let fans = match h.map(|h| h.fans.len()) {
        Some(n) if n > 0 => format!("{n} (read-only)"),
        _ => "none exposed (kernel 6.16+)".into(),
    };
    out.push(("fans", fans));
    let clocks = if dev.gts.is_empty() {
        "none exposed".to_string()
    } else {
        let parts: Vec<String> = dev
            .gts
            .iter()
            .filter_map(|g| match (g.rpn.value(), g.rp0.value()) {
                (Some(rpn), Some(rp0)) => Some(format!("gt{} {}..{} MHz", g.id, rpn, rp0)),
                _ => None,
            })
            .collect();
        if parts.is_empty() {
            "none exposed".into()
        } else {
            format!("{} (writable)", parts.join(", "))
        }
    };
    out.push(("GT clocks", clocks));
    let profile = match dev
        .gts
        .first()
        .map(|g| sysfs::read_string(&g.dir.join("freq0/power_profile")))
    {
        Some(Avail::Value(text)) => {
            let tokens: Vec<&str> = text
                .split_whitespace()
                .map(|t| t.trim_matches(|c| c == '[' || c == ']'))
                .collect();
            format!("supported ({})", tokens.join(", "))
        }
        _ => "not supported (kernel 6.18+)".into(),
    };
    out.push(("power profile", profile));
    let throttle = match dev
        .gts
        .first()
        .map(|g| g.dir.join("freq0/throttle/reasons").exists())
    {
        Some(true) => "aggregate `reasons` attribute present",
        _ => "derived from reason_* files (aggregate attribute needs kernel 6.19+)",
    };
    out.push(("throttle reasons", throttle.to_string()));
    let vram = match dev.vram_total.value() {
        Some(t) => match t.source {
            crate::probe::VramSource::Bar => {
                format!("{} MiB (BAR aperture, {} bytes)", mib(t.bytes), t.bytes)
            }
            crate::probe::VramSource::Table => format!("{} MiB (SKU table)", mib(t.bytes)),
        },
        None => "unknown (BAR not resized and no SKU entry)".into(),
    };
    out.push(("VRAM total", vram));
    // Any visible client with `drm-total-cycles` proves it; otherwise the kernel version decides
    // (the attribute exists on every xe fdinfo since 6.11, ).
    let cycles = if scan.clients.iter().any(|c| {
        c.classes
            .iter()
            .any(|cl| cl.and_then(|x| x.total).is_some())
    }) || !first_lt(
        "6.11",
        &crate::sysfs::read_string(&roots.procfs.join("sys/kernel/osrelease"))
            .value()
            .cloned()
            .unwrap_or_default(),
    ) {
        "supported".to_string()
    } else {
        "not supported (kernel 6.11+)".to_string()
    };
    out.push(("fdinfo engine cycles", cycles));
    out.push((
        "persistence",
        if persist::installed(roots) {
            "installed"
        } else {
            "not installed"
        }
        .to_string(),
    ));
    if dev.kabi.vram().is_some() {
        out.push(("kernel memory regions", "available (render node)".into()));
        let mut fw: Vec<String> = Vec::new();
        if let Some(u) = dev.kabi.guc.value() {
            fw.push(format!("GuC {}.{}.{}", u.major, u.minor, u.patch));
        }
        if let Some(u) = dev.kabi.huc.value() {
            fw.push(format!("HuC {}.{}.{}", u.major, u.minor, u.patch));
        }
        if !fw.is_empty() {
            out.push(("firmware versions", fw.join(", ")));
        }
    }
    if let Some(nodes) = dev.kabi.ras.value() {
        out.push((
            "RAS counters",
            if nodes.is_empty() {
                "not registered (kernel 7.2+)".into()
            } else {
                format!("{} nodes", nodes.len())
            },
        ));
    }
    out.push((
        "reset methods",
        dev.placement
            .reset_methods
            .value()
            .cloned()
            .unwrap_or_else(|| "not exposed".into()),
    ));
    out.push(("SR-IOV", crate::format::hw::sriov_line(dev)));
    let caps = crate::cgroup::capacity(roots);
    out.push((
        "cgroup dmem",
        match caps {
            Some(rs) if !rs.is_empty() => format!(
                "present ({} capacity {} MiB)",
                rs[0].0,
                crate::format::fields::mib(rs[0].1)
            ),
            _ => "absent (no dmem controller in /sys/fs/cgroup)".into(),
        },
    ));
    out.push((
        "AER statistics",
        if dev.placement.aer.is_some() {
            "exposed".into()
        } else {
            "not exposed for this device".into()
        },
    ));
    let dumps = crate::crash::list_for_pci(roots, &dev.pci);
    let value = if dumps.is_empty() {
        "none pending".to_string()
    } else {
        let ids: Vec<String> = dumps.iter().map(|d| format!("devcd{}", d.id)).collect();
        format!(
            "{} pending ({}) \u{2014} run: xe-gmi crash save {}",
            dumps.len(),
            ids.join(", "),
            dumps[0].id
        )
    };
    out.push(("crash dumps", value));
    let cs = crate::sriov::connectors(roots, dev);
    out.push((
        "connectors",
        format!(
            "{} connected of {}",
            cs.iter().filter(|c| c.status == "connected").count(),
            cs.len()
        ),
    ));
    if let Some(kp) = &roots.kmsg {
        if std::fs::File::open(kp).is_err() {
            out.push((
                "kernel log (wedged)",
                "N/A (needs root to read the kernel log)".to_string(),
            ));
        } else if let Some(m) = crate::crash::wedged_message(roots, &dev.pci) {
            out.push(("kernel log (wedged)", m));
        }
    }
    out
}

fn split_label(s: &str) -> Option<(&str, usize)> {
    let k = s.rfind('_')?;
    let n: usize = s[k + 1..].parse().ok()?;
    Some((&s[..k], n))
}

/// `vram_ch_0, vram_ch_1,..., vram_ch_7` → `vram_ch_0..vram_ch_7` (runs of ≥3, ).
fn collapse_runs(labels: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < labels.len() {
        let split = split_label;
        if let Some((prefix, start)) = split(&labels[i]) {
            let mut j = i;
            while j + 1 < labels.len() {
                match split(&labels[j + 1]) {
                    Some((p, n)) if p == prefix && n == { j - i } + start + 1 => j += 1,
                    _ => break,
                }
            }
            if j > i + 1 {
                let end = &labels[j];
                out.push(format!("{prefix}_{start}..{end}"));
                i = j + 1;
                continue;
            }
        }
        out.push(labels[i].clone());
        i += 1;
    }
    out.join(", ")
}

fn kv(out: &mut String, indent: usize, label: &str, value: &str) {
    out.push_str(&format!("{:<32}: {}\n", " ".repeat(indent) + label, value));
}

pub fn render(d: &Doctor, roots: &Roots) -> String {
    let mut out = String::new();
    out.push_str(&format!("xe-gmi {} doctor\n", env!("CARGO_PKG_VERSION")));
    kv(&mut out, 0, "Kernel", &d.kernel);
    kv(
        &mut out,
        0,
        "Distro",
        d.distro.as_deref().unwrap_or(crate::format::NA),
    );
    let module = match &d.module_state {
        ModuleState::Loaded => format!("loaded ({})", d.initstate),
        ModuleState::AvailableNotLoaded => format!(
            "available for this kernel but not loaded ({})",
            d.module_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ),
        ModuleState::NotBuilt => format!(
            "not built for this kernel (no xe.ko under {}/kernel/drivers/gpu/drm/xe)",
            roots.modules.join(&d.kernel).display()
        ),
        ModuleState::Unknown => "unknown".into(),
    };
    kv(&mut out, 0, "xe module", &module);
    out.push_str("GPUs on the PCI bus\n");
    if d.gpus.is_empty() {
        out.push_str("    (none)\n");
    }
    for g in &d.gpus {
        let driver_col = match &g.driver {
            Some(dr) => format!("driver {dr}"),
            None => "no driver".to_string(),
        };
        out.push_str(&format!(
            "    {}  {:04x}:{:04x}  {:<16}{}\n",
            g.pci, g.vendor, g.device, driver_col, g.verdict
        ));
    }
    kv(
        &mut out,
        0,
        "Usable xe devices",
        &d.usable.len().to_string(),
    );
    let usable_pci: Vec<&str> = d.usable.iter().map(|x| x.pci.as_str()).collect();
    let scan = proc_scan::scan(roots, &usable_pci);
    for dev in &d.usable {
        out.push_str(&format!("Capabilities of {}\n", dev.pci));
        for (label, value) in cap_lines(roots, dev, &scan) {
            kv(&mut out, 4, label, &value);
        }
    }
    if d.usable.is_empty() {
        kv(&mut out, 0, "Result", "no usable xe device (exit 3)");
    } else {
        kv(&mut out, 0, "Result", "ok (exit 0)");
    }
    out
}

pub fn render_json(d: &Doctor, roots: &Roots, ts: &str) -> String {
    use crate::format::json::{num_u, Json};
    let state = match d.module_state {
        ModuleState::Loaded => "loaded",
        ModuleState::AvailableNotLoaded => "available_not_loaded",
        ModuleState::NotBuilt => "not_built",
        ModuleState::Unknown => "unknown",
    };
    let gpus: Vec<Json> = d
        .gpus
        .iter()
        .map(|g| {
            Json::Obj(vec![
                ("pci_address".into(), Json::Str(g.pci.clone())),
                ("vendor_id".into(), Json::Str(format!("{:04x}", g.vendor))),
                ("device_id".into(), Json::Str(format!("{:04x}", g.device))),
                (
                    "driver".into(),
                    g.driver.clone().map(Json::Str).unwrap_or(Json::Null),
                ),
                ("verdict".into(), Json::Str(g.verdict.clone())),
                (
                    "first_kernel".into(),
                    g.first_kernel
                        .map(|k| Json::Str(k.into()))
                        .unwrap_or(Json::Null),
                ),
            ])
        })
        .collect();
    let usable_pci: Vec<&str> = d.usable.iter().map(|x| x.pci.as_str()).collect();
    let scan = proc_scan::scan(roots, &usable_pci);
    let caps: Vec<Json> = d
        .usable
        .iter()
        .map(|dev| {
            let mut v = vec![("pci_address".to_string(), Json::Str(dev.pci.clone()))];
            for (label, value) in cap_lines(roots, dev, &scan) {
                v.push((label.replace(' ', "_"), Json::Str(value)));
            }
            Json::Obj(v)
        })
        .collect();
    let j = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("generated_at".into(), Json::Str(ts.into())),
        ("kernel".into(), Json::Str(d.kernel.clone())),
        (
            "distro".into(),
            d.distro.clone().map(Json::Str).unwrap_or(Json::Null),
        ),
        (
            "xe_module".into(),
            Json::Obj(vec![
                ("state".into(), Json::Str(state.into())),
                (
                    "path".into(),
                    d.module_path
                        .as_ref()
                        .map(|p| Json::Str(p.display().to_string()))
                        .unwrap_or(Json::Null),
                ),
            ]),
        ),
        ("gpus".into(), Json::Arr(gpus)),
        ("usable_xe_devices".into(), num_u(d.usable.len() as u64)),
        ("capabilities".into(), Json::Arr(caps)),
        (
            "result".into(),
            Json::Str(
                if d.usable.is_empty() {
                    "no usable xe device"
                } else {
                    "ok"
                }
                .into(),
            ),
        ),
    ]);
    let mut s = String::new();
    crate::format::json::write(&j, &mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::{collapse_runs, first_lt, kernel_maj_min};

    #[test]
    fn collapse_vram_channels() {
        let labels: Vec<String> = ["pkg", "vram", "mctrl", "pcie"]
            .iter()
            .map(|s| s.to_string())
            .chain((0..8).map(|i| format!("vram_ch_{i}")))
            .collect();
        assert_eq!(
            collapse_runs(&labels),
            "pkg, vram, mctrl, pcie, vram_ch_0..vram_ch_7"
        );
        let two: Vec<String> = (0..2).map(|i| format!("vram_ch_{i}")).collect();
        assert_eq!(collapse_runs(&two), "vram_ch_0, vram_ch_1");
    }

    #[test]
    fn kernel_ordering() {
        assert!(first_lt("6.17", "6.8.0-79-generic"));
        assert!(!first_lt("6.17", "7.1.4-200.fc43.x86_64"));
        assert!(first_lt("6.11", "6.10.0"));
    }

    #[test]
    fn kernel_version_parse() {
        assert_eq!(kernel_maj_min("6.8.0-79-generic"), (6, 8));
        assert_eq!(kernel_maj_min("7.1.4-200.fc43.x86_64"), (7, 1));
        assert_eq!(kernel_maj_min("6.12.43+deb13-amd64"), (6, 12));
        assert_eq!(kernel_maj_min("7.0.9-arch1-1"), (7, 0));
        assert_eq!(kernel_maj_min(""), (0, 0));
        assert_eq!(kernel_maj_min("garbage"), (0, 0));
    }
}
