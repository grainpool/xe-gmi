//! Two-sample collection and rate computation.

use crate::avail::{Avail, Reason};
use crate::paths::Roots;
use crate::probe::Device;
use crate::proc_scan::{self, ClientSample, Scan};
use crate::sysfs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// One snapshot: hwmon energy, per-GT idle residency, and the visible clients.
#[derive(Debug)]
pub struct Sample {
    pub t_ms: u64,
    pub energy_card: Avail<u64>,
    pub energy_pkg: Avail<u64>,
    pub idle_ms: Vec<Avail<u64>>, // per GT, device order
    pub clients: Vec<ClientSample>,
}

#[derive(Debug, Clone)]
pub struct EnginePct {
    pub rcs: Avail<f64>,
    pub ccs: Avail<f64>,
    pub vcs: Avail<f64>,
    pub vecs: Avail<f64>,
    pub bcs: Avail<f64>,
}

/// Engine rates for one client (index into proc_scan::CLASSES).
#[derive(Debug)]
pub struct ClientRates {
    pub client: ClientSample,
    pub pct: [Avail<f64>; 5],
}

#[derive(Debug)]
pub struct Rates {
    pub draw_card_w: Avail<f64>,
    /// energy*_input of the latest sample.
    pub energy_card: Avail<u64>,
    pub energy_pkg: Avail<u64>,
    pub draw_pkg_w: Avail<f64>,
    pub gt_util_pct: Vec<Avail<f64>>,
    pub engine_pct: EnginePct,
    pub per_client: Vec<ClientRates>,
    pub dt_ms: u64,
}

/// `second = true` reads the T1 fixture roots when the seam is set.
pub fn take(roots: &Roots, dev: &Device, second: bool) -> Sample {
    let (sysfs_root, procfs_root) = if second {
        (
            roots
                .sysfs_t1
                .clone()
                .unwrap_or_else(|| roots.sysfs.clone()),
            roots
                .procfs_t1
                .clone()
                .unwrap_or_else(|| roots.procfs.clone()),
        )
    } else {
        (roots.sysfs.clone(), roots.procfs.clone())
    };
    let t_ms = match (roots.fixture_mode(), second, roots.fixture_dt_ms) {
        (true, true, Some(dt)) => dt,
        (true, false, Some(_)) => 0,
        _ => started().elapsed().as_millis() as u64,
    };
    let dev_dir = map_under(&roots.sysfs, &sysfs_root, &dev.dev_dir);
    let (energy_card, energy_pkg, _idle_skip) = match &dev.hwmon {
        Some(hw) => {
            let mapped = map_under(&roots.sysfs, &sysfs_root, &hw.dir);
            (
                sysfs::read_u64(&mapped.join("energy1_input")),
                sysfs::read_u64(&mapped.join("energy2_input")),
                Vec::<Avail<u64>>::new(),
            )
        }
        None => (
            Avail::NotAvailable(Reason::Missing(dev_dir.join("hwmon"))),
            Avail::NotAvailable(Reason::Missing(dev_dir.join("hwmon"))),
            Vec::<Avail<u64>>::new(),
        ),
    };
    let idle_ms = dev
        .gts
        .iter()
        .map(|g| {
            let gdir = map_under(&roots.sysfs, &sysfs_root, &g.dir);
            sysfs::read_u64(&gdir.join("gtidle/idle_residency_ms"))
        })
        .collect();
    let scan_roots = Roots {
        procfs: procfs_root,
        ..roots.clone()
    };
    let scan = proc_scan::scan(&scan_roots, &[dev.pci.as_str()]);
    Sample {
        t_ms,
        energy_card,
        energy_pkg,
        idle_ms,
        clients: scan.clients,
    }
}

fn na_f64() -> Avail<f64> {
    Avail::NotAvailable(Reason::FirstSample)
}

