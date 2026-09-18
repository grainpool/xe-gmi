//! `sriov status`: PF state and per-VF provisioning from `sriov_admin/` (spec/23 §8, spec/22).

use crate::paths::Roots;
use crate::probe::Device;
use crate::sysfs;

pub struct VfStatus {
    pub index: u64,
    pub vram_quota: u64,
    pub exec_quantum_ms: u64,
    pub preempt_timeout_us: u64,
    pub sched_priority: String,
    pub pci_address: Option<String>,
    pub driver: Option<String>,
}

pub struct Status {
    pub total_vfs: u64,
    pub num_vfs: u64,
    pub autoprobe: bool,
    pub pf_quantum_ms: u64,
    pub pf_preempt_us: u64,
    pub pf_priority: String,
    pub vfs: Vec<VfStatus>,
}

fn num(p: &std::path::Path, default: u64) -> u64 {
    sysfs::read_u64(p).value().copied().unwrap_or(default)
}

/// `[low] normal high` → the bracketed (current) token; bare text passes through; absent → `low`.
pub fn current_priority(text: &str) -> String {
    let mut first = None;
    for tok in text.split_whitespace() {
        if tok.starts_with('[') && tok.ends_with(']') && tok.len() > 2 {
            return tok[1..tok.len() - 1].to_string();
        }
        first.get_or_insert_with(|| tok.to_string());
    }
    first.unwrap_or_else(|| "low".into())
}

pub fn status(dev: &Device) -> Option<Status> {
    let pf = dev.placement.sriov.as_ref()?;
    let admin = dev.dev_dir.join("sriov_admin");
    let prof = |dir: &str, name: &str| admin.join(dir).join("profile").join(name);
    let pf_priority = sysfs::read_string(&prof("pf", "sched_priority"))
        .value()
        .map(|s| current_priority(s))
        .unwrap_or_else(|| "low".into());
    let total = pf.total.value().copied().unwrap_or(0);
    let mut vfs = Vec::new();
    for n in 1..=total {
        let dir = format!("vf{n}");
        let device_link = admin.join(&dir).join("device");
        let vf_addr = sysfs::link_basename(&device_link).value().cloned();
        let driver = vf_addr
            .as_ref()
            .and_then(|a| pf.vfs.iter().find(|v| v.addr == *a))
            .and_then(|v| v.driver.value().cloned());
        vfs.push(VfStatus {
            index: n,
            vram_quota: num(&prof(&dir, "vram_quota"), 0),
            exec_quantum_ms: num(&prof(&dir, "exec_quantum_ms"), 0),
            preempt_timeout_us: num(&prof(&dir, "preempt_timeout_us"), 0),
            sched_priority: sysfs::read_string(&prof(&dir, "sched_priority"))
                .value()
                .map(|s| current_priority(s))
                .unwrap_or_else(|| "low".into()),
            pci_address: vf_addr,
            driver,
        });
    }
    Some(Status {
        total_vfs: total,
        num_vfs: pf.num.value().copied().unwrap_or(0),
        autoprobe: pf.autoprobe.value().copied().unwrap_or(0) != 0,
        pf_quantum_ms: num(&prof("pf", "exec_quantum_ms"), 0),
        pf_preempt_us: num(&prof("pf", "preempt_timeout_us"), 0),
        pf_priority,
        vfs,
    })
}

pub fn text(devs: &[&Device]) -> String {
    let mut out = String::new();
    for d in devs {
        out.push_str(&format!("GPU {} [{}]\n", d.index, d.pci));
        match status(d) {
            Some(s) => {
                let kv = |out: &mut String, label: &str, value: &str| {
                    out.push_str(&format!("  {:<25}: {}\n", label, value))
                };
                kv(&mut out, "Total VFs", &s.total_vfs.to_string());
                kv(&mut out, "Enabled VFs", &s.num_vfs.to_string());
                kv(
                    &mut out,
                    "Autoprobe",
                    if s.autoprobe { "yes" } else { "no" },
                );
                kv(
                    &mut out,
                    "PF profile",
                    &format!(
                        "quantum {} ms, preempt {} us, priority {}",
                        s.pf_quantum_ms, s.pf_preempt_us, s.pf_priority
                    ),
                );
                out.push_str(&format!(
                    "  {:<4}  {:<10}  {:<8}  {:<9}  {:<8}  DEVICE\n",
                    "VF", "VRAM QUOTA", "QUANTUM", "PREEMPT", "PRIORITY"
                ));
                for v in &s.vfs {
                    let device = match (&v.pci_address, &v.driver) {
                        (Some(a), Some(dr)) => format!("{a} ({dr})"),
                        (Some(a), None) => format!("{a} (no driver)"),
                        _ => "-".into(),
                    };
                    out.push_str(&format!(
                        "  {:<4}  {:<10}  {:<8}  {:<9}  {:<8}  {}\n",
                        format!("vf{}", v.index),
                        format!("{} MiB", v.vram_quota / (1 << 20)),
                        format!("{} ms", v.exec_quantum_ms),
                        format!("{} us", v.preempt_timeout_us),
                        v.sched_priority,
                        device
                    ));
                }
            }
            None => out.push_str("  N/A (no SR-IOV capability exposed for this device)\n"),
        }
    }
    out
}

