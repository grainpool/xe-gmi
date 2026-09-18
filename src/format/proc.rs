//! `processes` table.

use crate::avail::Avail;
use crate::cli::{GroupBy, ProcSort};
use crate::format::NA;
use crate::sample::Rates;

#[derive(Clone)]
pub struct Row {
    pub pid: u32,
    pub name: String,
    pub pci: String,
    pub client_id: u64,
    pub pct: [Avail<f64>; 5],
    pub resident_vram: u64,
    pub pids: Vec<u32>,
    pub uid: u32,
    pub cgroup: Option<String>,
}

/// Rows for one frame from the device-side rates (already merged clients).
pub fn rows(dev_pci: &str, rates: &Rates) -> Vec<Row> {
    rates
        .per_client
        .iter()
        .filter(|c| c.client.pdev == dev_pci)
        .map(|c| Row {
            pid: c.client.pid,
            name: c.client.name.clone(),
            pci: c.client.pdev.clone(),
            client_id: c.client.client_id,
            pct: c.pct.clone(),
            resident_vram: c.client.resident_vram,
            pids: c.client.pids.clone(),
            uid: c.client.uid,
            cgroup: c.client.cgroup.clone(),
        })
        .collect()
}

pub fn sort_rows(rows: &mut [Row], by: ProcSort) {
    match by {
        ProcSort::Vram => rows.sort_by(|a, b| {
            b.resident_vram
                .cmp(&a.resident_vram)
                .then(a.pid.cmp(&b.pid))
        }),
        ProcSort::Util => rows.sort_by(|a, b| {
            b.pct
                .iter()
                .filter_map(|p| p.value().copied())
                .sum::<f64>()
                .partial_cmp(&a.pct.iter().filter_map(|p| p.value().copied()).sum::<f64>())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.pid.cmp(&b.pid))
        }),
        ProcSort::Pid => rows.sort_by_key(|r| r.pid),
    }
}

pub fn render_grouped(rows: &[Row], by: GroupBy, visible_all_users: bool) -> String {
    match by {
        GroupBy::Pid => render(rows, visible_all_users),
        GroupBy::Client => render_client(rows, visible_all_users),
        GroupBy::Cgroup => render_group(rows, "CGROUP", 46, visible_all_users, |r| {
            r.cgroup.clone().unwrap_or_else(|| "N/A".into())
        }),
        GroupBy::User => render_group(rows, "UID", 6, visible_all_users, |r| r.uid.to_string()),
    }
}

fn sum_pct(rows: &[&Row]) -> [Avail<f64>; 5] {
    std::array::from_fn(|i| {
        let vals: Vec<f64> = rows
            .iter()
            .filter_map(|r| r.pct[i].value().copied())
            .collect();
        if vals.is_empty() {
            Avail::NotAvailable(crate::avail::Reason::NotSupported("no engine counters"))
        } else {
            Avail::Value(vals.iter().sum())
        }
    })
}

