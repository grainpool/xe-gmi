//! Kernel crash dumps (`class/devcoredump`, spec/23 §3) and the kmsg wedged scan (spec/23 §4).

use crate::error::{Error, Result};
use crate::paths::Roots;
use crate::probe::Device;
use crate::sysfs;
use crate::time;
use std::io::Read as _;
use std::path::{Path, PathBuf};

pub struct Dump {
    pub id: u32,
    pub pci: String,
    pub reason: Option<String>,
    /// Whole-second epoch from the dump header `Snapshot time:`.
    pub snapshot: Option<u64>,
    pub size: Option<u64>,
}

fn class_dir(roots: &Roots) -> PathBuf {
    roots.sysfs.join("class/devcoredump")
}

pub fn data_path(roots: &Roots, id: u32) -> PathBuf {
    class_dir(roots).join(format!("devcd{id}")).join("data")
}

/// Parse the first 4 KiB of a dump for `Reason:` and `Snapshot time:` (spec/23 §3).
pub fn parse_header(data: &Path) -> (Option<String>, Option<u64>, Option<u64>) {
    let Ok(mut f) = std::fs::File::open(data) else {
        return (None, None, None);
    };
    let mut buf = vec![0u8; 4096];
    let n = f.read(&mut buf).unwrap_or(0);
    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
    let size = f.metadata().ok().map(|m| m.len());
    let mut reason = None;
    let mut snapshot = None;
    for line in text.lines() {
        if let Some(r) = line.strip_prefix("Reason: ") {
            reason = Some(r.to_string());
        } else if let Some(t) = line.strip_prefix("Snapshot time: ") {
            snapshot = t.split('.').next().and_then(|s| s.parse().ok());
        }
    }
    (reason, snapshot, size)
}

fn dump_dir(roots: &Roots, id: u32) -> PathBuf {
    class_dir(roots).join(format!("devcd{id}"))
}

/// Pending dumps whose `failing_device` is one of the xe devices, id order.
pub fn list(roots: &Roots, devs: &[&Device]) -> Vec<Dump> {
    let mut out = Vec::new();
    for d in devs {
        out.extend(list_for_pci(roots, &d.pci));
    }
    out.sort_by_key(|d| d.id);
    out
}

