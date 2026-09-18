//! Per-process scan of `/proc/<pid>/fdinfo`. Reads only; every parse failure drops
//! that one entry rather than failing the command.

use crate::paths::Roots;
use std::collections::BTreeMap;
use std::fs;

/// Engine classes in canonical column order.
pub const CLASSES: [&str; 5] = ["rcs", "ccs", "vcs", "vecs", "bcs"];

/// One engine class of one client between two samples.
#[derive(Debug, Clone, Copy)]
pub struct ClassCounters {
    pub cycles: u64,
    pub total: Option<u64>,
    pub capacity: u64,
}

/// All five fdinfo statistics of one memory region (spec/23 §6).
#[derive(Debug, Clone, Copy, Default)]
pub struct MemRegion {
    pub total: Option<u64>,
    pub shared: Option<u64>,
    pub active: Option<u64>,
    pub resident: Option<u64>,
    pub purgeable: Option<u64>,
}

/// Region names in canonical order; present in output only when the fdinfo has them.
pub const REGIONS: [&str; 5] = ["system", "gtt", "vram0", "vram1", "stolen"];

/// One merged DRM client (unique on (`drm-pdev`, `drm-client-id`), ).
#[derive(Debug, Clone)]
pub struct ClientSample {
    pub pid: u32,
    pub name: String,
    pub pdev: String,
    pub client_id: u64,
    pub resident_vram: u64,
    pub total_vram: Option<u64>,
    pub active_vram: Option<u64>,
    pub classes: [Option<ClassCounters>; 5],
    /// Every pid holding a shared fd of this client, ascending.
    pub pids: Vec<u32>,
    /// `Uid:` first field of the owning pid.
    pub uid: u32,
    /// `0::/path` of the owning pid.
    pub cgroup: Option<String>,
    pub regions: Vec<(String, MemRegion)>,
}

pub struct Scan {
    pub clients: Vec<ClientSample>,
    /// `Uid:` first field of the fixture/real self status is 0.
    pub visible_all_users: bool,
}

/// `N`, `N KiB`, `N MiB` — `drm_fdinfo_print_size` output, bytes when plain.
pub fn parse_fdinfo_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix(" KiB") {
        return Some(rest.trim().parse::<u64>().ok()? * 1024);
    }
    if let Some(rest) = s.strip_suffix(" MiB") {
        return Some(rest.trim().parse::<u64>().ok()? * 1048576);
    }
    s.parse::<u64>().ok()
}

/// One parsed `key:\tvalue` fdinfo, already filtered to driver `xe`.
pub struct FdInfo {
    pub pid: u32,
    pub pdev: String,
    pub client_id: u64,
    pub values: BTreeMap<String, String>,
}

/// Parse `key:\tvalue` lines of one fdinfo file; `None` unless `drm-driver` is `xe`.
pub fn parse_fdinfo(pid: u32, text: &str) -> Option<FdInfo> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let (k, v) = line.split_once(':')?;
        values.insert(k.trim().to_string(), v.trim().to_string());
    }
    if values.get("drm-driver").map(|s| s.as_str()) != Some("xe") {
        return None;
    }
    Some(FdInfo {
        pid,
        pdev: values.get("drm-pdev")?.clone(),
        client_id: values.get("drm-client-id")?.parse().ok()?,
        values,
    })
}

/// Merge per-fd infos into unique clients. Several fds of one process (or several processes
/// sharing an fd) report the same client id → counted once; the first pid seen owns it.
pub fn merge_clients(infos: Vec<FdInfo>) -> Vec<ClientSample> {
    let mut order: Vec<(String, u64)> = Vec::new();
    let mut by_key: BTreeMap<(String, u64), ClientSample> = BTreeMap::new();
    for info in infos {
        let key = (info.pdev.clone(), info.client_id);
        if let Some(existing) = by_key.get_mut(&key) {
            if !existing.pids.contains(&info.pid) {
                existing.pids.push(info.pid);
            }
            continue;
        }
        let size = |name: &str| info.values.get(name).and_then(|v| parse_fdinfo_size(v));
        let regions: Vec<(String, MemRegion)> = REGIONS
            .iter()
            .filter_map(|r| {
                let g = |s: &str| {
                    info.values
                        .get(&format!("drm-{s}-{r}"))
                        .and_then(|v| parse_fdinfo_size(v))
                };
                let m = MemRegion {
                    total: g("total"),
                    shared: g("shared"),
                    active: g("active"),
                    resident: g("resident"),
                    purgeable: g("purgeable"),
                };
                [m.total, m.shared, m.active, m.resident, m.purgeable]
                    .iter()
                    .any(|x| x.is_some())
                    .then(|| (r.to_string(), m))
            })
            .collect();
        let classes = std::array::from_fn(|i| {
            let c = CLASSES[i];
            info.values
                .get(&format!("drm-cycles-{c}"))
                .and_then(|v| v.parse::<u64>().ok())
                .map(|cycles| ClassCounters {
                    cycles,
                    total: info
                        .values
                        .get(&format!("drm-total-cycles-{c}"))
                        .and_then(|v| v.parse().ok()),
                    capacity: info
                        .values
                        .get(&format!("drm-engine-capacity-{c}"))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1),
                })
        });
        order.push(key.clone());
        by_key.insert(
            key,
            ClientSample {
                pid: info.pid,
                name: String::new(), // filled from comm below in scan; merge keeps first owner
                pdev: info.pdev,
                client_id: info.client_id,
                resident_vram: size("drm-resident-vram0").unwrap_or(0),
                total_vram: size("drm-total-vram0"),
                active_vram: size("drm-active-vram0"),
                classes,
                pids: vec![info.pid],
                uid: 0,
                cgroup: None,
                regions,
            },
        );
    }
    order
        .into_iter()
        .filter_map(|k| {
            let mut c = by_key.remove(&k)?;
            c.pids.sort_unstable();
            c.pids.dedup();
            Some(c)
        })
        .collect()
}

