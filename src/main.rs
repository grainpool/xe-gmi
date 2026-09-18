//! xe-gmi — GPU management CLI for Intel GPUs on the xe kernel driver.
//!
//! Module map: `paths` (env seams) → `probe` (discovery) → `sample` (two-sample rates) →
//! `format` (all renderers); everything is wired in `dispatch` below.
#![deny(unsafe_code)]

#[allow(dead_code)]
mod avail;
mod cgroup;
mod cli;
mod crash;
mod sriov;
// allow(dead_code): helpers that only some code paths consult (seam lookups, fallback formatters);
// kept live on purpose as they document the file layout.
#[allow(dead_code)]
mod doctor;
#[allow(dead_code)]
mod error;
#[allow(dead_code)]
mod format;
#[allow(dead_code)]
mod kabi;
#[allow(dead_code)]
mod paths;
#[allow(dead_code)]
mod pciids;
#[allow(dead_code)]
mod persist;
#[allow(dead_code)]
mod probe;
#[allow(dead_code)]
mod proc_scan;
#[allow(dead_code)]
mod sample;
#[allow(dead_code)]
mod sysfs;
#[allow(dead_code)]
mod time;
#[allow(dead_code)]
mod write;

use clap::{CommandFactory, Parser};
use cli::{Cli, Command};
use error::{Error, Result};
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "xe-gmi: error: {e}");
            // ExitCode::from takes u8; every contract code fits.
            ExitCode::from(e.exit_code() as u8)
        }
    }
}

fn dispatch(cli: Cli) -> Result<()> {
    if cli.sample_ms < 100 {
        return Err(Error::Usage("--sample-ms must be at least 100".into()));
    }
    match cli.command.as_ref().unwrap_or(&Command::Status) {
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(*shell, &mut cmd, "xe-gmi", &mut std::io::stdout());
            Ok(())
        }
        Command::Man { out } => {
            let cmd = Cli::command();
            let man = clap_mangen::Man::new(cmd);
            let mut buf: Vec<u8> = Vec::new();
            man.render(&mut buf).map_err(|e| Error::Io {
                path: out.clone(),
                source: e,
            })?;
            let path = out.join("xe-gmi.1");
            std::fs::create_dir_all(out).map_err(|e| Error::Io {
                path: out.clone(),
                source: e,
            })?;
            std::fs::write(&path, buf).map_err(|e| Error::Io {
                path: path.clone(),
                source: e,
            })?;
            println!("wrote {}", path.display());
            Ok(())
        }
        Command::List => cmd_list(&cli),
        Command::Status => cmd_status(&cli),
        Command::Info { section } => cmd_info(&cli, section),
        Command::Query {
            fields,
            no_header,
            no_units,
        } => cmd_query(&cli, fields, *no_header, *no_units),
        Command::Fields => cmd_fields(&cli),
        Command::Processes { sort, group_by } => cmd_processes(&cli, *sort, *group_by),
        Command::Pmon {
            group_by,
            show_exited,
        } => cmd_pmon(&cli, *group_by, *show_exited),
        Command::Get { what } => cmd_get(&cli, what),
        Command::Firmware => cmd_firmware(&cli),
        Command::Doctor { bundle, no_redact } => cmd_doctor(&cli, bundle.as_ref(), *no_redact),
        Command::Topology { hardware: true } => cmd_topology_hardware(&cli),
        Command::Topology { hardware: false } => cmd_topology(&cli),
        Command::Pcie => cmd_pcie(&cli),
        Command::Ras { clear } => cmd_ras(&cli, *clear),
        Command::Recover {
            dry_run,
            force,
            method,
            no_save_crash,
        } => cmd_recover(&cli, *dry_run, *force, method.as_ref(), *no_save_crash),
        Command::Events => cmd_events(&cli),
        Command::Crash { action } => cmd_crash(&cli, action),
        Command::Sriov { action } => cmd_sriov(&cli, action),
        Command::Cgroups => cmd_cgroups(&cli),
        Command::Cgroup { action } => cmd_cgroup(&cli, action),
        Command::Set { what } => cmd_set(&cli, what),
        Command::Reset { what, no_persist } => cmd_reset(&cli, *what, *no_persist),
        Command::Persist { action } => cmd_persist(&cli, action),
    }
}

// --------------------------------------------------------------------------- read commands

/// `-u SECS` is only valid on status, query and processes.
fn check_update(cli: &Cli, allowed: bool) -> Result<()> {
    if cli.update.is_some() && !allowed {
        return Err(Error::Usage(
            "--update is only valid for status, query and processes".into(),
        ));
    }
    Ok(())
}

/// Discovery + selection for one read-command run. The device list is leaked into the process
/// (a CLI runs once; the leak is bounded to one Vec per invocation) so the selected references
/// and the frame-loop `Rates` can share one lifetime without self-referential structs.
struct Selected {
    roots: paths::Roots,
    kernel: String,
    devices: Vec<&'static probe::Device>,
}

fn select_devices(cli: &Cli) -> Result<Selected> {
    let roots = paths::Roots::from_env();
    let kernel = sysfs::read_string(&roots.procfs.join("sys/kernel/osrelease"))
        .value()
        .cloned()
        .unwrap_or_else(|| "unknown".into());
    let all: &'static Vec<probe::Device> = Box::leak(Box::new(probe::discover(&roots)));
    let devices = probe::select(all.as_slice(), cli.device.as_deref())?;
    Ok(Selected {
        roots,
        kernel,
        devices,
    })
}

/// `Persistence` / `persistence.installed` for this run.
fn persistence_installed(roots: &paths::Roots) -> bool {
    persist::installed(roots)
}

fn now_iso() -> String {
    time::iso8601(paths::now_epoch())
}

/// One frame: two samples per device → per-device rates. `prev` is the previous frame's second
/// sample (frames ≥ 2 without the T1 seam); `update_mode` marks the first frame of `-u`, which
/// uses a single sample so every rate is N/A.
fn collect_frame(
    roots: &paths::Roots,
    sample_ms: u64,
    devs: &[&probe::Device],
    prev: Option<&Vec<sample::Sample>>,
    update_mode: bool,
) -> (Vec<sample::Sample>, Vec<sample::Rates>) {
    if roots.sysfs_t1.is_some() {
        let a: Vec<sample::Sample> = devs.iter().map(|d| sample::take(roots, d, false)).collect();
        let b: Vec<sample::Sample> = devs.iter().map(|d| sample::take(roots, d, true)).collect();
        let rates = a
            .iter()
            .zip(&b)
            .map(|(x, y)| sample::rates(x, y, false))
            .collect();
        (b, rates)
    } else if let Some(a) = prev {
        let b: Vec<sample::Sample> = devs.iter().map(|d| sample::take(roots, d, false)).collect();
        let rates = a
            .iter()
            .zip(&b)
            .map(|(x, y)| sample::rates(x, y, false))
            .collect();
        (b, rates)
    } else {
        let a: Vec<sample::Sample> = devs.iter().map(|d| sample::take(roots, d, false)).collect();
        if update_mode {
            let rates = a.iter().map(|x| sample::rates(x, x, true)).collect();
            (a, rates)
        } else {
            std::thread::sleep(std::time::Duration::from_millis(sample_ms));
            let b: Vec<sample::Sample> =
                devs.iter().map(|d| sample::take(roots, d, false)).collect();
            let rates = a
                .iter()
                .zip(&b)
                .map(|(x, y)| sample::rates(x, y, false))
                .collect();
            (b, rates)
        }
    }
}

