//! `info` report and the three `get` renderers.

use crate::avail::{Avail, Reason};
use crate::cli::Section;
use crate::format::fields::{self, mc_to_c, mib, View};
use crate::format::table::pair;
use crate::format::NA;
use crate::paths::Roots;
use crate::persist;
use crate::probe::gt::GtKind;
use crate::probe::{Device, VramSource};

pub struct Report<'a> {
    pub views: Vec<(&'a Device, View<'a>)>,
    pub timestamp: String,
    pub kernel: String,
    pub initstate: String,
    pub visible_all_users: bool,
    pub roots: &'a Roots,
    pub verbose: bool,
    pub sections: Vec<Section>,
}

impl Report<'_> {
    fn want(&self, s: Section) -> bool {
        self.sections.is_empty()
            || self.sections.contains(&Section::All)
            || self.sections.contains(&s)
    }
}

pub(crate) fn kv(out: &mut String, indent: usize, label: &str, value: &str) {
    out.push_str(&format!("{:<32}: {}\n", " ".repeat(indent) + label, value));
}

fn kv_av(out: &mut String, indent: usize, label: &str, value: Option<String>, verbose: bool) {
    match value {
        Some(v) => kv(out, indent, label, &v),
        None => {
            kv(out, indent, label, NA);
            let _ = verbose;
        }
    }
}

fn kv_av_why(
    out: &mut String,
    indent: usize,
    label: &str,
    value: Option<String>,
    why: Option<&Reason>,
    field: &str,
    verbose: bool,
) {
    match value {
        Some(v) => kv(out, indent, label, &v),
        None => {
            kv(out, indent, label, NA);
            if verbose {
                if let Some(r) = why {
                    match super::fields::def(field) {
                        Some(d) => out.push_str(&format!(
                            "{}(why: {r}) [source: {}, scope: {}, quality: {}]\n",
                            " ".repeat(indent + 4),
                            d.prov.source,
                            d.prov.scope,
                            d.prov.quality
                        )),
                        None => {
                            out.push_str(&format!("{}(why: {r})\n", " ".repeat(indent + 4)));
                        }
                    }
                }
            }
        }
    }
}

/// const-constructible so N/A why lines on the effective limit can borrow it.
static NO_LIMIT_REASON: Reason = Reason::NotSupported(crate::probe::hwmon::NO_LIMIT_MAILBOX);

fn s_avail(a: &Avail<String>) -> Option<String> {
    a.value().cloned()
}

fn w_avail(a: &Avail<u64>) -> Option<String> {
    a.value().map(|v| format!("{:.2} W", *v as f64 / 1e6))
}

fn w_text(uw: u64) -> String {
    format!("{:.2} W", uw as f64 / 1e6)
}

