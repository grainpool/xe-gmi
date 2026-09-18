//! Control writes: the allowlist, verified writes, boot-default records and the set/reset ops
//!. This is the only module that opens files under `$SYSFS` for writing.

use crate::avail::Avail;
use crate::cli::{Channel, LimitKind, Profile, ResetWhat};
use crate::error::{Error, Result};
use crate::format::fields::uw_w;
use crate::paths::Roots;
use crate::persist;
use crate::probe::gt::Gt;
use crate::probe::{hwmon::effective_limit, Device};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// How `set` should treat the persistence state file.
#[derive(Clone, Copy)]
pub struct PersistFlags {
    pub no_persist: bool,
    pub persist: bool,
}

/// The only path shapes this crate may write. Matched against the device-relative
/// form of the target.
pub fn allowed(dev_dir: &Path, target: &Path) -> bool {
    let Ok(rel) = target.strip_prefix(dev_dir) else {
        return false;
    };
    let owned: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let parts: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [hwmon_dir, hw, attr]
            if *hwmon_dir == "hwmon"
                && hw.strip_prefix("hwmon").is_some_and(digits)
                && matches!(
                    *attr,
                    "power1_max"
                        | "power1_cap"
                        | "power2_max"
                        | "power2_cap"
                        | "power1_max_interval"
                        | "power1_cap_interval"
                        | "power2_max_interval"
                        | "power2_cap_interval"
                ) =>
        {
            true
        }
        [tile, gt, freq0, attr]
            if tile.starts_with("tile")
                && tile[4..].bytes().all(|b| b.is_ascii_digit())
                && gt.starts_with("gt")
                && gt[2..].bytes().all(|b| b.is_ascii_digit())
                && *freq0 == "freq0"
                && matches!(*attr, "min_freq" | "max_freq" | "power_profile") =>
        {
            true
        }
        // Half B: PCI reset pair and SR-IOV administration. Every one of these goes through a
        // preconditioned command (`recover`, `sriov`), never through a bare value pass-through.
        ["reset" | "reset_method" | "sriov_numvfs"] => true,
        [admin, bulk, attr]
            if *admin == "sriov_admin"
                && *bulk == ".bulk_profile"
                && matches!(
                    *attr,
                    "vram_quota" | "exec_quantum_ms" | "preempt_timeout_us" | "sched_priority"
                ) =>
        {
            true
        }
        [admin, vf, "stop"]
            if *admin == "sriov_admin"
                && vf.starts_with("vf")
                && vf[2..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            true
        }
        [admin, vf, "profile", attr]
            if *admin == "sriov_admin"
                && (*vf == "pf"
                    || (vf.starts_with("vf") && vf[2..].bytes().all(|b| b.is_ascii_digit())))
                && matches!(
                    *attr,
                    "vram_quota" | "exec_quantum_ms" | "preempt_timeout_us" | "sched_priority"
                ) =>
        {
            true
        }
        _ => false,
    }
}

/// Parses the VALUE argument of `set power-limit` (hand-rolled, no regex crate). Returns µW.
pub fn parse_power_value(s: &str) -> Result<u64> {
    let s = s.trim();
    let bad = || {
        Error::Usage(format!(
            "invalid power value {s:?}: accepted forms are watts (150, 150W, 150.5W), \
             milliwatts (150000mW) and microwatts (150000000uW)"
        ))
    };
    if s.starts_with(['-', '+']) {
        return Err(bad());
    }
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, suffix) = s.split_at(split);
    if num.is_empty() || num.matches('.').count() > 1 || num.ends_with('.') || num.starts_with('.')
    {
        return Err(bad());
    }
    let v: f64 = num.parse().map_err(|_| bad())?;
    let suffix = suffix.trim();
    let uw: f64 = match suffix {
        "" | "W" | "w" => v * 1e6,
        "mW" | "mw" => v * 1e3,
        "uW" | "uw" | "µW" => v,
        _ => return Err(bad()),
    };
    let uw = (uw + 0.5).floor() as u64; // round half up
    if uw < 1000 {
        return Err(Error::Usage(format!(
            "power value {s:?} rounds below 1000 µW"
        )));
    }
    Ok(uw)
}

/// The single verified write of the crate. Returns the readback text
/// (fixture readback sidecars honored).
pub fn write_verified(
    roots: &Roots,
    dev_dir: &Path,
    target: &Path,
    value: &str,
    sudo_hint: &str,
) -> Result<String> {
    if !allowed(dev_dir, target) {
        return Err(Error::Internal(format!(
            "write outside the allowlist: {}",
            target.display()
        )));
    }
    write_at(roots, target, value, sudo_hint)
}

/// System-level write path of spec/27 §1: only the xe driver bind/unbind pair lives here
/// (devcoredump data and cgroup dmem have their own checked helpers).
pub fn allowed_system(roots: &Roots, target: &Path) -> bool {
    let drivers_xe = roots.sysfs.join("bus/pci/drivers/xe");
    target.parent() == Some(drivers_xe.as_path())
        && matches!(
            target.file_name().and_then(|n| n.to_str()),
            Some("bind") | Some("unbind")
        )
}

