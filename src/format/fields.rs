//! Field catalog: name → (unit, description, resolver). Text and JSON
//! values for query/fields/status-JSON come from the same resolver.

use crate::avail::{Avail, Reason};
use crate::format::json::{num_f, num_u, Json};
use crate::probe::gt::Gt;
use crate::probe::hwmon::effective_limit;
use crate::probe::Device;
use crate::sample::Rates;

#[derive(Debug, Clone)]
pub struct Cell {
    pub text: String,
    pub json: Json,
}

pub fn c_str(s: impl Into<String>) -> Cell {
    let s = s.into();
    Cell {
        text: s.clone(),
        json: Json::Str(s),
    }
}
pub fn c_int(v: u64) -> Cell {
    Cell {
        text: v.to_string(),
        json: num_u(v),
    }
}
pub fn c_num(v: f64, dec: usize) -> Cell {
    Cell {
        text: format!("{v:.dec$}", dec = dec),
        json: num_f(v, dec),
    }
}
pub fn c_w(uw: u64) -> Cell {
    c_num(uw as f64 / 1e6, 2)
}

/// Availability probe for `fields` (no samples taken): rate fields report `yes` when their
/// source files exist rather than computing a value.
fn avail_marker() -> Cell {
    Cell {
        text: String::new(),
        json: Json::Null,
    }
}

/// Where a field's value comes from, shown by `fields` and `fields --json`.
#[derive(Debug, Clone, Copy)]
pub struct Prov {
    pub source: &'static str,
    pub scope: &'static str,
    pub access: &'static str,
    pub quality: &'static str,
}

const P_SYSFS: Prov = Prov {
    source: "sysfs",
    scope: "device",
    access: "user",
    quality: "authoritative",
};

#[derive(Debug)]
pub struct FieldDef {
    pub name: &'static str,
    pub unit: &'static str,
    pub desc: &'static str,
    pub prov: Prov,
}

macro_rules! f {
    ($name:literal, $unit:literal, $desc:literal) => {
        f!(
            $name,
            $unit,
            $desc,
            "sysfs",
            "device",
            "user",
            "authoritative"
        )
    };
    ($name:literal, $unit:literal, $desc:literal, $source:literal, $scope:literal, $access:literal, $quality:literal) => {
        FieldDef {
            name: $name,
            unit: $unit,
            desc: $desc,
            prov: Prov {
                source: $source,
                scope: $scope,
                access: $access,
                quality: $quality,
            },
        }
    };
}

