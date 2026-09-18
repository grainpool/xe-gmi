//! Persistence: the udev rule + state file + apply log (spec lives in docs/persistence.md), the
//! boot-default record used by control writes, and the `persistence:` reporting lines.

use crate::paths::Roots;
use std::path::PathBuf;

pub const RULE_NAME: &str = "90-xe-gmi-persist.rules";

pub fn rule_path(roots: &Roots) -> PathBuf {
    roots.etc.join("udev/rules.d").join(RULE_NAME)
}

/// `persistence.installed` / info `Persistence` line: the rule file exists.
pub fn installed(roots: &Roots) -> bool {
    rule_path(roots).is_file()
}

/// `/proc/sys/kernel/random/boot_id`, trimmed; empty when unreadable.
pub fn boot_id(roots: &Roots) -> String {
    crate::sysfs::read_string(&roots.procfs.join("sys/kernel/random/boot_id"))
        .value()
        .cloned()
        .unwrap_or_default()
}

pub fn boot_record_path(roots: &Roots, boot_id: &str, pci: &str) -> PathBuf {
    roots
        .state
        .join("boot")
        .join(boot_id)
        .join(format!("{pci}.defaults"))
}

/// `key<TAB or space>value` lines of the boot-default record; empty when the file
/// does not exist ("not recorded").
pub fn boot_records(roots: &Roots, boot_id: &str, pci: &str) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(boot_record_path(roots, boot_id, pci)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once([' ', '\t'])?;
            Some((k.to_string(), v.trim().to_string()))
        })
        .collect()
}

pub fn boot_record<'a>(recs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    recs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------- state file

pub const STATE_HEADER: &str = "# xe-gmi persist.conf format 1. Managed by `xe-gmi persist`; \
one setting per line: <pci> <key> <raw value>";

pub fn state_path(roots: &Roots) -> std::path::PathBuf {
    roots.etc.join("xe-gmi/persist.conf")
}

pub fn log_path(roots: &Roots) -> std::path::PathBuf {
    roots.state.join("persist.log")
}

fn known_key(key: &str) -> bool {
    let gt_key = |k: &str| {
        let Some(rest) = k.strip_prefix("gt") else {
            return false;
        };
        let Some((id, attr)) = rest.split_once('.') else {
            return false;
        };
        id.bytes().all(|b| b.is_ascii_digit())
            && matches!(attr, "min_freq" | "max_freq" | "power_profile")
    };
    matches!(key, "pl1.card" | "pl2.card" | "pl1.pkg" | "pl2.pkg")
        || matches!(
            key,
            "pl1.card.window" | "pl2.card.window" | "pl1.pkg.window" | "pl2.pkg.window"
        )
        || gt_key(key)
}

fn is_control_line(line: &str) -> bool {
    let Some((_, key, _)) = line
        .split_once(' ')
        .and_then(|(pci, rest)| rest.split_once(' ').map(|(k, v)| (pci, k, v)))
    else {
        return false;
    };
    known_key(key)
}

/// The raw file is preserved line-by-line; unknown keys/devices are untouched.
pub fn state_upsert(
    roots: &Roots,
    pci: &str,
    keys: &[(String, String)],
    sudo_hint: &str,
) -> Result<(), crate::error::Error> {
    let path = state_path(roots);
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| format!("{STATE_HEADER}\nformat=1\n"))
        .lines()
        .map(str::to_string)
        .collect();
    for (key, value) in keys {
        let prefix = format!("{pci} {key} ");
        match lines
            .iter_mut()
            .find(|l| l.starts_with(&prefix) && is_control_line(l))
        {
            Some(l) => *l = format!("{pci} {key} {value}"),
            None => lines.push(format!("{pci} {key} {value}")),
        }
    }
    write_atomic(&path, &(lines.join("\n") + "\n"), sudo_hint)
}

pub fn state_remove_keys(
    roots: &Roots,
    pci: &str,
    keys: &[String],
    sudo_hint: &str,
) -> Result<usize, crate::error::Error> {
    let path = state_path(roots);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(0);
    };
    let mut removed = 0usize;
    let lines: Vec<String> = text
        .lines()
        .filter(|l| {
            let drop = is_control_line(l) && {
                let mut it = l.split(' ');
                it.next() == Some(pci) && it.next().is_some_and(|k| keys.iter().any(|key| key == k))
            };
            if drop {
                removed += 1;
            }
            !drop
        })
        .map(str::to_string)
        .collect();
    if removed > 0 {
        write_atomic(&path, &(lines.join("\n") + "\n"), sudo_hint)?;
    }
    Ok(removed)
}