/// `recover`: fire write to `drivers/xe/bind` / `unbind`.
pub fn write_system(roots: &Roots, target: &Path, value: &str, sudo_hint: &str) -> Result<()> {
    if !allowed_system(roots, target) {
        return Err(Error::Internal(format!(
            "system write outside the allowlist: {}",
            target.display()
        )));
    }
    write_at(roots, target, value, sudo_hint)?;
    Ok(())
}

/// Fire-and-forget write inside the device tree (bulk profile, VF stop). Allowlist still applies.
pub fn write_fire(
    roots: &Roots,
    dev_dir: &Path,
    target: &Path,
    value: &str,
    sudo_hint: &str,
) -> Result<()> {
    if !allowed(dev_dir, target) {
        return Err(Error::Internal(format!(
            "write outside the allowlist: {}",
            target.display()
        )));
    }
    write_at(roots, target, value, sudo_hint)?;
    Ok(())
}

/// `cgroup set`: verified write of a whole `dmem.max` file inside the cgroup tree. Own allowlist:
/// strictly under the cgroup root, plain relative path, file named exactly `dmem.max`/`dmem.min`/`dmem.low`.
pub fn write_cgroup_dmem(
    roots: &Roots,
    cgroup_dir: &Path,
    file: &str,
    value: &str,
    sudo_hint: &str,
) -> Result<String> {
    let rel = cgroup_dir.strip_prefix(&roots.cgroup).map_err(|_| {
        Error::Internal(format!(
            "cgroup write outside the cgroup root: {}",
            cgroup_dir.display()
        ))
    })?;
    let plain = !rel.as_os_str().is_empty()
        && rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !plain || !matches!(file, "dmem.max" | "dmem.min" | "dmem.low") {
        return Err(Error::Internal(format!(
            "cgroup write outside the allowlist: {file}"
        )));
    }
    write_at(roots, &cgroup_dir.join(file), value, sudo_hint)
}

/// `crash release`: writing anything to `class/devcoredump/devcd<N>/data` releases the dump.
pub fn release_coredump(roots: &Roots, target: &Path, sudo_hint: &str) -> Result<()> {
    let rel = target
        .strip_prefix(roots.sysfs.join("class/devcoredump"))
        .map_err(|_| {
            Error::Internal(format!(
                "coredump write outside class/devcoredump: {}",
                target.display()
            ))
        })?;
    let name = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let ok = matches!(name.as_slice(), [dir, file] if dir.starts_with("devcd") && dir[5..].bytes().all(|b| b.is_ascii_digit()) && *file == "data");
    if !ok {
        return Err(Error::Internal(format!(
            "coredump write outside the allowlist: {}",
            target.display()
        )));
    }
    write_at(roots, target, "1", sudo_hint)?;
    Ok(())
}

/// The only OpenOptions in the crate. Device-control writes and coredump releases only.
fn write_at(roots: &Roots, target: &Path, value: &str, sudo_hint: &str) -> Result<String> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(target)
        .map_err(|e| Error::from_write(target.to_path_buf(), e, sudo_hint.into()))?;
    f.write_all(format!("{value}\n").as_bytes())
        .map_err(|e| Error::from_write(target.to_path_buf(), e, sudo_hint.into()))?;
    f.flush().ok();
    drop(f);
    let sidecar = {
        let mut p = target.as_os_str().to_os_string();
        p.push(".readback");
        PathBuf::from(p)
    };
    if roots.fixture_mode() && sidecar.exists() {
        return Ok(crate::sysfs::read_string(&sidecar)
            .value()
            .cloned()
            .unwrap_or_default());
    }
    Ok(crate::sysfs::read_string(target)
        .value()
        .cloned()
        .unwrap_or_default())
}

/// Records the boot defaults of every writable control; idempotent per boot (never overwritten).
pub fn record_boot_defaults(roots: &Roots, dev: &Device, sudo_hint: &str) -> Result<()> {
    let bid = persist::boot_id(roots);
    let path = persist::boot_record_path(roots, &bid, &dev.pci);
    if path.exists() {
        return Ok(());
    }
    let mut out = format!(
        "# xe-gmi boot defaults, format 1, recorded {}\n",
        crate::time::iso8601(crate::paths::now_epoch())
    );
    if let Some(h) = &dev.hwmon {
        for (ch, name) in [(&h.card, "card"), (&h.pkg, "pkg")] {
            if let Avail::Value(v) = &ch.pl1 {
                out.push_str(&format!("pl1.{name} {v}\n"));
            }
            if let Avail::Value(v) = &ch.pl2 {
                out.push_str(&format!("pl2.{name} {v}\n"));
            }
            if let Avail::Value(v) = &ch.pl1_window_ms {
                out.push_str(&format!("pl1.{name}.window {v}\n"));
            }
            if let Avail::Value(v) = &ch.pl2_window_ms {
                out.push_str(&format!("pl2.{name}.window {v}\n"));
            }
        }
    }
    for g in &dev.gts {
        if let Avail::Value(v) = &g.min {
            out.push_str(&format!("gt{}.min_freq {v}\n", g.id));
        }
        if let Avail::Value(v) = &g.max {
            out.push_str(&format!("gt{}.max_freq {v}\n", g.id));
        }
        if let Avail::Value(t) = &g.profile {
            out.push_str(&format!("gt{}.power_profile {t}\n", g.id));
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| perm_or_io(dir, e, sudo_hint))?;
        // prune old boot directories: keep only the current boot
        let boots = dir.parent();
        if let Some(boots) = boots {
            if let Ok(rd) = std::fs::read_dir(boots) {
                for e in rd.flatten() {
                    if e.path() != dir && e.path().is_dir() {
                        let _ = std::fs::remove_dir_all(e.path());
                    }
                }
            }
        }
    }
    std::fs::write(&path, out).map_err(|e| perm_or_io(&path, e, sudo_hint))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
    }
    Ok(())
}