pub fn render(r: &Report<'_>) -> String {
    let mut out = String::new();
    out.push_str(&format!("xe-gmi {} report\n", env!("CARGO_PKG_VERSION")));
    kv(&mut out, 0, "Timestamp", &r.timestamp);
    kv(&mut out, 0, "Kernel", &r.kernel);
    kv(
        &mut out,
        0,
        "Driver",
        &format!("xe (module {})", r.initstate),
    );
    kv(&mut out, 0, "Devices", &r.views.len().to_string());
    for (dev, view) in &r.views {
        out.push_str(&format!("\nGPU {} [{}]\n", dev.index, dev.pci));
        device_block(&mut out, r, dev, view);
        if r.want(Section::Thermal) {
            thermal_block(&mut out, dev);
        }
        if r.want(Section::Power) {
            power_block(&mut out, r, dev, view);
        }
        if r.want(Section::Memory) {
            memory_block(&mut out, r, dev, view);
        }
        if r.want(Section::Clocks) {
            for gt in &dev.gts {
                gt_block(&mut out, r, view, gt);
            }
        } else if r.want(Section::Throttle) {
            for gt in &dev.gts {
                out.push_str(&format!("    {}\n", gt_header(gt)));
                let reasons = match &gt.throttle_reasons {
                    Avail::Value(rs) if rs.is_empty() => Some("none".to_string()),
                    Avail::Value(rs) => Some(rs.join(",")),
                    Avail::NotAvailable(_) => None,
                };
                kv_av_why(
                    &mut out,
                    8,
                    "Throttle",
                    reasons,
                    reason_of(&gt.throttle_reasons),
                    "throttle.reasons",
                    r.verbose,
                );
            }
        }
        if r.want(Section::Engines) {
            fans_block(&mut out, dev);
            engines_block(&mut out, view);
        }
        if r.want(Section::Processes) {
            let n = view.used_clients;
            let v = if r.visible_all_users {
                format!("{n} clients")
            } else {
                format!("{n} visible clients (run as root to see every user)")
            };
            kv(&mut out, 4, "Processes", &v);
        }
        if r.want(Section::Topology) {
            super::hw::info_topology(&mut out, dev);
        }
        if r.want(Section::Pcie) {
            super::hw::info_pcie_health(&mut out, dev);
        }
        if r.want(Section::Connectors) {
            let cs = crate::sriov::connectors(r.roots, dev);
            if !cs.is_empty() {
                out.push_str("    Connectors\n");
                for c in &cs {
                    let v = match c.status.as_str() {
                        "connected" if c.enabled => {
                            format!("connected, enabled, {} modes", c.modes)
                        }
                        "connected" => "connected, disabled".into(),
                        other => other.to_string(),
                    };
                    kv(&mut out, 8, &c.name, &v);
                }
            }
        }
        if r.want(Section::Sriov) {
            match dev.placement.sriov.as_ref() {
                Some(pf) => {
                    out.push_str("    SR-IOV\n");
                    let t = pf.total.value().copied().unwrap_or(0);
                    let n = pf.num.value().copied().unwrap_or(0);
                    kv(&mut out, 8, "VFs", &format!("{n} of {t} enabled"));
                }
                None => {
                    out.push_str("    SR-IOV\n");
                    kv(&mut out, 8, "VFs", NA);
                }
            }
        }
        if r.want(Section::Crash) {
            let dumps = crate::crash::list_for_pci(r.roots, &dev.pci);
            let v = if dumps.is_empty() {
                "none pending".to_string()
            } else {
                let ids: Vec<String> = dumps.iter().map(|x| format!("devcd{}", x.id)).collect();
                format!("{} pending ({})", dumps.len(), ids.join(", "))
            };
            kv(&mut out, 4, "Crash dumps", &v);
        }
        if r.want(Section::Firmware)
            && (dev.kabi.guc.value().is_some() || dev.kabi.huc.value().is_some())
        {
            out.push_str("    Firmware\n");
            if let Some(u) = dev.kabi.guc.value() {
                kv(
                    &mut out,
                    8,
                    "GuC (submission)",
                    &format!("{}.{}.{}", u.major, u.minor, u.patch),
                );
            }
            if let Some(u) = dev.kabi.huc.value() {
                kv(
                    &mut out,
                    8,
                    "HuC",
                    &format!("{}.{}.{}", u.major, u.minor, u.patch),
                );
            }
        }
        if r.want(Section::Hardware) {
            if let Some(lines) = super::hw::hardware_lines(dev) {
                out.push_str("    Hardware\n");
                out.push_str(&lines);
            }
        }
    }
    out
}

