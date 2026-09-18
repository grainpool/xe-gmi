//! The kernel-interface module: the only code that goes beyond plain file reads.
//!
//! Three transports, one dispatch: DRM device queries (`ioctl.rs`, the single kernel-boundary
//! implementation in this crate), the drm-ras generic-netlink client (`genl.rs`) and the kernel
//! uevent stream (`uevent.rs`). `replay.rs` is the file-backed fake behind `XE_GMI_KABI_REPLAY`
//! that the tests use; every public entry point below dispatches on the backend so callers never
//! branch on it themselves.

pub mod genl;
pub mod ioctl;
pub mod parse;
pub mod replay;
pub mod uevent;

use std::path::{Path, PathBuf};

use crate::avail::{Avail, Reason};
use crate::error::Error;
use crate::paths::Roots;

pub use genl::{RasCounter, RasNode};

pub use genl::RasSnapshot;
pub use uevent::Uevent;

/// Kernel-managed memory region as reported by the MEM_REGIONS query.
#[derive(Debug, Clone, PartialEq)]
pub struct MemRegion {
    pub class: u16,
    pub instance: u16,
    pub min_page_size: u32,
    pub total: u64,
    pub used: u64,
    pub cpu_visible: u64,
    pub cpu_visible_used: u64,
}

/// One engine as reported by the ENGINES query (vm-bind engines are filtered out at parse time).
#[derive(Debug, Clone, PartialEq)]
pub struct Engine {
    pub class: u16,
    pub instance: u16,
    pub gt_id: u16,
}

/// One GT as reported by the GT_LIST query.
#[derive(Debug, Clone, PartialEq)]
pub struct GtInfo {
    pub kind: u16,
    pub tile: u16,
    pub gt: u16,
    pub reference_clock: u32,
    pub near_mem: u64,
    pub far_mem: u64,
    pub ip: (u16, u16, u16),
}

/// One hardware-config mask as reported by the GT_TOPOLOGY query (count = popcount).
#[derive(Debug, Clone, PartialEq)]
pub struct TopologyMask {
    pub gt: u16,
    pub kind: u16,
    pub mask: Vec<u8>,
}

/// A microcontroller firmware version (UC_FW_VERSION query).
#[derive(Debug, Clone, PartialEq)]
pub struct UcFw {
    pub uc_type: u16,
    pub branch: u32,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// Device configuration from the CONFIG query.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub device_id: u16,
    pub revision: u8,
    pub flags: u64,
    pub min_alignment: u64,
    pub va_bits: u64,
    pub max_prio: u64,
}

pub const UC_GUC: u16 = 0;
pub const UC_HUC: u16 = 1;

#[derive(Debug, Clone)]
pub enum Backend {
    Real,
    Replay(PathBuf),
}

pub fn backend(roots: &Roots) -> Backend {
    match &roots.kabi_replay {
        Some(dir) => Backend::Replay(dir.clone()),
        None => Backend::Real,
    }
}

/// An open handle to one device's kernel interface: a render-node fd, or its replay directory.
/// `rpm_error` is the device's runtime-PM status at open time; it gives the otherwise ambiguous
/// `EINVAL` from the query ioctl a second possible meaning (refusal during a broken power state).
pub enum Handle {
    Real { fd: std::fs::File, rpm_error: bool },
    Replay { dir: PathBuf },
}

/// Opens the render node (falling back to the card node). Unavailability is a value-level `N/A`,
/// never an error: a device without kabi access keeps every other feature working.
pub fn open_device(
    roots: &Roots,
    pci: &str,
    card: &str,
    render: Option<&str>,
    rpm_error: bool,
) -> Avail<Handle> {
    match backend(roots) {
        Backend::Replay(root) => {
            let dir = root.join("drm-query").join(pci);
            if dir.is_dir() {
                Avail::Value(Handle::Replay { dir })
            } else {
                Avail::NotAvailable(Reason::Detail(format!(
                    "render node not accessible (/dev/dri/{pci}: no replay data)"
                )))
            }
        }
        Backend::Real => {
            let name = render.unwrap_or(card);
            let node = PathBuf::from("/dev/dri").join(name);
            // Fixture runs never touch the machine's real render nodes (they could belong to an
            // unrelated GPU); only the explicit replay seam answers queries there.
            if roots.fixture_mode() {
                return Avail::NotAvailable(Reason::Detail(format!(
                    "render node not accessible ({}: No such file or directory (os error 2))",
                    node.display()
                )));
            }
            let mut opts = std::fs::OpenOptions::new();
            opts.read(true).write(true);
            std::os::unix::fs::OpenOptionsExt::custom_flags(&mut opts, 0o2000000); // O_CLOEXEC
            match opts.open(&node) {
                Ok(fd) => Avail::Value(Handle::Real { fd, rpm_error }),
                Err(e) => Avail::NotAvailable(Reason::Detail(format!(
                    "render node not accessible ({}: {e})",
                    node.display()
                ))),
            }
        }
    }
}

