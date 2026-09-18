//! `topology` and `pcie` renderers plus the matching `info` sections (spec/22).

use crate::format::json::{num_u, Json};
use crate::probe::placement::Placement;
use crate::probe::Device;

const NA: &str = "N/A";

fn kv25(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("  {:<25}: {}\n", label, value));
}

fn srt(o: Option<&String>) -> Json {
    match o {
        Some(v) => Json::Str(v.clone()),
        None => Json::Null,
    }
}

fn numa_json(p: &Placement) -> Json {
    p.numa_display()
        .map(|n| num_u(n.parse().unwrap_or(0)))
        .unwrap_or(Json::Null)
}

fn iommu_json(p: &Placement) -> Json {
    match p.iommu_group.value().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => num_u(n),
        None => match p.iommu_group.value() {
            Some(s) => Json::Str(s.clone()),
            None => Json::Null,
        },
    }
}

fn sriov_json(dev: &Device) -> Json {
    let p = &dev.placement;
    if let Some(pf) = &p.sriov {
        let vfs = Json::Arr(
            pf.vfs
                .iter()
                .map(|v| {
                    Json::Obj(vec![
                        ("address".into(), Json::Str(v.addr.clone())),
                        (
                            "driver".into(),
                            match v.driver.value() {
                                Some(d) => Json::Str(d.clone()),
                                None => Json::Null,
                            },
                        ),
                    ])
                })
                .collect(),
        );
        let num = |x: &crate::avail::Avail<u64>| x.value().map(|v| num_u(*v)).unwrap_or(Json::Null);
        Json::Obj(vec![
            ("mode".into(), Json::Str("pf".into())),
            ("total_vfs".into(), num(&pf.total)),
            ("num_vfs".into(), num(&pf.num)),
            ("vfs".into(), vfs),
        ])
    } else if let Some(pf) = p.vf_of.value() {
        Json::Obj(vec![
            ("mode".into(), Json::Str("vf".into())),
            ("pf".into(), Json::Str(pf.clone())),
        ])
    } else {
        Json::Null
    }
}