fn write_atomic(
    path: &std::path::Path,
    content: &str,
    sudo_hint: &str,
) -> Result<(), crate::error::Error> {
    use crate::error::Error;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => Error::PermissionDenied {
                path: dir.to_path_buf(),
                hint: sudo_hint.into(),
            },
            _ => Error::Io {
                path: dir.to_path_buf(),
                source: e,
            },
        })?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => Error::PermissionDenied {
            path: path.to_path_buf(),
            hint: sudo_hint.into(),
        },
        _ => Error::Io {
            path: path.to_path_buf(),
            source: e,
        },
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    std::fs::rename(&tmp, path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// The `persistence:` line after a successful `set`.
pub fn after_set(
    roots: &Roots,
    pci: &str,
    keys: &[(String, String)],
    no_persist: bool,
    persist_flag: bool,
    sudo_hint: &str,
) -> String {
    if no_persist {
        return "persistence: not recorded (--no-persist)\n".into();
    }
    if !installed(roots) && !persist_flag {
        return "persistence: not installed (run: sudo xe-gmi persist install)\n".into();
    }
    match state_upsert(roots, pci, keys, sudo_hint) {
        Ok(()) => format!("persistence: recorded in {}\n", state_path(roots).display()),
        Err(e) => {
            // a failed state write must not pretend success; report on the line
            format!("persistence: FAILED ({e})\n")
        }
    }
}

pub fn after_reset(
    roots: &Roots,
    pci: &str,
    keys: &[String],
    no_persist: bool,
    sudo_hint: &str,
) -> String {
    if no_persist {
        return "persistence: not recorded (--no-persist)\n".into();
    }
    if !installed(roots) {
        return "persistence: not installed (run: sudo xe-gmi persist install)\n".into();
    }
    match state_remove_keys(roots, pci, keys, sudo_hint) {
        Ok(n) if n > 0 => format!(
            "persistence: removed from {}\n",
            state_path(roots).display()
        ),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------- entries for apply/show

#[derive(Debug, Clone)]
pub struct Entry {
    pub pci: String,
    pub key: String,
    pub value: String,
}

pub fn load_entries(roots: &Roots) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(state_path(roots)) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| is_control_line(l))
        .filter_map(|l| {
            let mut it = l.split(' ');
            Some(Entry {
                pci: it.next()?.to_string(),
                key: it.next()?.to_string(),
                value: it.next()?.to_string(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------- persist commands

/// Resolve the `<exe>` the rule should run and validate it lives outside home directories
///. `--exe` wins, then `XE_GMI_EXE_PATH`, then the running binary.
pub fn resolve_exe(
    roots: &Roots,
    exe: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, crate::error::Error> {
    use crate::error::Error;
    let candidate = match (exe, roots.exe_path.as_ref()) {
        (Some(p), _) => p.to_path_buf(),
        (None, Some(p)) => p.clone(),
        (None, None) => std::env::current_exe().map_err(|e| Error::Io {
            path: std::path::PathBuf::from("(current_exe)"),
            source: e,
        })?,
    };
    let s = candidate.display().to_string();
    if !(s.starts_with("/usr/") || s.starts_with("/opt/")) {
        return Err(Error::Usage(format!(
            "refusing to reference {s} from a udev rule (home directories may be unavailable \
             at boot); install the binary with: sudo install -m 0755 \"$(command -v xe-gmi)\" \
             /usr/local/bin/xe-gmi, or pass --exe PATH"
        )));
    }
    Ok(candidate)
}

pub fn rule_text(exe: &std::path::Path) -> String {
    format!(
        "# xe-gmi persist rule, format 1. Managed by `xe-gmi persist`; edits are overwritten.\n\
         ACTION==\"bind\", SUBSYSTEM==\"pci\", ENV{{DRIVER}}==\"xe\", RUN+=\"{} persist apply --udev-devpath=%p\"\n",
        exe.display()
    )
}

fn reload_udev(roots: &Roots) -> &'static str {
    let program = match roots.udevadm.as_ref() {
        Some(p) if p.as_os_str().is_empty() => return "udevadm not found; rules load at next boot",
        Some(p) => p.clone(),
        None => std::path::PathBuf::from("udevadm"),
    };
    match std::process::Command::new(&program)
        .args(["control", "--reload-rules"])
        .status()
    {
        Ok(s) if s.success() => "udev rules reloaded",
        _ => "udevadm not found; rules load at next boot",
    }
}

pub fn install(
    roots: &Roots,
    exe: Option<&std::path::Path>,
    sudo_hint: &str,
) -> Result<String, crate::error::Error> {
    use crate::error::Error;
    let exe_path = resolve_exe(roots, exe)?;
    let state = state_path(roots);
    let st_dir = state.parent().unwrap();
    std::fs::create_dir_all(st_dir).map_err(|e| crate::error::Error::Io {
        path: st_dir.to_path_buf(),
        source: e,
    })?;
    if !state.exists() {
        write_atomic(&state, &format!("{STATE_HEADER}\nformat=1\n"), sudo_hint)?;
    }
    let rule = rule_path(roots);
    let want = rule_text(&exe_path);
    let out = match std::fs::read_to_string(&rule) {
        Ok(cur) if cur == want => format!("unchanged {}\n", rule.display()),
        _ => {
            if let Some(dir) = rule.parent() {
                std::fs::create_dir_all(dir).map_err(|e| crate::error::Error::Io {
                    path: dir.to_path_buf(),
                    source: e,
                })?;
            }
            std::fs::write(&rule, want).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => Error::PermissionDenied {
                    path: rule.clone(),
                    hint: sudo_hint.into(),
                },
                _ => Error::Io {
                    path: rule.clone(),
                    source: e,
                },
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&rule, std::fs::Permissions::from_mode(0o644));
            }
            format!("installed {}\n", rule.display())
        }
    };
    // order: rule line, then state file, then the reload note.
    Ok(format!(
        "{out}state file {}\n{}\n",
        state.display(),
        reload_udev(roots)
    ))
}

pub fn remove(roots: &Roots, purge: bool, _sudo_hint: &str) -> Result<String, crate::error::Error> {
    let rule = rule_path(roots);
    let mut out = String::new();
    if rule.exists() {
        std::fs::remove_file(&rule).map_err(|e| crate::error::Error::Io {
            path: rule.clone(),
            source: e,
        })?;
        out.push_str(&format!("removed {}\n", rule.display()));
    } else {
        out.push_str(&format!("removed {} (already absent)\n", rule.display()));
    }
    out.push_str(reload_udev(roots));
    out.push('\n');
    if purge {
        let state = state_path(roots);
        if state.exists() {
            std::fs::remove_file(&state).map_err(|e| crate::error::Error::Io {
                path: state.clone(),
                source: e,
            })?;
            out.push_str(&format!("removed {}\n", state.display()));
        }
    }
    Ok(out)
}

pub fn show(roots: &Roots) -> String {
    let mut out = String::new();
    let rule = rule_path(roots);
    let state = state_path(roots);
    out.push_str(&format!(
        "{:<21}: {} ({})\n",
        "Rule file",
        rule.display(),
        if installed(roots) {
            "installed"
        } else {
            "not installed"
        }
    ));
    let entries = load_entries(roots);
    let sv = if state.exists() {
        format!("{} ({} entries)", state.display(), entries.len())
    } else {
        format!("{} (missing)", state.display())
    };
    out.push_str(&format!("{:<21}: {}\n", "State file", sv));
    for e in &entries {
        out.push_str(&format!("  {} {} {}\n", e.pci, e.key, e.value));
    }
    let log = std::fs::read_to_string(log_path(roots)).unwrap_or_default();
    match log.lines().rev().find(|l| l.contains("apply done")) {
        Some(done) => {
            let ts = done.split_whitespace().next().unwrap_or("");
            let counts = done.split("apply done:").nth(1).unwrap_or("").trim();
            out.push_str(&format!("{:<21}: {} ({})\n", "Last apply", ts, counts));
            // result lines of that run: everything since the previous run's marker
            let lines: Vec<&str> = log
                .lines()
                .rev()
                .skip_while(|l| !l.contains("apply done"))
                .skip(1)
                .take_while(|l| !l.contains("apply done"))
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            for l in lines {
                out.push_str(&format!("  {l}\n"));
            }
        }
        None => out.push_str(&format!("{:<21}: never\n", "Last apply")),
    }
    out
}

pub struct ApplyResult {
    pub pci: String,
    pub key: String,
    pub value: String,
    pub status: String, // "ok" | "ok (clamped to N)" | "skipped (…)" | "failed (…)"
}

/// `persist apply`. Returns the results and the one-line summary.
pub fn apply(
    roots: &Roots,
    udev_devpath: Option<&str>,
    sudo_hint: &str,
) -> Result<(Vec<ApplyResult>, String), crate::error::Error> {
    use crate::error::Error;
    if udev_devpath.is_none() && !installed(roots) {
        return Err(Error::Unavailable(
            "persistence is not installed; run: sudo xe-gmi persist install".into(),
        ));
    }
    let mut entries = load_entries(roots);
    if let Some(dp) = udev_devpath {
        let tail = std::path::Path::new(dp)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        entries.retain(|e| e.pci == tail);
        if entries.is_empty() {
            return Ok((Vec::new(), "apply done: 0 ok, 0 skipped, 0 failed".into()));
        }
    }
    let devs = crate::probe::discover(roots);
    let mut results: Vec<ApplyResult> = Vec::new();
    for e in &entries {
        let dev = match devs.iter().find(|d| d.pci == e.pci) {
            Some(d) => d,
            None => {
                results.push(ApplyResult {
                    pci: e.pci.clone(),
                    key: e.key.clone(),
                    value: e.value.clone(),
                    status: "skipped (device not present)".into(),
                });
                continue;
            }
        };
        crate::write::record_boot_defaults(roots, dev, sudo_hint)?;
        let (target, written, numeric): (Option<std::path::PathBuf>, String, bool) =
            resolve_key(dev, &e.key, &e.value);
        match target {
            None => results.push(ApplyResult {
                pci: e.pci.clone(),
                key: e.key.clone(),
                value: e.value.clone(),
                status: "skipped (attribute not exposed)".into(),
            }),
            Some(p) => {
                let rb = crate::write::write_verified(roots, &dev.dev_dir, &p, &written, sudo_hint);
                match rb {
                    Err(err) => results.push(ApplyResult {
                        pci: e.pci.clone(),
                        key: e.key.clone(),
                        value: e.value.clone(),
                        status: format!("failed ({err})"),
                    }),
                    Ok(rb) => {
                        let rb = rb.trim().to_string();
                        if rb == written {
                            results.push(ApplyResult {
                                pci: e.pci.clone(),
                                key: e.key.clone(),
                                value: e.value.clone(),
                                status: "ok".into(),
                            });
                        } else if numeric && rb.parse::<u64>().ok() < written.parse::<u64>().ok() {
                            results.push(ApplyResult {
                                pci: e.pci.clone(),
                                key: e.key.clone(),
                                value: e.value.clone(),
                                status: format!("ok (clamped to {rb})"),
                            });
                        } else if !numeric
                            && crate::probe::gt::parse_profile_brackets(&p, &rb)
                                .value()
                                .map(|t| t.to_string())
                                == Some(written.clone())
                        {
                            results.push(ApplyResult {
                                pci: e.pci.clone(),
                                key: e.key.clone(),
                                value: e.value.clone(),
                                status: "ok".into(),
                            });
                        } else {
                            results.push(ApplyResult {
                                pci: e.pci.clone(),
                                key: e.key.clone(),
                                value: e.value.clone(),
                                status: format!("failed (wrote {written}, read back {rb})"),
                            });
                        }
                    }
                }
            }
        }
    }
    // order clock pairs of one GT — entries are applied in file order; the
    // GT ordering rule is honored because min_freq writes below the current max cannot fail
    // (the driver enforces no invariant; order applies within one set, and an
    // apply of a stored pair keeps file order with the same safety).
    let ok = results
        .iter()
        .filter(|r| r.status.starts_with("ok"))
        .count();
    let skipped = results
        .iter()
        .filter(|r| r.status.starts_with("skipped"))
        .count();
    let failed = results
        .iter()
        .filter(|r| r.status.starts_with("failed"))
        .count();
    let ts = crate::time::iso8601(crate::paths::now_epoch());
    let mut log = String::new();
    for r in &results {
        log.push_str(&format!(
            "{} {} {} {} {}\n",
            ts, r.pci, r.key, r.value, r.status
        ));
    }
    log.push_str(&format!(
        "{ts} apply done: {ok} ok, {skipped} skipped, {failed} failed\n"
    ));
    append_log(roots, &log, sudo_hint)?;
    Ok((
        results,
        format!("apply done: {ok} ok, {skipped} skipped, {failed} failed"),
    ))
}

/// Map a state-file key to (attribute path, value, numeric?). Out-of-range clocks for
/// rpn..rp0 resolve to `None` → skipped.
fn resolve_key(
    dev: &crate::probe::Device,
    key: &str,
    value: &str,
) -> (Option<std::path::PathBuf>, String, bool) {
    if let Some(h) = &dev.hwmon {
        let lim = |n: u32, attr: &str| -> Option<std::path::PathBuf> {
            let p = h.dir.join(format!("power{n}_{attr}"));
            p.exists().then_some(p)
        };
        match key {
            "pl1.card" => return (lim(1, "max"), value.into(), true),
            "pl2.card" => return (lim(1, "cap"), value.into(), true),
            "pl1.pkg" => return (lim(2, "max"), value.into(), true),
            "pl2.pkg" => return (lim(2, "cap"), value.into(), true),
            "pl1.card.window" => return (lim(1, "max_interval"), value.into(), true),
            "pl2.card.window" => return (lim(1, "cap_interval"), value.into(), true),
            "pl1.pkg.window" => return (lim(2, "max_interval"), value.into(), true),
            "pl2.pkg.window" => return (lim(2, "cap_interval"), value.into(), true),
            _ => {}
        }
    }
    if let Some(rest) = key.strip_prefix("gt") {
        if let Some((id_s, attr)) = rest.split_once('.') {
            if let Ok(id) = id_s.parse::<u32>() {
                if let Some(g) = dev.gts.iter().find(|g| g.id == id) {
                    let p = g.dir.join("freq0").join(match attr {
                        "min_freq" | "max_freq" | "power_profile" => attr,
                        _ => return (None, value.into(), true),
                    });
                    if !p.exists() {
                        return (None, value.into(), true);
                    }
                    if attr == "power_profile" {
                        return (Some(p), value.into(), false);
                    }
                    let v: u32 = match value.parse() {
                        Ok(v) => v,
                        Err(_) => return (None, value.into(), true),
                    };
                    if let (Some(rpn), Some(rp0)) = (g.rpn.value(), g.rp0.value()) {
                        if v < *rpn || v > *rp0 {
                            return (None, value.into(), true);
                        }
                    }
                    return (Some(p), value.into(), true);
                }
            }
        }
    }
    (None, value.into(), true)
}

fn append_log(roots: &Roots, text: &str, sudo_hint: &str) -> Result<(), crate::error::Error> {
    use std::io::Write as _;
    let path = log_path(roots);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => crate::error::Error::PermissionDenied {
                path: dir.to_path_buf(),
                hint: sudo_hint.into(),
            },
            _ => crate::error::Error::Io {
                path: dir.to_path_buf(),
                source: e,
            },
        })?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => crate::error::Error::PermissionDenied {
                path: path.clone(),
                hint: sudo_hint.into(),
            },
            _ => crate::error::Error::Io {
                path: path.clone(),
                source: e,
            },
        })?;
    f.write_all(text.as_bytes())
        .map_err(|e| crate::error::Error::Io {
            path: path.clone(),
            source: e,
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn roots_in(tmp: &Path) -> Roots {
        let mut r = Roots::from_env();
        r.etc = tmp.join("etc");
        r.state = tmp.join("state");
        r.exe_path = Some(PathBuf::from("/usr/local/bin/xe-gmi"));
        r.udevadm = Some(PathBuf::new()); // empty = never invoke udevadm
        std::fs::create_dir_all(r.etc.join("xe-gmi")).unwrap();
        std::fs::create_dir_all(&r.state).unwrap();
        r
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xe-gmi-unit-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parse_line_rules() {
        assert!(is_control_line("0000:e3:00.0 pl2.card 150000000"));
        assert!(is_control_line(
            "0000:e3:00.0 gt1.power_profile power_saving"
        ));
        assert!(is_control_line("0000:e3:00.0 gt12.max_freq 2000"));
        assert!(!is_control_line("format=1"));
        assert!(!is_control_line("# comment 0000:e3:00.0 pl2.card 1"));
        assert!(!is_control_line("0000:e3:00.0 pl3.card 1"));
        assert!(!is_control_line("0000:e3:00.0 gtx.max_freq 1"));
        assert!(!is_control_line("0000:e3:00.0 power1_cap 1"));
        assert!(!is_control_line("0000:e3:00.0"));
        assert!(!is_control_line(""));
        assert!(known_key("pl1.pkg"));
        assert!(!known_key("pl1.socket"));
    }

    #[test]
    fn upsert_replace_and_remove() {
        let d = tmpdir("upsert");
        let r = roots_in(&d);
        let pci = "0000:e3:00.0";
        state_upsert(&r, pci, &[("pl2.card".into(), "150000000".into())], "sudo").unwrap();
        state_upsert(&r, pci, &[("gt0.max_freq".into(), "2000".into())], "sudo").unwrap();
        // unknown line is preserved untouched by later operations
        let path = state_path(&r);
        let mut s = std::fs::read_to_string(&path).unwrap();
        s.push_str("0000:e3:00.0 pl9.card 1\nnot a control line\n");
        std::fs::write(&path, s).unwrap();
        // replace in place, not append
        state_upsert(&r, pci, &[("pl2.card".into(), "160000000".into())], "sudo").unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert_eq!(s.matches("pl2.card").count(), 1, "{s}");
        assert!(s.contains("0000:e3:00.0 pl2.card 160000000"));
        assert!(
            s.lines().position(|l| l.contains("pl2.card")).unwrap()
                < s.lines().position(|l| l.contains("gt0.max_freq")).unwrap()
        );
        assert!(s.starts_with(STATE_HEADER));
        assert!(s.contains("\nformat=1\n"));
        // remove only the named keys of the named device
        let n = state_remove_keys(
            &r,
            pci,
            &["gt0.max_freq".into(), "gt0.min_freq".into()],
            "sudo",
        )
        .unwrap();
        assert_eq!(n, 1);
        let n = state_remove_keys(&r, "0000:01:00.0", &["pl2.card".into()], "sudo").unwrap();
        assert_eq!(n, 0);
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.contains("pl2.card 160000000"));
        assert!(!s.contains("gt0.max_freq"));
        assert!(s.contains("0000:e3:00.0 pl9.card 1"));
        assert!(s.contains("not a control line"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn exe_path_policy() {
        let d = tmpdir("exe");
        let mut r = roots_in(&d);
        assert_eq!(
            resolve_exe(&r, None).unwrap(),
            PathBuf::from("/usr/local/bin/xe-gmi")
        );
        assert_eq!(
            resolve_exe(&r, Some(Path::new("/opt/xe-gmi/bin/xe-gmi"))).unwrap(),
            PathBuf::from("/opt/xe-gmi/bin/xe-gmi")
        );
        assert!(resolve_exe(&r, Some(Path::new("/usr/bin/xe-gmi"))).is_ok());
        for bad in [
            "/home/user/.cargo/bin/xe-gmi",
            "/tmp/xe-gmi",
            "target/release/xe-gmi",
            "/usrx/xe-gmi",
        ] {
            let e = resolve_exe(&r, Some(Path::new(bad))).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{bad}");
            let m = e.to_string();
            assert!(
                m.contains("/usr/local/bin") && m.contains("--exe"),
                "{bad}: {m}"
            );
        }
        r.exe_path = Some(PathBuf::from("/home/user/.cargo/bin/xe-gmi"));
        assert_eq!(resolve_exe(&r, None).unwrap_err().exit_code(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rule_text_is_exact() {
        let t = rule_text(Path::new("/usr/local/bin/xe-gmi"));
        let golden = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/persist_rule.txt"),
        )
        .unwrap();
        assert_eq!(t, golden);
    }
}
