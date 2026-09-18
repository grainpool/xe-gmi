mod common;
use common::*;

#[test]
fn info_new_sections_by_name() {
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "info",
        "--section",
        "topology,connectors,sriov,crash",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "    Topology\n        NUMA node               : 0");
    assert_contains(
        &s,
        "        HDMI-A-1                : connected, enabled, 3 modes",
    );
    assert_contains(&s, "        VFs                     : 0 of 7 enabled");
    assert_contains(&s, "    Crash dumps                 : none pending");
    assert_not_contains(&s, "    Power\n");
}

#[test]
fn info_json_new_objects() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["info", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"topology\": {");
    assert_contains(&s, "\"connectors\": [");
    assert_contains(&s, "\"sriov\": {");
    assert_contains(&s, "\"crash_dumps\": [");
    assert_contains(&s, "\"aer\": null");
}

#[test]
fn display_query_fields() {
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "query",
        "--fields",
        "display.connected,display.active,display.connectors,crash.pending,health.survivability",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "1, 1, 2, 0, no\n");
}