pub fn topology_text(devs: &[&Device]) -> String {
    let mut out = String::new();
    for d in devs {
        out.push_str(&format!("GPU {} [{}]\n", d.index, d.pci));
        kv25(
            &mut out,
            "NUMA node",
            &d.placement.numa_display().unwrap_or_else(|| NA.into()),
        );
        kv25(
            &mut out,
            "Local CPUs",
            d.placement
                .local_cpus
                .value()
                .map(String::as_str)
                .unwrap_or(NA),
        );
        kv25(
            &mut out,
            "IOMMU group",
            d.placement
                .iommu_group
                .value()
                .map(String::as_str)
                .unwrap_or(NA),
        );
        kv25(&mut out, "PCI path", &d.placement.path.join(" > "));
        kv25(&mut out, "SR-IOV", &sriov_line(d));
    }
    if devs.len() >= 2 {
        out.push('\n');
        out.push_str("Affinity (xe devices only)\n");
        let mut header = " ".repeat(6);
        for d in devs {
            header.push_str(&format!("{:<6}", format!("GPU{}", d.index)));
        }
        out.push_str(header.trim_end());
        out.push('\n');
        for a in devs {
            let mut row = format!("{:<6}", format!("GPU{}", a.index));
            for b in devs {
                let cell = if std::ptr::eq(a, b) {
                    "X".to_string()
                } else {
                    Placement::relation(&a.placement, &b.placement).to_string()
                };
                row.push_str(&format!("{:<6}", cell));
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out.push_str("Legend: PIX same root port, PHB same host bridge, NODE same NUMA node, SYS different NUMA nodes\n");
    }
    out
}

pub fn sriov_line(dev: &Device) -> String {
    let p = &dev.placement;
    match &p.sriov {
        Some(pf) => {
            let total = pf.total.value().copied().unwrap_or(0);
            let num = pf.num.value().copied().unwrap_or(0);
            let mut line = format!("PF, {num} of {total} VFs enabled");
            if num > 0 && !pf.vfs.is_empty() {
                let vfs = pf
                    .vfs
                    .iter()
                    .map(|v| match v.driver.value() {
                        Some(d) => format!("{} {}", v.addr, d),
                        None => format!("{} no driver", v.addr),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                line.push_str(&format!(" ({vfs})"));
            }
            line
        }
        None => match p.vf_of.value() {
            Some(pf) => format!("VF of {pf}"),
            None => NA.into(),
        },
    }
}

pub fn topology_json(devs: &[&Device], ts: &str) -> String {
    let devices = Json::Arr(
        devs.iter()
            .map(|d| {
                Json::Obj(vec![
                    ("pci_address".into(), Json::Str(d.pci.clone())),
                    ("numa_node".into(), numa_json(&d.placement)),
                    ("local_cpus".into(), srt(d.placement.local_cpus.value())),
                    ("iommu_group".into(), iommu_json(&d.placement)),
                    (
                        "pci_path".into(),
                        Json::Arr(
                            d.placement
                                .path
                                .iter()
                                .map(|c| Json::Str(c.clone()))
                                .collect(),
                        ),
                    ),
                    ("root_port".into(), srt(d.placement.root_port.value())),
                    ("host_bridge".into(), srt(d.placement.host_bridge.value())),
                    ("sriov".into(), sriov_json(d)),
                    ("hardware".into(), Json::Null),
                ])
            })
            .collect(),
    );
    let mut aff: Vec<(String, Json)> = Vec::new();
    for (i, a) in devs.iter().enumerate() {
        for b in devs.iter().skip(i + 1) {
            aff.push((
                format!("GPU{}-GPU{}", a.index, b.index),
                Json::Str(Placement::relation(&a.placement, &b.placement).into()),
            ));
        }
    }
    let root = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("generated_at".into(), Json::Str(ts.into())),
        ("devices".into(), devices),
        ("affinity".into(), Json::Obj(aff)),
    ]);
    let mut s = String::new();
    super::json::write(&root, &mut s);
    s
}

fn link_line(dev: &Device) -> String {
    match dev.link.value() {
        Some(l) => {
            let g = |o: Option<u8>| o.map(|v| v.to_string()).unwrap_or_else(|| NA.into());
            let w = |o: Option<u32>| o.map(|v| v.to_string()).unwrap_or_else(|| NA.into());
            format!(
                "gen{} x{} (max gen{} x{})",
                g(l.gen_cur),
                w(l.width_cur),
                g(l.gen_max),
                w(l.width_max)
            )
        }
        None => NA.into(),
    }
}

fn totals_line(c: &str, n: &str, f: &str) -> String {
    format!("correctable {c}, non-fatal {n}, fatal {f}")
}

pub fn pcie_text(devs: &[&Device]) -> String {
    let mut out = String::new();
    for d in devs {
        out.push_str(&format!("GPU {} [{}]\n", d.index, d.pci));
        kv25(&mut out, "Link", &link_line(d));
        match &d.placement.aer {
            Some(a) => {
                let t = |x: &crate::avail::Avail<u64>| {
                    x.value()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| NA.into())
                };
                kv25(
                    &mut out,
                    "AER (endpoint)",
                    &totals_line(&t(&a.correctable), &t(&a.nonfatal), &t(&a.fatal)),
                );
                for n in &a.named {
                    let label = match n.kind {
                        1 => format!("{} (non-fatal)", n.name),
                        2 => format!("{} (fatal)", n.name),
                        _ => n.name.clone(),
                    };
                    out.push_str(&format!("    {:<23}: {}\n", label, n.count));
                }
            }
            None => kv25(
                &mut out,
                "AER (endpoint)",
                "N/A (no AER statistics exposed for this device)",
            ),
        }
        match d.placement.root_port.value() {
            Some(rp) => {
                let v = match &d.placement.root_port_aer {
                    Some(t) => {
                        let x = |o: &crate::avail::Avail<u64>| {
                            o.value()
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| NA.into())
                        };
                        totals_line(&x(&t.correctable), &x(&t.nonfatal), &x(&t.fatal))
                    }
                    None => "N/A (no AER statistics exposed on the root port)".into(),
                };
                kv25(&mut out, &format!("Root port {rp}"), &v);
            }
            None => kv25(
                &mut out,
                "Root port",
                "N/A (no root port between the host bridge and the device)",
            ),
        }
    }
    out
}

fn totals_json(t: &crate::probe::placement::AerTotals) -> Json {
    let n = |x: &crate::avail::Avail<u64>| x.value().map(|v| num_u(*v)).unwrap_or(Json::Null);
    Json::Obj(vec![
        ("correctable".into(), n(&t.correctable)),
        ("nonfatal".into(), n(&t.nonfatal)),
        ("fatal".into(), n(&t.fatal)),
    ])
}

