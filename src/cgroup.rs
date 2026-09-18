//! cgroup v2 dmem controller (spec/23 §7): walk, per-region accounting, capacity.

use crate::error::{Error, Result};
use crate::format::fields::mib;
use crate::paths::Roots;
use std::path::{Path, PathBuf};

/// Per-region limit map entry: `bytes` value or `max` (None = no limit).
pub type Regions = Vec<(String, u64)>;

/// Limit entries: `None` value = `max`.
pub type Limits = Vec<(String, Option<u64>)>;

/// (pid, name, client_id) of an attributed client.
pub type ClientInfo = (u32, String, u64);

#[derive(Default)]
pub struct Entry {
    pub path: String,
    pub current: Regions,
    pub max: Limits,
    pub min: Regions,
    pub low: Regions,
}

fn parse_region_file(p: &Path) -> Option<Regions> {
    let text = std::fs::read_to_string(p).ok()?;
    let mut out = Vec::new();
    for line in text.lines() {
        let (region, val) = line.split_once(char::is_whitespace)?;
        let val = val.trim();
        if val != "max" {
            out.push((region.to_string(), val.parse::<u64>().ok()?));
        }
    }
    Some(out)
}

fn parse_max_file(p: &Path) -> Option<Limits> {
    let text = std::fs::read_to_string(p).ok()?;
    let mut out = Vec::new();
    for line in text.lines() {
        let (region, val) = line.split_once(char::is_whitespace)?;
        let val = val.trim();
        out.push((region.to_string(), val.parse::<u64>().ok()));
    }
    Some(out)
}

/// Bundle redaction of a cgroup path: the whole path is replaced by `<redacted>/` plus the first
/// eight hex digits of its FNV-1a 64-bit hash, so equal paths stay equal across a bundle without
/// revealing slices, unit names or user ids.
pub fn redact_path(path: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("<redacted>/{:08x}", h >> 32)
}

/// Every cgroup directory with `dmem.current`, depth ≤ 8, host-view paths.
pub fn walk(roots: &Roots) -> Vec<Entry> {
    let mut out = Vec::new();
    walk_at(roots, &roots.cgroup.clone(), 0, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn walk_at(roots: &Roots, dir: &Path, depth: u32, out: &mut Vec<Entry>) {
    if depth > 8 {
        return;
    }
    if dir.join("dmem.current").exists() {
        let rel = dir.strip_prefix(&roots.cgroup).unwrap_or(Path::new(""));
        let path = format!("/{}", rel.to_string_lossy().trim_start_matches('/'));
        // the root cgroup just mirrors the sum of everything; the table lists leaf groups
        let is_root = rel.as_os_str().is_empty();
        let mut e = Entry {
            path,
            ..Default::default()
        };
        e.current = parse_region_file(&dir.join("dmem.current")).unwrap_or_default();
        e.max = parse_max_file(&dir.join("dmem.max")).unwrap_or_default();
        e.min = parse_region_file(&dir.join("dmem.min")).unwrap_or_default();
        e.low = parse_region_file(&dir.join("dmem.low")).unwrap_or_default();
        if !is_root {
            out.push(e);
        }
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subs.sort();
    for s in subs {
        walk_at(roots, &s, depth + 1, out);
    }
}

/// `cgroup set --vram-max`: `max` (None) or `<digits>[K|M|G|T]` (bytes).
pub fn parse_limit(s: &str) -> Result<Option<u64>> {
    let t = s.trim();
    if t == "max" {
        return Ok(None);
    }
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let bad = || {
        Error::Usage(format!(
            "invalid size {s:?}: accepted forms are bytes, 512K, 8M, 8G, 2T and max"
        ))
    };
    if digits == 0 {
        return Err(bad());
    }
    let mult = match &t[digits..] {
        "" => 1u64,
        "K" => 1024,
        "M" => 1024 * 1024,
        "G" => 1024 * 1024 * 1024,
        "T" => 1024u64 * 1024 * 1024 * 1024,
        _ => return Err(bad()),
    };
    t[..digits]
        .parse::<u64>()
        .ok()
        .and_then(|v| v.checked_mul(mult))
        .map(Some)
        .ok_or_else(bad)
}

/// Controller presence = root `dmem.capacity` exists.
pub fn capacity(roots: &Roots) -> Option<Regions> {
    parse_region_file(&roots.cgroup.join("dmem.capacity"))
}

pub fn require_controller(roots: &Roots) -> Result<Regions> {
    capacity(roots).ok_or_else(|| {
        Error::Unavailable(format!(
            "dmem cgroup controller not present (no dmem.capacity under {})",
            roots.cgroup.display()
        ))
    })
}

/// Bytes of the selected devices' VRAM regions summed for one entry.
pub fn entry_vram(e: &Entry, xe_pcis: &[&str]) -> u64 {
    e.current
        .iter()
        .filter(|(r, _)| is_vram_for(r, xe_pcis))
        .map(|(_, b)| *b)
        .sum()
}

/// Sum of numeric limits across the selected devices; `None` renders as `max`
/// (regions limited to `max` contribute nothing to the sum).
pub fn entry_vram_max(e: &Entry, xe_pcis: &[&str]) -> Option<u64> {
    let mut sum = 0u64;
    let mut any_numeric = false;
    for (r, v) in &e.max {
        if let (true, Some(b)) = (is_vram_for(r, xe_pcis), v) {
            sum += b;
            any_numeric = true;
        }
    }
    any_numeric.then_some(sum)
}

fn is_vram_for(region: &str, xe_pcis: &[&str]) -> bool {
    let Some(rest) = region.strip_prefix("drm/") else {
        return false;
    };
    let Some((pci, mem)) = rest.split_once('/') else {
        return false;
    };
    mem.starts_with("vram") && xe_pcis.contains(&pci)
}

pub fn row_visible(e: &Entry, xe_pcis: &[&str], clients: usize) -> bool {
    entry_vram(e, xe_pcis) > 0
        || clients > 0
        || e.max
            .iter()
            .any(|(r, v)| is_vram_for(r, xe_pcis) && v.is_some())
}

/// `CGROUP` ljust(46), `VRAM USED` rjust(10), `VRAM MAX` rjust(10), `CLIENTS` rjust(8), two spaces.
pub fn text(
    entries: &[Entry],
    capacities: &[(String, u64)],
    xe_pcis: &[&str],
    clients_of: &dyn Fn(&str) -> usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<47}  {:<13}{}",
        "CGROUP",
        "VRAM USED",
        format_args!("{:<11}CLIENTS", "VRAM MAX")
    ));
    out.push('\n');
    for e in entries {
        let used = entry_vram(e, xe_pcis);
        let mx = entry_vram_max(e, xe_pcis);
        let n = clients_of(&e.path);
        let mx_s = match mx {
            None => "max".to_string(),
            Some(b) => format!("{} MiB", mib(b)),
        };
        out.push_str(&format!(
            "{:<47}  {:>9}  {:>10}   {:>7}\n",
            e.path,
            format!("{} MiB", mib(used)),
            mx_s,
            n
        ));
    }
    for (region, bytes) in capacities {
        out.push_str(&format!("Capacity: {} MiB ({})\n", mib(*bytes), region));
    }
    out
}

