mod common;
use common::*;

#[test]
fn processes_b65_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "processes_b65.txt");
}

#[test]
fn processes_sort_by_pid_and_util() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes", "--sort", "pid"]);
    let s = stdout(&o);
    let first = s.lines().nth(1).unwrap();
    assert!(first.starts_with("4242"), "{s}");
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes", "--sort", "util"]);
    let s = stdout(&o);
    assert!(s.lines().nth(1).unwrap().starts_with("4242"), "{s}");
}

#[test]
fn processes_ignores_zombies_and_foreign_gpus() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes"]);
    let s = stdout(&o);
    assert_not_contains(&s, "5000"); // zombie
    assert_not_contains(&s, "6100"); // uses the NVIDIA card only
    assert_not_contains(&s, "cuda-app");
}

#[test]
fn processes_json() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"pid\": 4242");
    assert_contains(&s, "\"client_id\": 42");
    assert_contains(&s, "\"resident_vram_bytes\": 1073741824");
    assert_contains(&s, "\"rcs\": 20.0");
}

#[test]
fn processes_none_visible() {
    let o = Runner::fixture("b580-g21-k6.14").run(&["processes"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_eq!(s.lines().count(), 2, "{s}"); // header + note
    assert_contains(&s, "no visible DRM clients");
    assert_contains(&s, "root"); // fixture is uid 1000 -> hint to run as root
}