fn device_block(out: &mut String, r: &Report<'_>, dev: &Device, view: &View<'_>) {
    if r.want(Section::Device) {
        kv(out, 4, "Product name", &dev.name);
        let mut extras: Vec<String> = Vec::new();
        if let Some((a, b)) = dev.ids.subsystem.value() {
            extras.push(format!("subsystem {a:04x}:{b:04x}"));
        }
        if let Some(rev) = dev.ids.revision.value() {
            extras.push(format!("revision {rev:02x}"));
        }
        let pci_id = format!("{:04x}:{:04x}", dev.ids.vendor, dev.ids.device);
        let v = if extras.is_empty() {
            pci_id
        } else {
            format!("{pci_id} ({})", extras.join(", "))
        };
        kv(out, 4, "PCI id", &v);
        let drm = match &dev.render {
            Some(rd) => format!("{}, {}", dev.card, rd),
            None => dev.card.clone(),
        };
        kv(out, 4, "DRM nodes", &drm);
    }
    if r.want(Section::Pcie) {
        match dev.link.value() {
            Some(l) => {
                let g = |o: Option<u8>| o.map(|v| v.to_string()).unwrap_or_else(|| NA.into());
                let w = |o: Option<u32>| o.map(|v| v.to_string()).unwrap_or_else(|| NA.into());
                kv(
                    out,
                    4,
                    "PCIe link",
                    &format!(
                        "gen{} x{} (max gen{} x{})",
                        g(l.gen_cur),
                        w(l.width_cur),
                        g(l.gen_max),
                        w(l.width_max)
                    ),
                );
            }
            None => kv_av(out, 4, "PCIe link", None, r.verbose),
        }
    }
    if r.want(Section::Device) {
        kv(
            out,
            4,
            "Persistence",
            if view.persistence {
                "installed"
            } else {
                "not installed"
            },
        );
        if let Some(c) = dev.kabi.config.value() {
            kv(out, 8, "VA bits", &c.va_bits.to_string());
            let align = if c.min_alignment >= 1024 * 1024 {
                format!("{} MiB", c.min_alignment / (1024 * 1024))
            } else {
                format!("{} KiB", c.min_alignment / 1024)
            };
            kv(out, 8, "Min alignment", &align);
        }
    }
}

fn thermal_block(out: &mut String, dev: &Device) {
    out.push_str("    Thermal\n");
    let Some(h) = &dev.hwmon else {
        out.push_str("        (none exposed)\n");
        return;
    };
    if h.temps.is_empty() {
        out.push_str("        (none exposed)\n");
        return;
    }
    for t in &h.temps {
        let mut limits: Vec<String> = Vec::new();
        if let Some(m) = t.max_mc {
            limits.push(format!("max {}", mc_to_c(m)));
        }
        if let Some(c) = t.crit_mc {
            limits.push(format!("crit {}", mc_to_c(c)));
        }
        if let Some(e) = t.emergency_mc {
            limits.push(format!("emergency {}", mc_to_c(e)));
        }
        let v = format!("{} C", mc_to_c(t.input_mc));
        let v = if limits.is_empty() {
            v
        } else {
            format!("{v} ({})", limits.join(", "))
        };
        kv(out, 8, &t.label, &v);
    }
}

fn power_block(out: &mut String, r: &Report<'_>, dev: &Device, view: &View<'_>) {
    out.push_str("    Power\n");
    let ms = view.rates.map(|x| x.dt_ms).unwrap_or(0);
    let energy2 = dev
        .hwmon
        .as_ref()
        .is_some_and(|h| h.dir.join("energy2_input").exists());
    let draw = |a: &Avail<f64>| {
        a.value()
            .map(|v| format!("{v:.2} W (energy delta over {ms} ms)"))
    };
    if let Some(rates) = view.rates {
        kv_av_why(
            out,
            8,
            "Draw card",
            draw(&rates.draw_card_w),
            reason_of(&rates.draw_card_w),
            "power.draw",
            r.verbose,
        );
        if energy2 {
            kv_av_why(
                out,
                8,
                "Draw pkg",
                draw(&rates.draw_pkg_w),
                reason_of(&rates.draw_pkg_w),
                "power.draw.pkg",
                r.verbose,
            );
        }
    } else {
        kv(out, 8, "Draw card", NA);
        if energy2 {
            kv(out, 8, "Draw pkg", NA);
        }
    }
    let eff = fields::effective_of(dev);
    let limit = eff
        .as_ref()
        .map(|(uw, kind, chan)| format!("{:.2} W ({kind} {chan})", *uw as f64 / 1e6));
    kv_av_why(
        out,
        8,
        "Limit",
        limit,
        if eff.is_none() {
            Some(&NO_LIMIT_REASON)
        } else {
            None
        },
        "power.limit",
        r.verbose,
    );
    if let Some(h) = &dev.hwmon {
        channel_lines(out, r, &h.card, 1, "card", true);
        if h.dir.join("power2_label").exists() {
            channel_lines(out, r, &h.pkg, 2, "pkg", false);
        }
        if h.dir.join("power1_crit").exists() {
            let v = w_avail(&h.card.crit);
            kv_av_why(
                out,
                8,
                "Critical card",
                v,
                reason_of(&h.card.crit),
                "power.crit",
                r.verbose,
            );
        }
        if h.dir.join("power1_rated_max").exists() {
            let v = w_avail(&h.card.rated_max);
            kv_av_why(
                out,
                8,
                "Rated max card",
                v,
                reason_of(&h.card.rated_max),
                "power.rated_max",
                r.verbose,
            );
        }
        if h.curr_crit_ma.value().is_some() {
            let v = h.curr_crit_ma.value().map(|v| format!("{v} mA"));
            kv(out, 8, "Critical current card", &v.unwrap());
        }
        let e_card = match view.rates {
            Some(x) => x.energy_card.clone(),
            None => h.energy_card.clone(),
        };
        let e_pkg = match view.rates {
            Some(x) => x.energy_pkg.clone(),
            None => h.energy_pkg.clone(),
        };
        let e1 = e_card.value().map(|v| format!("{:.3} J", *v as f64 / 1e6));
        kv_av_why(
            out,
            8,
            "Energy card",
            e1,
            reason_of(&e_card),
            "energy.card",
            r.verbose,
        );
        if energy2 {
            let e2 = e_pkg.value().map(|v| format!("{:.3} J", *v as f64 / 1e6));
            kv_av_why(
                out,
                8,
                "Energy pkg",
                e2,
                reason_of(&e_pkg),
                "energy.pkg",
                r.verbose,
            );
        }
        if let Some((label, mv)) = h.voltage.first() {
            let name = if label == "in0" {
                "Voltage card"
            } else {
                "Voltage pkg"
            };
            kv(out, 8, name, &format!("{mv} mV"));
        }
    } else {
        for label in [
            "Limit",
            "PL1 sustained card",
            "PL2 burst card",
            "Energy card",
        ] {
            kv(out, 8, label, NA);
        }
    }
}