/// Catalog in order.
pub const CATALOG: &[FieldDef] = &[
    f!(
        "index",
        "",
        "device ordinal in PCI-address order",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "name",
        "",
        "product name from pci.ids or the fallback table",
        "table",
        "device",
        "user",
        "authoritative"
    ),
    f!("pci.address", "", "PCI address DDDD:BB:DD.F"),
    f!("pci.vendor_id", "", "PCI vendor id, 4 hex digits"),
    f!("pci.device_id", "", "PCI device id, 4 hex digits"),
    f!("pci.subsystem", "", "subsystem vendor:device ids"),
    f!("pci.revision", "", "PCI revision id, 2 hex digits"),
    f!("pci.link.gen.current", "", "current PCIe generation"),
    f!("pci.link.gen.max", "", "maximum PCIe generation"),
    f!("pci.link.width.current", "", "current PCIe lane width"),
    f!("pci.link.width.max", "", "maximum PCIe lane width"),
    f!("driver", "", "kernel driver bound to the device"),
    f!(
        "driver.kernel",
        "",
        "kernel release",
        "procfs",
        "system",
        "user",
        "authoritative"
    ),
    f!("drm.card", "", "DRM card node"),
    f!("drm.render", "", "DRM renderD node"),
    f!("temp.pkg", "C", "package temperature"),
    f!("temp.vram", "C", "VRAM temperature"),
    f!("temp.mctrl", "C", "memory-controller temperature"),
    f!("temp.pcie", "C", "PCIe-logic temperature"),
    f!("temp.pkg.max", "C", "package temperature limit"),
    f!("temp.pkg.crit", "C", "package critical temperature"),
    f!("temp.pkg.emergency", "C", "package emergency temperature"),
    f!("temp.vram.crit", "C", "VRAM critical temperature"),
    f!("temp.vram.emergency", "C", "VRAM emergency temperature"),
    f!(
        "power.draw",
        "W",
        "card power draw from the energy delta",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "power.draw.pkg",
        "W",
        "package power draw from the energy delta",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "power.limit",
        "W",
        "effective power limit",
        "sysfs",
        "device",
        "root",
        "authoritative"
    ),
    f!(
        "power.limit.kind",
        "",
        "which limit is effective (pl1 or pl2)",
        "sysfs",
        "device",
        "root",
        "authoritative"
    ),
    f!("power.limit.pl1", "W", "PL1 sustained limit, card channel"),
    f!("power.limit.pl2", "W", "PL2 burst limit, card channel"),
    f!(
        "power.limit.pl1.pkg",
        "W",
        "PL1 sustained limit, pkg channel"
    ),
    f!("power.limit.pl2.pkg", "W", "PL2 burst limit, pkg channel"),
    f!(
        "power.limit.pl1.window",
        "ms",
        "PL1 window (Tau)",
        "sysfs",
        "device",
        "root",
        "authoritative"
    ),
    f!(
        "power.limit.pl2.window",
        "ms",
        "PL2 window",
        "sysfs",
        "device",
        "root",
        "authoritative"
    ),
    f!("power.crit", "W", "critical (I1) power limit"),
    f!("power.rated_max", "W", "rated maximum (default TDP)"),
    f!(
        "power.profile",
        "",
        "firmware power profile (mixed when GTs differ)",
        "sysfs",
        "device",
        "root",
        "authoritative"
    ),
    f!("energy.card", "J", "accumulated card energy"),
    f!("energy.pkg", "J", "accumulated package energy"),
    f!("voltage.pkg", "mV", "package rail voltage"),
    f!(
        "memory.total",
        "MiB",
        "VRAM total size",
        "sysfs",
        "device",
        "user",
        "derived"
    ),
    f!(
        "memory.used",
        "MiB",
        "resident VRAM over visible clients",
        "fdinfo",
        "visible-clients",
        "user",
        "partial"
    ),
    f!(
        "memory.free",
        "MiB",
        "VRAM total minus used",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "memory.total.source",
        "",
        "how the VRAM total was determined",
        "sysfs",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "memory.used.source",
        "",
        "how the VRAM used figure was determined",
        "fdinfo",
        "visible-clients",
        "user",
        "authoritative"
    ),
    f!(
        "memory.cpu_visible.total",
        "MiB",
        "CPU-visible aperture size",
        "drm-uapi",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "memory.cpu_visible.used",
        "MiB",
        "CPU-visible aperture usage",
        "drm-uapi",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "memory.min_page_size",
        "KiB",
        "smallest backing page size of the VRAM region",
        "drm-uapi",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "ras.correctable",
        "count",
        "sum of correctable-error node counters",
        "drm-ras",
        "device",
        "root",
        "authoritative"
    ),
    f!(
        "ras.uncorrectable",
        "count",
        "sum of uncorrectable-error node counters",
        "drm-ras",
        "device",
        "root",
        "authoritative"
    ),
    f!(
        "firmware.guc",
        "",
        "GuC submission firmware version",
        "drm-uapi",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "firmware.huc",
        "",
        "HuC firmware version",
        "drm-uapi",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "utilization.gt",
        "%",
        "mean GT active time (idle residency)",
        "computed",
        "gt",
        "user",
        "derived"
    ),
    f!(
        "utilization.render",
        "%",
        "render engine utilization (rcs)",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "utilization.compute",
        "%",
        "compute engine utilization (ccs)",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "utilization.video",
        "%",
        "video engine utilization (vcs)",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "utilization.enhance",
        "%",
        "enhance engine utilization (vecs)",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!(
        "utilization.copy",
        "%",
        "copy engine utilization (bcs)",
        "computed",
        "device",
        "user",
        "derived"
    ),
    f!("clock.cur", "MHz", "requested frequency"),
    f!("clock.act", "MHz", "actual frequency"),
    f!("clock.min", "MHz", "software minimum frequency"),
    f!("clock.max", "MHz", "software maximum frequency"),
    f!("clock.rp0", "MHz", "hardware maximum frequency"),
    f!("clock.rpe", "MHz", "efficient frequency"),
    f!("clock.rpn", "MHz", "hardware minimum frequency"),
    f!("clock.rpa", "MHz", "achievable frequency"),
    f!(
        "clock.mem",
        "MHz",
        "VRAM frequency (the kernel exposes it only on PVC)"
    ),
    f!(
        "throttle.active",
        "",
        "any GT reports an active throttle reason"
    ),
    f!(
        "throttle.reasons",
        "",
        "active throttle reasons over all GTs"
    ),
    f!("idle.status", "", "gt0 C-state (gt-c0 active, gt-c6 idle)"),
    f!("fan.rpm", "RPM", "fan1 tachometer"),
    f!(
        "fan.percent",
        "%",
        "fan speed percentage (no fan control interface exists)"
    ),
    f!(
        "processes.count",
        "",
        "unique visible DRM clients",
        "fdinfo",
        "visible-clients",
        "user",
        "partial"
    ),
    f!(
        "persistence.installed",
        "",
        "boot persistence rule installed",
        "computed",
        "system",
        "root",
        "authoritative"
    ),
    f!(
        "timestamp",
        "",
        "generation time, UTC ISO-8601",
        "computed",
        "system",
        "user",
        "authoritative"
    ),
    f!(
        "ecc.mode",
        "",
        "ECC mode (no ECC interface in the kernel)",
        "table",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "uuid",
        "",
        "device UUID (no per-device identifier in the kernel)",
        "table",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "serial",
        "",
        "serial number (no per-device identifier in the kernel)",
        "table",
        "device",
        "user",
        "authoritative"
    ),
    f!("pci.numa_node", "", "NUMA node of the device (-1 unknown)"),
    f!("pci.local_cpus", "", "local_cpulist of the device"),
    f!("pci.iommu_group", "", "IOMMU group number"),
    f!(
        "pci.root_port",
        "",
        "root port between the host bridge and the device",
        "sysfs",
        "device",
        "user",
        "derived"
    ),
    f!(
        "pci.aer.correctable",
        "",
        "AER correctable error total (endpoint)",
        "pci-sysfs",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "pci.aer.nonfatal",
        "",
        "AER non-fatal error total (endpoint)",
        "pci-sysfs",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "pci.aer.fatal",
        "",
        "AER fatal error total (endpoint)",
        "pci-sysfs",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "pci.aer.rootport.correctable",
        "",
        "AER correctable error total (root port)",
        "pci-sysfs",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "pci.aer.rootport.nonfatal",
        "",
        "AER non-fatal error total (root port)",
        "pci-sysfs",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "pci.aer.rootport.fatal",
        "",
        "AER fatal error total (root port)",
        "pci-sysfs",
        "device",
        "user",
        "authoritative"
    ),
    f!("sriov.total_vfs", "", "total virtual functions supported"),
    f!("sriov.num_vfs", "", "virtual functions currently enabled"),
    f!("display.connectors", "", "display connectors exposed"),
    f!("display.connected", "", "connectors with a connected sink"),
    f!("display.active", "", "connected and enabled connectors"),
    f!(
        "crash.pending",
        "",
        "pending kernel crash dumps (devcoredump)",
        "devcoredump",
        "device",
        "user",
        "authoritative"
    ),
    f!(
        "health.survivability",
        "",
        "device in survivability mode (driver without DRM card)"
    ),
];

/// Fields that have per-GT variants `gt<N>.<field>`.
pub const GT_VARIANT_BASES: &[&str] = &[
    "clock.cur",
    "clock.act",
    "clock.min",
    "clock.max",
    "clock.rp0",
    "clock.rpe",
    "clock.rpn",
    "clock.rpa",
    "utilization.gt",
    "throttle.active",
    "throttle.reasons",
    "idle.status",
    "power.profile",
];

pub fn def(name: &str) -> Option<&'static FieldDef> {
    CATALOG.iter().find(|d| d.name == name)
}