/// Every device query, taken once at discovery. A device without kernel-interface access keeps
/// all these as `NotAvailable` with the open failure as the shared reason; everything else the
/// tool reports is untouched.
#[derive(Debug, Clone)]
pub struct Kabi {
    pub mem: Avail<Vec<MemRegion>>,
    pub engines: Avail<Vec<Engine>>,
    pub gt_list: Avail<Vec<GtInfo>>,
    pub gt_topology: Avail<Vec<TopologyMask>>,
    pub guc: Avail<UcFw>,
    pub huc: Avail<UcFw>,
    pub config: Avail<Config>,
    /// RAS nodes with counters, filtered to this device (empty where none are registered).
    pub ras: Avail<RasSnapshot>,
}

impl Default for Kabi {
    fn default() -> Self {
        fn na<T>() -> Avail<T> {
            Avail::NotAvailable(Reason::Detail("render node not open".into()))
        }
        Kabi {
            mem: na(),
            engines: na(),
            gt_list: na(),
            gt_topology: na(),
            guc: na(),
            huc: na(),
            config: na(),
            ras: na(),
        }
    }
}

impl Kabi {
    /// `rpm_error`: the device's `power/runtime_status` reading `error` at open time — see
    /// `Handle`; it disambiguates a query `EINVAL`.
    pub fn read(
        roots: &Roots,
        pci: &str,
        card: &str,
        render: Option<&str>,
        rpm_error: bool,
    ) -> Kabi {
        let h = match open_device(roots, pci, card, render, rpm_error) {
            Avail::Value(h) => h,
            Avail::NotAvailable(r) => {
                fn na<T>(r: &Reason) -> Avail<T> {
                    Avail::NotAvailable(r.clone())
                }
                return Kabi {
                    mem: na(&r),
                    engines: na(&r),
                    gt_list: na(&r),
                    gt_topology: na(&r),
                    guc: na(&r),
                    huc: na(&r),
                    config: na(&r),
                    ras: na(&r),
                };
            }
        };
        Kabi {
            mem: mem_regions(&h),
            engines: engines(&h),
            gt_list: gt_list(&h),
            gt_topology: gt_topology(&h),
            guc: uc_fw(&h, UC_GUC),
            huc: uc_fw(&h, UC_HUC),
            config: config(&h),
            ras: match ras_nodes(roots) {
                Ok(nodes) => {
                    let mine: Vec<RasNode> =
                        nodes.into_iter().filter(|n| n.device == pci).collect();
                    let mut out = Vec::new();
                    let mut failed: Option<Error> = None;
                    for n in mine {
                        match ras_counters(roots, n.id) {
                            Ok(cs) => out.push((n, cs)),
                            Err(e) => failed = Some(e),
                        }
                    }
                    match failed {
                        Some(_) => {
                            Avail::NotAvailable(Reason::Detail("ras counters unreadable".into()))
                        }
                        None => Avail::Value(out),
                    }
                }
                Err(Error::Unavailable(m)) => Avail::NotAvailable(Reason::Detail(m)),
                Err(Error::PermissionDenied { .. }) => Avail::NotAvailable(Reason::Detail(
                    "admin permission required for RAS counters".into(),
                )),
                Err(_) => Avail::NotAvailable(Reason::Detail("ras counters unreadable".into())),
            },
        }
    }

    /// The kernel-managed VRAM region, if the MEM_REGIONS query answered.
    pub fn vram(&self) -> Option<&MemRegion> {
        self.mem
            .value()
            .and_then(|rs| rs.iter().find(|r| r.class == 1))
    }
}

fn query(h: &Handle, name: &str, query: u32, prefill: &[u8]) -> Avail<Vec<u8>> {
    match h {
        Handle::Replay { dir } => match std::fs::read(dir.join(name)) {
            Ok(bytes) => Avail::Value(bytes),
            // Absent replay file = the captured kernel did not answer this query.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                unsupported("query not supported by this kernel")
            }
            Err(e) => Avail::NotAvailable(Reason::Detail(format!(
                "query fixture unreadable ({}: {e})",
                dir.join(name).display()
            ))),
        },
        Handle::Real { fd, rpm_error } => match ioctl::query(fd, query, prefill) {
            Ok(bytes) => Avail::Value(bytes),
            // The kernel answers EINVAL both for a query type it does not implement and for an
            // ioctl refused while the device runtime power state is in error — only the
            // runtime_status read tells the two apart, so name both readings when it does.
            Err(e) if e == rustix::io::Errno::INVAL && *rpm_error => Avail::NotAvailable(
                Reason::Detail("query failed: Invalid argument; the device runtime power state \
                                reports error, so this is a refused query, not a missing kernel feature"
                    .into()),
            ),
            Err(e) if e == rustix::io::Errno::INVAL => {
                unsupported("query not supported by this kernel")
            }
            Err(e) => Avail::NotAvailable(Reason::Detail(format!("query failed: {e}"))),
        },
    }
}