fn started() -> std::time::Instant {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Re-root `path` (known to live under `from`) at `to`.
fn map_under(from: &Path, to: &Path, path: &Path) -> PathBuf {
    match path.strip_prefix(from) {
        Ok(rel) => to.join(rel),
        Err(_) => path.to_path_buf(),
    }
}

/// Scan for `processes`/`memory.used` against the sample B roots.
pub fn scan_of(roots: &Roots, devs: &[&Device], second: bool) -> Scan {
    let procfs = if second {
        roots
            .procfs_t1
            .clone()
            .unwrap_or_else(|| roots.procfs.clone())
    } else {
        roots.procfs.clone()
    };
    let owned: Vec<&str> = devs.iter().map(|d| d.pci.as_str()).collect();
    proc_scan::scan(
        &Roots {
            procfs,
            ..roots.clone()
        },
        &owned,
    )
}

fn power_w(a: &Avail<u64>, b: &Avail<u64>, dt_ms: u64) -> Avail<f64> {
    match (a, b) {
        (Avail::Value(x), Avail::Value(y)) => {
            if dt_ms < 1 {
                return Avail::NotAvailable(Reason::FirstSample);
            }
            if y < x {
                // counter reset between samples
                return Avail::NotAvailable(Reason::Unparseable(
                    PathBuf::from("energy_input"),
                    format!("{x} -> {y}"),
                ));
            }
            Avail::Value((y - x) as f64 / dt_ms as f64 / 1000.0)
        }
        (Avail::NotAvailable(r), _) | (_, Avail::NotAvailable(r)) => Avail::NotAvailable(r.clone()),
    }
}

fn util_pct(a: &Avail<u64>, b: &Avail<u64>, dt_ms: u64) -> Avail<f64> {
    match (a, b) {
        (Avail::Value(x), Avail::Value(y)) => {
            if dt_ms < 1 || y < x {
                return Avail::NotAvailable(Reason::FirstSample);
            }
            let idle = (y - x) as f64 / dt_ms as f64 * 100.0;
            Avail::Value((100.0 - idle).clamp(0.0, 100.0))
        }
        (Avail::NotAvailable(r), _) | (_, Avail::NotAvailable(r)) => Avail::NotAvailable(r.clone()),
    }
}

fn class_pct(a: &ClientSample, b: &ClientSample, i: usize) -> Avail<f64> {
    // Client appeared during the window → N/A.
    let (Some(ca), Some(cb)) = (a.classes[i], b.classes[i]) else {
        return Avail::NotAvailable(Reason::FirstSample);
    };
    match (ca.total, cb.total) {
        (Some(ta), Some(tb)) if tb > ta => {
            let cycles = cb.cycles.saturating_sub(ca.cycles) as f64;
            Avail::Value(
                (100.0 * cycles / (tb - ta) as f64 / cb.capacity.max(1) as f64).clamp(0.0, 100.0),
            )
        }
        _ => Avail::NotAvailable(Reason::NotSupported(
            "drm-total-cycles absent for this class on this kernel",
        )),
    }
}

/// Rates from two samples of one device. `first_frame` = single-shot frame with no sample B.
pub fn rates(a: &Sample, b: &Sample, first_frame: bool) -> Rates {
    let dt_ms = b.t_ms.saturating_sub(a.t_ms);
    if first_frame {
        let na = Avail::NotAvailable(Reason::FirstSample);
        return Rates {
            energy_card: b.energy_card.clone(),
            energy_pkg: b.energy_pkg.clone(),
            draw_card_w: na.clone(),
            draw_pkg_w: na.clone(),
            gt_util_pct: a.idle_ms.iter().map(|_| na.clone()).collect(),
            engine_pct: EnginePct {
                rcs: na.clone(),
                ccs: na.clone(),
                vcs: na.clone(),
                vecs: na.clone(),
                bcs: na,
            },
            per_client: b
                .clients
                .iter()
                .map(|c| ClientRates {
                    client: c.clone(),
                    pct: std::array::from_fn(|_| na_f64()),
                })
                .collect(),
            dt_ms: 0,
        };
    }
    let mut gt_util_pct = Vec::new();
    for i in 0..a.idle_ms.len().min(b.idle_ms.len()) {
        gt_util_pct.push(util_pct(&a.idle_ms[i], &b.idle_ms[i], dt_ms));
    }
    // Device engine sums over its clients.
    let mut sums = [0.0f64; 5];
    let mut per_client = Vec::new();
    for cb in &b.clients {
        let mut pct = std::array::from_fn(|_| na_f64());
        for i in 0..5 {
            pct[i] = match a
                .clients
                .iter()
                .find(|ca| ca.client_id == cb.client_id && ca.pdev == cb.pdev)
            {
                Some(ca) => class_pct(ca, cb, i),
                None => Avail::NotAvailable(Reason::FirstSample),
            };
            if let Some(v) = pct[i].value() {
                sums[i] += *v;
            }
        }
        per_client.push(ClientRates {
            client: cb.clone(),
            pct,
        });
    }
    let summed = |i: usize| Avail::Value(sums[i].clamp(0.0, 100.0));
    Rates {
        energy_card: b.energy_card.clone(),
        energy_pkg: b.energy_pkg.clone(),
        draw_card_w: power_w(&a.energy_card, &b.energy_card, dt_ms),
        draw_pkg_w: power_w(&a.energy_pkg, &b.energy_pkg, dt_ms),
        gt_util_pct,
        engine_pct: EnginePct {
            rcs: summed(0),
            ccs: summed(1),
            vcs: summed(2),
            vecs: summed(3),
            bcs: summed(4),
        },
        per_client,
        dt_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(energy: u64, idle: u64, t_ms: u64) -> Sample {
        Sample {
            t_ms,
            energy_card: Avail::Value(energy),
            energy_pkg: Avail::NotAvailable(Reason::Missing("energy2_input".into())),
            idle_ms: vec![Avail::Value(idle)],
            clients: Vec::new(),
        }
    }

    #[test]
    fn rates_math() {
        // 24_800_000 µJ over 2000 ms → 12.40 W.
        let r = rates(
            &sample(100_000_000, 1000, 0),
            &sample(124_800_000, 2900, 2000),
            false,
        );
        assert_eq!(r.draw_card_w.value(), Some(&12.40));
        assert_eq!(r.dt_ms, 2000);
        // idle Δ1900/2000 → 5.0 % utilization.
        assert_eq!(r.gt_util_pct[0].value(), Some(&5.0));
        // negative Δenergy → N/A.
        let r = rates(&sample(200, 0, 0), &sample(100, 0, 2000), false);
        assert!(r.draw_card_w.value().is_none());
        // cycles 400k/2M/cap1 → 20.0 % on rcs.
        let mk = |cycles: u64| {
            let mut classes: [Option<proc_scan::ClassCounters>; 5] = [None; 5];
            classes[0] = Some(proc_scan::ClassCounters {
                cycles,
                total: Some(0),
                capacity: 1,
            });
            ClientSample {
                pid: 1,
                name: "x".into(),
                pdev: "0000:e3:00.0".into(),
                client_id: 1,
                resident_vram: 0,
                total_vram: None,
                active_vram: None,
                classes,
                pids: vec![1],
                uid: 0,
                cgroup: None,
                regions: Vec::new(),
            }
        };
        let mut a = sample(0, 0, 0);
        a.clients = vec![mk(1_000_000)];
        a.clients[0].classes[0] = Some(proc_scan::ClassCounters {
            cycles: 1_000_000,
            total: Some(5_000_000),
            capacity: 1,
        });
        let mut b = sample(0, 0, 2000);
        b.clients = vec![mk(1_400_000)];
        b.clients[0].classes[0] = Some(proc_scan::ClassCounters {
            cycles: 1_400_000,
            total: Some(7_000_000),
            capacity: 1,
        });
        let r = rates(&a, &b, false);
        assert_eq!(r.per_client[0].pct[0].value(), Some(&20.0));
        assert_eq!(r.engine_pct.rcs.value(), Some(&20.0));
        // first frame → every rate is N/A
        let r = rates(&b, &b, true);
        assert!(
            r.draw_card_w.value().is_none()
                && r.gt_util_pct[0].value().is_none()
                && r.engine_pct.rcs.value().is_none()
        );
    }
}