/// Resolve a user-given field name (handles `gt<N>.<field>` and the `pcie.` alias used by
/// query) into its catalog entry and GT index. `None` = unknown field (exit 2 in query).
pub fn parse_name(name: &str) -> Option<(&'static FieldDef, Option<u32>)> {
    if let Some(d) = def(name) {
        return Some((d, None));
    }
    if let Some(rest) = name.strip_prefix("pcie.") {
        let owned = format!("pci.{rest}");
        return def(&owned).map(|d| (d, None));
    }
    let (gt_part, rest) = name.split_once('.')?;
    let gt = gt_part.strip_prefix("gt")?.parse::<u32>().ok()?;
    // `gt<N>.utilization` is the listed variant name of the per-GT `utilization.gt`.
    let rest = if rest == "utilization" {
        "utilization.gt"
    } else {
        rest
    };
    def(rest).map(|d| (d, Some(gt)))
}

/// Everything a field can be resolved against: one device in one frame.
pub struct View<'a> {
    pub dev: &'a Device,
    /// `None` = probe-only pass (`fields`): rate fields report availability from their sources.
    pub rates: Option<&'a Rates>,
    pub used_bytes: u64,
    pub used_clients: usize,
    pub persistence: bool,
    pub roots: &'a crate::paths::Roots,
    pub kernel: String,
    pub timestamp: String,
}

fn gt_of<'a>(v: &'a View, want: Option<u32>) -> Option<&'a Gt> {
    match want {
        Some(n) => v.dev.gts.iter().find(|g| g.id == n),
        None => v.dev.gts.first(),
    }
}

pub fn mc_to_c(mc: i64) -> u64 {
    ((mc + 500) / 1000).max(0) as u64
}

/// µW rendered as watts with 2 decimals (shared by get/set/reset texts).
pub fn uw_w(uw: u64) -> String {
    format!("{:.2}", uw as f64 / 1e6)
}

pub fn mib(bytes: u64) -> u64 {
    bytes / (1 << 20)
}

fn missing(dev: &Device, rel: &str) -> Reason {
    Reason::Missing(dev.dev_dir.join(rel))
}

