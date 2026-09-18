//! JSON report/query/processes/fields/get builders, key order = schema property order
//! (schema/xe-gmi-v1.schema.json is normative).

use crate::avail::Avail;
use crate::format::csv::ParsedField;
use crate::format::fields::{self, ListRow, View};
use crate::format::json::{num_f, num_i, num_u, Json};
use crate::persist;
use crate::probe::gt::GtKind;
use crate::probe::{Device, VramSource};
use std::path::Path;

fn opt_u(o: Option<u64>) -> Json {
    o.map(num_u).unwrap_or(Json::Null)
}
fn opt_i(o: Option<i64>) -> Json {
    o.map(num_i).unwrap_or(Json::Null)
}
fn opt_s(o: Option<String>) -> Json {
    o.map(Json::Str).unwrap_or(Json::Null)
}
fn avail_u(a: &Avail<u64>) -> Json {
    opt_u(a.value().copied())
}
fn avail_s(a: &Avail<String>) -> Json {
    opt_s(a.value().cloned())
}
fn w(a: &Avail<f64>) -> Json {
    a.value().map(|v| num_f(*v, 2)).unwrap_or(Json::Null)
}
fn pct(a: &Avail<f64>) -> Json {
    a.value().map(|v| num_f(*v, 1)).unwrap_or(Json::Null)
}

fn report_json(
    roots: &crate::paths::Roots,
    kernel: &str,
    generated_at: &str,
    views: &[(&Device, View<'_>)],
    driver: &str,
) -> Json {
    let devices: Vec<Json> = views
        .iter()
        .map(|(d, v)| device_json(d, v, roots))
        .collect();
    let mut unavailable = Vec::new();
    for (_, v) in views {
        for d in fields::CATALOG {
            if let Avail::NotAvailable(r) = fields::resolve(d.name, None, v) {
                unavailable.push(Json::Obj(vec![
                    ("device".into(), Json::Str(v.dev.pci.clone())),
                    ("field".into(), Json::Str(d.name.to_string())),
                    ("reason".into(), Json::Str(r.to_string())),
                    ("source".into(), Json::Str(d.prov.source.to_string())),
                ]));
            }
        }
    }
    Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        (
            "tool".into(),
            Json::Obj(vec![
                ("name".into(), Json::Str("xe-gmi".into())),
                (
                    "version".into(),
                    Json::Str(env!("CARGO_PKG_VERSION").into()),
                ),
            ]),
        ),
        ("generated_at".into(), Json::Str(generated_at.into())),
        ("kernel".into(), Json::Str(kernel.into())),
        ("driver".into(), Json::Str(driver.into())),
        ("devices".into(), Json::Arr(devices)),
        ("unavailable".into(), Json::Arr(unavailable)),
    ])
}

