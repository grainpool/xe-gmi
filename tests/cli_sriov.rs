mod common;
use common::*;

#[test]
fn sriov_status_goldens() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["sriov", "status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "sriov_status_b65.txt");
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["-i", "0", "sriov", "status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "sriov_status_srv2_gpu0.txt");
}

#[test]
fn sriov_status_without_pf_support() {
    let o = Runner::fixture("dg2-a770-k6.12-forced").run(&["sriov", "status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(
        &stdout(&o),
        "N/A (no SR-IOV capability exposed for this device)",
    );
}

#[test]
fn sriov_json() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["-i", "0", "sriov", "status", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"total_vfs\": 7");
    assert_contains(&s, "\"num_vfs\": 2");
    assert_contains(&s, "\"vram_quota_bytes\": 4294967296");
    assert_contains(&s, "\"sched_priority\": \"normal\"");
    assert_contains(&s, "\"driver\": \"vfio-pci\"");
}
