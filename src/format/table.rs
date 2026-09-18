//! `status` table.

use crate::avail::Avail;
use crate::format::fields::{self, View};
use crate::format::truncate_28;
use crate::format::NA;

/// Content widths of the six columns; total rendered line length is exactly 100.
pub const COLS: [usize; 6] = [3, 28, 12, 8, 16, 14];

pub fn sep(fill: char) -> String {
    let mut s = String::from("+");
    for (i, w) in COLS.iter().enumerate() {
        if i > 0 {
            s.push('+');
        }
        s.push_str(&fill.to_string().repeat(w + 2));
    }
    s.push('+');
    s
}

fn cells(vals: [&str; 6]) -> String {
    let mut s = String::new();
    for (i, v) in vals.iter().enumerate() {
        s.push('|');
        s.push(' ');
        s.push_str(v);
        let pad = COLS[i].saturating_sub(v.chars().count());
        s.extend(std::iter::repeat(' ').take(pad));
        s.push(' ');
    }
    s.push_str("|\n");
    s
}

/// Pair cell `a / b`; both N/A collapse to the single token `N/A`.
pub fn pair(a: Option<String>, b: Option<String>) -> String {
    match (a, b) {
        (None, None) => NA.to_string(),
        (Some(a), Some(b)) => format!("{a} / {b}"),
        (None, Some(b)) => format!("{NA} / {b}"),
        (Some(a), None) => format!("{a} / {NA}"),
    }
}

fn cell_text(a: Avail<crate::format::fields::Cell>) -> Option<String> {
    a.value().map(|c| c.text.clone())
}

/// One frame of the table for the selected devices with their per-device views.
pub fn render(
    kernel: &str,
    timestamp: &str,
    frames: &[(&crate::probe::Device, View<'_>)],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "xe-gmi {} | driver xe | kernel {kernel} | {timestamp}\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str(&sep('-'));
    out.push('\n');
    out.push_str(&cells([
        "Idx",
        "Device",
        "Bus Id",
        "Temp C",
        "Power W",
        "Memory MiB",
    ]));
    out.push_str(&cells([
        "GT",
        "Clock MHz",
        "Profile",
        "Util %",
        "Throttle",
        "Fan RPM",
    ]));
    out.push_str(&sep('='));
    out.push('\n');
    for (_dev, view) in frames {
        out.push_str(&device_row(view));
        for (k, gt) in view.dev.gts.iter().enumerate() {
            out.push_str(&gt_row(view, k, gt));
        }
    }
    out.push_str(&sep('-'));
    out.push('\n');
    out.push_str(
        "Temp = pkg / vram; Power = draw / limit; Memory = used / total; Clock = cur / max.\n",
    );
    let kernel_memory = frames
        .iter()
        .any(|(d, _)| d.kabi.vram().is_some_and(|k| k.used > 0 || k.total == 0));
    if kernel_memory {
        out.push_str("Memory used = kernel allocator (every allocation on the device).\n");
    } else {
        out.push_str("Memory used = resident VRAM summed over visible DRM clients (run as root to see every user).\n");
    }
    out
}

fn device_row(view: &View<'_>) -> String {
    let r = |n: &str| cell_text(fields::resolve(n, None, view));
    let name = truncate_28(&view.dev.name);
    cells([
        &view.dev.index.to_string(),
        &name,
        &view.dev.pci,
        &pair(r("temp.pkg"), r("temp.vram")),
        &pair(r("power.draw"), r("power.limit")),
        &pair(r("memory.used"), r("memory.total")),
    ])
}

fn gt_row(view: &View<'_>, k: usize, gt: &crate::probe::gt::Gt) -> String {
    let clock = pair(
        gt.cur.value().map(|v| v.to_string()),
        gt.max.value().map(|v| v.to_string()),
    );
    let profile = gt
        .profile
        .value()
        .cloned()
        .unwrap_or_else(|| NA.to_string());
    let util = match view.rates.and_then(|r| r.gt_util_pct.get(k)) {
        Some(Avail::Value(v)) => format!("{v:.1}"),
        _ => NA.to_string(),
    };
    let throttle = match &gt.throttle_reasons {
        Avail::Value(rs) if rs.is_empty() => "none".to_string(),
        Avail::Value(rs) => {
            let mut v = rs.clone();
            v.sort_by(|a, b| fields::reason_rank(a).cmp(&fields::reason_rank(b)));
            v.join(",")
        }
        Avail::NotAvailable(_) => NA.to_string(),
    };
    let fan = match view.dev.hwmon.as_ref().map(|h| h.fans.as_slice()) {
        None | Some([]) => NA.to_string(),
        Some(fans) => match fans.iter().find(|(id, _)| *id == k as u32 + 1) {
            Some((_, Avail::Value(rpm))) => rpm.to_string(),
            Some((_, Avail::NotAvailable(_))) => NA.to_string(),
            None => String::new(),
        },
    };
    cells([
        &format!("gt{}", gt.id),
        &clock,
        &profile,
        &util,
        &throttle,
        &fan,
    ])
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_line_is_100_columns() {
        assert_eq!(super::sep('-').chars().count(), 100);
        assert_eq!(super::sep('=').chars().count(), 100);
        let line = super::cells(["a", "b", "c", "d", "e", "f"]);
        assert_eq!(line.trim_end().chars().count(), 100, "{line}");
        let wide = super::cells(["aaa", "b", "c", "d", "e", "f"]);
        assert_eq!(wide.trim_end().chars().count(), 100);
        assert!(wide.starts_with("| aaa |"));
    }

    #[test]
    fn pair_cell_rules() {
        use super::pair;
        assert_eq!(pair(None, None), "N/A");
        assert_eq!(pair(Some("41".into()), Some("38".into())), "41 / 38");
        assert_eq!(pair(None, Some("200.00".into())), "N/A / 200.00");
        assert_eq!(pair(Some("0.00".into()), None), "0.00 / N/A");
    }
}