pub fn status_json(
    roots: &crate::paths::Roots,
    kernel: &str,
    ts: &str,
    views: &[(&Device, View<'_>)],
) -> String {
    let mut s = String::new();
    crate::format::json::write(&report_json(roots, kernel, ts, views, "xe"), &mut s);
    s
}

pub fn device_json(dev: &Device, view: &View<'_>, roots: &crate::paths::Roots) -> Json {
    let link = Json::Obj(vec![
        (
            "gen_current".into(),
            avail_gen(dev.link.value().and_then(|l| l.gen_cur)),
        ),
        (
            "gen_max".into(),
            avail_gen(dev.link.value().and_then(|l| l.gen_max)),
        ),
        (
            "width_current".into(),
            opt_u(dev.link.value().and_then(|l| l.width_cur).map(|v| v as u64)),
        ),
        (
            "width_max".into(),
            opt_u(dev.link.value().and_then(|l| l.width_max).map(|v| v as u64)),
        ),
    ]);
    fn avail_gen(g: Option<u8>) -> Json {
        opt_u(g.map(|v| v as u64))
    }
    let ids = Json::Obj(vec![
        (
            "vendor_id".into(),
            Json::Str(format!("{:04x}", dev.ids.vendor)),
        ),
        (
            "device_id".into(),
            Json::Str(format!("{:04x}", dev.ids.device)),
        ),
        (
            "subsystem_vendor_id".into(),
            opt_s(dev.ids.subsystem.value().map(|(a, _)| format!("{a:04x}"))),
        ),
        (
            "subsystem_device_id".into(),
            opt_s(dev.ids.subsystem.value().map(|(_, b)| format!("{b:04x}"))),
        ),
        (
            "revision".into(),
            opt_s(dev.ids.revision.value().map(|r| format!("{r:02x}"))),
        ),
        ("link".into(), link),
    ]);
    let thermal: Vec<Json> = dev
        .hwmon
        .iter()
        .flat_map(|h| h.temps.iter())
        .map(|t| {
            Json::Obj(vec![
                ("label".into(), Json::Str(t.label.clone())),
                ("input_c".into(), num_u(fields::mc_to_c(t.input_mc))),
                ("max_c".into(), opt_u(t.max_mc.map(fields::mc_to_c))),
                ("crit_c".into(), opt_u(t.crit_mc.map(fields::mc_to_c))),
                (
                    "emergency_c".into(),
                    opt_u(t.emergency_mc.map(fields::mc_to_c)),
                ),
            ])
        })
        .collect();
    let h = dev.hwmon.as_ref();
    let eff = fields::effective_of(dev);
    let lim = Json::Obj(vec![
        (
            "effective_w".into(),
            eff.map(|(uw, _, _)| num_f(uw as f64 / 1e6, 2))
                .unwrap_or(Json::Null),
        ),
        ("kind".into(), opt_s(eff.map(|(_, k, _)| k.to_string()))),
        ("channel".into(), opt_s(eff.map(|(_, _, c)| c.to_string()))),
        ("pl1_w".into(), watt(h.map(|h| &h.card.pl1))),
        ("pl2_w".into(), watt(h.map(|h| &h.card.pl2))),
        ("pl1_pkg_w".into(), watt(h.map(|h| &h.pkg.pl1))),
        ("pl2_pkg_w".into(), watt(h.map(|h| &h.pkg.pl2))),
        (
            "pl1_window_ms".into(),
            h.map(|h| avail_u(&h.card.pl1_window_ms))
                .unwrap_or(Json::Null),
        ),
        (
            "pl2_window_ms".into(),
            h.map(|h| avail_u(&h.card.pl2_window_ms))
                .unwrap_or(Json::Null),
        ),
        ("crit_w".into(), watt(h.map(|h| &h.card.crit))),
        ("rated_max_w".into(), watt(h.map(|h| &h.card.rated_max))),
    ]);
    fn watt(a: Option<&Avail<u64>>) -> Json {
        a.and_then(|x| x.value())
            .map(|uw| num_f(*uw as f64 / 1e6, 2))
            .unwrap_or(Json::Null)
    }
    let power = Json::Obj(vec![
        (
            "draw_w".into(),
            view.rates.map(|r| w(&r.draw_card_w)).unwrap_or(Json::Null),
        ),
        (
            "draw_pkg_w".into(),
            view.rates.map(|r| w(&r.draw_pkg_w)).unwrap_or(Json::Null),
        ),
        ("sample_ms".into(), opt_u(view.rates.map(|r| r.dt_ms))),
        ("limit".into(), lim),
        (
            "energy_card_j".into(),
            h.map(|h| avail_u(&h.energy_card)).unwrap_or(Json::Null),
        ),
        (
            "energy_pkg_j".into(),
            h.map(|h| avail_u(&h.energy_pkg)).unwrap_or(Json::Null),
        ),
        (
            "voltage_pkg_mv".into(),
            opt_u(h.and_then(|h| h.voltage.first()).map(|(_, mv)| *mv)),
        ),
    ]);
    // energy joules are divided by 1e6 in JSON too (schema: number).
    let energy_j = |a: &Avail<u64>| -> Json {
        a.value()
            .map(|uj| num_f(*uj as f64 / 1e6, 3))
            .unwrap_or(Json::Null)
    };
    let e_card = match view.rates {
        Some(r) => r.energy_card.clone(),
        None => h
            .map(|x| x.energy_card.clone())
            .unwrap_or(Avail::NotAvailable(crate::avail::Reason::Missing(
                std::path::PathBuf::from("hwmon"),
            ))),
    };
    let e_pkg = match view.rates {
        Some(r) => r.energy_pkg.clone(),
        None => h
            .map(|x| x.energy_pkg.clone())
            .unwrap_or(Avail::NotAvailable(crate::avail::Reason::Missing(
                std::path::PathBuf::from("hwmon"),
            ))),
    };
    let power = match power {
        Json::Obj(mut v) => {
            for slot in v.iter_mut() {
                if slot.0 == "energy_card_j" {
                    slot.1 = energy_j(&e_card);
                } else if slot.0 == "energy_pkg_j" {
                    slot.1 = energy_j(&e_pkg);
                }
            }
            Json::Obj(v)
        }
        other => other,
    };
    let total = dev.vram_total.value();
    let memory = Json::Obj(vec![
        ("total_bytes".into(), opt_u(total.map(|t| t.bytes))),
        (
            "total_source".into(),
            opt_s(
                total
                    .map(|t| match t.source {
                        VramSource::Bar => "bar",
                        VramSource::Table => "table",
                    })
                    .map(str::to_string),
            ),
        ),
        ("used_bytes".into(), num_u(view.used_bytes)),
        ("used_clients".into(), num_u(view.used_clients as u64)),
        (
            "free_bytes".into(),
            opt_u(total.map(|t| t.bytes.saturating_sub(view.used_bytes))),
        ),
    ]);
    let gts: Vec<Json> = dev
        .gts
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let util = view.rates.and_then(|r| r.gt_util_pct.get(i));
            Json::Obj(vec![
                ("id".into(), num_u(g.id as u64)),
                ("tile".into(), num_u(g.tile as u64)),
                (
                    "kind".into(),
                    Json::Str(
                        match g.kind {
                            GtKind::Render => "render",
                            GtKind::Media => "media",
                            GtKind::Unknown => "unknown",
                        }
                        .into(),
                    ),
                ),
                ("name".into(), avail_s(&g.idle_name)),
                (
                    "clock_mhz".into(),
                    Json::Obj(vec![
                        ("cur".into(), opt_u(g.cur.value().map(|v| *v as u64))),
                        ("act".into(), opt_u(g.act.value().map(|v| *v as u64))),
                        ("min".into(), opt_u(g.min.value().map(|v| *v as u64))),
                        ("max".into(), opt_u(g.max.value().map(|v| *v as u64))),
                        ("rp0".into(), opt_u(g.rp0.value().map(|v| *v as u64))),
                        ("rpe".into(), opt_u(g.rpe.value().map(|v| *v as u64))),
                        ("rpn".into(), opt_u(g.rpn.value().map(|v| *v as u64))),
                        ("rpa".into(), opt_u(g.rpa.value().map(|v| *v as u64))),
                    ]),
                ),
                (
                    "utilization_pct".into(),
                    util.map(pct).unwrap_or(Json::Null),
                ),
                ("idle_status".into(), avail_s(&g.idle_status)),
                ("power_profile".into(), avail_s(&g.profile)),
                (
                    "throttle".into(),
                    Json::Obj(vec![
                        (
                            "active".into(),
                            g.throttle_status
                                .value()
                                .map(|a| Json::Bool(*a))
                                .unwrap_or(Json::Null),
                        ),
                        (
                            "reasons".into(),
                            Json::Arr(
                                g.throttle_reasons
                                    .value()
                                    .cloned()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(Json::Str)
                                    .collect(),
                            ),
                        ),
                    ]),
                ),
            ])
        })
        .collect();
    let fans = Json::Arr(
        h.map(|h| {
            h.fans
                .iter()
                .map(|(id, rpm)| {
                    Json::Obj(vec![
                        ("id".into(), num_u(*id as u64)),
                        ("rpm".into(), avail_u(rpm)),
                    ])
                })
                .collect()
        })
        .unwrap_or_default(),
    );
    let e = view.rates.map(|r| &r.engine_pct);
    let engines = Json::Obj(vec![
        ("rcs".into(), e.map(|x| pct(&x.rcs)).unwrap_or(Json::Null)),
        ("ccs".into(), e.map(|x| pct(&x.ccs)).unwrap_or(Json::Null)),
        ("vcs".into(), e.map(|x| pct(&x.vcs)).unwrap_or(Json::Null)),
        ("vecs".into(), e.map(|x| pct(&x.vecs)).unwrap_or(Json::Null)),
        ("bcs".into(), e.map(|x| pct(&x.bcs)).unwrap_or(Json::Null)),
    ]);
    let mut items = vec![
        ("index".into(), num_u(dev.index as u64)),
        ("pci_address".into(), Json::Str(dev.pci.clone())),
        ("name".into(), Json::Str(dev.name.clone())),
        ("pci".into(), ids),
        (
            "drm".into(),
            Json::Obj(vec![
                ("card".into(), Json::Str(dev.card.clone())),
                ("render".into(), opt_s(dev.render.clone())),
            ]),
        ),
        ("thermal".into(), Json::Arr(thermal)),
        ("power".into(), power),
        ("memory".into(), memory),
        ("gts".into(), Json::Arr(gts)),
        ("fans".into(), fans),
        ("engines".into(), engines),
        ("processes_count".into(), num_u(view.used_clients as u64)),
        ("persistence_installed".into(), Json::Bool(view.persistence)),
    ];
    items.extend(placement_json(roots, dev));
    Json::Obj(items)
}