/// Frames to emit for `-u/--count`.
fn frame_count(cli: &Cli) -> u64 {
    if cli.update.is_some() {
        cli.count.unwrap_or(u64::MAX).max(1)
    } else {
        1
    }
}

fn visible_all_users(roots: &paths::Roots) -> bool {
    let mut r2 = roots.clone();
    if let Some(p) = &roots.procfs_t1 {
        r2.procfs = p.clone();
    }
    paths::uid(&r2) == 0
}

fn make_views<'a>(
    devs: &[&'a probe::Device],
    rates: &'a [sample::Rates],
    roots: &'a paths::Roots,
    kernel: &str,
    timestamp: &str,
) -> Vec<(&'a probe::Device, format::fields::View<'a>)> {
    let persistence = persistence_installed(roots);
    devs.iter()
        .zip(rates)
        .map(|(d, r)| {
            let used_bytes = r.per_client.iter().map(|c| c.client.resident_vram).sum();
            let used_clients = r.per_client.len();
            (
                *d,
                format::fields::View {
                    dev: d,
                    roots,
                    rates: Some(r),
                    used_bytes,
                    used_clients,
                    persistence,
                    kernel: kernel.to_string(),
                    timestamp: timestamp.to_string(),
                },
            )
        })
        .collect()
}

fn cmd_list(cli: &Cli) -> Result<()> {
    let sel = select_devices(cli)?;
    if cli.json {
        let ts = now_iso();
        let (_samples, rates) = collect_frame(&sel.roots, cli.sample_ms, &sel.devices, None, false);
        let views = make_views(&sel.devices, &rates, &sel.roots, &sel.kernel, &ts);
        println!(
            "{}",
            format::report::list_json(&sel.roots, &sel.kernel, &ts, &views)
        );
        return Ok(());
    }
    for d in sel.devices {
        let nodes = match &d.render {
            Some(r) => format!("{} {}", d.card, r),
            None => d.card.clone(),
        };
        println!("{}  {}  {}  {}", d.index, d.pci, d.name, nodes);
    }
    Ok(())
}

fn cmd_status(cli: &Cli) -> Result<()> {
    check_update(cli, true)?;
    let sel = select_devices(cli)?;
    let total = frame_count(cli);
    let mut prev: Option<Vec<sample::Sample>> = None;
    for i in 0..total {
        let ts = now_iso();
        let (samples, rates) = collect_frame(
            &sel.roots,
            cli.sample_ms,
            &sel.devices,
            prev.as_ref(),
            cli.update.is_some(),
        );
        let views = make_views(&sel.devices, &rates, &sel.roots, &sel.kernel, &ts);
        if cli.json {
            println!(
                "{}",
                format::report::status_json(&sel.roots, &sel.kernel, &ts, &views)
            );
        } else {
            if cli.update.is_some() && i > 0 {
                println!();
            }
            print!("{}", format::table::render(&sel.kernel, &ts, &views));
        }
        prev = Some(samples);
        if i + 1 < total {
            std::thread::sleep(std::time::Duration::from_secs_f64(
                cli.update.unwrap_or(0.0).max(0.05),
            ));
        }
    }
    Ok(())
}

fn cmd_topology(cli: &Cli) -> Result<()> {
    let sel = select_devices(cli)?;
    if cli.json {
        println!("{}", format::hw::topology_json(&sel.devices, &now_iso()));
        return Ok(());
    }
    print!("{}", format::hw::topology_text(&sel.devices));
    Ok(())
}

fn cmd_topology_hardware(cli: &Cli) -> Result<()> {
    let sel = select_devices(cli)?;
    if !sel.devices.iter().any(|d| d.kabi.gt_list.value().is_some()) {
        let reason = match sel.devices.first().map(|d| &d.kabi.gt_list).unwrap_or(
            &crate::avail::Avail::NotAvailable(crate::avail::Reason::Detail(
                "no device selected".into(),
            )),
        ) {
            crate::avail::Avail::NotAvailable(r) => r.to_string(),
            _ => "unavailable".into(),
        };
        return Err(Error::Unavailable(format!(
            "hardware topology needs the render node ({reason})"
        )));
    }
    if cli.json {
        println!("{}", format::hw::hardware_json(&sel.devices, &now_iso()));
        return Ok(());
    }
    print!("{}", format::hw::hardware_text(&sel.devices));
    Ok(())
}

fn cmd_firmware(cli: &Cli) -> Result<()> {
    let sel = select_devices(cli)?;
    if cli.json {
        println!("{}", format::hw::firmware_json(&sel.devices, &now_iso()));
        return Ok(());
    }
    print!("{}", format::hw::firmware_text(&sel.devices, cli.verbose));
    Ok(())
}

fn cmd_pcie(cli: &Cli) -> Result<()> {
    let sel = select_devices(cli)?;
    if cli.json {
        println!("{}", format::hw::pcie_json(&sel.devices, &now_iso()));
        return Ok(());
    }
    print!("{}", format::hw::pcie_text(&sel.devices));
    Ok(())
}