fn perm_or_io(path: &Path, e: std::io::Error, hint: &str) -> Error {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => Error::PermissionDenied {
            path: path.to_path_buf(),
            hint: hint.into(),
        },
        _ => Error::Io {
            path: path.to_path_buf(),
            source: e,
        },
    }
}

fn read_u64(p: &Path) -> Option<u64> {
    crate::sysfs::read_u64(p).value().copied()
}

fn limit_attr(n: u32, kind: &str) -> &'static str {
    match (n, kind) {
        (1, "pl1") | (2, "pl1") => "max",
        (_, "pl2") => "cap",
        _ => "max",
    }
}

/// Output includes the persistence line.
#[allow(clippy::too_many_arguments)]
/// How a power-limit readback compares to the value requested.
pub enum Readback {
    Exact,
    Clamped,
    Higher,
    Disabled,
}

/// Pure classification of a power-limit write's readback.
pub fn classify_readback(requested: u64, readback: u64) -> Readback {
    if readback == 0 {
        Readback::Disabled
    } else if readback == requested {
        Readback::Exact
    } else if readback < requested {
        Readback::Clamped
    } else {
        Readback::Higher
    }
}

pub fn set_power_limit(
    roots: &Roots,
    dev: &Device,
    value: &str,
    limit: LimitKind,
    channel: Channel,
    flags: PersistFlags,
    sudo_hint: &str,
) -> Result<String> {
    let req = parse_power_value(value)?;
    let rule_installed = persist::installed(roots);
    if flags.persist && !rule_installed {
        return Err(Error::Unavailable(
            "persistence is not installed; run: sudo xe-gmi persist install".into(),
        ));
    }
    let n = match channel {
        Channel::Card => 1u32,
        Channel::Pkg => 2,
    };
    let chan_name = match channel {
        Channel::Card => "card",
        Channel::Pkg => "pkg",
    };
    let h = dev
        .hwmon
        .as_ref()
        .ok_or_else(|| Error::Unavailable("hwmon directory not exposed (kernel 6.15+)".into()))?;
    let ch = if n == 1 { &h.card } else { &h.pkg };
    let kind: &'static str = match limit {
        LimitKind::Pl1 => "pl1",
        LimitKind::Pl2 => "pl2",
        LimitKind::Auto => match effective_limit(ch).value() {
            Some((_, k)) => k,
            None => {
                // fall back to the other channel only when the selected one has nothing at all
                return Err(Error::Unavailable(
                    crate::probe::hwmon::NO_LIMIT_MAILBOX.into(),
                ));
            }
        },
    };
    let attr = limit_attr(n, kind);
    let target = h.dir.join(format!("power{n}_{attr}"));
    if !target.exists() {
        return Err(Error::Unavailable(format!(
            "power{n}_{attr} not exposed ({})",
            crate::probe::hwmon::NO_LIMIT_MAILBOX
        )));
    }
    record_boot_defaults(roots, dev, sudo_hint)?;
    let rb_s = write_verified(roots, &dev.dev_dir, &target, &req.to_string(), sudo_hint)?;
    let rb: u64 = rb_s.trim().parse().map_err(|_| {
        Error::WriteFailed(format!(
            "write verification failed: wrote {req} uW, read back unparseable {rb_s:?}"
        ))
    })?;
    let mut out = String::new();
    match classify_readback(req, rb) {
        Readback::Exact => out.push_str(&format!(
            "GPU {} [{}]: power limit ({kind} {chan_name}) set to {} W\n",
            dev.index,
            dev.pci,
            uw_w(rb)
        )),
        Readback::Clamped => out.push_str(&format!(
            "GPU {} [{}]: power limit ({kind} {chan_name}) set to {} W \
             (requested {} W; clamped by the driver to the firmware maximum)\n",
            dev.index,
            dev.pci,
            uw_w(rb),
            uw_w(req)
        )),
        Readback::Disabled => {
            return Err(Error::WriteFailed(
                "write verification failed: limit reads back as disabled (0)".into(),
            ))
        }
        Readback::Higher => {
            return Err(Error::WriteFailed(format!(
                "write verification failed: wrote {req} uW, read back {rb} uW"
            )))
        }
    }
    let key = format!("{kind}.{chan_name}");
    out.push_str(&persist::after_set(
        roots,
        &dev.pci,
        &[(key, req.to_string())],
        flags.no_persist,
        flags.persist,
        sudo_hint,
    ));
    Ok(out)
}