/// Resolve one catalog field to its text/JSON cell, or the reason it is N/A.
pub fn resolve(name: &str, g: Option<u32>, v: &View) -> Avail<Cell> {
    let dev = v.dev;
    let na = Avail::<Cell>::NotAvailable;
    match name {
        "index" => Avail::Value(c_int(dev.index as u64)),
        "name" => Avail::Value(c_str(dev.name.clone())),
        "pci.address" => Avail::Value(c_str(dev.pci.clone())),
        "pci.vendor_id" => Avail::Value(c_str(format!("{:04x}", dev.ids.vendor))),
        "pci.device_id" => Avail::Value(c_str(format!("{:04x}", dev.ids.device))),
        "pci.subsystem" => dev
            .ids
            .subsystem
            .clone()
            .map(|(a, b)| c_str(format!("{a:04x}:{b:04x}"))),
        "pci.revision" => dev.ids.revision.clone().map(|r| c_str(format!("{r:02x}"))),
        "pci.link.gen.current"
        | "pci.link.gen.max"
        | "pci.link.width.current"
        | "pci.link.width.max" => match dev.link.value() {
            Some(link) => {
                let cell = match name {
                    "pci.link.gen.current" => link.gen_cur.map(|x| c_int(x as u64)),
                    "pci.link.gen.max" => link.gen_max.map(|x| c_int(x as u64)),
                    "pci.link.width.current" => link.width_cur.map(|w| c_int(w as u64)),
                    _ => link.width_max.map(|w| c_int(w as u64)),
                };
                match cell {
                    Some(c) => Avail::Value(c),
                    None => na(missing(dev, "current_link_speed")),
                }
            }
            None => reason_of_avail(&dev.link),
        },
        "driver" => Avail::Value(c_str("xe")),
        "driver.kernel" => Avail::Value(c_str(v.kernel.clone())),
        "drm.card" => Avail::Value(c_str(dev.card.clone())),
        "drm.render" => match dev.render.as_ref() {
            Some(r) => Avail::Value(c_str(r.clone())),
            None => na(missing(dev, "drm/renderD* node absent")),
        },
        "temp.pkg" | "temp.vram" | "temp.mctrl" | "temp.pcie" => {
            match temp_by_label(v, name.trim_start_matches("temp.")) {
                Some(t) => Avail::Value(c_int(mc_to_c(t.input_mc))),
                None => na(Reason::NotSupported("temperature channel not exposed")),
            }
        }
        "temp.pkg.max"
        | "temp.pkg.crit"
        | "temp.pkg.emergency"
        | "temp.vram.crit"
        | "temp.vram.emergency" => {
            let base = if name.contains(".pkg.") {
                "pkg"
            } else {
                "vram"
            };
            let suffix = name.rsplit('.').next().unwrap_or("");
            match temp_by_label(v, base).and_then(|t| match suffix {
                "max" => t.max_mc,
                "crit" => t.crit_mc,
                _ => t.emergency_mc,
            }) {
                Some(mc) => Avail::Value(c_int(mc_to_c(mc))),
                None => na(missing(dev, &format!("hwmon temp limit for {base}"))),
            }
        }
        "power.draw" | "power.draw.pkg" => {
            let pkg = name.ends_with(".pkg");
            match v.rates {
                Some(r) => (if pkg {
                    r.draw_pkg_w.clone()
                } else {
                    r.draw_card_w.clone()
                })
                .map(|w| c_num(w, 2)),
                None => energy_avail(dev, pkg),
            }
        }
        "power.limit" | "power.limit.kind" => match effective_of(dev) {
            Some((uw, kind, _chan)) => Avail::Value(if name == "power.limit" {
                c_w(uw)
            } else {
                c_str(kind)
            }),
            None => na(Reason::NotSupported(crate::probe::hwmon::NO_LIMIT_MAILBOX)),
        },
        "power.limit.pl1" | "power.limit.pl2" | "power.limit.pl1.pkg" | "power.limit.pl2.pkg" => {
            match dev.hwmon.as_ref() {
                Some(h) => {
                    let ch = if name.ends_with(".pkg") {
                        &h.pkg
                    } else {
                        &h.card
                    };
                    if name.contains("pl1") {
                        ch.pl1.clone()
                    } else {
                        ch.pl2.clone()
                    }
                }
                None => Avail::NotAvailable(missing(dev, "hwmon")),
            }
            .map(c_w)
        }
        "power.limit.pl1.window" | "power.limit.pl2.window" => match dev.hwmon.as_ref() {
            Some(h) => {
                if name.contains("pl1") {
                    h.card.pl1_window_ms.clone()
                } else {
                    h.card.pl2_window_ms.clone()
                }
            }
            None => Avail::NotAvailable(missing(dev, "hwmon")),
        }
        .map(c_int),
        "power.crit" => match dev.hwmon.as_ref() {
            Some(h) => h.card.crit.clone(),
            None => Avail::NotAvailable(missing(dev, "hwmon")),
        }
        .map(c_w),
        "power.rated_max" => match dev.hwmon.as_ref() {
            Some(h) => match &h.card.rated_max {
                Avail::Value(v) => Avail::Value(c_w(*v)),
                Avail::NotAvailable(_) => na(Reason::NotSupported(
                    "power1_rated_max is never visible on mailbox platforms",
                )),
            },
            None => na(missing(dev, "hwmon")),
        },
        "power.profile" => match gt_of(v, g) {
            Some(gt) => profile_of(v, gt),
            None => na(Reason::NotSupported("no GTs exposed")),
        },
        "energy.card" | "energy.pkg" => {
            let e = match (v.rates, dev.hwmon.as_ref()) {
                (Some(r), _) => Some(if name == "energy.card" {
                    r.energy_card.clone()
                } else {
                    r.energy_pkg.clone()
                }),
                (None, Some(h)) => Some(if name == "energy.card" {
                    h.energy_card.clone()
                } else {
                    h.energy_pkg.clone()
                }),
                (None, None) => None,
            };
            match e {
                Some(a) => a.map(|uj| c_num(uj as f64 / 1e6, 3)),
                None => na(missing(dev, "hwmon")),
            }
        }
        "voltage.pkg" => match dev.hwmon.as_ref().and_then(|h| h.voltage.first()) {
            Some((_, mv)) => Avail::Value(c_int(*mv)),
            None => na(missing(dev, "in1_input")),
        },
        "memory.total" | "memory.total.source" => match (dev.kabi.vram(), dev.vram_total.value()) {
            (Some(k), _) => Avail::Value(if name.ends_with(".source") {
                c_str("drm-uapi")
            } else {
                c_int(mib(k.total))
            }),
            (None, Some(t)) if name.ends_with(".source") => Avail::Value(c_str(match t.source {
                crate::probe::VramSource::Bar => "bar",
                crate::probe::VramSource::Table => "table",
            })),
            (None, Some(t)) => Avail::Value(c_int(mib(t.bytes))),
            (None, None) => reason_of_avail(&dev.vram_total),
        },
        "memory.used" | "memory.used.source" => match dev.kabi.vram() {
            Some(k) => {
                // A zero on 6.x without CAP_PERFMON is a permission artifact, not an
                // observation: the figure is refused rather than trusted.
                let trusted = k.used > 0 || k.total == 0 || v.used_bytes == 0;
                if !trusted {
                    na(crate::avail::Reason::Detail(
                        "kernel reports 0 (CAP_PERFMON needed before 7.0)".into(),
                    ))
                } else if name.ends_with(".source") {
                    Avail::Value(c_str("drm-uapi"))
                } else {
                    Avail::Value(c_int(mib(k.used)))
                }
            }
            None => {
                if name.ends_with(".source") {
                    Avail::Value(c_str("fdinfo"))
                } else {
                    Avail::Value(c_int(mib(v.used_bytes)))
                }
            }
        },
        "memory.cpu_visible.total" | "memory.cpu_visible.used" => match dev.kabi.vram() {
            Some(k) => Avail::Value(c_int(mib(if name.ends_with(".total") {
                k.cpu_visible
            } else {
                k.cpu_visible_used
            }))),
            None => reason_of_avail(&dev.kabi.mem),
        },
        "memory.min_page_size" => match dev.kabi.vram() {
            Some(k) => Avail::Value(c_int((k.min_page_size / 1024) as u64)),
            None => reason_of_avail(&dev.kabi.mem),
        },
        "ras.correctable" | "ras.uncorrectable" => match dev.kabi.ras.value() {
            Some(nodes) => {
                let want_corr = name.ends_with("correctable") && !name.contains("un");
                let sum: u32 = nodes
                    .iter()
                    .filter(|(n, _)| {
                        (n.name.contains("correctable") && !n.name.contains("uncorrectable"))
                            == want_corr
                    })
                    .flat_map(|(_, cs)| cs.iter())
                    .map(|c| c.value)
                    .sum();
                Avail::Value(c_int(sum as u64))
            }
            None => reason_of_avail(&dev.kabi.ras),
        },
        "firmware.guc" => avail_ufw(&dev.kabi.guc),
        "firmware.huc" => avail_ufw(&dev.kabi.huc),
        "pci.numa_node" => match dev.placement.numa_display() {
            Some(n) => Avail::Value(c_int(n.parse().unwrap_or(0))),
            None => reason_of_avail(&dev.placement.numa),
        },
        "pci.local_cpus" => match dev.placement.local_cpus.value() {
            Some(s) => Avail::Value(c_str(s.clone())),
            None => reason_of_avail(&dev.placement.local_cpus),
        },
        "pci.iommu_group" => match dev.placement.iommu_group.value() {
            Some(s) => Avail::Value(c_str(s.clone())),
            None => reason_of_avail(&dev.placement.iommu_group),
        },
        "pci.root_port" => match dev.placement.root_port.value() {
            Some(s) => Avail::Value(c_str(s.clone())),
            None => reason_of_avail(&dev.placement.root_port),
        },
        "pci.aer.correctable" => match dev.placement.aer.as_ref().map(|a| &a.correctable) {
            Some(Avail::Value(n)) => Avail::Value(c_int(*n)),
            Some(other) => reason_of_avail(other),
            None => na(Reason::NotSupported(
                "no AER statistics exposed for this device",
            )),
        },
        "pci.aer.nonfatal"
        | "pci.aer.fatal"
        | "pci.aer.rootport.correctable"
        | "pci.aer.rootport.nonfatal"
        | "pci.aer.rootport.fatal" => {
            let tot = if name.starts_with("pci.aer.rootport") {
                dev.placement.root_port_aer.as_ref().map(|t| match name {
                    _ if name.ends_with("nonfatal") => &t.nonfatal,
                    _ if name.ends_with("fatal") => &t.fatal,
                    _ => &t.correctable,
                })
            } else {
                dev.placement.aer.as_ref().map(|a| match name {
                    _ if name.ends_with("nonfatal") => &a.nonfatal,
                    _ if name.ends_with("fatal") => &a.fatal,
                    _ => &a.correctable,
                })
            };
            match tot {
                Some(Avail::Value(n)) => Avail::Value(c_int(*n)),
                Some(other) => reason_of_avail(other),
                None if name.starts_with("pci.aer.rootport") => na(Reason::NotSupported(
                    "no AER statistics exposed on the root port",
                )),
                None => na(Reason::NotSupported(
                    "no AER statistics exposed for this device",
                )),
            }
        }
        "sriov.total_vfs" | "sriov.num_vfs" => match dev.placement.sriov.as_ref() {
            Some(pf) => {
                let a = if name.ends_with("num_vfs") {
                    &pf.num
                } else {
                    &pf.total
                };
                match a {
                    Avail::Value(n) => Avail::Value(c_int(*n)),
                    other => reason_of_avail(other),
                }
            }
            None => na(Reason::NotSupported("no SR-IOV on this device")),
        },
        "display.connectors" | "display.connected" | "display.active" => {
            let cs = crate::sriov::connectors(v.roots, dev);
            let n = match name {
                "display.connectors" => cs.len(),
                "display.connected" => cs.iter().filter(|c| c.status == "connected").count(),
                _ => cs.iter().filter(|c| c.enabled).count(),
            };
            Avail::Value(c_int(n as u64))
        }
        "crash.pending" => Avail::Value(c_int(
            crate::crash::list_for_pci(v.roots, &dev.pci).len() as u64
        )),
        "health.survivability" => {
            if dev.dev_dir.join("survivability_mode").exists() {
                Avail::Value(c_str("yes"))
            } else {
                Avail::Value(c_str("no"))
            }
        }
        "memory.free" => match (dev.kabi.vram(), dev.vram_total.value()) {
            (Some(k), _) => Avail::Value(c_int(mib(k.total).saturating_sub(mib(k.used)))),
            (None, Some(t)) => Avail::Value(c_int(mib(t.bytes).saturating_sub(mib(v.used_bytes)))),
            (None, None) => reason_of_avail(&dev.vram_total),
        },
        "utilization.gt" => match v.rates {
            Some(r) if g.is_some() => {
                let idx = v.dev.gts.iter().position(|x| Some(x.id) == g);
                match idx.and_then(|i| r.gt_util_pct.get(i)) {
                    Some(Avail::Value(x)) => Avail::Value(c_num(*x, 1)),
                    Some(Avail::NotAvailable(rr)) => na(rr.clone()),
                    None => na(Reason::NotSupported("no such GT on this device")),
                }
            }
            Some(r) => {
                let vals: Vec<f64> = r
                    .gt_util_pct
                    .iter()
                    .filter_map(|a| a.value().copied())
                    .collect();
                if vals.is_empty() {
                    na(missing(dev, "gtidle/idle_residency_ms"))
                } else {
                    Avail::Value(c_num(vals.iter().sum::<f64>() / vals.len() as f64, 1))
                }
            }
            None => match dev.gts.iter().find(|x| x.idle_ms.value().is_some()) {
                Some(_) => Avail::Value(avail_marker()),
                None => na(missing(dev, "gtidle/idle_residency_ms")),
            },
        },
        "utilization.render"
        | "utilization.compute"
        | "utilization.video"
        | "utilization.enhance"
        | "utilization.copy" => match v.rates {
            Some(r) => {
                let e = &r.engine_pct;
                let a = match name {
                    "utilization.render" => e.rcs.clone(),
                    "utilization.compute" => e.ccs.clone(),
                    "utilization.video" => e.vcs.clone(),
                    "utilization.enhance" => e.vecs.clone(),
                    _ => e.bcs.clone(),
                };
                a.map(|x| c_num(x, 1))
            }
            None => Avail::Value(avail_marker()),
        },
        "clock.cur" | "clock.act" | "clock.min" | "clock.max" | "clock.rp0" | "clock.rpe"
        | "clock.rpn" | "clock.rpa" => match gt_of(v, g) {
            Some(gt) => {
                let a = match name {
                    "clock.cur" => gt.cur.clone(),
                    "clock.act" => gt.act.clone(),
                    "clock.min" => gt.min.clone(),
                    "clock.max" => gt.max.clone(),
                    "clock.rp0" => gt.rp0.clone(),
                    "clock.rpe" => gt.rpe.clone(),
                    "clock.rpn" => gt.rpn.clone(),
                    _ => gt.rpa.clone(),
                };
                a.map(|m| c_int(m as u64))
            }
            None => na(Reason::NotSupported("no such GT on this device")),
        },
        "clock.mem" => {
            match crate::sysfs::read_u64(&dev.dev_dir.join("tile0/memory/freq0/max_freq")).value() {
                Some(v) => Avail::Value(c_int(*v)),
                None => na(Reason::NotSupported(
                    "VRAM frequency sysfs exists only on PVC",
                )),
            }
        }
        "throttle.active" => match gt_of(v, g) {
            Some(gt) => match gt.throttle_status.clone() {
                Avail::Value(active) => Avail::Value(c_str(if active { "yes" } else { "no" })),
                Avail::NotAvailable(r) => na(r),
            },
            None => na(Reason::NotSupported("no such GT on this device")),
        },
        "throttle.reasons" => throttle_reasons_for(v.dev, g),
        "idle.status" => match gt_of(v, g) {
            Some(gt) => gt.idle_status.clone().map(c_str),
            None => na(Reason::NotSupported("no such GT on this device")),
        },
        "fan.rpm" => match dev.hwmon.as_ref().and_then(|h| h.fans.first()) {
            Some((_, rpm)) => rpm.clone().map(c_int),
            None => na(missing(dev, "fan1_input")),
        },
        "fan.percent" => na(Reason::NotSupported(
            "no fan PWM or maximum-RPM interface in the kernel",
        )),
        "processes.count" => Avail::Value(c_int(v.used_clients as u64)),
        "persistence.installed" => Avail::Value(c_str(if v.persistence { "yes" } else { "no" })),
        "timestamp" => Avail::Value(c_str(v.timestamp.clone())),
        "ecc.mode" => na(Reason::NotSupported("no ECC interface in the kernel")),
        "uuid" | "serial" => na(Reason::NotSupported(
            "no per-device identifier beyond the PCI address",
        )),
        _ => na(Reason::NotSupported("unknown field")),
    }
}

