mod common;
use common::*;

#[test]
fn events_replay_golden() {
    let o = Runner::fixture("srv-2gpu-k7.2").kabi().run(&["events"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "events_srv2.txt");
}

#[test]
fn events_json_lines() {
    let o = Runner::fixture("srv-2gpu-k7.2")
        .kabi()
        .run(&["events", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_eq!(s.lines().count(), 4, "{s}");
    for l in s.lines() {
        assert!(
            l.starts_with("{\"schema_version\": 1, ") && l.ends_with('}'),
            "{l}"
        );
    }
    assert_contains(&s, "\"event\": \"wedged\"");
    assert_contains(&s, "\"recovery\": [\"rebind\", \"bus-reset\"]");
}

#[test]
fn events_need_root_without_replay() {
    let t = writable_copy("b65-g31-k7.1");
    t.as_uid(1000);
    let o = Runner::temp(&t, "b65-g31-k7.1").run(&["events"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_contains(&stderr(&o), "sudo xe-gmi events");
}
