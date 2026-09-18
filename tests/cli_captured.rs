//! Evidence run against real-hardware snapshots captured with `scripts/capture-fixtures.sh`
//! (read-only, run by any contributor on their own machine). Captures live in
//! `fixtures/captured/<host>-<kernel>/`; when none is present the tests skip (public CI runs
//! them skipped). Control paths are exercised on real hardware only by verify/run-verify.sh.

mod common;

use common::{code, stderr, stdout, Runner};
use std::path::{Path, PathBuf};

fn captures() -> Vec<PathBuf> {
    let root = common::manifest().join("fixtures/captured");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("sys").is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn skip_hint(test: &str) {
    eprintln!(
        "[{test}] no capture under fixtures/captured/ — run `scripts/capture-fixtures.sh` \
         (read-only, no root needed) to enable this evidence test; skipping."
    );
}

fn captured_runner(dir: &Path) -> Runner {
    Runner {
        tree: dir.to_path_buf(),
        sys: dir.join("sys"),
        proc_: dir.join("proc"),
        modules: dir.join("lib/modules"),
        etc: dir.join("etc"),
        state: std::env::temp_dir().join(format!("xe-gmi-captured-state-{}", std::process::id())),
        t1: None,
        kabi: false,
        extra: vec![],
    }
}

fn assert_clean(o: &std::process::Output, cmd: &str) {
    assert_eq!(code(o), 0, "`{cmd}` exit {}: {}", code(o), stderr(o));
    let both = format!("{}{}", stdout(o), stderr(o));
    assert!(!both.contains("panicked"), "`{cmd}` panicked:\n{both}");
}

/// The one xe device captured, as (pci, device_id) — read from the tree, not assumed.
fn xe_device(dir: &Path) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for p in glob_devices(dir) {
        let Ok(ue) = std::fs::read_to_string(p.join("uevent")) else {
            continue;
        };
        if !ue.contains("DRIVER=xe") {
            continue;
        }
        let id = std::fs::read_to_string(p.join("device"))
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        out.push((p.file_name().unwrap().to_string_lossy().into_owned(), id));
    }
    out
}

fn glob_devices(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let devices = dir.join("sys").join("bus").join("pci").join("devices");
    if let Ok(entries) = std::fs::read_dir(&devices) {
        for e in entries.flatten() {
            // capture stores entries as symlinks; resolve through them
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.contains(':') {
                continue;
            }
            let dev_dir = dir.join("sys").join("devices");
            if let Ok(abs) = std::fs::canonicalize(e.path()) {
                if abs.starts_with(dev_dir) {
                    v.push(abs);
                }
            }
        }
    }
    v
}

#[test]
fn captured_list_and_doctor_run_clean() {
    let cs = captures();
    if cs.is_empty() {
        return skip_hint("captured_list_and_doctor_run_clean");
    }
    for c in &cs {
        if xe_device(c).is_empty() {
            eprintln!("[captured_status_and_info_run_clean] capture has no xe device; value tests not applicable");
            continue;
        }
        let r = captured_runner(c);
        let o = r.run(&["list"]);
        assert_clean(&o, "list");
        let n = stdout(&o).lines().count();
        let o = r.run(&["doctor"]);
        // doctor must agree with list about whether a usable xe device exists
        assert_eq!(
            code(&o),
            if n >= 1 { 0 } else { 3 },
            "doctor vs list disagree on {}\n{}",
            c.display(),
            stdout(&o)
        );
    }
}

#[test]
fn captured_status_and_info_run_clean() {
    let cs = captures();
    if cs.is_empty() {
        return skip_hint("captured_status_and_info_run_clean");
    }
    for c in &cs {
        if xe_device(c).is_empty() {
            eprintln!("[captured_query_all_fields_runs_clean] capture has no xe device; value tests not applicable");
            continue;
        }
        let r = captured_runner(c);
        let o = r.run(&["status"]);
        assert_clean(&o, "status");
        let o = r.run(&["info"]);
        assert_clean(&o, "info");
        let out = stdout(&o);
        assert!(
            out.contains("gt0"),
            "no gt0 in info on {}\n{out}",
            c.display()
        );
        for (pci, id) in xe_device(c) {
            let out = stdout(&o);
            assert!(out.contains(&pci), "info lacks {pci} on {}", c.display());
            if id == 0xe222 {
                // Facts proven by the reference capture of the Arc Pro B65: two graphics tiles.
                assert!(
                    out.contains("gt1"),
                    "B65 lacks gt1 on {}\n{out}",
                    c.display()
                );
                assert!(
                    !out.contains("gt2 "),
                    "unexpected gt2 on {}\n{out}",
                    c.display()
                );
            }
        }
    }
}

#[test]
fn captured_status_reports_hardware_facts() {
    let cs = captures();
    if cs.is_empty() {
        return skip_hint("captured_status_reports_hardware_facts");
    }
    for c in &cs {
        if xe_device(c).is_empty() {
            eprintln!("[captured_status_reports_hardware_facts] capture has no xe device; value tests not applicable");
            continue;
        }
        for (pci, id) in xe_device(c) {
            let o = captured_runner(c).run(&["status"]);
            let out = stdout(&o);
            assert!(out.contains(&pci), "status lacks {pci}:\n{out}");
            if id == 0xe222 {
                // The reference B65 capture: PL2 200.00 W (power1_cap 200000000 uW) and
                // 57344 MiB VRAM from the resized BAR.
                assert!(
                    out.contains("0.00 / 200.00"),
                    "B65 lacks 200.00 W limit:\n{out}"
                );
                assert!(out.contains("57344"), "B65 lacks 57344 MiB VRAM:\n{out}");
            }
        }
    }
}

#[test]
fn captured_query_all_fields_runs_clean() {
    let cs = captures();
    if cs.is_empty() {
        return skip_hint("captured_query_all_fields_runs_clean");
    }
    for c in &cs {
        if xe_device(c).is_empty() {
            eprintln!("[captured_processes_and_json_rounds_run_clean] capture has no xe device; value tests not applicable");
            continue;
        }
        let r = captured_runner(c);
        let fields_out = r.run(&["fields"]);
        assert_clean(&fields_out, "fields");
        let names: Vec<String> = stdout(&fields_out)
            .lines()
            .skip(1) // header line
            .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
            .filter(|n| !n.is_empty())
            .collect();
        assert!(!names.is_empty(), "fields listed nothing");
        let joined = names.join(",");
        let o = r.run(&["query", "--no-units", "--fields", &joined]);
        assert_clean(&o, "query (all fields)");
        // One header line plus one row per device (one row per xe device in a capture).
        let devices = xe_device(c).len().max(1);
        assert_eq!(
            stdout(&o).lines().count(),
            devices + 1,
            "header + one row per device expected on {}",
            c.display()
        );
    }
}

#[test]
fn captured_processes_and_json_rounds_run_clean() {
    let cs = captures();
    if cs.is_empty() {
        return skip_hint("captured_processes_and_json_rounds_run_clean");
    }
    for c in &cs {
        let r = captured_runner(c);
        let o = r.run(&["processes"]);
        assert_clean(&o, "processes");
        for args in [
            vec!["status", "--json"],
            vec!["info", "--json"],
            vec!["query", "--json", "--fields", "memory.total"],
            vec!["processes", "--json"],
        ] {
            let o = r.run(&args);
            assert_clean(&o, &args.join(" "));
            assert!(
                stdout(&o).trim_start().starts_with('{'),
                "not a JSON object: {}",
                stdout(&o)
            );
        }
    }
}
