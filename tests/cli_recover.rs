mod common;
use common::*;

const F: &str = "b65-g31-k7.1";
const PCI: &str = "0000:e3:00.0";

#[test]
fn recover_dry_run_golden_and_refusal() {
    let o = Runner::fixture(F).run(&["recover", "--dry-run"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_golden(&stdout(&o), "recover_dryrun_b65.txt");
    // without --dry-run the same plan is printed and nothing is written
    let t = writable_copy(F);
    let o = Runner::temp(&t, F).run(&["recover"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_eq!(t.read("reset", PCI), "");
}

#[test]
fn recover_force_flr_sequence() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    let o = r.run(&["recover", "--force"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read_abs("sys/bus/pci/drivers/xe/unbind"), PCI);
    assert_eq!(t.read("reset_method", PCI), "flr");
    assert_eq!(t.read("reset", PCI), "1");
    assert_eq!(t.read_abs("sys/bus/pci/drivers/xe/bind"), PCI);
    let s = stdout(&o);
    assert_contains(&s, "unbound xe");
    assert_contains(&s, "reset (flr) done");
    assert_contains(&s, "bound xe");
    assert_contains(&s, "recovered 0000:e3:00.0");
}

#[test]
fn recover_rebind_method_skips_reset() {
    let t = writable_copy(F);
    let o = Runner::temp(&t, F).run(&["recover", "--force", "--method", "rebind"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read("reset", PCI), "");
    assert_eq!(t.read_abs("sys/bus/pci/drivers/xe/bind"), PCI);
}

#[test]
fn recover_hard_preconditions() {
    // enabled VFs: hard refusal even with --force
    let t = writable_copy("srv-2gpu-k7.2");
    let o = Runner::temp(&t, "srv-2gpu-k7.2").run(&["-i", "0", "recover", "--force"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_contains(
        &stdout(&o),
        "enabled VFs              : 2 -> refused (disable VFs first)",
    );
    // bus method with another function on the bus (the two VFs are 0000:17:00.1/.2): hard refusal
    let o = Runner::temp(&t, "srv-2gpu-k7.2")
        .run(&["-i", "1", "recover", "--force", "--method", "bus"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o)); // GPU 1 shares its bus with nothing
    let o = Runner::temp(&t, "srv-2gpu-k7.2")
        .run(&["-i", "0", "recover", "--force", "--method", "bus"]);
    assert_eq!(code(&o), 5);
    // unknown method
    let o = Runner::temp(&t, "srv-2gpu-k7.2").run(&["-i", "1", "recover", "--method", "warm"]);
    assert_eq!(code(&o), 2);
}

#[test]
fn recover_saves_pending_crash_first() {
    let t = writable_copy("srv-2gpu-k7.2");
    let r = Runner::temp(&t, "srv-2gpu-k7.2");
    let o = r.run(&["-i", "2", "recover", "--force"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "saved crash dump 1 (");
    assert_contains(&s, "xe-crash-0000:67:00.0-");
    assert_eq!(t.read_abs("sys/class/devcoredump/devcd1/data"), "1"); // released after saving
    let o = r.run(&["-i", "2", "recover", "--force", "--no-save-crash"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn recover_reapplies_persisted_settings() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    assert_eq!(code(&r.run(&["set", "power-limit", "150"])), 0);
    std::fs::write(t.device(PCI).join("hwmon/hwmon7/power1_cap"), "200000000\n").unwrap();
    let o = r.run(&["recover", "--force"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read("hwmon/hwmon7/power1_cap", PCI), "150000000");
    assert_contains(&stdout(&o), "persist apply: 1 ok");
}

#[test]
fn recover_needs_root() {
    if is_root() {
        return;
    }
    let t = writable_copy(F);
    t.as_uid(1000);
    let p = t.tree().join("sys/bus/pci/drivers/xe/unbind");
    std::fs::write(&p, "").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
    let o = Runner::temp(&t, F).run(&["recover", "--force"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_contains(&stderr(&o), "sudo xe-gmi recover --force");
}