pub fn json(
    entries: &[Entry],
    capacities: &[(String, u64)],
    xe_pcis: &[&str],
    clients_of: &dyn Fn(&str) -> Vec<ClientInfo>,
) -> String {
    use crate::format::json::{num_u, Json};
    let arr = Json::Arr(
        entries
            .iter()
            .map(|e| {
                let cs = clients_of(&e.path);
                Json::Obj(vec![
                    ("path".into(), Json::Str(e.path.clone())),
                    ("current_bytes".into(), num_u(entry_vram(e, xe_pcis))),
                    (
                        "max_bytes".into(),
                        entry_vram_max(e, xe_pcis).map(num_u).unwrap_or(Json::Null),
                    ),
                    ("min_bytes".into(), num_u(sum_regions(&e.min, xe_pcis))),
                    ("low_bytes".into(), num_u(sum_regions(&e.low, xe_pcis))),
                    (
                        "clients".into(),
                        Json::Arr(
                            cs.into_iter()
                                .map(|(pid, name, cid)| {
                                    Json::Obj(vec![
                                        ("pid".into(), num_u(pid as u64)),
                                        ("name".into(), Json::Str(name)),
                                        ("client_id".into(), num_u(cid)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    );
    let root = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("cgroups".into(), arr),
        (
            "capacity".into(),
            Json::Arr(
                capacities
                    .iter()
                    .map(|(r, b)| {
                        Json::Obj(vec![
                            ("region".into(), Json::Str(r.clone())),
                            ("bytes".into(), num_u(*b)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    let mut s = String::new();
    crate::format::json::write(&root, &mut s);
    s
}

fn sum_regions(rs: &Regions, xe_pcis: &[&str]) -> u64 {
    rs.iter()
        .filter(|(r, _)| is_vram_for(r, xe_pcis))
        .map(|(_, b)| *b)
        .sum()
}

/// `cgroup show PATH`: per region `  <region>` then `    current/max/...` (25-wide rule).
pub fn show(
    roots: &Roots,
    path: &str,
    xe_pcis: &[&str],
    clients_of: &dyn Fn(&str) -> Vec<String>,
) -> Result<String> {
    require_controller(roots)?;
    let rel = path.trim_start_matches('/');
    let dir = roots.cgroup.join(rel);
    let current = parse_region_file(&dir.join("dmem.current"))
        .ok_or_else(|| Error::Unavailable(format!("no cgroup at {} (no dmem.current)", path)))?;
    let max = parse_max_file(&dir.join("dmem.max")).unwrap_or_default();
    let min = parse_region_file(&dir.join("dmem.min")).unwrap_or_default();
    let low = parse_region_file(&dir.join("dmem.low")).unwrap_or_default();
    let mut out = String::new();
    let mut regions: Vec<&String> = current.iter().map(|(r, _)| r).collect();
    regions.extend(min.iter().chain(low.iter()).map(|(r, _)| r));
    regions.sort();
    regions.dedup();
    for r in regions {
        if !xe_pcis.iter().any(|p| r.contains(p)) {
            continue;
        }
        out.push_str(&format!("  {r}\n"));
        let get_cur = || {
            current
                .iter()
                .find(|(x, _)| x == r)
                .map(|(_, b)| format!("{} MiB", mib(*b)))
        };
        let get_lim = |v: &Option<u64>| {
            v.map(|b| format!("{} MiB", mib(b)))
                .unwrap_or_else(|| "max".into())
        };
        let lim = max.iter().find(|(x, _)| x == r).map(|(_, v)| get_lim(v));
        let cs = clients_of(path);
        for (label, value) in [
            ("current", get_cur().unwrap_or_else(|| "0 MiB".into())),
            ("max", lim.unwrap_or_else(|| "max".into())),
            (
                "min",
                min.iter()
                    .find(|(x, _)| x == r)
                    .map(|(_, b)| format!("{} MiB", mib(*b)))
                    .unwrap_or_else(|| "0 MiB".into()),
            ),
            (
                "low",
                low.iter()
                    .find(|(x, _)| x == r)
                    .map(|(_, b)| format!("{} MiB", mib(*b)))
                    .unwrap_or_else(|| "0 MiB".into()),
            ),
            (
                "clients",
                if cs.is_empty() {
                    "0".into()
                } else {
                    format!("{} ({})", cs.len(), cs.join(", "))
                },
            ),
        ] {
            out.push_str(&format!("    {:<25}: {}\n", label, value));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cgroup_parse_limit_units_and_max() {
        assert_eq!(parse_limit("max").unwrap(), None);
        assert_eq!(parse_limit("8G").unwrap(), Some(8 << 30));
        assert_eq!(parse_limit("512K").unwrap(), Some(512 * 1024));
        assert_eq!(parse_limit("2T").unwrap(), Some(2u64 << 40));
        assert_eq!(parse_limit("123").unwrap(), Some(123));
        assert!(parse_limit("-1").is_err());
        assert!(parse_limit("8g").is_err());
        assert!(parse_limit("8 Gi").is_err());
        assert!(parse_limit("").is_err());
    }

    use super::*;

    #[test]
    fn vram_filter_matches_regions_of_selected_devices() {
        assert!(is_vram_for("drm/0000:e3:00.0/vram0", &["0000:e3:00.0"]));
        assert!(!is_vram_for("drm/0000:e3:00.0/vram0", &["0000:17:00.0"]));
        assert!(!is_vram_for("drm/0000:e3:00.0/stolen", &["0000:e3:00.0"]));
        assert!(!is_vram_for("something", &["0000:e3:00.0"]));
    }

    fn tmp(name: &str, text: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("xe-gmi-cgroup-{}-{name}", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn dmem_parse_current_max() {
        let cur = tmp(
            "current",
            "drm/0000:e3:00.0/vram0 1073741824\ndrm/0000:03:00.0/vram0 0\n",
        );
        let max = tmp(
            "max",
            "drm/0000:e3:00.0/vram0 21474836480\ndrm/0000:03:00.0/vram0 max\n",
        );
        let c = parse_region_file(&cur).unwrap();
        assert_eq!(
            c,
            vec![
                ("drm/0000:e3:00.0/vram0".to_string(), 1073741824),
                ("drm/0000:03:00.0/vram0".to_string(), 0)
            ]
        );
        let m = parse_max_file(&max).unwrap();
        assert_eq!(
            m[0],
            ("drm/0000:e3:00.0/vram0".to_string(), Some(21474836480))
        );
        assert_eq!(m[1], ("drm/0000:03:00.0/vram0".to_string(), None));
        // a malformed line makes the whole file unusable rather than partially parsed
        let bad = tmp("bad", "drm/0000:e3:00.0/vram0\n");
        assert!(parse_region_file(&bad).is_none());
        for p in [cur, max, bad] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn redaction_stable_hash() {
        let a = redact_path("/user.slice/user-1000.slice/session-2.scope");
        assert_eq!(
            a,
            redact_path("/user.slice/user-1000.slice/session-2.scope")
        );
        assert!(
            a.starts_with("<redacted>/") && a.len() == "<redacted>/".len() + 8,
            "{a}"
        );
        assert!(!a.contains("user"));
        assert_ne!(a, redact_path("/system.slice/llama.service"));
    }
}
