mod common;
use common::*;

#[test]
fn set_power_window_writes_the_matching_interval() {
    let t = writable_copy("b65-g31-k7.1");
    let o = Runner::temp(&t, "b65-g31-k7.1").run(&["set", "power-window", "20ms"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        t.read("hwmon/hwmon7/power1_cap_interval", "0000:e3:00.0"),
        "20"
    );
    assert_contains(
        &stdout(&o),
        "GPU 0 [0000:e3:00.0]: power window (pl2 card) set to 20 ms",
    );
    let t = writable_copy("dg2-a770-k6.12-forced");
    let o = Runner::temp(&t, "dg2-a770-k6.12-forced").run(&["set", "power-window", "28s"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        t.read("hwmon/hwmon2/power1_max_interval", "0000:03:00.0"),
        "28000"
    );
    assert_contains(&stdout(&o), "(pl1 card) set to 28000 ms");
}

#[test]
fn set_power_window_rejects_bad_values_and_reports_rounding() {
    for bad in ["0", "abc", "-5ms", "5h"] {
        let t = writable_copy("b65-g31-k7.1");
        let o = Runner::temp(&t, "b65-g31-k7.1").run(&["set", "power-window", bad]);
        assert_eq!(code(&o), 2, "{bad}: {}", stderr(&o));
        assert_eq!(
            t.read("hwmon/hwmon7/power1_cap_interval", "0000:e3:00.0"),
            "15"
        );
    }
    let t = writable_copy("b65-g31-k7.1");
    t.readback("hwmon/hwmon7/power1_cap_interval", "0000:e3:00.0", "16");
    let o = Runner::temp(&t, "b65-g31-k7.1").run(&["set", "power-window", "17ms"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(
        &stdout(&o),
        "set to 16 ms (requested 17 ms; rounded by the firmware)",
    );
}

#[test]
fn power_window_is_recorded_persisted_and_reset() {
    let t = writable_copy("b65-g31-k7.1");
    let r = Runner::temp(&t, "b65-g31-k7.1");
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    assert_eq!(code(&r.run(&["set", "power-window", "20ms"])), 0);
    let rec = t
        .state()
        .join("boot/a1b2c3d4-0000-4000-8000-000000000001/0000:e3:00.0.defaults");
    assert_contains(
        &std::fs::read_to_string(&rec).unwrap(),
        "pl2.card.window 15",
    );
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_contains(&conf, "0000:e3:00.0 pl2.card.window 20");
    assert_eq!(code(&r.run(&["reset", "power-limit"])), 0);
    assert_eq!(
        t.read("hwmon/hwmon7/power1_cap_interval", "0000:e3:00.0"),
        "15"
    );
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_not_contains(&conf, "window");
}

#[test]
fn get_power_limit_shows_windows() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["get", "power-limit"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "pl2 card        : 200.00 W (window 15 ms)");
}