/// Enumerate processes owning fds on xe DRM nodes. `comm` provides the name.
pub fn scan(roots: &Roots, xe_pcis: &[&str]) -> Scan {
    let visible_all_users = crate::paths::uid(roots) == 0;
    let mut infos: Vec<FdInfo> = Vec::new();
    let mut pids: Vec<u32> = match fs::read_dir(&roots.procfs) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    pids.sort_unstable();
    for pid in pids {
        let pdir = roots.procfs.join(pid.to_string());
        let Ok(status) = fs::read_to_string(pdir.join("status")) else {
            continue;
        };
        if status.lines().any(|l| {
            l.starts_with("State:")
                && l.trim_start()
                    .trim_start_matches("State:")
                    .trim_start()
                    .starts_with('Z')
        }) {
            continue; // zombies have no fds
        }
        let Ok(fd_rd) = fs::read_dir(pdir.join("fd")) else {
            continue;
        };
        let mut fds: Vec<u32> = fd_rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
            .collect();
        fds.sort_unstable();
        for fd in fds {
            let target = match fs::read_link(pdir.join("fd").join(fd.to_string())) {
                Ok(t) => t.to_string_lossy().into_owned(),
                Err(_) => continue,
            };
            if !target.starts_with("/dev/dri/") {
                continue; // never open /dev nodes, just read the link target
            }
            let Ok(info) = fs::read_to_string(pdir.join("fdinfo").join(fd.to_string())) else {
                continue;
            };
            if let Some(fi) = parse_fdinfo(pid, &info) {
                if xe_pcis.contains(&fi.pdev.as_str()) {
                    infos.push(fi);
                }
            }
        }
    }
    // First pid (ascending) owns each client and supplies the name.
    let mut clients = merge_clients(infos);
    for c in &mut clients {
        // name = comm of the lowest pid holding the client
        let name_pid = c.pids.first().copied().unwrap_or(c.pid);
        if let Ok(comm) = fs::read_to_string(roots.procfs.join(name_pid.to_string()).join("comm")) {
            c.name = comm.trim_end_matches(['\n', '\r']).to_string();
        }
        let pdir = roots.procfs.join(c.pid.to_string());
        if let Ok(status) = fs::read_to_string(pdir.join("status")) {
            if let Some(line) = status.lines().find(|l| l.starts_with("Uid:")) {
                c.uid = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
        }
        if let Ok(cg) = fs::read_to_string(pdir.join("cgroup")) {
            // `0::/path`; legacy hierarchies (no `0::`) are ignored
            c.cgroup = cg
                .trim()
                .strip_prefix("0::")
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
        }
    }
    Scan {
        clients,
        visible_all_users,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::parse_fdinfo_size as parse_size;

    #[test]
    fn parse_fdinfo_size() {
        assert_eq!(parse_size("0"), Some(0));
        assert_eq!(parse_size("192 KiB"), Some(196_608));
        assert_eq!(parse_size("1024 MiB"), Some(1_073_741_824));
        assert_eq!(parse_size("17 MiB"), Some(17 * 1048576));
        assert_eq!(parse_size("garbage"), None);
    }

    #[test]
    fn merge_dedupes_client_ids() {
        let mk = |pid: u32, pdev: &str, id: u64| FdInfo {
            pid,
            pdev: pdev.into(),
            client_id: id,
            values: BTreeMap::new(),
        };
        // two fds, same pdev + client → one client; different pdev → two.
        let merged = merge_clients(vec![
            mk(4300, "0000:e3:00.0", 7),
            mk(4300, "0000:e3:00.0", 7),
            mk(4300, "0000:17:00.0", 7),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].pid, 4300);
        assert_eq!(merged[0].client_id, 7);
        // first pid seen owns the client
        let merged = merge_clients(vec![mk(500, "0000:e3:00.0", 3), mk(400, "0000:e3:00.0", 3)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].pid, 500);
    }

    #[test]
    fn fdinfo_requires_xe_driver() {
        let nvidia = "drm-driver:\tnvidia\n";
        assert!(parse_fdinfo(1, nvidia).is_none());
        let xe = "drm-driver:\txe\ndrm-client-id:\t42\ndrm-pdev:\t0000:e3:00.0\n";
        let fi = parse_fdinfo(1, xe).unwrap();
        assert_eq!(fi.client_id, 42);
        assert_eq!(fi.pdev, "0000:e3:00.0");
    }
}
