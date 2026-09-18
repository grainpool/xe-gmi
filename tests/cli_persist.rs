mod common;
use common::*;

const F: &str = "b65-g31-k7.1";
const PCI: &str = "0000:e3:00.0";

#[test]
fn persist_install_writes_rule_and_state_file_idempotently() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    let o = r.run(&["persist", "install"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let rule = t.etc().join("udev/rules.d/90-xe-gmi-persist.rules");
    let conf = t.etc().join("xe-gmi/persist.conf");
    assert!(rule.exists());
    assert!(conf.exists());
    assert_golden(&std::fs::read_to_string(&rule).unwrap(), "persist_rule.txt");
    assert_contains(&std::fs::read_to_string(&conf).unwrap(), "format=1");
    assert_contains(&stdout(&o), "installed");
    // second install: unchanged, still exit 0
    let o = r.run(&["persist", "install"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "unchanged");
}

#[test]
fn persist_install_refuses_home_directory_binaries() {
    let t = writable_copy(F);
    let o = Runner::temp(&t, F)
        .env("XE_GMI_EXE_PATH", "/home/user/.cargo/bin/xe-gmi")
        .run(&["persist", "install"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_contains(&stderr(&o), "/usr/local/bin");
    assert_contains(&stderr(&o), "--exe");
    let o = Runner::temp(&t, F)
        .env("XE_GMI_EXE_PATH", "/home/user/.cargo/bin/xe-gmi")
        .run(&["persist", "install", "--exe", "/opt/xe-gmi/bin/xe-gmi"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(
        &std::fs::read_to_string(t.etc().join("udev/rules.d/90-xe-gmi-persist.rules")).unwrap(),
        "/opt/xe-gmi/bin/xe-gmi persist apply",
    );
}

#[test]
fn set_is_sticky_when_persistence_is_installed() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    let o = r.run(&["set", "power-limit", "150"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "persistence: recorded");
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_contains(&conf, "0000:e3:00.0 pl2.card 150000000");
    // clocks and profile too
    assert_eq!(code(&r.run(&["set", "clocks", "--max", "2000"])), 0);
    assert_eq!(
        code(&r.run(&["set", "power-profile", "power-saving", "--gt", "1"])),
        0
    );
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_contains(&conf, "0000:e3:00.0 gt0.max_freq 2000");
    assert_contains(&conf, "0000:e3:00.0 gt1.max_freq 2000");
    assert_contains(&conf, "0000:e3:00.0 gt1.power_profile power_saving");
    assert_not_contains(&conf, "gt0.power_profile");
    // re-setting replaces the line instead of appending
    assert_eq!(code(&r.run(&["set", "power-limit", "160"])), 0);
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_eq!(conf.matches("pl2.card").count(), 1, "{conf}");
    assert_contains(&conf, "pl2.card 160000000");
}

#[test]
fn set_no_persist_and_persist_flags() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    assert_eq!(
        code(&r.run(&["set", "power-limit", "150", "--no-persist"])),
        0
    );
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_not_contains(&conf, "pl2.card");
    // --persist without installation is an error (exit 5) and must not write the limit either
    let t2 = writable_copy(F);
    let o = Runner::temp(&t2, F).run(&["set", "power-limit", "150", "--persist"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_contains(&stderr(&o), "persist install");
    assert_eq!(t2.read("hwmon/hwmon7/power1_cap", PCI), "200000000");
}

#[test]
fn reset_removes_persisted_lines() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    assert_eq!(code(&r.run(&["set", "power-limit", "150"])), 0);
    assert_eq!(code(&r.run(&["set", "clocks", "--max", "2000"])), 0);
    assert_eq!(code(&r.run(&["reset", "clocks"])), 0);
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_not_contains(&conf, "max_freq");
    assert_contains(&conf, "pl2.card 150000000");
    assert_eq!(code(&r.run(&["reset", "all"])), 0);
    let conf = std::fs::read_to_string(t.etc().join("xe-gmi/persist.conf")).unwrap();
    assert_not_contains(&conf, "0000:e3:00.0 ");
}

#[test]
fn persist_apply_from_udev_devpath_applies_only_that_device_and_logs() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    assert_eq!(code(&r.run(&["set", "power-limit", "150"])), 0);
    assert_eq!(code(&r.run(&["set", "clocks", "--min", "900"])), 0);
    // simulate a fresh boot: firmware defaults are back, new boot_id, no defaults record
    std::fs::write(t.device(PCI).join("hwmon/hwmon7/power1_cap"), "200000000\n").unwrap();
    std::fs::write(t.device(PCI).join("tile0/gt0/freq0/min_freq"), "1200\n").unwrap();
    std::fs::write(
        t.proc_().join("sys/kernel/random/boot_id"),
        "b0b0b0b0-0000-4000-8000-000000000002\n",
    )
    .unwrap();
    let o = r.run(&[
        "persist",
        "apply",
        "--udev-devpath",
        "/devices/pci0000:e3/0000:e3:00.0",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read("hwmon/hwmon7/power1_cap", PCI), "150000000");
    assert_eq!(t.read("tile0/gt0/freq0/min_freq", PCI), "900");
    // boot defaults of the new boot were recorded before applying
    let rec = t
        .state()
        .join("boot/b0b0b0b0-0000-4000-8000-000000000002/0000:e3:00.0.defaults");
    assert_contains(
        &std::fs::read_to_string(&rec).unwrap(),
        "pl2.card 200000000",
    );
    assert_contains(&std::fs::read_to_string(&rec).unwrap(), "gt0.min_freq 1200");
    // log line written
    let log = std::fs::read_to_string(t.state().join("persist.log")).unwrap();
    assert_contains(&log, "0000:e3:00.0 pl2.card 150000000 ok");
    assert_contains(&log, "0000:e3:00.0 gt0.min_freq 900 ok");
    // a devpath for a different device applies nothing
    std::fs::write(t.device(PCI).join("hwmon/hwmon7/power1_cap"), "200000000\n").unwrap();
    let o = r.run(&[
        "persist",
        "apply",
        "--udev-devpath",
        "/devices/pci0000:01/0000:01:00.0",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(t.read("hwmon/hwmon7/power1_cap", PCI), "200000000");
}

#[test]
fn persist_apply_tolerates_missing_attribute_and_reports() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    // hand-edit the state file with a key that does not exist on this card
    let conf = t.etc().join("xe-gmi/persist.conf");
    let mut s = std::fs::read_to_string(&conf).unwrap();
    s.push_str("0000:e3:00.0 pl1.card 100000000\n0000:e3:00.0 gt0.max_freq 2000\n");
    std::fs::write(&conf, s).unwrap();
    let o = r.run(&["persist", "apply"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o)); // apply never fails the whole run for one bad line
    assert_eq!(t.read("tile0/gt0/freq0/max_freq", PCI), "2000");
    let log = std::fs::read_to_string(t.state().join("persist.log")).unwrap();
    assert_contains(&log, "pl1.card 100000000 skipped");
}

#[test]
fn persist_show_and_remove() {
    let t = writable_copy(F);
    let r = Runner::temp(&t, F);
    assert_eq!(code(&r.run(&["persist", "install"])), 0);
    assert_eq!(code(&r.run(&["set", "power-limit", "150"])), 0);
    let o = r.run(&["persist", "show"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "installed");
    assert_contains(&s, "0000:e3:00.0 pl2.card 150000000");
    assert_contains(&s, "Last apply");
    let o = r.run(&["persist", "remove"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!t
        .etc()
        .join("udev/rules.d/90-xe-gmi-persist.rules")
        .exists());
    assert!(
        t.etc().join("xe-gmi/persist.conf").exists(),
        "state kept without --purge"
    );
    let o = r.run(&["persist", "show"]);
    assert_contains(&stdout(&o), "not installed");
    let o = r.run(&["persist", "remove", "--purge"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!t.etc().join("xe-gmi/persist.conf").exists());
}

#[test]
fn persist_apply_without_installation_is_exit_5() {
    let t = writable_copy(F);
    let o = Runner::temp(&t, F).run(&["persist", "apply"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
}
