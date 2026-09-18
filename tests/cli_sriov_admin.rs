mod common;
use common::*;

const F: &str = "b65-g31-k7.1";
const PCI: &str = "0000:e3:00.0";

#[test]
fn sriov_enable_disable() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    let o = r.run(&["sriov", "enable", "2"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read("sriov_numvfs", PCI), "2");
    assert_contains(&stdout(&o), "GPU 0 [0000:e3:00.0]: sriov_numvfs set to 2");
    let o = r.run(&["sriov", "enable", "9"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o)); // above sriov_totalvfs
    let o = r.run(&["sriov", "disable"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read("sriov_numvfs", PCI), "0");
}

#[test]
fn sriov_enable_refuses_when_vfs_already_enabled() {
    let t = writable_copy("srv-2gpu-k7.2");
    let o = Runner::temp(&t, "srv-2gpu-k7.2").run(&["-i", "0", "sriov", "enable", "3"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_contains(&stderr(&o), "disable");
}

#[test]
fn sriov_vf_set_profile() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    let o = r.run(&[
        "sriov",
        "vf",
        "1",
        "set",
        "--vram",
        "8G",
        "--quantum",
        "10ms",
        "--timeout",
        "20000us",
        "--priority",
        "normal",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        t.read("sriov_admin/vf1/profile/vram_quota", PCI),
        "8589934592"
    );
    assert_eq!(t.read("sriov_admin/vf1/profile/exec_quantum_ms", PCI), "10");
    assert_eq!(
        t.read("sriov_admin/vf1/profile/preempt_timeout_us", PCI),
        "20000"
    );
    assert_eq!(
        t.read("sriov_admin/vf1/profile/sched_priority", PCI),
        "normal"
    );
    assert_eq!(t.read("sriov_admin/vf2/profile/vram_quota", PCI), "0");
    let o = r.run(&["sriov", "vf", "8", "set", "--vram", "8G"]);
    assert_eq!(code(&o), 2);
    let o = r.run(&["sriov", "vf", "1", "set"]);
    assert_eq!(code(&o), 2); // nothing to set
    let o = r.run(&["sriov", "vf", "1", "set", "--priority", "urgent"]);
    assert_eq!(code(&o), 2);
}

#[test]
fn sriov_vf_stop_requires_force() {
    let t = writable_copy("srv-2gpu-k7.2");
    let r = Runner::temp(&t, "srv-2gpu-k7.2");
    let o = r.run(&["-i", "0", "sriov", "vf", "1", "stop"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_eq!(t.read("sriov_admin/vf1/stop", "0000:17:00.0"), "");
    let o = r.run(&["-i", "0", "sriov", "vf", "1", "stop", "--force"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read("sriov_admin/vf1/stop", "0000:17:00.0"), "1");
}

#[test]
fn sriov_bulk_profile() {
    let t = writable_copy(F);
    let o = Runner::temp(&t, F).run(&["sriov", "vf", "all", "set", "--quantum", "5ms"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        t.read("sriov_admin/.bulk_profile/exec_quantum_ms", PCI),
        "5"
    );
    assert_eq!(t.read("sriov_admin/vf1/profile/exec_quantum_ms", PCI), "0"); // bulk file, not per-VF files
}

#[test]
fn sriov_ops_not_persisted() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    assert_eq!(code(&r.run(&["sriov", "enable", "1"])), 0);
    assert_eq!(
        code(&r.run(&["sriov", "vf", "1", "set", "--vram", "8G"])),
        0
    );
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_not_contains(&conf, "sriov");
    assert_not_contains(&conf, "vf1");
}
