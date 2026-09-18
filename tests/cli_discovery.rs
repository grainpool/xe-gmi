mod common;
use common::*;

#[test]
fn list_b65_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "list_b65.txt");
}

#[test]
fn other_vendors_are_never_enumerated() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["list"]);
    let s = stdout(&o);
    assert_not_contains(&s, "0000:01:00.0");
    assert_not_contains(&s, "0000:81:00.0");
    assert_not_contains(&s, "10de");
}

#[test]
fn no_gpu_exits_3() {
    let o = Runner::fixture("no-gpu").run(&["list"]);
    assert_eq!(code(&o), 3);
    assert_contains(&stderr(&o), "no xe device");
    assert_contains(&stderr(&o), "xe-gmi doctor");
}

#[test]
fn unbound_device_exits_3_with_pointer_to_doctor() {
    let o = Runner::fixture("no-xe-ubuntu-6.8").run(&["status"]);
    assert_eq!(code(&o), 3);
    let e = stderr(&o);
    assert_contains(&e, "no xe device");
    assert_contains(&e, "xe-gmi doctor");
}

#[test]
fn device_selector_forms() {
    for sel in ["0", "0000:e3:00.0", "e3:00.0"] {
        let o = Runner::fixture("b65-g31-k7.1").run(&["-i", sel, "list"]);
        assert_eq!(code(&o), 0, "selector {sel}: {}", stderr(&o));
        assert_contains(&stdout(&o), "0000:e3:00.0");
    }
}

#[test]
fn device_selector_miss_exits_3() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["-i", "1", "list"]);
    assert_eq!(code(&o), 3);
    assert_contains(&stderr(&o), "no xe device matches");
    let o = Runner::fixture("b65-g31-k7.1").run(&["-i", "0000:01:00.0", "list"]);
    assert_eq!(code(&o), 3);
    assert_contains(&stderr(&o), "not an xe device");
}

#[test]
fn ordering_and_indices_follow_pci_address() {
    // multi-tile fixture has a single device at 0000:17:00.0 -> index 0
    let o = Runner::fixture("multi-tile-2x2-k7.1").run(&["list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).starts_with("0  0000:17:00.0"), "{}", stdout(&o));
}

#[test]
fn dg2_forced_is_a_normal_xe_device() {
    let o = Runner::fixture("dg2-a770-k6.12-forced").run(&["list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "DG2 [Arc A770]");
}
