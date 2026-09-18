//! The parked-card fixture: one B65 in D3cold with the driver's runtime-PM state wedged in
//! `error`, the KB 000094587 endpoint artifact, powered-down hwmon sentinels, a lying
//! `idle_status` mirror and kmsg underflow records. Locks the 0.2.2 honesty gates.
mod common;
use common::*;

#[test]
fn d3cold_pcie_golden() {
    let o = Runner::fixture("b65-g31-k7.1-d3cold").kabi().run(&["pcie"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "pcie_d3cold.txt");
}

#[test]
fn d3cold_info_golden() {
    let o = Runner::fixture("b65-g31-k7.1-d3cold").kabi().run(&["info"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "info_d3cold.txt");
}

#[test]
fn d3cold_doctor_shows_power_state_and_kmsg_underflow() {
    let o = Runner::fixture("b65-g31-k7.1-d3cold")
        .kabi()
        .run(&["doctor"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "power state");
    assert_contains(&s, "D3cold / runtime error / D3cold allowed yes");
    assert_contains(&s, "aspm");
    assert_contains(
        &s,
        "policy powersupersave, L1 endpoint disabled, L1 root port enabled",
    );
    assert_contains(&s, "Runtime PM usage count underflow!");
}

#[test]
fn d3cold_query_gates_sentinels_and_derives_idle() {
    // utilization comes from the -t1 window; temps/fan/draw are powered-down sentinels and
    // must gate to N/A; idle.status is derived (act_freq 0) although the raw mirror lies.
    let o = Runner::fixture("b65-g31-k7.1-d3cold").kabi().run(&[
        "query",
        "--fields",
        "utilization.gt,idle.status,pci.power_state,pci.runtime_status,pci.d3cold_allowed,pci.aspm.policy,temp.pkg,fan.rpm,power.draw",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "0.0, gt-c6, D3cold, error, yes, powersupersave, N/A, N/A, N/A\n"
    );
}

#[test]
fn d3cold_huc_zero_version_is_not_a_version() {
    let o = Runner::fixture("b65-g31-k7.1-d3cold")
        .kabi()
        .run(&["firmware"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "GuC (submission)         : 70.44.1");
    assert_contains(&s, "HuC                      : N/A");
    assert_not_contains(&s, "0.0.0");
}