fn cmd_recover(
    cli: &Cli,
    dry_run: bool,
    force: bool,
    method: Option<&cli::RecoverMethod>,
    no_save_crash: bool,
) -> Result<()> {
    let sel = select_devices(cli)?;
    let dev = one_control_device(cli, &sel, "recover")?;
    let hint = {
        let mut h = String::from("sudo xe-gmi recover");
        if dry_run {
            h.push_str(" --dry-run");
        }
        if force {
            h.push_str(" --force");
        }
        if no_save_crash {
            h.push_str(" --no-save-crash");
        }
        if let Some(m) = method {
            h.push_str(&format!(" --method {}", recover_method_str(m)));
        }
        h
    };
    let advertised: Vec<String> = sysfs::read_string(&dev.dev_dir.join("reset_method"))
        .value()
        .map(|s| {
            s.replace(['[', ']'], " ")
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let method = match method {
        Some(m) => recover_method_str(m),
        None if advertised.iter().any(|a| a == "flr") => "flr",
        None => "rebind",
    };
    let method_hard = method != "rebind" && !advertised.iter().any(|a| a == method);
    let numvfs = sysfs::read_string(&dev.dev_dir.join("sriov_numvfs"))
        .value()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let siblings: Vec<String> = {
        let mut out = Vec::new();
        if let Some(parent) = dev.dev_dir.parent() {
            if let Ok(rd) = std::fs::read_dir(parent) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name != dev.pci && crate::kabi::uevent::is_pci_shape(&name) {
                        out.push(name);
                    }
                }
            }
        }
        out.sort();
        out
    };
    let bus_hard = method == "bus" && !siblings.is_empty();
    let vf_hard = numvfs > 0;
    let bound = std::fs::read_link(dev.dev_dir.join("driver")).is_ok();
    let dumps = crash::list_for_pci(&sel.roots, &dev.pci);

    let mut soft = 0usize;
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "method                   : {} (advertised: {}){}",
        method,
        if advertised.is_empty() {
            "none".to_string()
        } else {
            advertised.join(" ")
        },
        if method_hard {
            format!(" -> refused (method {method} not advertised)")
        } else {
            String::new()
        }
    ));
    lines.push(format!(
        "driver                   : {}",
        if bound {
            "xe (bound)"
        } else {
            "none (unbound)"
        }
    ));
    let crash_line = match dumps.first() {
        None => "none pending".to_string(),
        Some(d) => {
            if no_save_crash {
                format!("devcd{} pending (left in place: --no-save-crash)", d.id)
            } else {
                soft += 1;
                format!("devcd{} pending -> saved first", d.id)
            }
        }
    };
    lines.push(format!("crash dump               : {crash_line}"));
    let scan = proc_scan::scan(&sel.roots, &[dev.pci.as_str()]);
    let mut client_names: Vec<String> = Vec::new();
    let mut client_count = 0usize;
    for c in scan.clients.iter().filter(|c| c.pdev == dev.pci) {
        client_count += 1;
        if !client_names.contains(&c.name) && client_names.len() < 3 {
            client_names.push(c.name.clone());
        }
    }
    if client_count > 0 {
        soft += 1;
    }
    lines.push(format!(
        "DRM clients              : {}{}{}",
        client_count,
        if client_names.is_empty() {
            String::new()
        } else {
            format!(" ({})", client_names.join(", "))
        },
        if client_count > 0 {
            " -> requires --force"
        } else {
            ""
        }
    ));
    let active: Vec<String> = sriov::connectors(&sel.roots, dev)
        .into_iter()
        .filter(|c| c.enabled)
        .map(|c| c.name)
        .collect();
    if !active.is_empty() {
        soft += 1;
    }
    lines.push(format!(
        "active connectors        : {}{}",
        if active.is_empty() {
            "none".to_string()
        } else {
            active.join(", ")
        },
        if active.is_empty() {
            ""
        } else {
            " -> requires --force"
        }
    ));
    lines.push(format!(
        "enabled VFs              : {}{}",
        numvfs,
        if vf_hard {
            " -> refused (disable VFs first)"
        } else {
            ""
        }
    ));
    lines.push(format!(
        "other functions on bus   : {}{}",
        if siblings.is_empty() {
            "none".to_string()
        } else {
            siblings.join(", ")
        },
        if bus_hard {
            " -> refused (bus reset would hit them)"
        } else {
            ""
        }
    ));
    let persisted = persist::load_entries(&sel.roots);
    let my_entries = persisted.iter().filter(|e| e.pci == dev.pci).count();
    lines.push(format!(
        "after recovery           : {}",
        if persist::installed(&sel.roots) {
            format!("persist apply ({my_entries} entries)")
        } else {
            "persist apply (not installed: nothing to reapply)".into()
        }
    ));

    println!("GPU {} [{}]: recovery plan", dev.index, dev.pci);
    for l in &lines {
        println!("  {l}");
    }
    if method_hard || vf_hard || bus_hard {
        println!("Result: refused (hard precondition)");
        return Err(Error::Unavailable(
            "recover refused (hard precondition)".into(),
        ));
    }
    if soft > 0 && !force {
        println!("Result: refused ({soft} soft preconditions failed; pass --force to override)");
        return Err(Error::Unavailable(format!(
            "recover refused ({soft} soft preconditions failed; pass --force)"
        )));
    }
    if dry_run {
        println!("Result: proceeding (dry run)");
        return Ok(());
    }
    println!("Result: proceeding");

    if !no_save_crash {
        for d in &dumps {
            let bytes = crash::read_dump(&sel.roots, d.id)?;
            let dir = sel.roots.state.join("crash");
            std::fs::create_dir_all(&dir).map_err(|e| Error::Io {
                path: dir.clone(),
                source: e,
            })?;
            let path = dir.join(format!(
                "xe-crash-{}-{}.txt",
                dev.pci,
                now_iso().replace(':', "")
            ));
            std::fs::write(&path, &bytes).map_err(|e| Error::Io {
                path: path.clone(),
                source: e,
            })?;
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| Error::Io {
                    path: path.clone(),
                    source: e,
                },
            )?;
            println!(
                "saved crash dump {} ({} bytes) to {}",
                d.id,
                bytes.len(),
                path.display()
            );
            crash::release(&sel.roots, d.id)?;
            println!("released crash dump {}", d.id);
        }
    }
    if bound {
        crate::write::write_system(
            &sel.roots,
            &sel.roots.sysfs.join("bus/pci/drivers/xe/unbind"),
            &dev.pci,
            &hint,
        )
        .map_err(|e| named_step("unbind", e))?;
        println!("unbound xe from {}", dev.pci);
    }
    if method != "rebind" {
        let rb = crate::write::write_verified(
            &sel.roots,
            &dev.dev_dir,
            &dev.dev_dir.join("reset_method"),
            method,
            &hint,
        )
        .map_err(|e| named_step("reset method", e))?;
        let ok = rb.trim() == method || rb.split_whitespace().any(|t| t == format!("[{method}]"));
        if !ok {
            return Err(Error::WriteFailed(format!(
                "reset method {method}: read back {}; run: sudo xe-gmi recover --method rebind",
                rb.trim()
            )));
        }
        crate::write::write_fire(
            &sel.roots,
            &dev.dev_dir,
            &dev.dev_dir.join("reset"),
            "1",
            &hint,
        )
        .map_err(|e| named_step("reset", e))?;
        println!("reset ({method}) done");
    }
    crate::write::write_system(
        &sel.roots,
        &sel.roots.sysfs.join("bus/pci/drivers/xe/bind"),
        &dev.pci,
        &hint,
    )
    .map_err(|e| named_step("bind", e))?;
    let started = std::time::Instant::now();
    if !sel.roots.fixture_mode() {
        for _ in 0..600 {
            if std::fs::read_link(dev.dev_dir.join("driver")).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    println!(
        "bound xe to {} ({} ms)",
        dev.pci,
        started.elapsed().as_millis()
    );
    if persist::installed(&sel.roots) {
        let (results, _) = persist::apply(&sel.roots, Some(&dev.dev_dir.to_string_lossy()), &hint)?;
        let (mut ok, mut sk, mut fail) = (0usize, 0usize, 0usize);
        for r in &results {
            if r.status.starts_with("ok") {
                ok += 1;
            } else if r.status.starts_with("skipped") {
                sk += 1;
            } else {
                fail += 1;
            }
        }
        println!("persist apply: {ok} ok, {sk} skipped, {fail} failed");
    }
    println!("recovered {}", dev.pci);
    Ok(())
}

fn recover_method_str(m: &cli::RecoverMethod) -> &'static str {
    match m {
        cli::RecoverMethod::Flr => "flr",
        cli::RecoverMethod::Bus => "bus",
        cli::RecoverMethod::Rebind => "rebind",
    }
}

fn named_step(step: &str, e: Error) -> Error {
    match e {
        Error::WriteFailed(m) => Error::WriteFailed(format!(
            "{step}: {m}; run: sudo xe-gmi recover --method rebind"
        )),
        other => other,
    }
}