pub fn pcie_json(devs: &[&Device], ts: &str) -> String {
    let devices = Json::Arr(
        devs.iter()
            .map(|d| {
                let link = d.link.value().map(|l| {
                    let n8 = |o: Option<u8>| o.map(|v| num_u(v as u64)).unwrap_or(Json::Null);
                    let n32 = |o: Option<u32>| o.map(|v| num_u(v as u64)).unwrap_or(Json::Null);
                    Json::Obj(vec![
                        ("gen_current".into(), n8(l.gen_cur)),
                        ("gen_max".into(), n8(l.gen_max)),
                        ("width_current".into(), n32(l.width_cur)),
                        ("width_max".into(), n32(l.width_max)),
                    ])
                });
                let aer = d.placement.aer.as_ref().map(|a| {
                    let mut named = [Vec::<(String, Json)>::new(), Vec::new(), Vec::new()];
                    for n in &a.named {
                        named[n.kind as usize].push((n.name.clone(), num_u(n.count)));
                    }
                    let tot = Json::Obj(vec![
                        (
                            "correctable".into(),
                            a.correctable
                                .value()
                                .map(|v| num_u(*v))
                                .unwrap_or(Json::Null),
                        ),
                        (
                            "nonfatal".into(),
                            a.nonfatal.value().map(|v| num_u(*v)).unwrap_or(Json::Null),
                        ),
                        (
                            "fatal".into(),
                            a.fatal.value().map(|v| num_u(*v)).unwrap_or(Json::Null),
                        ),
                    ]);
                    Json::Obj(vec![
                        ("endpoint".into(), tot),
                        ("correctable".into(), Json::Obj(named[0].clone())),
                        ("nonfatal".into(), Json::Obj(named[1].clone())),
                        ("fatal".into(), Json::Obj(named[2].clone())),
                    ])
                });
                let rp_aer = d.placement.root_port_aer.as_ref().map(totals_json);
                Json::Obj(vec![
                    ("pci_address".into(), Json::Str(d.pci.clone())),
                    (
                        "pcie".into(),
                        Json::Obj(vec![
                            ("link".into(), link.unwrap_or(Json::Null)),
                            ("aer".into(), aer.unwrap_or(Json::Null)),
                            ("root_port".into(), srt(d.placement.root_port.value())),
                            ("root_port_aer".into(), rp_aer.unwrap_or(Json::Null)),
                        ]),
                    ),
                ])
            })
            .collect(),
    );
    let root = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("generated_at".into(), Json::Str(ts.into())),
        ("devices".into(), devices),
    ]);
    let mut s = String::new();
    super::json::write(&root, &mut s);
    s
}

/// `info` `Topology` section block (spec/22 golden layout).
pub fn info_topology(out: &mut String, dev: &Device) {
    out.push_str("    Topology\n");
    super::info::kv(
        out,
        8,
        "NUMA node",
        &dev.placement.numa_display().unwrap_or_else(|| NA.into()),
    );
    super::info::kv(
        out,
        8,
        "Local CPUs",
        dev.placement
            .local_cpus
            .value()
            .map(String::as_str)
            .unwrap_or(NA),
    );
    super::info::kv(
        out,
        8,
        "IOMMU group",
        dev.placement
            .iommu_group
            .value()
            .map(String::as_str)
            .unwrap_or(NA),
    );
    super::info::kv(out, 8, "PCI path", &dev.placement.path.join(" > "));
}

/// `info` `PCIe health` section block.
pub fn info_pcie_health(out: &mut String, dev: &Device) {
    out.push_str("    PCIe health\n");
    let v = match &dev.placement.aer {
        Some(a) => {
            let x = |o: &crate::avail::Avail<u64>| {
                o.value()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| NA.into())
            };
            totals_line(&x(&a.correctable), &x(&a.nonfatal), &x(&a.fatal))
        }
        None => NA.into(),
    };
    super::info::kv(out, 8, "AER endpoint", &v);
    let rp = match dev.placement.root_port.value() {
        Some(_) => match &dev.placement.root_port_aer {
            Some(t) => {
                let x = |o: &crate::avail::Avail<u64>| {
                    o.value()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| NA.into())
                };
                totals_line(&x(&t.correctable), &x(&t.nonfatal), &x(&t.fatal))
            }
            None => NA.into(),
        },
        None => NA.into(),
    };
    super::info::kv(out, 8, "Root port", &rp);
}