/// Pending dumps for one PCI address, id order.
pub fn list_for_pci(roots: &Roots, pci: &str) -> Vec<Dump> {
    let Ok(entries) = std::fs::read_dir(class_dir(roots)) else {
        return Vec::new();
    };
    let mut ids: Vec<u32> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(n) = name.strip_prefix("devcd") {
            if let Ok(id) = n.parse::<u32>() {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();
    let mut out = Vec::new();
    for id in ids {
        let dir = dump_dir(roots, id);
        let Some(failing) = sysfs::link_basename(&dir.join("failing_device"))
            .value()
            .cloned()
        else {
            continue;
        };
        if failing != pci {
            continue;
        }
        let (reason, snapshot, size) = parse_header(&dir.join("data"));
        out.push(Dump {
            id,
            pci: failing,
            reason,
            snapshot,
            size,
        });
    }
    out
}

/// One dump by id; `Unavailable("no crash dump with id N")` (contract) when absent or non-xe.
pub fn get(roots: &Roots, devs: &[&Device], id: u32) -> Result<Dump> {
    let dir = dump_dir(roots, id);
    let failing = sysfs::link_basename(&dir.join("failing_device"));
    let pci = failing.value().cloned();
    let matched = match &pci {
        Some(p) => devs.iter().any(|d| d.pci == *p),
        None => false,
    };
    if !matched {
        return Err(Error::Unavailable(format!("no crash dump with id {id}")));
    }
    let data = dir.join("data");
    let (reason, snapshot, size) = parse_header(&data);
    Ok(Dump {
        id,
        pci: pci.unwrap_or_default(),
        reason,
        snapshot,
        size,
    })
}

/// Full dump bytes (for `show`/`save`; the header-only limit does not apply here).
pub fn read_dump(roots: &Roots, id: u32) -> Result<Vec<u8>> {
    std::fs::read(data_path(roots, id)).map_err(|e| Error::Io {
        path: data_path(roots, id),
        source: e,
    })
}

/// `crash release`: any write releases the dump; the tool writes `1` (spec/23 §3).
pub fn release(roots: &Roots, id: u32) -> Result<()> {
    if !dir_exists(roots, id) {
        return Err(Error::Unavailable(format!("no crash dump with id {id}")));
    }
    crate::write::release_coredump(roots, &data_path(roots, id), "sudo xe-gmi crash release")?;
    Ok(())
}

fn dir_exists(roots: &Roots, id: u32) -> bool {
    dump_dir(roots, id).join("failing_device").exists()
}

/// Last wedged/coredump/GuC-timeout kmsg record for `pci`, cleaned for display (spec/23 §4).
/// Returns None when the log is absent or has no matching record.
pub fn wedged_message(roots: &Roots, pci: &str) -> Option<String> {
    let path = roots.kmsg.as_ref()?;
    let text = read_kmsg(path).ok()?;
    let prefix = format!("xe {pci}: ");
    let mut last_any: Option<String> = None;
    let mut last_wedged: Option<String> = None;
    for line in text.lines() {
        // record format: `prio,seq,timestamp,-;message`
        let Some((_, msg)) = line.split_once(";") else {
            continue;
        };
        let Some(rest) = msg.strip_prefix(&prefix) else {
            continue;
        };
        let clean = clean_drm(rest);
        let lc = clean.to_lowercase();
        if lc.contains("wedged") {
            last_wedged = Some(clean.clone());
        } else if lc.contains("coredump") || lc.contains("guc timeout") {
            last_any = Some(clean);
        }
    }
    last_wedged.or(last_any).map(|m| format!("{pci} {m}"))
}

fn read_kmsg(path: &Path) -> Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0o4000) // O_NONBLOCK: never wait on a live console
        .open(path)
        .map_err(|e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
    let mut s = String::new();
    f.read_to_string(&mut s).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(s)
}

/// Strip the driver's log decorations: `[drm] `, `*ERROR* ` (repeated prefixes).
fn clean_drm(msg: &str) -> String {
    let mut s = msg.trim_start();
    loop {
        let t = s
            .strip_prefix("[drm] ")
            .or_else(|| s.strip_prefix("*ERROR* "))
            .or_else(|| s.strip_prefix("*NOTICE* "))
            .or_else(|| s.strip_prefix("*WARNING* "));
        match t {
            Some(x) => s = x.trim_start(),
            None => break,
        }
    }
    s.to_string()
}

impl Dump {
    pub fn snapshot_iso(&self) -> String {
        match self.snapshot {
            Some(s) => time::iso8601(s),
            None => "N/A".into(),
        }
    }
}

/// Text table for `crash list` (spec/22).
pub fn list_text(dumps: &[Dump]) -> String {
    if dumps.is_empty() {
        return "(no pending crash dumps)\n".into();
    }
    let mut out = format!(
        "{:<3}  {:<12}  {:<40}  {}\n",
        "ID", "DEVICE", "REASON", "SNAPSHOT TIME"
    );
    for d in dumps {
        out.push_str(&format!(
            "{:<3}  {:<12}  {:<40}  {}\n",
            d.id,
            d.pci,
            d.reason.clone().unwrap_or_else(|| "N/A".into()),
            d.snapshot_iso()
        ));
    }
    out
}

pub fn list_json(dumps: &[Dump], ts: &str) -> String {
    use crate::format::json::{num_u, Json};
    let arr = Json::Arr(
        dumps
            .iter()
            .map(|d| {
                Json::Obj(vec![
                    ("id".into(), num_u(d.id as u64)),
                    ("pci_address".into(), Json::Str(d.pci.clone())),
                    (
                        "reason".into(),
                        d.reason.clone().map(Json::Str).unwrap_or(Json::Null),
                    ),
                    (
                        "snapshot_time".into(),
                        match d.snapshot {
                            Some(s) => Json::Str(time::iso8601(s)),
                            None => Json::Null,
                        },
                    ),
                    ("size_bytes".into(), d.size.map(num_u).unwrap_or(Json::Null)),
                ])
            })
            .collect(),
    );
    let root = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("generated_at".into(), Json::Str(ts.into())),
        ("crash_dumps".into(), arr),
    ]);
    let mut s = String::new();
    crate::format::json::write(&root, &mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kmsg_records_parse_and_clean() {
        assert_eq!(
            clean_drm("[drm] *ERROR* device wedged, needs recovery"),
            "device wedged, needs recovery"
        );
        assert_eq!(
            clean_drm("Xe device coredump has been created"),
            "Xe device coredump has been created"
        );
        // record split: prio,seq,ts,-;message
        let line = "3,1201,1789400000123456,-;xe 0000:67:00.0: [drm] *ERROR* device wedged, needs recovery";
        let msg = line.split_once(';').unwrap().1;
        assert_eq!(
            msg,
            "xe 0000:67:00.0: [drm] *ERROR* device wedged, needs recovery"
        );
        let rest = msg.strip_prefix("xe 0000:67:00.0: ").unwrap();
        assert_eq!(clean_drm(rest), "device wedged, needs recovery");
    }

    #[test]
    fn snapshot_seconds_to_iso() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/synthetic/srv-2gpu-k7.2/sys/class/devcoredump/devcd1/data");
        let (r, s, _) = parse_header(&path);
        assert_eq!(r.as_deref(), Some("GuC timeout on gt0 (exec queue 12)"));
        assert_eq!(s, Some(1789400000));
    }
}