fn cmd_events(cli: &Cli) -> Result<()> {
    let sel = select_devices(cli)?;
    let known: Vec<&str> = sel.devices.iter().map(|d| d.pci.as_str()).collect();
    let stream = crate::kabi::uevent_stream(&sel.roots)?;
    for u in stream {
        let dcd = crate::kabi::uevent::subsystem(&u) == Some("devcoredump");
        let pci = crate::kabi::uevent::pci_slot(&u)
            .or_else(|| crate::kabi::uevent::pci_in_devpath(&u))
            .filter(|p| known.contains(p));
        if !dcd && pci.is_none() {
            continue;
        }
        let wedged = crate::kabi::uevent::env(&u, "WEDGED").map(|s| s.to_string());
        let (event, detail) = if let Some(w) = &wedged {
            ("wedged".to_string(), format!("recovery={w}"))
        } else if dcd && u.action == "add" {
            (
                "devcoredump".to_string(),
                format!("created {}", u.devpath.rsplit('/').next().unwrap_or("?")),
            )
        } else if crate::kabi::uevent::subsystem(&u) == Some("pci") {
            let drv = crate::kabi::uevent::env(&u, "DRIVER").unwrap_or("?");
            (u.action.clone(), format!("driver={drv}"))
        } else {
            (u.action.clone(), String::new())
        };
        if cli.json {
            let ts = now_iso();
            let pcis = pci.unwrap_or("-").to_string();
            let mut obj = vec![
                ("schema_version".to_string(), format::json::num_u(1)),
                ("timestamp".to_string(), format::json::Json::Str(ts.clone())),
                (
                    "pci_address".to_string(),
                    format::json::Json::Str(pcis.clone()),
                ),
                ("event".to_string(), format::json::Json::Str(event.clone())),
            ];
            if let Some(w) = &wedged {
                obj.push((
                    "recovery".to_string(),
                    format::json::Json::Arr(
                        w.split(',')
                            .map(|s| format::json::Json::Str(s.to_string()))
                            .collect(),
                    ),
                ));
            } else if !detail.is_empty() {
                obj.push((
                    "detail".to_string(),
                    format::json::Json::Str(detail.clone()),
                ));
            }
            let mut s = String::new();
            format::json::write_line(&format::json::Json::Obj(obj), &mut s);
            println!("{s}");
        } else {
            let tail = if detail.is_empty() {
                String::new()
            } else {
                format!(" {detail}")
            };
            println!("{} {} {}{}", now_iso(), pci.unwrap_or("-"), event, tail);
        }
    }
    Ok(())
}