fn unsupported<T>(why: &'static str) -> Avail<T> {
    Avail::NotAvailable(Reason::NotSupported(why))
}

pub fn mem_regions(h: &Handle) -> Avail<Vec<MemRegion>> {
    query(h, "mem_regions.bin", parse::Q_MEM_REGIONS, &[]).map(|b| parse::mem_regions(&b))
}

pub fn engines(h: &Handle) -> Avail<Vec<Engine>> {
    query(h, "engines.bin", parse::Q_ENGINES, &[]).map(|b| parse::engines(&b))
}

pub fn gt_list(h: &Handle) -> Avail<Vec<GtInfo>> {
    query(h, "gt_list.bin", parse::Q_GT_LIST, &[]).map(|b| parse::gt_list(&b))
}

pub fn gt_topology(h: &Handle) -> Avail<Vec<TopologyMask>> {
    query(h, "gt_topology.bin", parse::Q_GT_TOPOLOGY, &[]).map(|b| parse::gt_topology(&b))
}

pub fn config(h: &Handle) -> Avail<Config> {
    query(h, "config.bin", parse::Q_CONFIG, &[]).map(|b| parse::config(&b))
}

pub fn uc_fw(h: &Handle, uc_type: u16) -> Avail<UcFw> {
    let prefill = uc_type.to_le_bytes();
    query(
        h,
        &format!("uc_fw_version_{uc_type}.bin"),
        parse::Q_UC_FW_VERSION,
        &prefill,
    )
    .map(|b| parse::uc_fw(&b))
    // A microcontroller that was never loaded answers with all-zero version fields, not an
    // error: zero is "absent", and 0.0.0 is not a firmware version anyone should be told.
    .and_then(|u| {
        if u.major == 0 && u.minor == 0 && u.patch == 0 && u.branch == 0 {
            Avail::NotAvailable(Reason::Detail(
                "the kernel reports a zero version: this microcontroller is not loaded".into(),
            ))
        } else {
            Avail::Value(u)
        }
    })
}

/// RAS nodes and counters. The replay tree's `ras/` directory stands in for the netlink family:
/// absent means the family is not present in this kernel.
pub fn ras_nodes(roots: &Roots) -> Result<Vec<RasNode>, Error> {
    match backend(roots) {
        Backend::Replay(root) => replay::ras_nodes(&root),
        // fixture runs never talk to the machine's netlink sockets either
        Backend::Real if roots.fixture_mode() => Err(Error::Unavailable(
            "drm-ras netlink family not present (kernel 7.2+ with xe RAS support)".into(),
        )),
        Backend::Real => genl::ras_nodes(),
    }
}

pub fn ras_counters(roots: &Roots, node: u32) -> Result<Vec<RasCounter>, Error> {
    match backend(roots) {
        Backend::Replay(root) => replay::ras_counters(&root, node),
        Backend::Real => genl::ras_counters(node),
    }
}

pub fn ras_clear(roots: &Roots, node: u32, error: u32) -> Result<(), Error> {
    match backend(roots) {
        Backend::Replay(root) => replay::ras_clear(&root, node, error),
        Backend::Real => genl::ras_clear(node, error),
    }
}

pub fn uevent_stream(roots: &Roots) -> Result<Box<dyn Iterator<Item = Uevent>>, Error> {
    match backend(roots) {
        Backend::Replay(root) => Ok(replay::uevent_stream(&root)),
        // the live kernel group needs a capability fixture runs do not have
        Backend::Real if roots.fixture_mode() => Err(Error::PermissionDenied {
            path: PathBuf::from("/run/xe-gmi/uevent"),
            hint: "sudo xe-gmi events".into(),
        }),
        Backend::Real => uevent::live_stream(),
    }
}

pub fn popcount(mask: &[u8]) -> u32 {
    mask.iter().map(|b| b.count_ones()).sum()
}

pub(crate) fn dir_exists(p: &Path) -> bool {
    p.is_dir()
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_unsafe_outside_kabi() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![];
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
        walk(&src, &mut files);
        assert!(files.len() > 10, "unexpectedly few source files");
        let mut offenders = vec![];
        for f in files {
            let rel = f.strip_prefix(&src).unwrap();
            if rel.starts_with("kabi") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap();
            for (i, line) in text.lines().enumerate() {
                let l = line.trim_start();
                if l.starts_with("//") {
                    continue;
                }
                if l.contains("unsafe")
                    && !l.contains("deny(unsafe_code)")
                    && !l.contains("forbid(unsafe_code)")
                {
                    offenders.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "unsafe outside src/kabi:\n{}",
            offenders.join("\n")
        );
        let kabi = src.join("kabi");
        assert!(
            kabi.join("ioctl.rs").exists(),
            "src/kabi/ioctl.rs must exist"
        );
    }
}
