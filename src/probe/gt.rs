//! GTs: `tile<T>/gt<G>/` frequency, throttle, idle.

use crate::avail::{Avail, Reason};
use crate::sysfs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtKind {
    Render,
    Media,
    Unknown,
}

#[derive(Debug)]
pub struct Gt {
    pub id: u32,
    pub tile: u32,
    pub dir: PathBuf,
    pub kind: GtKind,
    pub idle_name: Avail<String>,
    pub act: Avail<u32>,
    pub cur: Avail<u32>,
    pub min: Avail<u32>,
    pub max: Avail<u32>,
    pub rp0: Avail<u32>,
    pub rpe: Avail<u32>,
    pub rpn: Avail<u32>,
    pub rpa: Avail<u32>,
    pub profile: Avail<String>,
    pub throttle_status: Avail<bool>,
    pub throttle_reasons: Avail<Vec<String>>,
    pub idle_status: Avail<String>,
    pub idle_ms: Avail<u64>,
}

fn mhz(dir: &Path, name: &str) -> Avail<u32> {
    sysfs::read_u64(&dir.join(name)).map(|v| v.min(u32::MAX as u64) as u32)
}

impl Gt {
    /// The GT's C-state, told honestly. `act_freq` reads 0 while the GT is parked in C6
    /// (kernel comment in `xe_guc_pc_get_act_freq`), while `gtidle/idle_status` mirrors a
    /// register that keeps reporting `gt-c0` after the park on Battlemage. The raw file
    /// stays available for the verbose annotation (`idle_status`).
    pub fn idle_state(&self) -> Avail<String> {
        match self.act.value() {
            Some(0) => Avail::Value("gt-c6".to_string()),
            Some(_) => Avail::Value("gt-c0".to_string()),
            None => self.idle_status.clone(),
        }
    }
}

/// `tile<T>/gt<G>` sorted by (tile, gt).
pub fn discover_gts(dev_dir: &Path) -> Vec<Gt> {
    let mut found: Vec<(u32, u32, PathBuf)> = Vec::new();
    for tile_dir in sysfs::list_dir(dev_dir, |n| {
        n.strip_prefix("tile")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    }) {
        let tile = tile_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .and_then(|s| s.strip_prefix("tile").and_then(|t| t.parse().ok()))
            .unwrap_or(0);
        for gt_dir in sysfs::list_dir(&tile_dir, |n| {
            n.strip_prefix("gt")
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        }) {
            let id = gt_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .and_then(|s| s.strip_prefix("gt").and_then(|t| t.parse().ok()))
                .unwrap_or(0);
            found.push((tile, id, gt_dir));
        }
    }
    found.sort_by_key(|(tile, id, _)| (*tile, *id));
    found
        .into_iter()
        .map(|(tile, id, dir)| build_gt(tile, id, dir))
        .collect()
}

fn build_gt(tile: u32, id: u32, dir: PathBuf) -> Gt {
    let freq = dir.join("freq0");
    let idle = dir.join("gtidle");
    let idle_name = sysfs::read_string(&idle.join("name"));
    // Kind from the gtidle name: `gt0-rc` render, `gt1-mc` media.
    let kind = match idle_name
        .value()
        .map(|s| s.rsplit('-').next().unwrap_or(""))
    {
        Some("rc") => GtKind::Render,
        Some("mc") => GtKind::Media,
        _ => GtKind::Unknown,
    };
    let profile = match sysfs::read_string(&freq.join("power_profile")) {
        Avail::Value(text) => parse_profile_brackets(&freq.join("power_profile"), &text),
        Avail::NotAvailable(r) => Avail::NotAvailable(r),
    };
    let throttle_dir = freq.join("throttle");
    let throttle_status = sysfs::read_u64(&throttle_dir.join("status")).map(|v| v != 0);
    //: prefer the `reasons` file (6.19+), else derive from `reason_*` files.
    let throttle_reasons = match sysfs::read_string(&throttle_dir.join("reasons")) {
        Avail::Value(text) => Avail::Value(parse_reasons_file(&text)),
        Avail::NotAvailable(_) => {
            let mut flags = Vec::new();
            for name in reason_files(&throttle_dir) {
                if let Some(v) = sysfs::read_u64(&throttle_dir.join(&name)).value().copied() {
                    flags.push((name, v != 0));
                }
            }
            Avail::Value(derive_reasons_from_flags(&flags))
        }
    };
    Gt {
        id,
        tile,
        act: mhz(&freq, "act_freq"),
        cur: mhz(&freq, "cur_freq"),
        min: mhz(&freq, "min_freq"),
        max: mhz(&freq, "max_freq"),
        rp0: mhz(&freq, "rp0_freq"),
        rpe: mhz(&freq, "rpe_freq"),
        rpn: mhz(&freq, "rpn_freq"),
        rpa: mhz(&freq, "rpa_freq"),
        profile,
        throttle_status,
        throttle_reasons,
        idle_status: sysfs::read_string(&idle.join("idle_status")),
        idle_ms: sysfs::read_u64(&idle.join("idle_residency_ms")),
        idle_name,
        dir,
        kind,
    }
}

/// `power_profile` reads `[base] power_saving` or `base [power_saving]`;
/// the selected token is the one inside brackets.
pub fn parse_profile_brackets(p: &Path, text: &str) -> Avail<String> {
    let inner = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
    };
    match (text.find('['), text.rfind(']')) {
        (Some(open), Some(close)) if close > open + 1 => {
            let token = &text[open + 1..close];
            if inner(token) {
                Avail::Value(token.to_string())
            } else {
                Avail::NotAvailable(Reason::Unparseable(p.to_path_buf(), text.to_string()))
            }
        }
        _ => Avail::NotAvailable(Reason::Unparseable(p.to_path_buf(), text.to_string())),
    }
}