/// Placement / connector / SR-IOV / crash objects for the `info --json` schema (spec/22).
pub fn placement_json(roots: &crate::paths::Roots, dev: &Device) -> Vec<(String, Json)> {
    use crate::probe::placement::Placement;
    let p = &dev.placement;
    let num_opt = |a: &crate::avail::Avail<u64>| a.value().map(|v| num_u(*v)).unwrap_or(Json::Null);
    let topology = Json::Obj(vec![
        (
            "numa_node".into(),
            p.numa_display()
                .map(|n| num_u(n.parse().unwrap_or(0)))
                .unwrap_or(Json::Null),
        ),
        (
            "local_cpus".into(),
            p.local_cpus
                .value()
                .map(|s| Json::Str(s.clone()))
                .unwrap_or(Json::Null),
        ),
        (
            "iommu_group".into(),
            p.iommu_group
                .value()
                .and_then(|s| s.parse::<u64>().ok())
                .map(num_u)
                .unwrap_or(Json::Null),
        ),
        (
            "pci_path".into(),
            Json::Arr(p.path.iter().map(|c| Json::Str(c.clone())).collect()),
        ),
        (
            "root_port".into(),
            p.root_port
                .value()
                .map(|s| Json::Str(s.clone()))
                .unwrap_or(Json::Null),
        ),
        (
            "host_bridge".into(),
            p.host_bridge
                .value()
                .map(|s| Json::Str(s.clone()))
                .unwrap_or(Json::Null),
        ),
        (
            "aer".into(),
            match p.aer.as_ref() {
                Some(a) => Json::Obj(vec![
                    ("correctable".into(), num_opt(&a.correctable)),
                    ("nonfatal".into(), num_opt(&a.nonfatal)),
                    ("fatal".into(), num_opt(&a.fatal)),
                ]),
                None => Json::Null,
            },
        ),
    ]);
    let connectors = Json::Arr(
        crate::sriov::connectors(roots, dev)
            .iter()
            .map(|c| {
                Json::Obj(vec![
                    ("name".into(), Json::Str(c.name.clone())),
                    ("status".into(), Json::Str(c.status.clone())),
                    ("enabled".into(), Json::Bool(c.enabled)),
                    ("modes".into(), num_u(c.modes as u64)),
                ])
            })
            .collect(),
    );
    let sriov = match crate::sriov::status(dev) {
        Some(s) => Json::Obj(vec![
            ("total_vfs".into(), num_u(s.total_vfs)),
            ("num_vfs".into(), num_u(s.num_vfs)),
            ("autoprobe".into(), Json::Bool(s.autoprobe)),
        ]),
        None => Json::Null,
    };
    let crash_dumps = Json::Arr(
        crate::crash::list_for_pci(roots, &dev.pci)
            .iter()
            .map(|d| {
                Json::Obj(vec![
                    ("id".into(), num_u(d.id as u64)),
                    (
                        "reason".into(),
                        d.reason.clone().map(Json::Str).unwrap_or(Json::Null),
                    ),
                    (
                        "snapshot_time".into(),
                        d.snapshot
                            .map(|s| Json::Str(crate::time::iso8601(s)))
                            .unwrap_or(Json::Null),
                    ),
                ])
            })
            .collect(),
    );
    let _ = Placement::default;
    vec![
        ("topology".into(), topology),
        ("connectors".into(), connectors),
        ("sriov".into(), sriov),
        ("crash_dumps".into(), crash_dumps),
    ]
}

