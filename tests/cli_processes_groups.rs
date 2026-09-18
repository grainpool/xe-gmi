mod common;
use common::*;

#[test]
fn default_table_unchanged_with_shared_client() {
    // pid 4301 shares client 7 with 4300: the default table still shows one row per client, first pid
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "processes_b65.txt");
}

#[test]
fn group_by_client_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes", "--group-by", "client"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "processes_client_b65.txt");
}

#[test]
fn group_by_user() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes", "--group-by", "user"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert!(s.lines().next().unwrap().starts_with("UID "), "{s}");
    assert_eq!(
        s.lines().count(),
        2,
        "both clients belong to uid 1000:\n{s}"
    );
    assert_contains(&s, "1000 ");
}

#[test]
fn processes_json_has_pids_cgroup_and_memory_regions() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"pids\": [4300, 4301]");
    assert_contains(
        &s,
        "\"cgroup\": \"/user.slice/user-1000.slice/session-2.scope\"",
    );
    assert_contains(&s, "\"uid\": 1000");
    assert_contains(&s, "\"gtt\": {");
    assert_contains(&s, "\"resident_bytes\": 196608");
    assert_contains(&s, "\"shared_bytes\": 67108864");
}

#[test]
fn pmon_is_processes_in_loop_mode() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["pmon", "-u", "0.05", "--count", "2"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_eq!(s.matches("PID    NAME").count(), 2, "{s}");
}