fn cmd_ras(cli: &Cli, clear: bool) -> Result<()> {
    let sel = select_devices(cli)?;
    let mut nodes_by_dev: Vec<(&str, crate::kabi::RasSnapshot)> = Vec::new();
    let mut first_err: Option<Error> = None;
    for dev in &sel.devices {
        let nodes = match crate::kabi::ras_nodes(&sel.roots) {
            Ok(ns) => ns,
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
                Vec::new()
            }
        };
        let mut mine = Vec::new();
        for n in nodes.into_iter().filter(|n| n.device == dev.pci) {
            match crate::kabi::ras_counters(&sel.roots, n.id) {
                Ok(cs) => mine.push((n, cs)),
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        mine.sort_by_key(|(n, _)| n.id);
        nodes_by_dev.push((dev.pci.as_str(), mine));
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    if clear {
        for (pci, nodes) in &nodes_by_dev {
            let mut n = 0usize;
            for (node, counters) in nodes {
                for c in counters {
                    crate::kabi::ras_clear(&sel.roots, node.id, c.id)?;
                    n += 1;
                }
            }
            println!("cleared {n} counters on {pci}");
        }
        return Ok(());
    }
    if cli.json {
        let devs: Vec<format::json::Json> = nodes_by_dev
            .iter()
            .enumerate()
            .map(|(i, (pci, nodes))| {
                format::json::Json::Obj(vec![
                    ("index".into(), format::json::num_u(i as u64)),
                    (
                        "pci_address".into(),
                        format::json::Json::Str(pci.to_string()),
                    ),
                    (
                        "nodes".into(),
                        format::json::Json::Arr(
                            nodes
                                .iter()
                                .map(|(n, cs)| {
                                    format::json::Json::Obj(vec![
                                        ("node".into(), format::json::Json::Str(n.name.clone())),
                                        ("node_id".into(), format::json::num_u(n.id as u64)),
                                        (
                                            "counters".into(),
                                            format::json::Json::Obj(
                                                cs.iter()
                                                    .map(|c| {
                                                        (
                                                            c.name.clone(),
                                                            format::json::num_u(c.value as u64),
                                                        )
                                                    })
                                                    .collect(),
                                            ),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect();
        let root = format::json::Json::Obj(vec![
            ("schema_version".into(), format::json::num_u(1)),
            ("timestamp".into(), format::json::Json::Str(now_iso())),
            ("devices".into(), format::json::Json::Arr(devs)),
        ]);
        let mut s = String::new();
        format::json::write(&root, &mut s);
        println!("{s}");
        return Ok(());
    }
    for (i, (pci, nodes)) in nodes_by_dev.iter().enumerate() {
        println!("GPU {i} [{pci}]");
        if nodes.is_empty() {
            println!("  N/A (no RAS nodes registered for this device)");
            continue;
        }
        for (node, counters) in nodes {
            println!("  {}", node.name);
            for c in counters {
                println!("    {:<23}: {}", c.name, c.value);
            }
        }
    }
    Ok(())
}

fn cmd_crash(cli: &Cli, action: &cli::CrashAction) -> Result<()> {
    let sel = select_devices(cli)?;
    use cli::CrashAction::*;
    match action {
        List => {
            let dumps = crash::list(&sel.roots, &sel.devices);
            if cli.json {
                println!("{}", crash::list_json(&dumps, &now_iso()));
            } else {
                print!("{}", crash::list_text(&dumps));
            }
            Ok(())
        }
        Show { id } => {
            crash::get(&sel.roots, &sel.devices, *id)?;
            let bytes = crash::read_dump(&sel.roots, *id)?;
            std::io::stdout().write_all(&bytes).map_err(|e| Error::Io {
                path: "stdout".into(),
                source: e,
            })?;
            Ok(())
        }
        Save { id, out } => {
            crash::get(&sel.roots, &sel.devices, *id)?;
            let bytes = crash::read_dump(&sel.roots, *id)?;
            std::fs::write(out, &bytes).map_err(|e| Error::Io {
                path: out.clone(),
                source: e,
            })?;
            println!(
                "saved crash dump {id} ({} bytes) to {}",
                bytes.len(),
                out.display()
            );
            Ok(())
        }
        Release { id } => {
            crash::get(&sel.roots, &sel.devices, *id)?;
            crash::release(&sel.roots, *id)?;
            println!("released crash dump {id}");
            Ok(())
        }
    }
}

fn cmd_sriov(cli: &Cli, action: &cli::SriovAction) -> Result<()> {
    match action {
        cli::SriovAction::Status => {
            let sel = select_devices(cli)?;
            if cli.json {
                println!("{}", sriov::json(&sel.devices, &now_iso()));
            } else {
                print!("{}", sriov::text(&sel.devices));
            }
            Ok(())
        }

        cli::SriovAction::Enable { n: count } => {
            let sel = select_devices(cli)?;
            let dev = one_control_device(cli, &sel, "sriov enable")?;
            let total = sysfs::read_string(&dev.dev_dir.join("sriov_totalvfs"))
                .value()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(0);
            if *count < 1 || *count > total {
                return Err(Error::Usage(format!(
                    "sriov enable {count}: accepted 1..={total} (sriov_totalvfs)"
                )));
            }
            let cur = sysfs::read_string(&dev.dev_dir.join("sriov_numvfs"))
                .value()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(0);
            if cur != 0 {
                return Err(Error::Unavailable(format!(
                    "VFs already enabled ({cur}); run: sudo xe-gmi sriov disable first"
                )));
            }
            crate::write::write_verified(
                &sel.roots,
                &dev.dev_dir,
                &dev.dev_dir.join("sriov_numvfs"),
                &count.to_string(),
                "sudo xe-gmi sriov enable",
            )?;
            println!(
                "GPU {} [{}]: sriov_numvfs set to {}",
                dev.index, dev.pci, count
            );
            if !sel.roots.fixture_mode() {
                for k in 0..*count {
                    for _ in 0..200 {
                        if dev.dev_dir.join(format!("virtfn{k}")).exists() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
            for k in 0..*count {
                let vf = dev.dev_dir.join(format!("virtfn{k}"));
                if !vf.exists() {
                    continue;
                }
                let addr = std::fs::read_link(&vf)
                    .ok()
                    .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                let drv = std::fs::read_link(vf.join("driver"))
                    .ok()
                    .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "no driver".into());
                println!("  vf{k} {addr} {drv}");
            }
            Ok(())
        }
        cli::SriovAction::Disable => {
            let sel = select_devices(cli)?;
            let dev = one_control_device(cli, &sel, "sriov disable")?;
            crate::write::write_verified(
                &sel.roots,
                &dev.dev_dir,
                &dev.dev_dir.join("sriov_numvfs"),
                "0",
                "sudo xe-gmi sriov disable",
            )
            .map_err(|e| match e {
                Error::WriteFailed(_) => Error::WriteFailed("VFs are in use".into()),
                other => other,
            })?;
            println!("sriov_numvfs set to 0");
            Ok(())
        }
        cli::SriovAction::Vf { target, action } => {
            let sel = select_devices(cli)?;
            let dev = one_control_device(cli, &sel, "sriov vf")?;
            let total = sysfs::read_string(&dev.dev_dir.join("sriov_totalvfs"))
                .value()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let all = target == "all";
            let vf: u32 = if all {
                0
            } else {
                match target.parse::<u32>() {
                    Ok(n) if (1..=total).contains(&n) => n,
                    _ => {
                        return Err(Error::Usage(format!(
                            "vf {target:?}: accepted 1..={total} or all"
                        )))
                    }
                }
            };
            match action {
                cli::VfAction::Set {
                    vram,
                    quantum,
                    timeout,
                    priority,
                } => {
                    let mut writes: Vec<(String, String, String)> = Vec::new();
                    if let Some(v) = vram {
                        let bytes = cgroup::parse_limit(v)?
                            .ok_or_else(|| Error::Usage("vf vram: max not accepted here".into()))?;
                        writes.push(("vram_quota".into(), bytes.to_string(), v.clone()));
                    }
                    if let Some(q) = quantum {
                        writes.push((
                            "exec_quantum_ms".into(),
                            parse_ms_time(q, "quantum")?,
                            q.clone(),
                        ));
                    }
                    if let Some(t) = timeout {
                        writes.push((
                            "preempt_timeout_us".into(),
                            parse_us_time(t, "timeout")?,
                            t.clone(),
                        ));
                    }
                    if let Some(p) = priority {
                        let tok = match p {
                            cli::Priority::Low => "low",
                            cli::Priority::Normal => "normal",
                            cli::Priority::High => "high",
                        };
                        writes.push(("sched_priority".into(), tok.into(), tok.into()));
                    }
                    if writes.is_empty() {
                        return Err(Error::Usage(
                            "nothing to set: pass at least one of --vram --quantum --timeout --priority"
                                .into(),
                        ));
                    }
                    let dir = if all {
                        ".bulk_profile".to_string()
                    } else {
                        format!("vf{vf}")
                    };
                    for (attr, value, shown) in writes {
                        let target_path = if all {
                            dev.dev_dir.join("sriov_admin").join(&dir).join(&attr)
                        } else {
                            dev.dev_dir
                                .join("sriov_admin")
                                .join(&dir)
                                .join("profile")
                                .join(&attr)
                        };
                        if all {
                            crate::write::write_fire(
                                &sel.roots,
                                &dev.dev_dir,
                                &target_path,
                                &value,
                                "sudo xe-gmi sriov vf all set",
                            )?;
                            println!("vf all {attr} = {shown}");
                        } else {
                            let rb = crate::write::write_verified(
                                &sel.roots,
                                &dev.dev_dir,
                                &target_path,
                                &value,
                                "sudo xe-gmi sriov vf set",
                            )?;
                            if attr == "sched_priority" {
                                let rb = rb.trim().to_string();
                                let ok = rb == value
                                    || rb.split_whitespace().any(|t| t == format!("[{value}]"));
                                if !ok {
                                    return Err(Error::WriteFailed(format!(
                                        "vf{vf} sched_priority: wrote {value}, read back {rb}"
                                    )));
                                }
                            }
                            println!("vf{vf} {attr} = {shown}");
                        }
                    }
                    Ok(())
                }
                cli::VfAction::Stop { force } => {
                    if all {
                        return Err(Error::Usage(
                            "stop needs a VF index, all is not accepted".into(),
                        ));
                    }
                    if !force {
                        return Err(Error::Unavailable(
                            "stopping a VF interrupts its user; pass --force".into(),
                        ));
                    }
                    crate::write::write_fire(
                        &sel.roots,
                        &dev.dev_dir,
                        &dev.dev_dir
                            .join("sriov_admin")
                            .join(format!("vf{vf}"))
                            .join("stop"),
                        "1",
                        "sudo xe-gmi sriov vf stop",
                    )?;
                    println!("vf{vf} stopped");
                    Ok(())
                }
            }
        }
    }
}

fn one_control_device<'a>(
    cli: &Cli,
    sel: &'a Selected,
    what: &str,
) -> Result<&'a crate::probe::Device> {
    if sel.devices.len() > 1 && cli.device.is_none() {
        return Err(Error::Usage(format!(
            "{what} needs exactly one device but {} are present: pass -i <SEL>",
            sel.devices.len()
        )));
    }
    sel.devices
        .first()
        .cloned()
        .ok_or_else(|| Error::Internal("no device selected".into()))
}

/// `^\d+\s*(ms|s)?$` -> milliseconds.
fn parse_ms_time(s: &str, what: &str) -> Result<String> {
    let t = s.trim();
    let bad = || Error::Usage(format!("invalid {what} {s:?}: accepted 10ms, 1s, 500 (ms)"));
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return Err(bad());
    }
    let n: u64 = t[..digits].parse().map_err(|_| bad())?;
    let ms = match t[digits..].trim() {
        "" | "ms" => n,
        "s" => n * 1000,
        _ => return Err(bad()),
    };
    Ok(ms.to_string())
}

/// `^\d+\s*(us|s)?$` -> microseconds.
fn parse_us_time(s: &str, what: &str) -> Result<String> {
    let t = s.trim();
    let bad = || {
        Error::Usage(format!(
            "invalid {what} {s:?}: accepted 20000us, 1s, 500 (us)"
        ))
    };
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return Err(bad());
    }
    let n: u64 = t[..digits].parse().map_err(|_| bad())?;
    let us = match t[digits..].trim() {
        "" | "us" => n,
        "s" => n * 1_000_000,
        _ => return Err(bad()),
    };
    Ok(us.to_string())
}
fn cmd_cgroups(cli: &Cli) -> Result<()> {
    let sel = select_devices(cli)?;
    let caps = cgroup::require_controller(&sel.roots)?;
    let entries = visible_entries(&sel);
    let xes: Vec<&str> = sel.devices.iter().map(|d| d.pci.as_str()).collect();
    let scan = cgroup_clients(&sel);
    let clients_of = |path: &str| -> usize {
        scan.iter()
            .filter(|c| c.cgroup.as_deref() == Some(path))
            .count()
    };
    if cli.json {
        let clients_json = |path: &str| -> Vec<(u32, String, u64)> {
            scan.iter()
                .filter(|c| c.cgroup.as_deref() == Some(path))
                .map(|c| (c.pid, c.name.clone(), c.client_id))
                .collect()
        };
        println!("{}", cgroup::json(&entries, &caps, &xes, &clients_json));
    } else {
        print!("{}", cgroup::text(&entries, &caps, &xes, &clients_of));
    }
    Ok(())
}

fn cmd_cgroup(cli: &Cli, action: &cli::CgroupAction) -> Result<()> {
    let sel = select_devices(cli)?;
    let xes: Vec<&str> = sel.devices.iter().map(|d| d.pci.as_str()).collect();
    match action {
        cli::CgroupAction::Show { path } => {
            let scan = cgroup_clients(&sel);
            let clients_of = |p: &str| -> Vec<String> {
                scan.iter()
                    .filter(|c| c.cgroup.as_deref() == Some(p))
                    .map(|c| c.name.clone())
                    .collect()
            };
            print!("{}", cgroup::show(&sel.roots, path, &xes, &clients_of)?);
            Ok(())
        }
        cli::CgroupAction::Set { path, vram_max } => {
            let limit = cgroup::parse_limit(vram_max)?;
            let caps = cgroup::require_controller(&sel.roots)?;
            if sel.devices.len() > 1 && cli.device.is_none() {
                return Err(Error::Usage(format!(
                    "cgroup set needs exactly one device but {} are present: pass -i <SEL>",
                    sel.devices.len()
                )));
            }
            let dev = &sel.devices[0];
            let dir = sel.roots.cgroup.join(path.trim_start_matches('/'));
            if !dir.join("dmem.max").is_file() {
                return Err(Error::Unavailable(format!(
                    "cgroup {path} not found or without dmem.max"
                )));
            }
            let region = format!("drm/{}/vram0", dev.pci);
            let cap = caps.iter().find(|(r, _)| *r == region).map(|(_, c)| *c);
            if let (Some(bytes), Some(cap)) = (limit, cap) {
                if bytes > cap {
                    return Err(Error::Usage(format!(
                        "{vram_max} is above the {} capacity ({} MiB)",
                        region,
                        cap / 1024 / 1024
                    )));
                }
            }
            let current = std::fs::read_to_string(dir.join("dmem.max")).map_err(|e| Error::Io {
                path: dir.join("dmem.max"),
                source: e,
            })?;
            let value_text = match limit {
                Some(b) => b.to_string(),
                None => "max".to_string(),
            };
            let mut lines: Vec<String> = Vec::new();
            let mut replaced = false;
            for l in current.lines() {
                if l.split_whitespace().next() == Some(region.as_str()) {
                    lines.push(format!("{region} {value_text}"));
                    replaced = true;
                } else if !l.is_empty() {
                    lines.push(l.to_string());
                }
            }
            if !replaced {
                lines.push(format!("{region} {value_text}"));
            }
            let readback = crate::write::write_cgroup_dmem(
                &sel.roots,
                &dir,
                "dmem.max",
                &lines.join("\n"),
                "sudo xe-gmi cgroup set",
            )?;
            let shown = match limit {
                Some(b) => {
                    let mib = b / 1024 / 1024;
                    format!("= {mib} MiB")
                }
                None => "= max".to_string(),
            };
            let rounded = match limit {
                Some(b) => readback
                    .lines()
                    .find(|l| l.split_whitespace().next() == Some(region.as_str()))
                    .and_then(|l| l.split_whitespace().nth(1).map(|v| v != b.to_string()))
                    .unwrap_or(false),
                None => false,
            };
            println!(
                "cgroup {path}: dmem.max {region} {shown}{}",
                if rounded {
                    let rb = readback
                        .lines()
                        .find(|l| l.split_whitespace().next() == Some(region.as_str()))
                        .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
                        .unwrap_or_else(|| "?".to_string());
                    format!(" (read back {rb})")
                } else {
                    String::new()
                }
            );
            Ok(())
        }
    }
}

/// Entries visible per the row condition (non-zero current, a numeric limit, or clients).
fn visible_entries(sel: &Selected) -> Vec<cgroup::Entry> {
    let xes: Vec<&str> = sel.devices.iter().map(|d| d.pci.as_str()).collect();
    let scan = cgroup_clients(sel);
    cgroup::walk(&sel.roots)
        .into_iter()
        .filter(|e| {
            let n = scan
                .iter()
                .filter(|c| c.cgroup.as_deref() == Some(e.path.as_str()))
                .count();
            cgroup::row_visible(e, &xes, n)
        })
        .collect()
}

/// Clients attributed to cgroups (host-view paths from /proc/<pid>/cgroup).
fn cgroup_clients(sel: &Selected) -> Vec<proc_scan::ClientSample> {
    let xes: Vec<&str> = sel.devices.iter().map(|d| d.pci.as_str()).collect();
    proc_scan::scan(&sel.roots, &xes)
        .clients
        .into_iter()
        .filter(|c| c.cgroup.is_some())
        .collect()
}

fn cmd_pmon(cli: &Cli, group_by: cli::GroupBy, show_exited: u64) -> Result<()> {
    check_update(cli, true)?;
    let sel = select_devices(cli)?;
    let total = frame_count(cli);
    let mut prev: Option<Vec<sample::Sample>> = None;
    // last-seen rows per (pci, client id) for the "exited" markers
    let mut last: std::collections::BTreeMap<
        (String, u64),
        (format::proc::Row, std::time::Instant),
    > = std::collections::BTreeMap::new();
    for i in 0..total {
        let (_samples, rates) = collect_frame(
            &sel.roots,
            cli.sample_ms,
            &sel.devices,
            prev.as_ref(),
            cli.update.is_some(),
        );
        let visible = visible_all_users(&sel.roots);
        let mut rows: Vec<format::proc::Row> = Vec::new();
        for (d, r) in sel.devices.iter().zip(&rates) {
            rows.extend(format::proc::rows(&d.pci, r));
        }
        format::proc::sort_rows(&mut rows, cli::ProcSort::Vram);
        // mark and keep clients that vanished, for --show-exited seconds
        let now = std::time::Instant::now();
        let mut kept: Vec<format::proc::Row> = Vec::new();
        for ((pci, id), (row, seen)) in last.iter_mut() {
            if !rows.iter().any(|r| r.pci == *pci && r.client_id == *id) {
                if show_exited > 0 && now.duration_since(*seen).as_secs() < show_exited {
                    let mut gone = row.clone();
                    gone.name.push_str(" (exited)");
                    kept.push(gone);
                }
            } else if let Some(r) = rows.iter().find(|x| x.pci == *pci && x.client_id == *id) {
                *row = r.clone();
                *seen = now;
            }
        }
        for r in &rows {
            last.entry((r.pci.clone(), r.client_id))
                .or_insert_with(|| (r.clone(), now));
        }
        rows.extend(kept);
        if cli.json {
            println!(
                "{}",
                format::report::processes_json(&now_iso(), cli.sample_ms, visible, &rates, false)
            );
        } else {
            print!("{}", format::proc::render_grouped(&rows, group_by, visible));
        }
        prev = Some(_samples);
        if i + 1 < total {
            std::thread::sleep(std::time::Duration::from_secs_f64(
                cli.update.unwrap_or(1.0).max(0.05),
            ));
        }
    }
    Ok(())
}

fn cmd_info(cli: &Cli, sections: &[cli::Section]) -> Result<()> {
    check_update(cli, false)?;
    let sel = select_devices(cli)?;
    let ts = now_iso();
    let (_samples, rates) = collect_frame(&sel.roots, cli.sample_ms, &sel.devices, None, false);
    let views = make_views(&sel.devices, &rates, &sel.roots, &sel.kernel, &ts);
    if cli.json {
        println!(
            "{}",
            format::report::status_json(&sel.roots, &sel.kernel, &ts, &views)
        );
        return Ok(());
    }
    let initstate = sysfs::read_string(&sel.roots.sysfs.join("module/xe/initstate"))
        .value()
        .cloned()
        .unwrap_or_else(|| "unknown".into());
    let report = format::info::Report {
        roots: &sel.roots,
        views,
        timestamp: ts,
        kernel: sel.kernel.clone(),
        initstate,
        visible_all_users: visible_all_users(&sel.roots),
        verbose: cli.verbose,
        sections: sections.to_vec(),
    };
    print!("{}", format::info::render(&report));
    Ok(())
}

fn cmd_query(cli: &Cli, fields: &[String], no_header: bool, no_units: bool) -> Result<()> {
    check_update(cli, true)?;
    let parsed = format::csv::parse_fields(fields)
        .map_err(|n| Error::Usage(format!("unknown field: {n} (see: xe-gmi fields)")))?;
    let sel = select_devices(cli)?;
    let total = frame_count(cli);
    let mut prev: Option<Vec<sample::Sample>> = None;
    if !no_header && !cli.json {
        println!("{}", format::csv::header(&parsed, no_units));
    }
    for i in 0..total {
        let ts = now_iso();
        let (samples, rates) = collect_frame(
            &sel.roots,
            cli.sample_ms,
            &sel.devices,
            prev.as_ref(),
            cli.update.is_some(),
        );
        let views = make_views(&sel.devices, &rates, &sel.roots, &sel.kernel, &ts);
        if cli.json {
            println!("{}", format::report::query_json(&ts, &parsed, &views));
        } else {
            for v in &views {
                println!("{}", format::csv::row(&parsed, &v.1));
            }
        }
        prev = Some(samples);
        if i + 1 < total {
            std::thread::sleep(std::time::Duration::from_secs_f64(
                cli.update.unwrap_or(0.0).max(0.05),
            ));
        }
    }
    Ok(())
}

fn cmd_fields(cli: &Cli) -> Result<()> {
    check_update(cli, false)?;
    let sel = select_devices(cli)?;
    let ts = now_iso();
    let (_samples, rates) = collect_frame(&sel.roots, cli.sample_ms, &sel.devices, None, false);
    let views = make_views(&sel.devices, &rates, &sel.roots, &sel.kernel, &ts);
    let rows = format::fields::list_rows(&views[0].1);
    if cli.json {
        println!("{}", format::report::fields_json(&rows));
    } else {
        print!("{}", format::fields::render_text(&rows));
    }
    Ok(())
}

fn cmd_processes(cli: &Cli, sort: cli::ProcSort, group_by: cli::GroupBy) -> Result<()> {
    check_update(cli, true)?;
    let sel = select_devices(cli)?;
    let total = frame_count(cli);
    let mut prev: Option<Vec<sample::Sample>> = None;
    for _ in 0..total {
        let ts = now_iso();
        let (samples, rates) = collect_frame(
            &sel.roots,
            cli.sample_ms,
            &sel.devices,
            prev.as_ref(),
            cli.update.is_some(),
        );
        let visible = visible_all_users(&sel.roots);
        if cli.json {
            println!(
                "{}",
                format::report::processes_json(&ts, cli.sample_ms, visible, &rates, false)
            );
        } else {
            let mut rows: Vec<format::proc::Row> = Vec::new();
            for (d, r) in sel.devices.iter().zip(&rates) {
                rows.extend(format::proc::rows(&d.pci, r));
            }
            format::proc::sort_rows(&mut rows, sort);
            print!("{}", format::proc::render_grouped(&rows, group_by, visible));
        }
        prev = Some(samples);
        if total > 1 {
            std::thread::sleep(std::time::Duration::from_secs_f64(
                cli.update.unwrap_or(0.0).max(0.05),
            ));
        }
    }
    Ok(())
}

/// `sudo xe-gmi [-i SEL] <subcommand…>` from the actual argv.
fn sudo_hint() -> String {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut sel: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-i" || a == "--device" {
            sel = it.next().cloned();
        } else if let Some(v) = a.strip_prefix("--device=") {
            sel = Some(v.to_string());
        }
    }
    let cmd: Vec<&str> = args
        .iter()
        .skip_while(|a| !["set", "reset", "persist"].contains(&a.as_str()))
        .map(|a| a.as_str())
        .collect();
    let mut out = String::from("sudo xe-gmi");
    if let Some(s) = sel {
        out.push_str(&format!(" -i {s}"));
    }
    if !cmd.is_empty() {
        out.push(' ');
        out.push_str(&cmd.join(" "));
    }
    out
}

/// one device for set/reset.
fn select_one_for_control(cli: &Cli) -> Result<Selected> {
    let sel = select_devices(cli)?;
    if sel.devices.len() > 1 && cli.device.is_none() {
        return Err(Error::Usage(
            "more than one xe device; select one with -i".into(),
        ));
    }
    Ok(sel)
}

fn cmd_set(cli: &Cli, what: &cli::SetWhat) -> Result<()> {
    let hint = sudo_hint();
    let sel = select_one_for_control(cli)?;
    check_update(cli, false)?;
    let dev = sel.devices[0];
    let out = match what {
        cli::SetWhat::PowerLimit {
            value,
            limit,
            channel,
            no_persist,
            persist,
        } => {
            let flags = write::PersistFlags {
                no_persist: *no_persist,
                persist: *persist,
            };
            write::set_power_limit(&sel.roots, dev, value, *limit, *channel, flags, &hint)?
        }
        cli::SetWhat::PowerWindow {
            value,
            limit,
            channel,
            no_persist,
            persist,
        } => {
            let flags = write::PersistFlags {
                no_persist: *no_persist,
                persist: *persist,
            };
            write::set_power_window(&sel.roots, dev, value, *limit, *channel, flags, &hint)?
        }
        cli::SetWhat::Clocks {
            min,
            max,
            gt,
            no_persist,
            persist,
        } => {
            let flags = write::PersistFlags {
                no_persist: *no_persist,
                persist: *persist,
            };
            write::set_clocks(&sel.roots, dev, *min, *max, *gt, flags, &hint)?
        }
        cli::SetWhat::PowerProfile {
            profile,
            gt,
            no_persist,
            persist,
        } => {
            let flags = write::PersistFlags {
                no_persist: *no_persist,
                persist: *persist,
            };
            write::set_profile(&sel.roots, dev, *profile, *gt, flags, &hint)?
        }
    };
    print!("{out}");
    Ok(())
}

fn cmd_reset(cli: &Cli, what: cli::ResetWhat, no_persist: bool) -> Result<()> {
    let hint = sudo_hint();
    let sel = select_one_for_control(cli)?;
    check_update(cli, false)?;
    let out = write::reset(&sel.roots, sel.devices[0], what, no_persist, &hint)?;
    print!("{out}");
    Ok(())
}

fn cmd_persist(_cli: &Cli, action: &cli::PersistAction) -> Result<()> {
    let hint = sudo_hint();
    let roots = paths::Roots::from_env();
    match action {
        cli::PersistAction::Install { exe } => {
            print!("{}", persist::install(&roots, exe.as_deref(), &hint)?);
        }
        cli::PersistAction::Remove { purge } => {
            print!("{}", persist::remove(&roots, *purge, &hint)?);
        }
        cli::PersistAction::Show => {
            print!("{}", persist::show(&roots));
        }
        cli::PersistAction::Apply { udev_devpath } => {
            let (_results, summary) = persist::apply(&roots, udev_devpath.as_deref(), &hint)?;
            println!("{summary}");
        }
    }
    Ok(())
}

fn cmd_doctor(cli: &Cli, bundle: Option<&std::path::PathBuf>, no_redact: bool) -> Result<()> {
    let roots = paths::Roots::from_env();
    let d = doctor::collect(&roots);
    let usable = !d.usable.is_empty();
    if let Some(dir) = bundle {
        if usable {
            write_bundle(cli, &roots, &d, dir, !no_redact)?;
        }
        if usable {
            return Ok(());
        }
        return Err(Error::NoDevice("no usable xe device".into()));
    }
    if cli.json {
        println!("{}", doctor::render_json(&d, &roots, &now_iso()));
    } else {
        print!("{}", doctor::render(&d, &roots));
    }
    if usable {
        Ok(())
    } else {
        Err(Error::NoDevice("no usable xe device".into()))
    }
}

/// `doctor --bundle DIR`: a directory of reports; redaction on unless --no-redact (spec/22).
fn write_bundle(
    cli: &Cli,
    roots: &paths::Roots,
    d: &doctor::Doctor,
    dir: &std::path::Path,
    redact: bool,
) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;
    let ts = now_iso();
    let devs: Vec<&probe::Device> = d.usable.iter().collect();
    let write = |name: &str, bytes: &[u8]| -> Result<()> {
        std::fs::write(dir.join(name), bytes).map_err(|e| Error::Io {
            path: dir.join(name),
            source: e,
        })
    };
    write("doctor.txt", doctor::render(d, roots).as_bytes())?;
    let (samples, rates) = collect_frame(roots, cli.sample_ms, &devs, None, false);
    let _ = samples;
    let kernel = sysfs::read_string(&roots.procfs.join("sys/kernel/osrelease"))
        .value()
        .cloned()
        .unwrap_or_else(|| "unknown".into());
    let views = make_views(&devs, &rates, roots, &kernel, &ts);
    write(
        "info.json",
        format::report::status_json(roots, &kernel, &ts, &views).as_bytes(),
    )?;
    let rows = format::fields::list_rows(&views[0].1);
    write("fields.json", format::report::fields_json(&rows).as_bytes())?;
    write("topology.txt", format::hw::topology_text(&devs).as_bytes())?;
    write("pcie.txt", format::hw::pcie_text(&devs).as_bytes())?;
    write(
        "crash.txt",
        crash::list_text(&crash::list(roots, &devs)).as_bytes(),
    )?;
    write("sriov.txt", sriov::text(&devs).as_bytes())?;
    let cgroups_text = match cgroup::capacity(roots) {
        Some(caps) => {
            let xes: Vec<&str> = devs.iter().map(|x| x.pci.as_str()).collect();
            let scan = proc_scan::scan(roots, &xes);
            let clients_of = |path: &str| -> usize {
                scan.clients
                    .iter()
                    .filter(|c| c.cgroup.as_deref() == Some(path))
                    .count()
            };
            let mut entries: Vec<cgroup::Entry> = cgroup::walk(roots)
                .into_iter()
                .filter(|e| cgroup::row_visible(e, &xes, clients_of(&e.path)))
                .collect();
            if redact {
                for e in &mut entries {
                    e.path = cgroup::redact_path(&e.path);
                }
            }
            cgroup::text(&entries, &caps, &xes, &clients_of)
        }
        None => "dmem cgroup controller not present\n".into(),
    };
    write("cgroups.txt", cgroups_text.as_bytes())?;
    write(
        "processes.json",
        format::report::processes_json(
            &ts,
            cli.sample_ms,
            visible_all_users(roots),
            &rates,
            redact,
        )
        .as_bytes(),
    )?;
    let kmsg_text = match &roots.kmsg {
        Some(p) => std::fs::read_to_string(p)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("xe 0000:") || l.contains(";xe "))
            .collect::<Vec<_>>()
            .join("\n"),
        None => String::new(),
    };
    write("kmsg-xe.txt", format!("{kmsg_text}\n").as_bytes())?;
    let mut manifest = format!(
        "xe-gmi {} bundle\nredaction: {}\n",
        env!("CARGO_PKG_VERSION"),
        if redact { "on" } else { "off" }
    );
    for name in [
        "doctor.txt",
        "info.json",
        "fields.json",
        "topology.txt",
        "pcie.txt",
        "crash.txt",
        "cgroups.txt",
        "sriov.txt",
        "processes.json",
        "kmsg-xe.txt",
    ] {
        let size = std::fs::metadata(dir.join(name))
            .map(|m| m.len())
            .unwrap_or(0);
        manifest.push_str(&format!("{name}\t{size}\n"));
    }
    write("MANIFEST.txt", manifest.as_bytes())?;
    println!("bundle written to {}", dir.display());
    Ok(())
}

fn cmd_get(cli: &Cli, what: &cli::GetWhat) -> Result<()> {
    check_update(cli, false)?;
    let sel = select_devices(cli)?;
    let ts = now_iso();
    let (_samples, rates) = collect_frame(&sel.roots, cli.sample_ms, &sel.devices, None, false);
    let views = make_views(&sel.devices, &rates, &sel.roots, &sel.kernel, &ts);
    if cli.json {
        println!(
            "{}",
            format::report::get_json(&sel.roots, &ts, what, &views)
        );
        return Ok(());
    }
    let bid = persist::boot_id(&sel.roots);
    let mut out = String::new();
    for (i, d) in sel.devices.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let recs = persist::boot_records(&sel.roots, &bid, &d.pci);
        out.push_str(&match what {
            cli::GetWhat::Clocks => format::info::get_clocks(d, &recs),
            cli::GetWhat::PowerLimit => format::info::get_power_limit(d, &recs),
            cli::GetWhat::PowerProfile => format::info::get_power_profile(d),
        });
    }
    print!("{out}");
    Ok(())
}
