mod common;
use common::*;

#[test]
fn crash_list_goldens() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["crash", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "crash_list_srv2.txt");
    let o = Runner::fixture("b65-g31-k7.1").run(&["crash", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "crash_list_b65.txt");
}

#[test]
fn crash_show_prints_the_dump() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["crash", "show", "1"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert!(s.starts_with("**** Xe Device Coredump ****\n"), "{s}");
    assert_contains(&s, "Reason: GuC timeout on gt0 (exec queue 12)");
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["crash", "show", "9"]);
    assert_eq!(code(&o), 5);
    assert_contains(&stderr(&o), "no crash dump with id 9");
}

#[test]
fn crash_save_and_release() {
    let t = writable_copy("srv-2gpu-k7.2");
    let r = Runner::temp(&t, "srv-2gpu-k7.2");
    let out = t.root.join("dump.txt");
    let o = r.run(&["crash", "save", "1", "--out", out.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let saved = std::fs::read_to_string(&out).unwrap();
    assert!(saved.starts_with("**** Xe Device Coredump ****"));
    assert_contains(
        &stdout(&o),
        &format!(
            "saved crash dump 1 ({} bytes) to {}",
            saved.len(),
            out.display()
        ),
    );
    let o = r.run(&["crash", "release", "1"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "released crash dump 1");
    // releasing = writing to data; the fixture shows the write
    assert_eq!(t.read_abs("sys/class/devcoredump/devcd1/data"), "1");
}

#[test]
fn doctor_warns_about_pending_dump_and_wedged_kmsg() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["doctor"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(
        &s,
        "crash dumps                 : 1 pending (devcd1) — run: xe-gmi crash save 1",
    );
    assert_contains(
        &s,
        "kernel log (wedged)         : 0000:67:00.0 device wedged, needs recovery",
    );
}

#[test]
fn crash_json() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["crash", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"id\": 1");
    assert_contains(&s, "\"pci_address\": \"0000:67:00.0\"");
    assert_contains(&s, "\"reason\": \"GuC timeout on gt0 (exec queue 12)\"");
}
