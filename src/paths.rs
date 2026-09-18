//! Filesystem roots and the environment seams.
//!
//! Production defaults are `/sys`, `/proc`, `/lib/modules`, `/etc`, `/var/lib/xe-gmi`; every seam
//! exists so the black-box tests can run the real binary against a fixture tree.

use std::env;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Roots {
    pub sysfs: PathBuf,
    pub procfs: PathBuf,
    pub modules: PathBuf,
    pub etc: PathBuf,
    pub state: PathBuf,
    // Fixture seams. An `Option` distinguishes unset from set-but-empty:
    // `XE_GMI_PCIIDS=""` disables lookup, `XE_GMI_UDEVADM=""` skips the reload.
    pub sysfs_t1: Option<PathBuf>,
    pub procfs_t1: Option<PathBuf>,
    pub fixture_dt_ms: Option<u64>,
    pub pciids: Option<PathBuf>,
    pub udevadm: Option<PathBuf>,
    pub exe_path: Option<PathBuf>,
    pub cgroup: PathBuf,
    pub kmsg: Option<PathBuf>,
    pub kabi_replay: Option<PathBuf>,
}

fn env_path(key: &str, default: &str) -> PathBuf {
    env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

/// Set-but-empty stays `Some("")` so callers can honor "empty string disables/skips".
fn env_opt(key: &str) -> Option<PathBuf> {
    env::var_os(key).map(PathBuf::from)
}

impl Roots {
    pub fn from_env() -> Roots {
        Roots {
            sysfs: env_path("XE_GMI_SYSFS_ROOT", "/sys"),
            procfs: env_path("XE_GMI_PROCFS_ROOT", "/proc"),
            modules: env_path("XE_GMI_MODULES_ROOT", "/lib/modules"),
            etc: env_path("XE_GMI_ETC_ROOT", "/etc"),
            state: env_path("XE_GMI_STATE_ROOT", "/var/lib/xe-gmi"),
            sysfs_t1: env_opt("XE_GMI_SYSFS_ROOT_T1"),
            procfs_t1: env_opt("XE_GMI_PROCFS_ROOT_T1"),
            fixture_dt_ms: env::var("XE_GMI_FIXTURE_DT_MS")
                .ok()
                .and_then(|s| s.parse().ok()),
            pciids: env_opt("XE_GMI_PCIIDS"),
            udevadm: env_opt("XE_GMI_UDEVADM"),
            exe_path: env_opt("XE_GMI_EXE_PATH"),
            cgroup: env_path("XE_GMI_CGROUP_ROOT", "/sys/fs/cgroup"),
            kmsg: env_opt("XE_GMI_KMSG").filter(|p| !p.as_os_str().is_empty()),
            kabi_replay: env_opt("XE_GMI_KABI_REPLAY").filter(|p| !p.as_os_str().is_empty()),
        }
    }

    /// Readback sidecars and other fixture-only behaviours are active iff the sysfs seam is set.
    pub fn fixture_mode(&self) -> bool {
        env::var_os("XE_GMI_SYSFS_ROOT").is_some()
    }
}

/// Fixed clock for golden tests (`XE_GMI_NOW`), else wall clock.
pub fn now_epoch() -> u64 {
    match env::var("XE_GMI_NOW").ok().and_then(|s| s.parse().ok()) {
        Some(v) => v,
        None => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    }
}

/// `Uid:` first field of `$PROCFS/self/status`, else `/proc/self/status`, else unknown.
pub fn uid(roots: &Roots) -> u32 {
    [
        roots.procfs.join("self/status").as_path(),
        Path::new("/proc/self/status"),
    ]
    .into_iter()
    .find_map(read_uid_line)
    .unwrap_or(u32::MAX)
}

fn read_uid_line(path: &Path) -> Option<u32> {
    let status = std::fs::read_to_string(path).ok()?;
    status.lines().find_map(|l| {
        l.strip_prefix("Uid:")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|f| f.parse().ok())
    })
}
