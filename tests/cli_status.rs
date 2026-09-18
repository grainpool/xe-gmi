mod common;
use common::*;

#[test]
fn status_b65_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "status_b65.txt");
}

#[test]
fn bare_invocation_is_status() {
    let a = Runner::fixture("b65-g31-k7.1").run(&[]);
    let b = Runner::fixture("b65-g31-k7.1").run(&["status"]);
    assert_eq!(code(&a), 0);
    assert_eq!(stdout(&a), stdout(&b));
}

#[test]
fn status_b580_k614_degrades_gracefully() {
    // 6.14 + BMG: no temps, no writable limit, no fan, no power_profile, no processes, non-root.
    let o = Runner::fixture("b580-g21-k6.14").run(&["status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "Battlemage G21 [Arc B580]");
    assert_contains(&s, "| N/A      |"); // temp cell entirely N/A
    assert_contains(&s, "0.00 / N/A"); // draw computed from two real samples (no delta), no limit
    assert_contains(&s, "0 / 12288"); // no visible clients, 12 GiB BAR
    assert_contains(&s, "100.0"); // idle residency did not advance => fully active
    assert_contains(&s, "| N/A          |"); // profile column when power_profile is absent
    assert_not_contains(&s, "panicked");
}

#[test]
fn status_both_limits_prefers_pl1_and_shows_throttle_reasons() {
    let o = Runner::fixture("bmg-both-pl1-pl2-k7.0").run(&["status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "/ 190.00"); // PL1 (power1_max) is the effective limit when present
    assert_contains(&s, "pl1,thermal");
    assert_contains(&s, "power_saving");
    assert_contains(&s, "45 / 42");
}

#[test]
fn status_update_loop_honours_count() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["-u", "0.05", "--count", "2", "status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_eq!(
        s.matches(&format!("xe-gmi {} | driver xe", env!("CARGO_PKG_VERSION")))
            .count(),
        2,
        "{s}"
    );
}

#[test]
fn status_json_has_schema_and_device() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["status", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"schema_version\": 1");
    assert_contains(&s, "\"pci_address\": \"0000:e3:00.0\"");
    assert_contains(&s, "\"draw_w\": 12.40");
    assert_contains(&s, "\"total_bytes\": 34359738368");
    assert!(s.trim_end().ends_with('}'), "JSON must be a single object");
}