/// Parses the VALUE of `set power-window`: `^\d+\s*(ms|s)?$`, milliseconds; `0` is rejected.
pub fn parse_window_value(s: &str) -> Result<u64> {
    let bad = || {
        Error::Usage(format!(
            "invalid window {s:?}: accepted forms are milliseconds (15, 15ms) and seconds (15s)"
        ))
    };
    let s = s.trim();
    if s.is_empty() || s.starts_with(['-', '+']) {
        return Err(bad());
    }
    let (num, mult) = if let Some(rest) = s.strip_suffix("ms") {
        (rest.trim_end(), 1u64)
    } else if let Some(rest) = s.strip_suffix('s') {
        (rest.trim_end(), 1000u64)
    } else {
        (s, 1u64)
    };
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let v: u64 = num.parse().map_err(|_| bad())?;
    let v = v * mult;
    if v == 0 {
        return Err(Error::Usage(
            "the averaging window must be at least 1 ms".into(),
        ));
    }
    Ok(v)
}

/// `set power-window`: write `power<N>_{max,cap}_interval`; firmware rounding is reported.
pub fn set_power_window(
    roots: &Roots,
    dev: &Device,
    value: &str,
    limit: LimitKind,
    channel: Channel,
    flags: PersistFlags,
    sudo_hint: &str,
) -> Result<String> {
    let req = parse_window_value(value)?;
    let rule_installed = persist::installed(roots);
    if flags.persist && !rule_installed {
        return Err(Error::Unavailable(
            "persistence is not installed; run: sudo xe-gmi persist install".into(),
        ));
    }
    let n = match channel {
        Channel::Card => 1u32,
        Channel::Pkg => 2,
    };
    let chan_name = match channel {
        Channel::Card => "card",
        Channel::Pkg => "pkg",
    };
    let h = dev
        .hwmon
        .as_ref()
        .ok_or_else(|| Error::Unavailable("hwmon directory not exposed (kernel 6.15+)".into()))?;
    let ch = if n == 1 { &h.card } else { &h.pkg };
    let kind: &'static str = match limit {
        LimitKind::Pl1 => "pl1",
        LimitKind::Pl2 => "pl2",
        LimitKind::Auto => match effective_limit(ch).value() {
            Some((_, k)) => k,
            None => {
                return Err(Error::Unavailable(
                    crate::probe::hwmon::NO_LIMIT_MAILBOX.into(),
                ));
            }
        },
    };
    let attr = format!("power{n}_{}_interval", limit_attr(n, kind));
    let target = h.dir.join(&attr);
    if !target.exists() {
        return Err(Error::Unavailable(format!(
            "{attr} not exposed for ({kind} {chan_name})"
        )));
    }
    record_boot_defaults(roots, dev, sudo_hint)?;
    let rb_s = write_verified(roots, &dev.dev_dir, &target, &req.to_string(), sudo_hint)?;
    let rb: u64 = rb_s.trim().parse().map_err(|_| {
        Error::WriteFailed(format!(
            "write verification failed: wrote {req} ms, read back unparseable {rb_s:?}"
        ))
    })?;
    let mut out = String::new();
    if rb == req {
        out.push_str(&format!(
            "GPU {} [{}]: power window ({kind} {chan_name}) set to {rb} ms\n",
            dev.index, dev.pci
        ));
    } else {
        out.push_str(&format!(
            "GPU {} [{}]: power window ({kind} {chan_name}) set to {rb} ms \
             (requested {req} ms; rounded by the firmware)\n",
            dev.index, dev.pci
        ));
    }
    let key = format!("{kind}.{chan_name}.window");
    out.push_str(&persist::after_set(
        roots,
        &dev.pci,
        &[(key, rb.to_string())],
        flags.no_persist,
        flags.persist,
        sudo_hint,
    ));
    Ok(out)
}

/// Targets of `set clocks`: every GT or the single global id.
fn target_gts(dev: &Device, gt: Option<u32>) -> Result<Vec<&Gt>> {
    let Some(id) = gt else {
        return Ok(dev.gts.iter().collect());
    };
    let found: Vec<&Gt> = dev.gts.iter().filter(|g| g.id == id).collect();
    if found.is_empty() {
        let names: Vec<String> = dev.gts.iter().map(|g| format!("gt{}", g.id)).collect();
        return Err(Error::Usage(format!(
            "no gt{id} on {}; GTs: {}",
            dev.pci,
            names.join(", ")
        )));
    }
    Ok(found)
}