/// Engine class names as `topology --hardware` prints them (order as the kernel returns).
pub fn engine_name(class: u16, instance: u16) -> String {
    let c = match class {
        0 => "rcs",
        1 => "bcs",
        2 => "vcs",
        3 => "vecs",
        4 => "ccs",
        _ => "eng",
    };
    format!("{c}{instance}")
}

fn engine_class_name(class: u16) -> &'static str {
    match class {
        0 => "rcs",
        1 => "bcs",
        2 => "vcs",
        3 => "vecs",
        4 => "ccs",
        _ => "eng",
    }
}

fn region_names(mask: u64, mem: &[crate::kabi::MemRegion]) -> String {
    let mut names = Vec::new();
    for (i, r) in mem.iter().enumerate() {
        if mask & (1u64 << i) != 0 {
            names.push(if r.class == 1 {
                format!("vram{}", r.instance)
            } else {
                "sysmem".to_string()
            });
        }
    }
    if names.is_empty() {
        NA.into()
    } else {
        names.join(", ")
    }
}

fn topo_count(dev: &Device, gt: u16, kind: u16) -> Option<u32> {
    dev.kabi
        .gt_topology
        .value()
        .and_then(|ts| ts.iter().find(|t| t.gt == gt && t.kind == kind))
        .map(|t| crate::kabi::popcount(&t.mask))
}

/// Hardware topology of one device without the `GPU ...` header (shared by the command,
/// `info --section hardware` and the `--hardware` report wrapper).
pub fn hardware_lines(dev: &Device) -> Option<String> {
    let gts = dev.kabi.gt_list.value()?;
    let engines = dev.kabi.engines.value();
    let mem = dev.kabi.mem.value();
    let mut out = String::new();
    let mut last_tile: Option<u16> = None;
    for gt in gts {
        if last_tile != Some(gt.tile) {
            out.push_str(&format!("  Tile {}\n", gt.tile));
            last_tile = Some(gt.tile);
        }
        let kind = if gt.kind == 1 { "media" } else { "main" };
        let near = mem
            .map(|m| region_names(gt.near_mem, m))
            .unwrap_or_else(|| NA.into());
        out.push_str(&format!(
            "    GT {} [{kind}] Xe {}.{}.{}, reference clock {} Hz, near memory {near}\n",
            gt.gt, gt.ip.0, gt.ip.1, gt.ip.2, gt.reference_clock
        ));
        if let Some(es) = engines {
            let names: Vec<String> = es
                .iter()
                .filter(|e| e.gt_id == gt.gt)
                .map(|e| engine_name(e.class, e.instance))
                .collect();
            if !names.is_empty() {
                out.push_str(&format!("      engines: {}\n", names.join(", ")));
            }
        }
        let kinds = [
            (1, "geometry DSS"),
            (2, "compute DSS"),
            (3, "L3 banks"),
            (4, "EUs per DSS"),
            (5, "SIMD16 EUs per DSS"),
        ];
        let parts: Vec<String> = kinds
            .iter()
            .filter_map(|(k, label)| topo_count(dev, gt.gt, *k).map(|c| format!("{label} {c}")))
            .collect();
        if !parts.is_empty() {
            out.push_str(&format!("      {}\n", parts.join(", ")));
        }
    }
    Some(out)
}

pub fn hardware_text(devs: &[&Device]) -> String {
    let mut out = String::new();
    for (i, dev) in devs.iter().enumerate() {
        out.push_str(&format!("GPU {i} [{}]\n", dev.pci));
        if let Some(lines) = hardware_lines(dev) {
            out.push_str(&lines);
        }
    }
    out
}