pub fn json(devs: &[&Device], ts: &str) -> String {
    use crate::format::json::{num_u, Json};
    let arr = Json::Arr(
        devs.iter()
            .map(|d| {
                let sriov = match status(d) {
                    Some(s) => Json::Obj(vec![
                        ("total_vfs".into(), num_u(s.total_vfs)),
                        ("num_vfs".into(), num_u(s.num_vfs)),
                        ("autoprobe".into(), Json::Bool(s.autoprobe)),
                        (
                            "pf_profile".into(),
                            Json::Obj(vec![
                                ("exec_quantum_ms".into(), num_u(s.pf_quantum_ms)),
                                ("preempt_timeout_us".into(), num_u(s.pf_preempt_us)),
                                ("sched_priority".into(), Json::Str(s.pf_priority)),
                            ]),
                        ),
                        (
                            "vfs".into(),
                            Json::Arr(
                                s.vfs
                                    .iter()
                                    .map(|v| {
                                        Json::Obj(vec![
                                            ("index".into(), num_u(v.index)),
                                            ("vram_quota_bytes".into(), num_u(v.vram_quota)),
                                            ("exec_quantum_ms".into(), num_u(v.exec_quantum_ms)),
                                            (
                                                "preempt_timeout_us".into(),
                                                num_u(v.preempt_timeout_us),
                                            ),
                                            (
                                                "sched_priority".into(),
                                                Json::Str(v.sched_priority.clone()),
                                            ),
                                            (
                                                "pci_address".into(),
                                                v.pci_address
                                                    .clone()
                                                    .map(Json::Str)
                                                    .unwrap_or(Json::Null),
                                            ),
                                            (
                                                "driver".into(),
                                                v.driver
                                                    .clone()
                                                    .map(Json::Str)
                                                    .unwrap_or(Json::Null),
                                            ),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ]),
                    None => Json::Null,
                };
                Json::Obj(vec![
                    ("pci_address".into(), Json::Str(d.pci.clone())),
                    ("sriov".into(), sriov),
                ])
            })
            .collect(),
    );
    let root = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("generated_at".into(), Json::Str(ts.into())),
        ("devices".into(), arr),
    ]);
    let mut s = String::new();
    crate::format::json::write(&root, &mut s);
    s
}

/// Connector entries of the card belonging to this device (spec/23 §9).
pub struct Connector {
    pub name: String,
    pub status: String,
    pub enabled: bool,
    pub modes: usize,
}

pub fn connectors(roots: &Roots, dev: &Device) -> Vec<Connector> {
    let card = match dev.card.strip_prefix("card") {
        Some(c) => c,
        None => return Vec::new(),
    };
    let drm = roots.sysfs.join("class/drm");
    let Ok(rd) = std::fs::read_dir(&drm) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&format!("card{card}-")) else {
            continue;
        };
        let status = sysfs::read_string(&e.path().join("status"))
            .value()
            .cloned()
            .unwrap_or_else(|| "unknown".into());
        let enabled = sysfs::read_string(&e.path().join("enabled"))
            .value()
            .map(|s| s == "enabled")
            .unwrap_or(false);
        let modes = sysfs::read_string(&e.path().join("modes"))
            .value()
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        out.push(Connector {
            name: rest.to_string(),
            status,
            enabled,
            modes,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::current_priority;

    #[test]
    fn priority_brackets_parse() {
        assert_eq!(current_priority("[low] normal high"), "low");
        assert_eq!(current_priority("low normal [high]"), "high");
        assert_eq!(current_priority("normal"), "normal");
        assert_eq!(current_priority(""), "low");
        assert_eq!(current_priority("[normal]"), "normal");
    }
}