/// `set clocks`: validate the resulting (min, max) pair against every target GT's hardware
/// range before any write, then write verified and report one line per GT.
pub fn set_clocks(
    roots: &Roots,
    dev: &Device,
    min: Option<u32>,
    max: Option<u32>,
    gt: Option<u32>,
    flags: PersistFlags,
    sudo_hint: &str,
) -> Result<String> {
    if min.is_none() && max.is_none() {
        return Err(Error::Usage("set clocks needs --min and/or --max".into()));
    }
    let rule_installed = persist::installed(roots);
    if flags.persist && !rule_installed {
        return Err(Error::Unavailable(
            "persistence is not installed; run: sudo xe-gmi persist install".into(),
        ));
    }
    let targets = target_gts(dev, gt)?;
    // validate everything before any write
    let mut planned: Vec<(&Gt, u32, u32)> = Vec::new();
    for g in targets {
        let cur_min = g.min.value().copied().ok_or_else(|| {
            Error::Unavailable(format!("freq0/min_freq not exposed on gt{}", g.id))
        })?;
        let cur_max = g.max.value().copied().ok_or_else(|| {
            Error::Unavailable(format!("freq0/max_freq not exposed on gt{}", g.id))
        })?;
        let (rpn, rp0) = (
            g.rpn.value().copied().unwrap_or(0),
            g.rp0.value().copied().unwrap_or(u32::MAX),
        );
        let (mn, mx) = (min.unwrap_or(cur_min), max.unwrap_or(cur_max));
        if mn < rpn {
            return Err(Error::Usage(format!(
                "--min {mn} is below rpn_freq {rpn} on gt{}",
                g.id
            )));
        }
        if mx > rp0 {
            return Err(Error::Usage(format!(
                "--max {mx} is above rp0_freq {rp0} on gt{}",
                g.id
            )));
        }
        if mn > mx {
            return Err(Error::Usage(format!(
                "--min {mn} is above --max {mx} on gt{} (hardware range {rpn}..{rp0} MHz)",
                g.id
            )));
        }
        planned.push((g, mn, mx));
    }
    record_boot_defaults(roots, dev, sudo_hint)?;
    let mut out = String::new();
    let mut persisted: Vec<(String, String)> = Vec::new();
    for (g, mn, mx) in &planned {
        let min_p = g.dir.join("freq0/min_freq");
        let max_p = g.dir.join("freq0/max_freq");
        let cur_min = read_u64(&min_p).unwrap_or(0) as u32;
        // §2.5 order: keep min <= max at every step
        if *mx < cur_min {
            write_clock(g, &dev.dev_dir, &min_p, *mn, sudo_hint, roots)?;
            write_clock(g, &dev.dev_dir, &max_p, *mx, sudo_hint, roots)?;
        } else {
            write_clock(g, &dev.dev_dir, &max_p, *mx, sudo_hint, roots)?;
            write_clock(g, &dev.dev_dir, &min_p, *mn, sudo_hint, roots)?;
        }
        out.push_str(&format!(
            "GPU {} [{}]: gt{} clocks min {} max {} MHz\n",
            dev.index, dev.pci, g.id, mn, mx
        ));
        persisted.push((format!("gt{}.min_freq", g.id), mn.to_string()));
        persisted.push((format!("gt{}.max_freq", g.id), mx.to_string()));
    }
    out.push_str(&persist::after_set(
        roots,
        &dev.pci,
        &persisted,
        flags.no_persist,
        flags.persist,
        sudo_hint,
    ));
    Ok(out)
}

fn write_clock(
    g: &Gt,
    dev_dir: &Path,
    p: &Path,
    v: u32,
    sudo_hint: &str,
    roots: &Roots,
) -> Result<()> {
    let rb = write_verified(roots, dev_dir, p, &v.to_string(), sudo_hint)?;
    if rb.trim() != v.to_string() {
        return Err(Error::WriteFailed(format!(
            "gt{} {}: wrote {}, read back {}",
            g.id,
            p.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            v,
            rb.trim()
        )));
    }
    Ok(())
}

fn profile_token(p: Profile) -> &'static str {
    match p {
        Profile::Base => "base",
        Profile::PowerSaving => "power_saving",
    }
}

/// `set power-profile`: write the bare token to every target GT and verify the bracketed
/// selection in the readback.
pub fn set_profile(
    roots: &Roots,
    dev: &Device,
    profile: Profile,
    gt: Option<u32>,
    flags: PersistFlags,
    sudo_hint: &str,
) -> Result<String> {
    let rule_installed = persist::installed(roots);
    if flags.persist && !rule_installed {
        return Err(Error::Unavailable(
            "persistence is not installed; run: sudo xe-gmi persist install".into(),
        ));
    }
    let targets = target_gts(dev, gt)?;
    for g in &targets {
        if !g.dir.join("freq0/power_profile").exists() {
            return Err(Error::Unavailable(
                "power_profile not exposed (kernel 6.18+)".into(),
            ));
        }
    }
    let tok = profile_token(profile);
    record_boot_defaults(roots, dev, sudo_hint)?;
    let mut out = String::new();
    let mut persisted = Vec::new();
    for g in &targets {
        let p = g.dir.join("freq0/power_profile");
        let rb = write_verified(roots, &dev.dev_dir, &p, tok, sudo_hint)?;
        let cur = crate::probe::gt::parse_profile_brackets(&p, rb.trim());
        // The kernel reports the selected token in brackets; a plain echo of what we wrote is
        // accepted too (fixture readbacks are not bracketed).
        let ok = match cur.value() {
            Some(t) => *t == tok,
            None => rb.trim() == tok,
        };
        if !ok {
            return Err(Error::WriteFailed(format!(
                "gt{} power_profile: wrote {tok}, read back {}",
                g.id,
                rb.trim()
            )));
        }
        out.push_str(&format!(
            "GPU {} [{}]: gt{} power profile set to {tok}\n",
            dev.index, dev.pci, g.id
        ));
        persisted.push((format!("gt{}.power_profile", g.id), tok.to_string()));
    }
    out.push_str(&persist::after_set(
        roots,
        &dev.pci,
        &persisted,
        flags.no_persist,
        flags.persist,
        sudo_hint,
    ));
    Ok(out)
}