/// The `reasons` file: `none` or space-separated active names.
pub fn parse_reasons_file(text: &str) -> Vec<String> {
    if text == "none" {
        return Vec::new();
    }
    text.split_whitespace().map(str::to_string).collect()
}

/// Canonical order for joined output, then any other `reason_*` in name order.
const CANONICAL_REASONS: [&str; 8] = [
    "pl1",
    "pl2",
    "pl4",
    "thermal",
    "prochot",
    "ratl",
    "vr_thermalert",
    "vr_tdc",
];

/// (`reason_*` file name, is set) pairs → active reason names, stripped of `reason_`.
pub fn derive_reasons_from_flags(flags: &[(String, bool)]) -> Vec<String> {
    let active = |n: &str| {
        flags
            .iter()
            .any(|(f, set)| *set && f.strip_prefix("reason_") == Some(n))
    };
    let mut out: Vec<String> = CANONICAL_REASONS
        .iter()
        .filter(|n| active(n))
        .map(|n| n.to_string())
        .collect();
    let mut others: Vec<String> = flags
        .iter()
        .filter(|(_, set)| *set)
        .filter_map(|(f, _)| f.strip_prefix("reason_"))
        .filter(|n| !CANONICAL_REASONS.contains(n))
        .map(str::to_string)
        .collect();
    others.sort();
    out.extend(others);
    out
}

/// All `reason_`-prefixed file names in a throttle dir, sorted.
pub fn reason_files(throttle_dir: &Path) -> Vec<String> {
    sysfs::list_dir(throttle_dir, |n| n.starts_with("reason_") && n != "reasons")
        .into_iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_profile_brackets as parse_profile;
    use super::GtKind;
    use super::{derive_reasons_from_flags, parse_reasons_file, Path};
    use crate::avail::{Avail, Reason};
    use std::path::PathBuf;

    fn gt(act: Option<u32>, file: Option<&str>) -> super::Gt {
        fn na<T>() -> Avail<T> {
            Avail::NotAvailable(Reason::Missing(PathBuf::new()))
        }
        super::Gt {
            id: 0,
            tile: 0,
            dir: PathBuf::new(),
            kind: GtKind::Render,
            idle_name: na(),
            act: act.map(Avail::Value).unwrap_or_else(na),
            cur: na(),
            min: na(),
            max: na(),
            rp0: na(),
            rpe: na(),
            rpn: na(),
            rpa: na(),
            profile: na(),
            throttle_status: na(),
            throttle_reasons: na(),
            idle_status: file.map(|s| Avail::Value(s.to_string())).unwrap_or_else(na),
            idle_ms: na(),
        }
    }

    #[test]
    fn idle_state_prefers_the_act_freq_truth() {
        // The Battlemage combination: parked GT (act 0) while the file still says c0.
        assert_eq!(
            gt(Some(0), Some("gt-c0")).idle_state().value(),
            Some(&"gt-c6".to_string())
        );
        // Active GT: c0 regardless of what the mirror file last said.
        assert_eq!(
            gt(Some(1200), Some("gt-c0")).idle_state().value(),
            Some(&"gt-c0".to_string())
        );
        // No act_freq at all: the raw file is all there is.
        assert_eq!(
            gt(None, Some("gt-c0")).idle_state().value(),
            Some(&"gt-c0".to_string())
        );
        assert!(gt(None, None).idle_state().value().is_none());
    }

    #[test]
    fn parse_profile_brackets() {
        let p = Path::new("power_profile");
        assert_eq!(
            parse_profile(p, "[base]    power_saving").value(),
            Some(&"base".to_string())
        );
        assert_eq!(
            parse_profile(p, "base    [power_saving]").value(),
            Some(&"power_saving".to_string())
        );
        assert!(parse_profile(p, "base power_saving").value().is_none());
        assert!(parse_profile(p, "[]").value().is_none());
        assert!(parse_profile(p, "[weird!!]").value().is_none());
    }

    #[test]
    fn throttle_reasons_derivation() {
        // From the `reasons` file (6.19+).
        assert_eq!(parse_reasons_file("pl1 thermal"), ["pl1", "thermal"]);
        assert!(parse_reasons_file("none").is_empty());
        // Derived from `reason_*` files: canonical order first regardless of file order.
        let flags = vec![
            ("reason_thermal".to_string(), true),
            ("reason_pl1".to_string(), true),
            ("reason_pl2".to_string(), false),
            ("reason_prochot".to_string(), false),
            ("reason_ratl".to_string(), false),
            ("reason_pl4".to_string(), false),
            ("reason_vr_tdc".to_string(), false),
            ("reason_vr_thermalert".to_string(), false),
        ];
        assert_eq!(derive_reasons_from_flags(&flags), ["pl1", "thermal"]);
        // Non-canonical names (Crescent Island) follow in name order.
        let mut extended = flags.clone();
        extended.push(("reason_psys_pl1".to_string(), true));
        extended.push(("reason_iccmax".to_string(), true));
        extended.push(("reason_soc_thermal".to_string(), true));
        assert_eq!(
            derive_reasons_from_flags(&extended),
            ["pl1", "thermal", "iccmax", "psys_pl1", "soc_thermal"]
        );
        // Nothing active → empty list, and it is a value, not N/A.
        let none = vec![("reason_pl1".to_string(), false)];
        assert!(derive_reasons_from_flags(&none).is_empty());
    }
}