fn reason_of<T>(a: &Avail<T>) -> Option<&Reason> {
    match a {
        Avail::Value(_) => None,
        Avail::NotAvailable(r) => Some(r),
    }
}

fn channel_lines(
    out: &mut String,
    r: &Report<'_>,
    ch: &crate::probe::hwmon::Channel,
    n: u32,
    chan: &str,
    windows: bool,
) {
    let mut line = |pl: &Avail<u64>, win: &Avail<u64>, label: &str, field: &str| {
        let v = pl.value().map(|uw| {
            let t = w_text(*uw);
            if windows {
                if let Some(w) = win.value() {
                    return format!("{t} (window {w} ms)");
                }
            }
            t
        });
        let why = pl_not_exposed_reason(ch, pl, n);
        kv_av_why(out, 8, label, v, why, field, r.verbose);
    };
    let name = |pl: &str| {
        if chan == "card" {
            format!("power.limit.{pl}")
        } else {
            format!("power.limit.{pl}.{chan}")
        }
    };
    line(
        &ch.pl1,
        &ch.pl1_window_ms,
        &format!("PL1 sustained {chan}"),
        &name("pl1"),
    );
    line(
        &ch.pl2,
        &ch.pl2_window_ms,
        &format!("PL2 burst {chan}"),
        &name("pl2"),
    );
}

/// For verbose why lines on channel limits: the raw per-file reason unless the probe already
/// recorded the firmware-disabled text.
fn pl_not_exposed_reason<'a>(
    _ch: &crate::probe::hwmon::Channel,
    pl: &'a Avail<u64>,
    _n: u32,
) -> Option<&'a Reason> {
    reason_of(pl)
}