pub fn list_json(
    roots: &crate::paths::Roots,
    kernel: &str,
    ts: &str,
    views: &[(&Device, View<'_>)],
) -> String {
    status_json(roots, kernel, ts, views)
}

pub fn query_json(ts: &str, fields: &[ParsedField], views: &[(&Device, View<'_>)]) -> String {
    let rows_json: Vec<Json> = views
        .iter()
        .map(|(_, view)| {
            Json::Obj(
                fields
                    .iter()
                    .map(|f| {
                        let v = match fields::resolve(f.def.name, f.gt, view) {
                            Avail::Value(c) => c.json.clone(),
                            Avail::NotAvailable(_) => Json::Null,
                        };
                        (f.requested.clone(), v)
                    })
                    .collect(),
            )
        })
        .collect();
    let mut s = String::new();
    crate::format::json::write(
        &Json::Obj(vec![
            ("schema_version".into(), num_u(1)),
            ("generated_at".into(), Json::Str(ts.into())),
            (
                "fields".into(),
                Json::Arr(
                    fields
                        .iter()
                        .map(|f| Json::Str(f.requested.clone()))
                        .collect(),
                ),
            ),
            ("rows".into(), Json::Arr(rows_json)),
        ]),
        &mut s,
    );
    s
}

pub fn processes_json(
    ts: &str,
    sample_ms: u64,
    visible: bool,
    rates: &[crate::sample::Rates],
    redact_names: bool,
) -> String {
    let mut procs = Vec::new();
    for r in rates {
        for c in &r.per_client {
            procs.push(Json::Obj(vec![
                ("pid".into(), num_u(c.client.pid as u64)),
                (
                    "name".into(),
                    Json::Str(if redact_names {
                        "<redacted>".into()
                    } else {
                        c.client.name.clone()
                    }),
                ),
                ("pci_address".into(), Json::Str(c.client.pdev.clone())),
                ("client_id".into(), num_u(c.client.client_id)),
                (
                    "pids".into(),
                    Json::Arr(c.client.pids.iter().map(|p| num_u(*p as u64)).collect()),
                ),
                ("uid".into(), num_u(c.client.uid as u64)),
                (
                    "cgroup".into(),
                    c.client.cgroup.clone().map(Json::Str).unwrap_or(Json::Null),
                ),
                (
                    "memory".into(),
                    Json::Obj(
                        c.client
                            .regions
                            .iter()
                            .map(|(name, m)| {
                                (
                                    name.clone(),
                                    Json::Obj(vec![
                                        ("total_bytes".into(), opt_u(m.total)),
                                        ("shared_bytes".into(), opt_u(m.shared)),
                                        ("active_bytes".into(), opt_u(m.active)),
                                        ("resident_bytes".into(), opt_u(m.resident)),
                                        ("purgeable_bytes".into(), opt_u(m.purgeable)),
                                    ]),
                                )
                            })
                            .collect(),
                    ),
                ),
                ("resident_vram_bytes".into(), num_u(c.client.resident_vram)),
                ("total_vram_bytes".into(), opt_u(c.client.total_vram)),
                ("active_vram_bytes".into(), opt_u(c.client.active_vram)),
                (
                    "engine_pct".into(),
                    Json::Obj(vec![
                        ("rcs".into(), pct(&c.pct[0])),
                        ("ccs".into(), pct(&c.pct[1])),
                        ("vcs".into(), pct(&c.pct[2])),
                        ("vecs".into(), pct(&c.pct[3])),
                        ("bcs".into(), pct(&c.pct[4])),
                    ]),
                ),
            ]));
        }
    }
    let mut s = String::new();
    crate::format::json::write(
        &Json::Obj(vec![
            ("schema_version".into(), num_u(1)),
            ("generated_at".into(), Json::Str(ts.into())),
            ("sample_ms".into(), num_u(sample_ms)),
            ("visible_all_users".into(), Json::Bool(visible)),
            ("processes".into(), Json::Arr(procs)),
        ]),
        &mut s,
    );
    s
}

pub fn fields_json(rows: &[ListRow]) -> String {
    let mut s = String::new();
    crate::format::json::write(
        &Json::Obj(vec![
            ("schema_version".into(), num_u(1)),
            (
                "fields".into(),
                Json::Arr(
                    rows.iter()
                        .map(|r| {
                            Json::Obj(vec![
                                ("name".into(), Json::Str(r.name.clone())),
                                (
                                    "unit".into(),
                                    if r.unit.is_empty() {
                                        Json::Null
                                    } else {
                                        Json::Str(r.unit.into())
                                    },
                                ),
                                ("available".into(), Json::Bool(r.available)),
                                ("source".into(), Json::Str(r.prov.source.into())),
                                ("scope".into(), Json::Str(r.prov.scope.into())),
                                ("access".into(), Json::Str(r.prov.access.into())),
                                ("quality".into(), Json::Str(r.prov.quality.into())),
                                ("description".into(), Json::Str(r.desc.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        &mut s,
    );
    s
}

/// `get` JSON: same device sub-objects as the report plus `boot_default`.
pub fn get_json(
    roots: &crate::paths::Roots,
    ts: &str,
    what: &crate::cli::GetWhat,
    views: &[(&Device, View<'_>)],
) -> String {
    let bid = persist::boot_id(roots);
    let mut devices = Vec::new();
    for (dev, view) in views {
        let obj = match what {
            crate::cli::GetWhat::PowerLimit => {
                let mut d = device_json(dev, view, roots);
                let recs = persist::boot_records(roots, &bid, &dev.pci);
                if let Json::Obj(ref mut v) = d {
                    let boot = ["pl1.card", "pl2.card", "pl1.pkg", "pl2.pkg"]
                        .iter()
                        .find_map(|k| persist::boot_record(&recs, k))
                        .and_then(|raw| raw.parse::<u64>().ok());
                    v.push((
                        "boot_default".into(),
                        boot.map(|uw| Json::Obj(vec![("watts".into(), num_f(uw as f64 / 1e6, 2))]))
                            .unwrap_or(Json::Null),
                    ));
                }
                d
            }
            crate::cli::GetWhat::Clocks => {
                let recs = persist::boot_records(roots, &bid, &dev.pci);
                let gts: Vec<Json> = dev
                    .gts
                    .iter()
                    .map(|g| {
                        Json::Obj(vec![
                            ("id".into(), num_u(g.id as u64)),
                            (
                                "clock_mhz".into(),
                                Json::Obj(vec![
                                    ("min".into(), opt_u(g.min.value().map(|v| *v as u64))),
                                    ("max".into(), opt_u(g.max.value().map(|v| *v as u64))),
                                    ("rpn".into(), opt_u(g.rpn.value().map(|v| *v as u64))),
                                    ("rpe".into(), opt_u(g.rpe.value().map(|v| *v as u64))),
                                    ("rpa".into(), opt_u(g.rpa.value().map(|v| *v as u64))),
                                    ("rp0".into(), opt_u(g.rp0.value().map(|v| *v as u64))),
                                ]),
                            ),
                            (
                                "boot_default".into(),
                                match (
                                    persist::boot_record(&recs, &format!("gt{}.min_freq", g.id)),
                                    persist::boot_record(&recs, &format!("gt{}.max_freq", g.id)),
                                ) {
                                    (Some(mn), Some(mx)) => Json::Obj(vec![
                                        ("min_mhz".into(), Json::Str(mn.into())),
                                        ("max_mhz".into(), Json::Str(mx.into())),
                                    ]),
                                    _ => Json::Null,
                                },
                            ),
                        ])
                    })
                    .collect();
                Json::Obj(vec![
                    ("pci_address".into(), Json::Str(dev.pci.clone())),
                    ("gts".into(), Json::Arr(gts)),
                ])
            }
            crate::cli::GetWhat::PowerProfile => {
                let gts: Vec<Json> = dev
                    .gts
                    .iter()
                    .map(|g| {
                        let raw = crate::sysfs::read_string(&g.dir.join("freq0/power_profile"));
                        let available: Vec<Json> = raw
                            .value()
                            .map(|t| {
                                t.split_whitespace()
                                    .map(|tok| {
                                        Json::Str(
                                            tok.trim_matches(|c| c == '[' || c == ']').to_string(),
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        Json::Obj(vec![
                            ("id".into(), num_u(g.id as u64)),
                            ("power_profile".into(), avail_s(&g.profile)),
                            ("available_profiles".into(), Json::Arr(available)),
                        ])
                    })
                    .collect();
                Json::Obj(vec![
                    ("pci_address".into(), Json::Str(dev.pci.clone())),
                    ("gts".into(), Json::Arr(gts)),
                ])
            }
        };
        devices.push(obj);
    }
    let mut s = String::new();
    crate::format::json::write(
        &Json::Obj(vec![
            ("schema_version".into(), num_u(1)),
            ("generated_at".into(), Json::Str(ts.into())),
            ("devices".into(), Json::Arr(devices)),
        ]),
        &mut s,
    );
    s
}

/// Device list for the report wrappers: index/name/pci only would also satisfy the schema, but
/// the report device shape is shared with status/info.
pub fn _path_hint(p: &Path) -> &Path {
    p
}

pub fn status_json_str(
    roots: &crate::paths::Roots,
    kernel: &str,
    ts: &str,
    views: &[(&Device, View<'_>)],
) -> String {
    status_json(roots, kernel, ts, views)
}

pub fn info_json(
    roots: &crate::paths::Roots,
    kernel: &str,
    ts: &str,
    views: &[(&Device, View<'_>)],
) -> String {
    status_json(roots, kernel, ts, views)
}