/// Returns report text; `nothing to reset` when no control exists.
pub fn reset(
    roots: &Roots,
    dev: &Device,
    what: ResetWhat,
    no_persist: bool,
    sudo_hint: &str,
) -> Result<String> {
    let recs = persist::boot_records(roots, &persist::boot_id(roots), &dev.pci);
    let rec = |k: &str| persist::boot_record(&recs, k).map(str::to_string);
    let mut out = String::new();
    let mut removed_keys: Vec<String> = Vec::new();
    let mut touched = false;

    if matches!(what, ResetWhat::PowerLimit | ResetWhat::All) {
        if let Some(h) = &dev.hwmon {
            let writable: Vec<(u32, &'static str, &'static str, PathBuf)> = {
                let mut v = Vec::new();
                for (n, chan) in [(1u32, "card"), (2, "pkg")] {
                    for (kind, attr) in [("pl1", "max"), ("pl2", "cap")] {
                        let p = h.dir.join(format!("power{n}_{attr}"));
                        if p.exists() {
                            v.push((n, kind, chan, p));
                        }
                    }
                }
                v
            };
            for (_n, kind, chan, p) in writable {
                touched = true;
                let key = format!("{kind}.{chan}");
                let pre = read_u64(&p);
                let sentinel: u64 = 2_000_000_000;
                let (written, msg, pre_written): (u64, String, bool) = match rec(&key)
                    .and_then(|s| s.parse().ok())
                {
                    Some(v) => (
                        v,
                        format!(
                            "power limit ({kind} {chan}) restored to recorded boot default {} W",
                            uw_w(v)
                        ),
                        false,
                    ),
                    None if h.card.rated_max.value().is_some() => {
                        let rated = h.card.rated_max.value().copied().unwrap();
                        (
                            rated,
                            format!(
                                "power limit ({kind} {chan}) restored to rated max {} W",
                                uw_w(rated)
                            ),
                            false,
                        )
                    }
                    None => {
                        let rb_s = write_verified(
                            roots,
                            &dev.dev_dir,
                            &p,
                            &sentinel.to_string(),
                            sudo_hint,
                        )?;
                        let rb: u64 = rb_s.trim().parse().map_err(|_| {
                            Error::WriteFailed(format!(
                                "reset: {key} read back unparseable {rb_s:?}"
                            ))
                        })?;
                        if rb == sentinel {
                            if let Some(pre) = pre {
                                let _ = write_verified(
                                    roots,
                                    &dev.dev_dir,
                                    &p,
                                    &pre.to_string(),
                                    sudo_hint,
                                );
                            }
                            return Err(Error::WriteFailed(format!(
                                    "driver did not clamp the reset sentinel; refusing to leave {sentinel} uW"
                                )));
                        }
                        (
                                rb,
                                format!(
                                    "power limit ({kind} {chan}) restored to firmware default {} W (driver clamp)",
                                    uw_w(rb)
                                ),
                                true,
                            )
                    }
                };
                if !pre_written {
                    let rb_s =
                        write_verified(roots, &dev.dev_dir, &p, &written.to_string(), sudo_hint)?;
                    let rb: u64 = rb_s.trim().parse().unwrap_or(u64::MAX);
                    if !matches!(classify_readback(written, rb), Readback::Exact) {
                        return Err(Error::WriteFailed(format!(
                            "write verification failed: wrote {written} uW, read back {rb} uW"
                        )));
                    }
                }
                out.push_str(&format!("GPU {} [{}]: {msg}\n", dev.index, dev.pci));
                removed_keys.push(key);
            }
            // averaging windows restore alongside the limits
            for (n, chan) in [(1u32, "card"), (2, "pkg")] {
                for (kind, attr) in [("pl1", "max_interval"), ("pl2", "cap_interval")] {
                    let p = h.dir.join(format!("power{n}_{attr}"));
                    if !p.exists() {
                        continue;
                    }
                    let key = format!("{kind}.{chan}.window");
                    if let Some(v) = rec(&key).and_then(|x| x.parse::<u64>().ok()) {
                        if read_u64(&p) != Some(v) {
                            touched = true;
                            write_verified(roots, &dev.dev_dir, &p, &v.to_string(), sudo_hint)?;
                            out.push_str(&format!(
                                "GPU {} [{}]: power window ({kind} {chan}) restored to recorded boot default {v} ms\n",
                                dev.index, dev.pci
                            ));
                        }
                    }
                    removed_keys.push(key);
                }
            }
        }
    }
    if matches!(what, ResetWhat::Clocks | ResetWhat::All) {
        for g in &dev.gts {
            let rpn = g.rpn.value().copied();
            let rp0 = g.rp0.value().copied();
            if rpn.is_none() || rp0.is_none() {
                continue; // control unavailable → skipped
            }
            touched = true;
            let (mn, mx, msg) = match (
                rec(&format!("gt{}.min_freq", g.id)).and_then(|s| s.parse().ok()),
                rec(&format!("gt{}.max_freq", g.id)).and_then(|s| s.parse().ok()),
            ) {
                (Some(mn), Some(mx)) => (
                    mn,
                    mx,
                    format!(
                        "gt{} clocks restored to recorded boot default min {mn} max {mx} MHz",
                        g.id
                    ),
                ),
                _ => {
                    let mut mn = rpn.unwrap();
                    // Battlemage graphics GTs keep 1200 as the floor
                    if g.kind == crate::probe::gt::GtKind::Render && dev.ids.device >= 0xe200 {
                        mn = mn.max(1200);
                    }
                    (
                        mn,
                        rp0.unwrap(),
                        format!(
                            "gt{} clocks restored to hardware range min {mn} max {} MHz",
                            g.id,
                            rp0.unwrap()
                        ),
                    )
                }
            };
            let min_p = g.dir.join("freq0/min_freq");
            let max_p = g.dir.join("freq0/max_freq");
            let cur_min = read_u64(&min_p).unwrap_or(0) as u32;
            if mx < cur_min {
                write_clock(g, &dev.dev_dir, &min_p, mn, sudo_hint, roots)?;
                write_clock(g, &dev.dev_dir, &max_p, mx, sudo_hint, roots)?;
            } else {
                write_clock(g, &dev.dev_dir, &max_p, mx, sudo_hint, roots)?;
                write_clock(g, &dev.dev_dir, &min_p, mn, sudo_hint, roots)?;
            }
            out.push_str(&format!("GPU {} [{}]: {msg}\n", dev.index, dev.pci));
            removed_keys.push(format!("gt{}.min_freq", g.id));
            removed_keys.push(format!("gt{}.max_freq", g.id));
        }
    }
    if matches!(what, ResetWhat::PowerProfile | ResetWhat::All) {
        for g in &dev.gts {
            if !g.dir.join("freq0/power_profile").exists() {
                continue;
            }
            touched = true;
            let tok = rec(&format!("gt{}.power_profile", g.id)).unwrap_or_else(|| "base".into());
            let p = g.dir.join("freq0/power_profile");
            let rb = write_verified(roots, &dev.dev_dir, &p, &tok, sudo_hint)?;
            let cur = crate::probe::gt::parse_profile_brackets(&p, rb.trim());
            let ok = match cur.value() {
                Some(t) => t.as_str() == tok,
                None => rb.trim() == tok,
            };
            if !ok {
                return Err(Error::WriteFailed(format!(
                    "gt{} power_profile: wrote {tok}, read back {}",
                    g.id,
                    rb.trim()
                )));
            }
            let msg = if rec(&format!("gt{}.power_profile", g.id)).is_some() {
                format!(
                    "gt{} power profile restored to recorded boot default {tok}",
                    g.id
                )
            } else {
                format!("gt{} power profile restored to base", g.id)
            };
            out.push_str(&format!("GPU {} [{}]: {msg}\n", dev.index, dev.pci));
            removed_keys.push(format!("gt{}.power_profile", g.id));
        }
    }
    if !touched {
        return Ok("nothing to reset\n".into());
    }
    out.push_str(&persist::after_reset(
        roots,
        &dev.pci,
        &removed_keys,
        no_persist,
        sudo_hint,
    ));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{allowed, classify_readback, parse_power_value, parse_window_value};
    use std::path::Path;

    #[test]
    fn allowlist_accepts_only_the_documented_shapes() {
        let d = Path::new("/sys/devices/.../0000:e3:00.0");
        assert!(allowed(d, &d.join("hwmon/hwmon7/power1_max")));
        assert!(allowed(d, &d.join("hwmon/hwmon7/power1_cap")));
        assert!(allowed(d, &d.join("hwmon/hwmon1/power2_cap")));
        assert!(allowed(d, &d.join("tile0/gt0/freq0/min_freq")));
        assert!(allowed(d, &d.join("tile1/gt3/freq0/max_freq")));
        assert!(allowed(d, &d.join("tile0/gt0/freq0/power_profile")));
        // averaging windows join the hwmon shapes (0.1.2)
        assert!(allowed(d, &d.join("hwmon/hwmon7/power1_max_interval")));
        assert!(allowed(d, &d.join("hwmon/hwmon7/power2_cap_interval")));
        // and rejects every other file
        assert!(!allowed(d, &d.join("hwmon/hwmon7/power1_crit")));
        assert!(!allowed(d, &d.join("hwmon/hwmon7/power3_cap_interval")));
        assert!(!allowed(d, &d.join("hwmon/hwmon7/power3_cap")));
        assert!(!allowed(d, &d.join("tile0/gt0/freq0/rp0_freq")));
        assert!(!allowed(d, &d.join("tile0/gt0/gtidle/idle_status")));
        assert!(!allowed(d, &d.join("remove")));
        assert!(!allowed(d, &d.join("power/control")));
        assert!(!allowed(d, &d.join("vram_d3cold_threshold")));
        assert!(!allowed(
            Path::new("/dev"),
            Path::new("/elsewhere/tile0/gt0/freq0/min_freq")
        ));
    }

    #[test]
    fn power_value_units() {
        assert_eq!(parse_power_value("150").unwrap(), 150_000_000);
        assert_eq!(parse_power_value("150W").unwrap(), 150_000_000);
        assert_eq!(parse_power_value("150.5w").unwrap(), 150_500_000);
        assert_eq!(parse_power_value("150000mW").unwrap(), 150_000_000);
        assert_eq!(parse_power_value("150000000uW").unwrap(), 150_000_000);
        assert_eq!(parse_power_value("150000000µW").unwrap(), 150_000_000);
        assert_eq!(parse_power_value(" 150.5 W ").unwrap(), 150_500_000);
        // half-up rounding at the µW boundary (150.0000005 W → 150000001 µW, not 150000000)
        assert_eq!(parse_power_value("150.0000005W").unwrap(), 150_000_001);
    }

    #[test]
    fn power_value_rejections() {
        for bad in ["0", "-5", "abc", "150kW", "0.0004", "15..5", "150Wx", ".5"] {
            assert_eq!(
                parse_power_value(bad)
                    .map(|_| ())
                    .err()
                    .map(|e| e.exit_code()),
                Some(2),
                "{bad} must be rejected with exit 2"
            );
        }
    }

    #[test]
    fn allowlist_matches_expected_file() {
        // tests/allowlist_b65_expected.txt lists every writable path of the B65 fixture.
        fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                // never follow symlinks: the tree links back into itself (drm/card1/device, driver)
                if e.file_type().map(|t| t.is_symlink()).unwrap_or(true) {
                    continue;
                }
                let p = e.path();
                if p.is_dir() {
                    walk(&p, f);
                } else {
                    f(&p);
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dev = root.join("fixtures/synthetic/b65-g31-k7.1/sys/devices/pci0000:e3/0000:e3:00.0");
        let expected: Vec<String> =
            std::fs::read_to_string(root.join("tests/allowlist_b65_expected.txt"))
                .expect("tests/allowlist_b65_expected.txt")
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
        assert!(
            expected.len() >= 50,
            "expected file changed shape: {expected:?}"
        );
        let mut total = 0usize;
        let mut passed: Vec<String> = Vec::new();
        walk(&dev, &mut |p| {
            total += 1;
            if allowed(&dev, p) {
                passed.push(p.strip_prefix(&dev).unwrap().display().to_string());
            }
        });
        passed.sort();
        assert_eq!(passed, expected);
        assert!(total > 100, "walked only {total} files; wrong directory?");
    }

    #[test]
    fn window_value_parse() {
        assert_eq!(parse_window_value("15").unwrap(), 15);
        assert_eq!(parse_window_value("15ms").unwrap(), 15);
        assert_eq!(parse_window_value("28s").unwrap(), 28_000);
        assert_eq!(parse_window_value(" 28 s ").unwrap(), 28_000);
        for bad in ["0", "0s", "abc", "-5ms", "5h", "", "1.5s", "+3"] {
            assert_eq!(
                parse_window_value(bad)
                    .map(|_| ())
                    .err()
                    .map(|e| e.exit_code()),
                Some(2),
                "{bad:?} must be rejected with exit 2"
            );
        }
    }

    #[test]
    fn clamp_classification() {
        use super::Readback::*;
        assert!(matches!(classify_readback(150_000_000, 150_000_000), Exact));
        assert!(matches!(
            classify_readback(250_000_000, 200_000_000),
            Clamped
        ));
        assert!(matches!(
            classify_readback(150_000_000, 160_000_000),
            Higher
        ));
        assert!(matches!(classify_readback(150_000_000, 0), Disabled));
        assert!(matches!(classify_readback(1, 1), Exact));
    }

    #[test]
    fn cgroup_write_allowlist() {
        let tmp = std::env::temp_dir().join(format!("xe-gmi-cgw-{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        let roots = crate::paths::Roots {
            cgroup: tmp.join("cg"),
            ..crate::paths::Roots::from_env()
        };
        std::fs::create_dir_all(roots.cgroup.join("system.slice/x")).unwrap();
        // plain relative dir + dmem.max is fine even if the file is absent (write error, not policy)
        assert!(!roots
            .cgroup
            .join("system.slice/x")
            .join("dmem.max")
            .exists());
        // outside the cgroup root: policy error before any IO
        let foreign = tmp.join("elsewhere");
        std::fs::create_dir_all(&foreign).unwrap();
        let e =
            crate::write::write_cgroup_dmem(&roots, &foreign, "dmem.max", "x", "sudo").unwrap_err();
        assert!(matches!(e, crate::error::Error::Internal(_)), "{e:?}");
        // dot components are rejected
        let sneaky = roots.cgroup.join("system.slice/../..");
        let e =
            crate::write::write_cgroup_dmem(&roots, &sneaky, "dmem.max", "x", "sudo").unwrap_err();
        assert!(matches!(e, crate::error::Error::Internal(_)), "{e:?}");
        // only the three dmem files
        let e = crate::write::write_cgroup_dmem(
            &roots,
            &roots.cgroup.join("system.slice/x"),
            "cgroup.procs",
            "x",
            "sudo",
        )
        .unwrap_err();
        assert!(matches!(e, crate::error::Error::Internal(_)), "{e:?}");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