fn memory_block(out: &mut String, r: &Report<'_>, dev: &Device, view: &View<'_>) {
    out.push_str("    Memory\n");
    if let Some(k) = dev.kabi.vram() {
        if k.used > 0 || k.total == 0 || view.used_bytes == 0 {
            kv(
                out,
                8,
                "Total",
                &format!("{} MiB (kernel memory regions)", mib(k.total)),
            );
            kv(
                out,
                8,
                "Used",
                &format!(
                    "{} MiB (kernel allocator; {} MiB resident in {} clients)",
                    mib(k.used),
                    mib(view.used_bytes),
                    view.used_clients
                ),
            );
            kv(
                out,
                8,
                "Free",
                &format!("{} MiB", mib(k.total).saturating_sub(mib(k.used))),
            );
            kv(
                out,
                8,
                "CPU-visible",
                &format!(
                    "{} MiB total, {} MiB used ({})",
                    mib(k.cpu_visible),
                    mib(k.cpu_visible_used),
                    if k.cpu_visible < k.total {
                        "small BAR"
                    } else {
                        "full BAR"
                    }
                ),
            );
            kv(
                out,
                8,
                "Min page size",
                &format!("{} KiB", k.min_page_size / 1024),
            );
            return;
        }
    }
    let total = dev.vram_total.value().map(|t| {
        let src = match t.source {
            VramSource::Bar => "BAR aperture",
            VramSource::Table => "SKU table",
        };
        format!("{} MiB ({src})", mib(t.bytes))
    });
    kv_av(out, 8, "Total", total, r.verbose);
    let used = if r.visible_all_users {
        format!(
            "{} MiB (resident VRAM of {} clients)",
            mib(view.used_bytes),
            view.used_clients
        )
    } else {
        format!(
            "{} MiB (resident VRAM of {} visible clients; run as root to see every user)",
            mib(view.used_bytes),
            view.used_clients
        )
    };
    let used = if r.verbose {
        let p = super::fields::def("memory.used").expect("catalog").prov;
        format!(
            "{used} [source: {}, scope: {}, quality: {}]",
            p.source, p.scope, p.quality
        )
    } else {
        used
    };
    kv(out, 8, "Used", &used);
    let free = dev
        .vram_total
        .value()
        .map(|t| format!("{} MiB", mib(t.bytes).saturating_sub(mib(view.used_bytes))));
    kv_av(out, 8, "Free", free, r.verbose);
}

fn gt_header(gt: &crate::probe::gt::Gt) -> String {
    let name = gt
        .idle_name
        .value()
        .cloned()
        .unwrap_or_else(|| format!("gt{}", gt.id));
    format!("GT {} (tile {}, {})", gt.id, gt.tile, name)
}

fn gt_block(out: &mut String, r: &Report<'_>, view: &View<'_>, gt: &crate::probe::gt::Gt) {
    out.push_str(&format!("    {}\n", gt_header(gt)));
    let mhz = |a: &Avail<u32>| a.value().map(|v| v.to_string());
    let dt = view.rates.map(|x| x.dt_ms).unwrap_or(0);
    kv(
        out,
        8,
        "Clock cur / act",
        &format!("{} MHz", pair(mhz(&gt.cur), mhz(&gt.act))),
    );
    kv(
        out,
        8,
        "Clock min / max",
        &format!("{} MHz", pair(mhz(&gt.min), mhz(&gt.max))),
    );
    let quad = [mhz(&gt.rpn), mhz(&gt.rpe), mhz(&gt.rpa), mhz(&gt.rp0)]
        .map(|o| o.unwrap_or_else(|| NA.to_string()))
        .join(" / ");
    kv(out, 8, "Clock rpn/rpe/rpa/rp0", &format!("{quad} MHz"));
    let idx = view.dev.gts.iter().position(|g| g.id == gt.id).unwrap_or(0);
    let util = match view.rates.and_then(|x| x.gt_util_pct.get(idx)) {
        Some(Avail::Value(v)) => Some(format!("{v:.1} % (active time over {dt} ms)")),
        Some(Avail::NotAvailable(_)) | None => None,
    };
    let why = view
        .rates
        .and_then(|x| x.gt_util_pct.get(idx))
        .and_then(reason_of);
    kv_av_why(
        out,
        8,
        "Utilization",
        util,
        why,
        "utilization.gt",
        r.verbose,
    );
    kv_av(out, 8, "Idle status", s_avail(&gt.idle_status), r.verbose);
    kv_av(out, 8, "Power profile", s_avail(&gt.profile), r.verbose);
    let reasons = match &gt.throttle_reasons {
        Avail::Value(rs) if rs.is_empty() => Some("none".to_string()),
        Avail::Value(rs) => Some(rs.join(",")),
        Avail::NotAvailable(_) => None,
    };
    kv_av_why(
        out,
        8,
        "Throttle",
        reasons.clone(),
        reason_of(&gt.throttle_reasons),
        "throttle.reasons",
        r.verbose,
    );
}

