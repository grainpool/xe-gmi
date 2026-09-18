mod common;
use common::*;

#[test]
fn doctor_no_xe_ubuntu68_golden_exit_3() {
    let o = Runner::fixture("no-xe-ubuntu-6.8").run(&["doctor"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_golden(&stdout(&o), "doctor_no_xe_ubuntu68.txt");
}

#[test]
fn doctor_no_gpu_golden_exit_3() {
    let o = Runner::fixture("no-gpu").run(&["doctor"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_golden(&stdout(&o), "doctor_no_gpu.txt");
}

#[test]
fn doctor_b65_golden_exit_0() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["doctor"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "doctor_b65.txt");
}

#[test]
fn doctor_b580_k614_names_the_missing_kernel_features() {
    let o = Runner::fixture("b580-g21-k6.14").run(&["doctor"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "not writable");
    assert_contains(&s, "6.16");
    assert_contains(
        &s,
        "temperatures                : none exposed (kernel 6.15+)",
    );
    assert_contains(
        &s,
        "power profile               : not supported (kernel 6.18+)",
    );
}

#[test]
fn doctor_json() {
    let o = Runner::fixture("no-xe-ubuntu-6.8").run(&["doctor", "--json"]);
    assert_eq!(code(&o), 3);
    let s = stdout(&o);
    assert_contains(&s, "\"schema_version\": 1");
    assert_contains(&s, "\"usable_xe_devices\": 0");
    assert_contains(&s, "\"first_kernel\": \"6.17\"");
}