fn pct_text(pct: &[Avail<f64>; 5]) -> String {
    (0..5)
        .map(|i| match pct[i].value() {
            Some(v) => format!("{v:.1}"),
            None => NA.to_string(),
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// `cgroup` / `user` grouped tables (spec/22): left key block, `BUS ID`, `CLIENTS` rjust(7),
/// right block identical to the default table.
fn render_group(
    rows: &[Row],
    key_label: &str,
    key_w: usize,
    visible_all_users: bool,
    key_of: fn(&Row) -> String,
) -> String {
    let mut groups: Vec<(String, String, Vec<&Row>)> = Vec::new();
    for r in rows {
        let k = key_of(r);
        match groups
            .iter_mut()
            .find(|(gk, gp, _)| *gk == k && *gp == r.pci)
        {
            Some((_, _, v)) => v.push(r),
            None => groups.push((k, r.pci.clone(), vec![r])),
        }
    }
    groups.sort_by(|a, b| {
        let va: u64 = a.2.iter().map(|r| r.resident_vram).sum();
        let vb: u64 = b.2.iter().map(|r| r.resident_vram).sum();
        vb.cmp(&va).then(a.0.cmp(&b.0))
    });
    let mut out = String::new();
    out.push_str(&format!(
        "{:<key_w$}  {:<12}  {:>7}  {:>7}  {:>8}  {:>6}  {:>8}  {:>5}  {:>8}\n",
        key_label,
        "BUS ID",
        "CLIENTS",
        "RENDER%",
        "COMPUTE%",
        "VIDEO%",
        "ENHANCE%",
        "COPY%",
        "VRAM MiB",
        key_w = key_w
    ));
    if groups.is_empty() {
        out.push_str(if visible_all_users {
            "(no visible DRM clients on xe devices)\n"
        } else {
            "(no visible DRM clients on xe devices; run as root to see every user)\n"
        });
        return out;
    }
    for (k, pci, members) in &groups {
        let pct = sum_pct(members);
        let vram: u64 = members.iter().map(|r| r.resident_vram).sum();
        let cell = |i: usize| match pct[i].value() {
            Some(v) => format!("{v:.1}"),
            None => NA.to_string(),
        };
        out.push_str(&format!(
            "{:<key_w$}  {:<12}  {:>7}  {:>7}  {:>8}  {:>6}  {:>8}  {:>5}  {:>8}\n",
            k,
            pci,
            members.len(),
            cell(0),
            cell(1),
            cell(2),
            cell(3),
            cell(4),
            vram / (1 << 20),
            key_w = key_w
        ));
    }
    out
}

/// `client` grouped table: one row per client with every sharing pid.
fn render_client(rows: &[Row], visible_all_users: bool) -> String {
    let pids_w = rows
        .iter()
        .map(|r| r.pids_string().chars().count())
        .max()
        .unwrap_or(0)
        .max(9);
    let name_w = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0)
        .max(12);
    let mut out = String::new();
    out.push_str(&format!(
        "{:<6}  {:<12}  {:<pids_w$}  {:<name_w$}  {:>7}  {:>8}  {:>6}  {:>8}  {:>5}  {:>8}\n",
        "CLIENT",
        "BUS ID",
        "PIDS",
        "NAME",
        "RENDER%",
        "COMPUTE%",
        "VIDEO%",
        "ENHANCE%",
        "COPY%",
        "VRAM MiB",
        pids_w = pids_w,
        name_w = name_w
    ));
    if rows.is_empty() {
        out.push_str(if visible_all_users {
            "(no visible DRM clients on xe devices)\n"
        } else {
            "(no visible DRM clients on xe devices; run as root to see every user)\n"
        });
        return out;
    }
    for r in rows {
        let cell = |i: usize| match r.pct[i].value() {
            Some(v) => format!("{v:.1}"),
            None => NA.to_string(),
        };
        out.push_str(&format!(
            "{:<6}  {:<12}  {:<pids_w$}  {:<name_w$}  {:>7}  {:>8}  {:>6}  {:>8}  {:>5}  {:>8}\n",
            r.client_id,
            r.pci,
            r.pids_string(),
            r.name,
            cell(0),
            cell(1),
            cell(2),
            cell(3),
            cell(4),
            r.resident_vram / (1 << 20),
            pids_w = pids_w,
            name_w = name_w
        ));
    }
    out
}

impl Row {
    pub fn pids_string(&self) -> String {
        self.pids
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Render the table; empty-visible note per
pub fn render(rows: &[Row], visible_all_users: bool) -> String {
    let name_w = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0)
        .max(12);
    let mut out = String::new();
    out.push_str(&format!(
        "{:<5}  {:<name_w$}  {:<12}  {:<6}  {:>7}  {:>8}  {:>6}  {:>8}  {:>5}  {:>8}\n",
        "PID",
        "NAME",
        "BUS ID",
        "CLIENT",
        "RENDER%",
        "COMPUTE%",
        "VIDEO%",
        "ENHANCE%",
        "COPY%",
        "VRAM MiB",
        name_w = name_w
    ));
    if rows.is_empty() {
        out.push_str(if visible_all_users {
            "(no visible DRM clients on xe devices)\n"
        } else {
            "(no visible DRM clients on xe devices; run as root to see every user)\n"
        });
        return out;
    }
    for r in rows {
        let pct = |i: usize| match r.pct[i].value() {
            Some(v) => format!("{v:.1}"),
            None => NA.to_string(),
        };
        out.push_str(&format!(
            "{:<5}  {:<name_w$}  {:<12}  {:<6}  {:>7}  {:>8}  {:>6}  {:>8}  {:>5}  {:>8}\n",
            r.pid,
            r.name,
            r.pci,
            r.client_id,
            pct(0),
            pct(1),
            pct(2),
            pct(3),
            pct(4),
            r.resident_vram / (1 << 20),
            name_w = name_w
        ));
    }
    out
}