fn fans_block(out: &mut String, dev: &Device) {
    out.push_str("    Fans\n");
    match dev.hwmon.as_ref().map(|h| &h.fans) {
        Some(fans) if !fans.is_empty() => {
            for (id, rpm) in fans {
                let v = rpm.value().map(|v| format!("{v} RPM"));
                kv_av(out, 8, &format!("fan{id}"), v, true);
            }
        }
        _ => out.push_str("        (none exposed)\n"),
    }
}

fn engines_block(out: &mut String, view: &View<'_>) {
    out.push_str("    Engines\n");
    let e = view.rates.map(|r| &r.engine_pct);
    let pct = |a: Option<&Avail<f64>>| a.and_then(|x| x.value()).map(|v| format!("{v:.1} %"));
    kv(
        out,
        8,
        "render (rcs)",
        &pct(e.map(|x| &x.rcs)).unwrap_or_else(|| NA.into()),
    );
    kv(
        out,
        8,
        "compute (ccs)",
        &pct(e.map(|x| &x.ccs)).unwrap_or_else(|| NA.into()),
    );
    kv(
        out,
        8,
        "video (vcs)",
        &pct(e.map(|x| &x.vcs)).unwrap_or_else(|| NA.into()),
    );
    kv(
        out,
        8,
        "enhance (vecs)",
        &pct(e.map(|x| &x.vecs)).unwrap_or_else(|| NA.into()),
    );
    kv(
        out,
        8,
        "copy (bcs)",
        &pct(e.map(|x| &x.bcs)).unwrap_or_else(|| NA.into()),
    );
}

// --------------------------------------------------------------------------- `get`