fn temp_by_label<'a>(v: &'a View, label: &str) -> Option<&'a crate::probe::TempSensor> {
    v.dev
        .hwmon
        .as_ref()?
        .temps
        .iter()
        .find(|t| t.label == label)
}

fn energy_avail(dev: &Device, pkg: bool) -> Avail<Cell> {
    match dev.hwmon.as_ref() {
        None => Avail::NotAvailable(missing(dev, "hwmon")),
        Some(h) => match if pkg { &h.energy_pkg } else { &h.energy_card } {
            Avail::Value(_) => Avail::Value(avail_marker()),
            Avail::NotAvailable(r) => Avail::NotAvailable(r.clone()),
        },
    }
}

/// gt0 token, or `mixed` when the GTs differ; absent attribute is KernelTooOld.
fn profile_of(v: &View, gt0: &Gt) -> Avail<Cell> {
    let first = match &gt0.profile {
        Avail::Value(t) => t,
        Avail::NotAvailable(Reason::Missing(_)) => {
            return Avail::NotAvailable(Reason::KernelTooOld {
                feature: "power_profile",
                first: "6.18",
            })
        }
        Avail::NotAvailable(r) => return Avail::NotAvailable(r.clone()),
    };
    if v.dev.gts.iter().all(|g| g.profile.value() == Some(first)) {
        Avail::Value(c_str(first.clone()))
    } else {
        Avail::Value(c_str("mixed"))
    }
}

