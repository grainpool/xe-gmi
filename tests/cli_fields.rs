mod common;
use common::*;

/// Every canonical field name. Adding a field means adding it here.
const CANONICAL: &[&str] = &[
    "index",
    "name",
    "pci.address",
    "pci.vendor_id",
    "pci.device_id",
    "pci.subsystem",
    "pci.revision",
    "pci.link.gen.current",
    "pci.link.gen.max",
    "pci.link.width.current",
    "pci.link.width.max",
    "pci.numa_node",
    "pci.local_cpus",
    "pci.iommu_group",
    "pci.root_port",
    "pci.aer.correctable",
    "pci.aer.nonfatal",
    "pci.aer.fatal",
    "pci.aer.rootport.correctable",
    "pci.aer.rootport.nonfatal",
    "pci.aer.rootport.fatal",
    "driver",
    "driver.kernel",
    "drm.card",
    "drm.render",
    "temp.pkg",
    "temp.pkg.max",
    "temp.pkg.crit",
    "temp.pkg.emergency",
    "temp.vram",
    "temp.vram.crit",
    "temp.vram.emergency",
    "temp.mctrl",
    "temp.pcie",
    "power.draw",
    "power.draw.pkg",
    "power.limit",
    "power.limit.kind",
    "power.limit.pl1",
    "power.limit.pl2",
    "power.limit.pl1.pkg",
    "power.limit.pl2.pkg",
    "power.limit.pl1.window",
    "power.limit.pl2.window",
    "power.crit",
    "power.rated_max",
    "power.profile",
    "energy.card",
    "energy.pkg",
    "voltage.pkg",
    "memory.total",
    "memory.used",
    "memory.free",
    "memory.total.source",
    "memory.used.source",
    "utilization.gt",
    "utilization.render",
    "utilization.compute",
    "utilization.video",
    "utilization.enhance",
    "utilization.copy",
    "clock.cur",
    "clock.act",
    "clock.min",
    "clock.max",
    "clock.rp0",
    "clock.rpe",
    "clock.rpn",
    "clock.rpa",
    "clock.mem",
    "throttle.active",
    "throttle.reasons",
    "idle.status",
    "fan.rpm",
    "fan.percent",
    "processes.count",
    "persistence.installed",
    "timestamp",
    "ecc.mode",
    "uuid",
    "serial",
    "sriov.total_vfs",
    "sriov.num_vfs",
    "display.connectors",
    "display.connected",
    "display.active",
    "crash.pending",
    "health.survivability",
    "memory.cpu_visible.total",
    "memory.cpu_visible.used",
    "memory.min_page_size",
    "firmware.guc",
    "firmware.huc",
];

#[test]
fn fields_lists_every_canonical_field_with_availability() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["fields"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    for f in CANONICAL {
        assert!(
            s.lines().any(|l| l.split_whitespace().next() == Some(*f)),
            "field {f} missing from `fields` output:\n{s}"
        );
    }
    // availability column shows N/A reasons for fields this card lacks
    let line = s.lines().find(|l| l.starts_with("clock.mem ")).unwrap();
    assert_contains(line, "PVC");
    let line = s.lines().find(|l| l.starts_with("fan.percent ")).unwrap();
    assert_contains(line, "N/A");
}

#[test]
fn fields_json() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["fields", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "\"name\": \"power.draw\"");
    assert_contains(&stdout(&o), "\"unit\": \"W\"");
}

#[test]
fn gt_indexed_fields_are_accepted_by_query() {
    let o = Runner::fixture("multi-tile-2x2-k7.1").run(&[
        "query",
        "--fields",
        "gt0.clock.cur,gt1.clock.cur,gt2.clock.cur,gt3.clock.cur,gt3.utilization",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "1200, 300, 1200, 300, ");
    let o =
        Runner::fixture("b65-g31-k7.1").run(&["query", "--fields", "gt5.clock.cur", "--no-header"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "N/A\n");
}