pub fn get_clocks(dev: &Device, recs: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str(&format!("GPU {} [{}]\n", dev.index, dev.pci));
    out.push_str(&get_clocks_columns(
        "GT",
        "Type",
        "Min",
        "Max",
        "RPn",
        "RPe",
        "RPa",
        "RP0",
        "Boot default",
    ));
    for gt in &dev.gts {
        let kind = match gt.kind {
            GtKind::Render => "render",
            GtKind::Media => "media",
            GtKind::Unknown => NA,
        };
        let boot = match (
            persist::boot_record(recs, &format!("gt{}.min_freq", gt.id)),
            persist::boot_record(recs, &format!("gt{}.max_freq", gt.id)),
        ) {
            (Some(min), Some(max)) => format!("{min}/{max}"),
            _ => "not recorded".to_string(),
        };
        out.push_str(&get_clocks_columns(
            &format!("gt{}", gt.id),
            kind,
            &gt.min
                .value()
                .map(|v| v.to_string())
                .unwrap_or_else(|| NA.into()),
            &gt.max
                .value()
                .map(|v| v.to_string())
                .unwrap_or_else(|| NA.into()),
            &gt.rpn
                .value()
                .map(|v| v.to_string())
                .unwrap_or_else(|| NA.into()),
            &gt.rpe
                .value()
                .map(|v| v.to_string())
                .unwrap_or_else(|| NA.into()),
            &gt.rpa
                .value()
                .map(|v| v.to_string())
                .unwrap_or_else(|| NA.into()),
            &gt.rp0
                .value()
                .map(|v| v.to_string())
                .unwrap_or_else(|| NA.into()),
            &boot,
        ));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn get_clocks_columns(
    gt: &str,
    kind: &str,
    min: &str,
    max: &str,
    rpn: &str,
    rpe: &str,
    rpa: &str,
    rp0: &str,
    boot: &str,
) -> String {
    format!("  {gt:<3}  {kind:<6}  {min:<4}  {max:<4}  {rpn:<4}  {rpe:<4}  {rpa:<4}  {rp0:<4}  {boot}\n")
}

fn get_line(label: &str, v: &str) -> String {
    format!("  {:<16}: {v}\n", label)
}

fn uw_text(uw: &u64) -> String {
    format!("{:.2}", *uw as f64 / 1e6)
}

fn pl_not_exposed(n: u32, attr: &str) -> String {
    format!("power{n}_{attr} not exposed")
}

pub fn get_power_limit(dev: &Device, recs: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str(&format!("GPU {} [{}]\n", dev.index, dev.pci));
    let mut push = |label: &str, v: &str| out.push_str(&get_line(label, v));
    let ch = dev.hwmon.as_ref();
    let eff = fields::effective_of(dev);
    match &eff {
        Some((uw, kind, chan)) => push("effective", &format!("{} W ({kind} {chan})", uw_text(uw))),
        None => push(
            "effective",
            &format!("{} ({})", NA, crate::probe::hwmon::NO_LIMIT_MAILBOX),
        ),
    }
    let pl_value = |pl: &Avail<u64>, win: Option<&Avail<u64>>, n: u32, attr: &str| -> String {
        match pl {
            Avail::Value(uw) => {
                let mut t = format!("{} W", uw_text(uw));
                if n == 1 {
                    if let Some(Some(w)) = win.map(|w| w.value()) {
                        t = format!("{t} (window {w} ms)");
                    }
                }
                t
            }
            Avail::NotAvailable(Reason::NotSupported(t)) if t.contains("firmware") => {
                format!("{NA} ({t})")
            }
            Avail::NotAvailable(_) => format!("{NA} ({})", pl_not_exposed(n, attr)),
        }
    };
    if let Some(h) = ch {
        push(
            "pl1 card",
            &pl_value(&h.card.pl1, Some(&h.card.pl1_window_ms), 1, "max"),
        );
        push(
            "pl2 card",
            &pl_value(&h.card.pl2, Some(&h.card.pl2_window_ms), 1, "cap"),
        );
        push("pl1 pkg", &pl_value(&h.pkg.pl1, None, 2, "max"));
        push("pl2 pkg", &pl_value(&h.pkg.pl2, None, 2, "cap"));
        if h.dir.join("power1_crit").exists() {
            let v = h
                .card
                .crit
                .value()
                .map(|uw| format!("{} W", uw_text(uw)))
                .unwrap_or_else(|| NA.to_string());
            push("critical card", &v);
        }
        if h.dir.join("power1_rated_max").exists() {
            let v = h
                .card
                .rated_max
                .value()
                .map(|uw| format!("{} W", uw_text(uw)))
                .unwrap_or_else(|| NA.to_string());
            push("rated max card", &v);
        }
    } else {
        for (label, n, attr) in [
            ("pl1 card", 1u32, "max"),
            ("pl2 card", 1, "cap"),
            ("pl1 pkg", 2, "max"),
            ("pl2 pkg", 2, "cap"),
        ] {
            push(label, &format!("{} ({})", NA, pl_not_exposed(n, attr)));
        }
    }
    let boot = ["pl1.card", "pl2.card", "pl1.pkg", "pl2.pkg"]
        .iter()
        .find_map(|k| persist::boot_record(recs, k))
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(|uw| format!("{} W (recorded)", uw_text(&uw)))
        .unwrap_or_else(|| {
            "not recorded (recorded on the first control write in this boot)".into()
        });
    push("boot default", &boot);
    let writable = match &eff {
        Some(_) => "yes (root required)".to_string(),
        None => format!("no ({})", crate::probe::hwmon::NO_LIMIT_MAILBOX),
    };
    push("writable", &writable);
    out
}

pub fn get_power_profile(dev: &Device) -> String {
    let mut out = String::new();
    out.push_str(&format!("GPU {} [{}]\n", dev.index, dev.pci));
    for gt in &dev.gts {
        let raw = crate::sysfs::read_string(&gt.dir.join("freq0/power_profile"));
        let line = match (&gt.profile, raw.value()) {
            (Avail::Value(tok), Some(text)) => {
                let available: Vec<&str> = text
                    .split_whitespace()
                    .map(|t| t.trim_matches(|c| c == '[' || c == ']'))
                    .collect();
                format!(
                    "  {:<4} {}  (available: {})",
                    format!("gt{}", gt.id),
                    tok,
                    available.join(", ")
                )
            }
            _ => format!(
                "  {:<4} {NA}  (power_profile not exposed: kernel 6.18+)",
                format!("gt{}", gt.id)
            ),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}