pub fn throttle_reasons_for(dev: &Device, g: Option<u32>) -> Avail<Cell> {
    let gts: Vec<&Gt> = match g {
        Some(n) => dev.gts.iter().filter(|x| x.id == n).collect(),
        None => dev.gts.iter().collect(),
    };
    if gts.is_empty() {
        return Avail::NotAvailable(Reason::NotSupported("no such GT on this device"));
    }
    let mut active: Vec<String> = Vec::new();
    let mut first_missing: Option<Reason> = None;
    for x in &gts {
        match &x.throttle_reasons {
            Avail::Value(rs) => active.extend(rs.iter().cloned()),
            Avail::NotAvailable(r) => {
                if first_missing.is_none() {
                    first_missing = Some(r.clone());
                }
            }
        }
    }
    if active.is_empty() {
        if let Some(r) = first_missing {
            return Avail::NotAvailable(r);
        }
    }
    active.sort_by(|a, b| reason_rank(a).cmp(&reason_rank(b)));
    active.dedup();
    let joined = if active.is_empty() {
        "none".to_string()
    } else {
        active.join(",")
    };
    Avail::Value(c_str(joined))
}

/// Canonical order of, then any other names after it.
pub fn reason_rank(name: &str) -> (u8, &str) {
    const CANON: [&str; 8] = [
        "pl1",
        "pl2",
        "pl4",
        "thermal",
        "prochot",
        "ratl",
        "vr_thermalert",
        "vr_tdc",
    ];
    match CANON.iter().position(|c| *c == name) {
        Some(i) => (i as u8, name),
        None => (100, name),
    }
}

