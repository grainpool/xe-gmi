//! The file-backed fake behind `XE_GMI_KABI_REPLAY`: DRM query bytes are the raw second-call
//! kernel buffers, `ras/` stands in for the netlink family, `uevents.txt` for the socket.

use std::path::{Path, PathBuf};

use super::genl::{RasCounter, RasNode};
use super::uevent::{parse_line, Uevent};
use crate::error::Error;

pub fn drm_query_dir(replay: &Path, pci: &str) -> PathBuf {
    replay.join("drm-query").join(pci)
}

fn read_text(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

pub fn ras_nodes(replay: &Path) -> Result<Vec<RasNode>, Error> {
    let nodes = match read_text(&replay.join("ras/nodes.txt")) {
        Some(t) => t,
        None => {
            return Err(Error::Unavailable(
                "drm-ras netlink family not present (kernel 7.2+ with xe RAS support)".into(),
            ))
        }
    };
    let mut out = Vec::new();
    for line in nodes.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 3 {
            if let Ok(id) = f[0].parse() {
                out.push(RasNode {
                    id,
                    device: f[1].to_string(),
                    name: f[2].to_string(),
                });
            }
        }
    }
    Ok(out)
}

pub fn ras_counters(replay: &Path, node: u32) -> Result<Vec<RasCounter>, Error> {
    let text = match read_text(&replay.join(format!("ras/counters-{node}.txt"))) {
        Some(t) => t,
        None => {
            return Err(Error::Unavailable(format!(
                "no RAS counters for node {node}"
            )))
        }
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 3 {
            if let (Ok(id), Ok(value)) = (f[0].parse(), f[2].parse()) {
                out.push(RasCounter {
                    id,
                    name: f[1].to_string(),
                    value,
                });
            }
        }
    }
    Ok(out)
}

pub fn ras_clear(replay: &Path, node: u32, error: u32) -> Result<(), Error> {
    use std::io::Write;
    let log = replay.join("ras/clear.log");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| Error::Io {
            path: log.clone(),
            source: e,
        })?;
    writeln!(f, "node {node} error {error}").map_err(|e| Error::Io {
        path: log,
        source: e,
    })?;
    Ok(())
}

pub fn uevent_stream(replay: &Path) -> Box<dyn Iterator<Item = Uevent>> {
    let lines = read_text(&replay.join("uevents.txt")).unwrap_or_default();
    let events: Vec<Uevent> = lines.lines().filter_map(parse_line).collect();
    Box::new(events.into_iter())
}
