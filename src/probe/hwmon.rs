//! hwmon: the single `hwmon/hwmon*` directory whose `name` is `xe`.

use crate::avail::{Avail, Reason};
use crate::sysfs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Hwmon {
    pub dir: PathBuf,
    pub index: u32,
    pub card: Channel,
    pub pkg: Channel,
    pub temps: Vec<TempSensor>,
    pub fans: Vec<(u32, Avail<u64>)>,
    pub energy_card: Avail<u64>,
    pub energy_pkg: Avail<u64>,
    /// label → mV.
    pub voltage: Vec<(String, u64)>,
    pub curr_crit_ma: Avail<u64>,
}

/// One hwmon channel: 1 = card, 2 = pkg (µW throughout).
#[derive(Debug)]
pub struct Channel {
    pub present: bool,
    pub pl1: Avail<u64>,
    pub pl2: Avail<u64>,
    pub pl1_window_ms: Avail<u64>,
    pub pl2_window_ms: Avail<u64>,
    pub crit: Avail<u64>,
    pub rated_max: Avail<u64>,
}

#[derive(Debug)]
pub struct TempSensor {
    pub idx: u32,
    pub label: String,
    pub input_mc: i64,
    pub max_mc: Option<i64>,
    pub crit_mc: Option<i64>,
    pub emergency_mc: Option<i64>,
}

/// A GPU at or above this reading is reporting the all-ones register (255 °C): the device is
/// powered down, not hot. Every temperature limit on these parts sits at 100–125 °C, so a
/// reading beyond 250 °C is never an observation.
pub const TEMP_SENTINEL_MC: i64 = 250_000;

impl TempSensor {
    pub fn input_avail(&self) -> Avail<i64> {
        if self.input_mc >= TEMP_SENTINEL_MC {
            Avail::NotAvailable(Reason::Detail(
                "the sensor reports its powered-down sentinel (255 C); the device is off, not hot"
                    .into(),
            ))
        } else {
            Avail::Value(self.input_mc)
        }
    }
}

fn num(dir: &Path, name: &str) -> Avail<u64> {
    sysfs::read_u64(&dir.join(name))
}

fn signed(dir: &Path, name: &str) -> Avail<i64> {
    sysfs::read_i64(&dir.join(name))
}

/// The device's hwmon dir, `None` when absent (e.g. oldest kernels): the first `hwmon<K>` in
/// name order whose `name` file reads exactly `xe`.
pub fn discover_hwmon(dev_dir: &Path) -> Option<Hwmon> {
    let base = dev_dir.join("hwmon");
    let dirs = sysfs::list_dir(&base, |n| {
        n.strip_prefix("hwmon")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    });
    let dir = dirs.into_iter().find(|d| {
        sysfs::read_string(&d.join("name"))
            .value()
            .is_some_and(|s| s == "xe")
    })?;
    let index = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .and_then(|s| s.strip_prefix("hwmon").and_then(|t| t.parse().ok()))
        .unwrap_or(0);

    Some(Hwmon {
        card: build_channel(&dir, 1),
        pkg: build_channel(&dir, 2),
        temps: build_temps(&dir),
        fans: build_fans(&dir),
        energy_card: num(&dir, "energy1_input"),
        energy_pkg: num(&dir, "energy2_input"),
        voltage: build_voltage(&dir),
        curr_crit_ma: num(&dir, "curr1_crit"),
        dir,
        index,
    })
}

fn build_channel(dir: &Path, n: u32) -> Channel {
    let pl1 = num(dir, &format!("power{n}_max"));
    let pl2 = num(dir, &format!("power{n}_cap"));
    //: PL1 absent while PL2 is present means firmware disabled PL1.
    let pl1 = match (&pl1, &pl2) {
        (Avail::NotAvailable(_), Avail::Value(_)) => {
            Avail::NotAvailable(Reason::NotSupported(pl_disabled_reason(n)))
        }
        _ => pl1,
    };
    let present = [
        "max",
        "cap",
        "max_interval",
        "cap_interval",
        "crit",
        "rated_max",
    ]
    .iter()
    .any(|suf| dir.join(format!("power{n}_{suf}")).exists())
        || dir.join(format!("energy{n}_input")).exists()
        || dir.join(format!("curr{n}_crit")).exists()
        || dir.join(format!("in{n}_input")).exists();
    Channel {
        present,
        pl2,
        pl1_window_ms: num(dir, &format!("power{n}_max_interval")),
        pl2_window_ms: num(dir, &format!("power{n}_cap_interval")),
        crit: num(dir, &format!("power{n}_crit")),
        rated_max: num(dir, &format!("power{n}_rated_max")),
        pl1,
    }
}

/// effective = pl1 when present, else pl2. The kind string drives
/// `set power-limit` output ("pl1"|"pl2").
pub fn effective_limit(ch: &Channel) -> Avail<(u64, &'static str)> {
    match &ch.pl1 {
        Avail::Value(v) => Avail::Value((*v, "pl1")),
        Avail::NotAvailable(_) => match &ch.pl2 {
            Avail::Value(v) => Avail::Value((*v, "pl2")),
            // "neither exists (any kernel)" is the exact doctor/set text of
            Avail::NotAvailable(_) => Avail::NotAvailable(Reason::NotSupported(NO_LIMIT_MAILBOX)),
        },
    }
}

/// Exact texts of
fn pl_disabled_reason(n: u32) -> &'static str {
    match n {
        1 => "power1_max not exposed: firmware has PL1 disabled",
        _ => "power2_max not exposed: firmware has PL1 disabled",
    }
}

pub const NO_LIMIT_MAILBOX: &str = "no writable power limit: power1_max/power1_cap not exposed \
    (mailbox limits need kernel 6.16+, PL2 cap 6.17+)";

fn build_temps(dir: &Path) -> Vec<TempSensor> {
    let mut out = Vec::new();
    for idx in 1..=64u32 {
        let Some(input_mc) = signed(dir, &format!("temp{idx}_input")).value().copied() else {
            continue;
        };
        let label = sysfs::read_string(&dir.join(format!("temp{idx}_label")))
            .value()
            .cloned()
            .unwrap_or_else(|| format!("temp{idx}"));
        let opt = |suffix: &str| signed(dir, &format!("temp{idx}_{suffix}")).value().copied();
        out.push(TempSensor {
            idx,
            label,
            input_mc,
            max_mc: opt("max"),
            crit_mc: opt("crit"),
            emergency_mc: opt("emergency"),
        });
    }
    out
}

fn build_fans(dir: &Path) -> Vec<(u32, Avail<u64>)> {
    let mut out = Vec::new();
    for idx in 1..=8u32 {
        if dir.join(format!("fan{idx}_input")).exists() {
            out.push((idx, num(dir, &format!("fan{idx}_input"))));
        }
    }
    out
}

fn build_voltage(dir: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for idx in 1..=8u32 {
        if let Some(mv) = num(dir, &format!("in{idx}_input")).value().copied() {
            let label = sysfs::read_string(&dir.join(format!("in{idx}_label")))
                .value()
                .cloned()
                .unwrap_or_else(|| format!("in{idx}"));
            out.push((label, mv));
        }
    }
    out
}