pub fn hardware_json(devs: &[&Device], ts: &str) -> String {
    let mut devices = Vec::new();
    for (i, dev) in devs.iter().enumerate() {
        let mut obj: Vec<(String, Json)> = vec![
            ("index".into(), num_u(i as u64)),
            ("pci_address".into(), Json::Str(dev.pci.clone())),
        ];
        if let Some(gts) = dev.kabi.gt_list.value() {
            let engines = dev.kabi.engines.value();
            let mem = dev.kabi.mem.value();
            let mut tiles: Vec<Json> = Vec::new();
            let mut cur_tile: Option<u16> = None;
            for gt in gts {
                if cur_tile != Some(gt.tile) {
                    tiles.push(Json::Obj(vec![
                        ("tile".into(), num_u(gt.tile as u64)),
                        ("gts".into(), Json::Arr(Vec::new())),
                    ]));
                    cur_tile = Some(gt.tile);
                }
                let gt_json = {
                    let mut g: Vec<(String, Json)> = vec![
                        ("gt".into(), num_u(gt.gt as u64)),
                        (
                            "type".into(),
                            Json::Str(if gt.kind == 1 {
                                "media".into()
                            } else {
                                "main".into()
                            }),
                        ),
                        (
                            "ip_version".into(),
                            Json::Str(format!("{}.{}.{}", gt.ip.0, gt.ip.1, gt.ip.2)),
                        ),
                        (
                            "reference_clock_hz".into(),
                            num_u(gt.reference_clock as u64),
                        ),
                        (
                            "near_memory".into(),
                            Json::Arr(
                                mem.map(|m| {
                                    region_names(gt.near_mem, m)
                                        .split(", ")
                                        .map(|s| Json::Str(s.into()))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            ),
                        ),
                        (
                            "engines".into(),
                            Json::Arr(
                                engines
                                    .map(|es| {
                                        es.iter()
                                            .filter(|e| e.gt_id == gt.gt)
                                            .map(|e| {
                                                Json::Obj(vec![
                                                    (
                                                        "class".into(),
                                                        Json::Str(
                                                            engine_class_name(e.class).into(),
                                                        ),
                                                    ),
                                                    ("instance".into(), num_u(e.instance as u64)),
                                                ])
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            ),
                        ),
                    ];
                    for (k, label) in [
                        (1u16, "geometry_dss"),
                        (2, "compute_dss"),
                        (3, "l3_banks"),
                        (4, "eus_per_dss"),
                        (5, "simd16_eus_per_dss"),
                    ] {
                        if let Some(c) = topo_count(dev, gt.gt, k) {
                            g.push((label.into(), num_u(c as u64)));
                        }
                    }
                    Json::Obj(g)
                };
                if let Some(Json::Obj(t)) = tiles.last_mut() {
                    if let Some((_, Json::Arr(list))) = t.iter_mut().find(|(k, _)| k == "gts") {
                        list.push(gt_json);
                    }
                }
            }
            obj.push(("tiles".into(), Json::Arr(tiles)));
        }
        devices.push(Json::Obj(obj));
    }
    let root = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("timestamp".into(), Json::Str(ts.into())),
        ("devices".into(), Json::Arr(devices)),
    ]);
    let mut s = String::new();
    super::json::write(&root, &mut s);
    s
}

fn ufw_text(a: &crate::avail::Avail<crate::kabi::UcFw>, verbose: bool) -> String {
    match a.value() {
        Some(u) => {
            let mut t = format!("{}.{}.{}", u.major, u.minor, u.patch);
            if u.branch != 0 {
                t.push_str(&format!(", branch {}", u.branch));
            }
            t
        }
        None if verbose => {
            let reason = match a {
                crate::avail::Avail::NotAvailable(r) => r.to_string(),
                _ => String::new(),
            };
            format!("{} ({})", NA, reason)
        }
        None => NA.into(),
    }
}

pub fn firmware_text(devs: &[&Device], verbose: bool) -> String {
    let mut out = String::new();
    for (i, dev) in devs.iter().enumerate() {
        out.push_str(&format!("GPU {i} [{}]\n", dev.pci));
        kv25(
            &mut out,
            "GuC (submission)",
            &ufw_text(&dev.kabi.guc, verbose),
        );
        kv25(&mut out, "HuC", &ufw_text(&dev.kabi.huc, verbose));
    }
    out
}

pub fn firmware_json(devs: &[&Device], ts: &str) -> String {
    let devices = devs
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let s = |a: &crate::avail::Avail<crate::kabi::UcFw>| match a.value() {
                Some(u) if u.branch != 0 => Json::Str(format!(
                    "{}.{}.{} branch {}",
                    u.major, u.minor, u.patch, u.branch
                )),
                Some(u) => Json::Str(format!("{}.{}.{}", u.major, u.minor, u.patch)),
                None => Json::Null,
            };
            Json::Obj(vec![
                ("index".into(), num_u(i as u64)),
                ("pci_address".into(), Json::Str(dev.pci.clone())),
                ("guc".into(), s(&dev.kabi.guc)),
                ("huc".into(), s(&dev.kabi.huc)),
            ])
        })
        .collect();
    let root = Json::Obj(vec![
        ("schema_version".into(), num_u(1)),
        ("timestamp".into(), Json::Str(ts.into())),
        ("devices".into(), Json::Arr(devices)),
    ]);
    let mut s = String::new();
    super::json::write(&root, &mut s);
    s
}