/// Device effective limit: card channel preferred. Returns watts-µW, kind and channel.
pub fn effective_of(dev: &Device) -> Option<(u64, &'static str, &'static str)> {
    let h = dev.hwmon.as_ref()?;
    match effective_limit(&h.card) {
        Avail::Value((v, k)) => Some((v, k, "card")),
        Avail::NotAvailable(_) => match effective_limit(&h.pkg) {
            Avail::Value((v, k)) => Some((v, k, "pkg")),
            Avail::NotAvailable(_) => None,
        },
    }
}

/// Map the availability of a probed struct (link, vram total) to a Cell availability.
pub fn avail_ufw(a: &crate::avail::Avail<crate::kabi::UcFw>) -> Avail<Cell> {
    match a.value() {
        Some(u) => {
            let mut t = format!("{}.{}.{}", u.major, u.minor, u.patch);
            if u.branch != 0 {
                t.push_str(&format!(", branch {}", u.branch));
            }
            Avail::Value(c_str(t))
        }
        None => reason_of_avail(a),
    }
}

fn reason_of_avail<A>(a: &Avail<A>) -> Avail<Cell> {
    match a {
        Avail::NotAvailable(r) => Avail::NotAvailable(r.clone()),
        Avail::Value(_) => Avail::Value(avail_marker()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_has_every_canonical_field_family() {
        assert!(super::def("clock.mem").is_some());
        assert!(super::def("fan.percent").is_some());
        assert!(super::def("ecc.mode").is_some());
        assert!(super::parse_name("gt1.clock.cur").is_some());
        assert!(super::parse_name("pcie.link.gen.current").is_some());
        assert!(super::parse_name("bogus.field").is_none());
    }

    #[test]
    fn parse_name_gt_and_alias_forms() {
        let (d, g) = super::parse_name("gt2.clock.cur").unwrap();
        assert_eq!(d.name, "clock.cur");
        assert_eq!(g, Some(2));
        let (d, g) = super::parse_name("pcie.link.width.max").unwrap();
        assert_eq!(d.name, "pci.link.width.max");
        assert_eq!(g, None);
        assert!(super::parse_name("gt7.not_a_field").is_none());
    }
}

/// One row of the `fields` listing (text and JSON share this).
#[derive(Debug)]
pub struct ListRow {
    pub name: String,
    pub unit: &'static str,
    pub available: bool,
    pub prov: Prov,
    pub desc: String,
}

/// All base fields in catalog order, then the per-GT variants of every GT the device has.
pub fn list_rows(view: &View) -> Vec<ListRow> {
    let mut rows: Vec<ListRow> = CATALOG
        .iter()
        .map(|d| ListRow {
            name: d.name.to_string(),
            unit: d.unit,
            available: resolve(d.name, None, view).value().is_some(),
            prov: d.prov,
            desc: d.desc.to_string(),
        })
        .collect();
    let mut gt_ids: Vec<u32> = view.dev.gts.iter().map(|g| g.id).collect();
    gt_ids.sort_unstable();
    for id in gt_ids {
        for base in GT_VARIANT_BASES {
            let shown = if *base == "utilization.gt" {
                "utilization"
            } else {
                base
            };
            rows.push(ListRow {
                name: format!("gt{id}.{shown}"),
                unit: def(base).map(|d| d.unit).unwrap_or(""),
                available: resolve(base, Some(id), view).value().is_some(),
                prov: def(base).map(|d| d.prov).unwrap_or(P_SYSFS),
                desc: format!("per-GT variant of {base}"),
            });
        }
    }
    rows
}

/// `fields` text output.
pub fn render_text(rows: &[ListRow]) -> String {
    let mut out = format!(
        "{:<30} {:<6} {:<10} {:<12} {:<8} {}\n",
        "FIELD", "UNIT", "AVAILABLE", "SOURCE", "ACCESS", "DESCRIPTION"
    );
    for r in rows {
        out.push_str(&format!(
            "{:<30} {:<6} {:<10} {:<12} {:<8} {}\n",
            r.name,
            r.unit,
            if r.available { "yes" } else { "N/A" },
            r.prov.source,
            r.prov.access,
            r.desc
        ));
    }
    out
}
