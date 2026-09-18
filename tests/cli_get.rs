mod common;
use common::*;

#[test]
fn get_clocks_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["get", "clocks"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "get_clocks_b65.txt");
}

#[test]
fn get_power_limit_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["get", "power-limit"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "get_power_limit_b65.txt");
}

#[test]
fn get_power_profile_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["get", "power-profile"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "get_power_profile_b65.txt");
}

#[test]
fn get_clocks_shows_recorded_boot_default_after_a_set() {
    let t = writable_copy("b65-g31-k7.1");
    let r = Runner::temp(&t, "b65-g31-k7.1");
    assert_eq!(code(&r.run(&["set", "clocks", "--max", "2000"])), 0);
    let o = r.run(&["get", "clocks"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(
        &s,
        "gt0  render  1200  2000  300   1800  2400  2850  1200/2850",
    );
    assert_contains(
        &s,
        "gt1  media   300   2000  300   1500  2100  2400  300/2400",
    );
}

#[test]
fn get_power_profile_unavailable_on_old_kernel() {
    let o = Runner::fixture("b580-g21-k6.14").run(&["get", "power-profile"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(
        &stdout(&o),
        "gt0  N/A  (power_profile not exposed: kernel 6.18+)",
    );
}

#[test]
fn get_json() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["get", "power-limit", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"schema_version\": 1");
    assert_contains(&s, "\"effective_w\": 200.00");
    assert_contains(&s, "\"kind\": \"pl2\"");
}
