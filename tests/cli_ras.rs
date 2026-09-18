mod common;
use common::*;

#[test]
fn ras_golden_server() {
    let o = Runner::fixture("srv-2gpu-k7.2").kabi().run(&["ras"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "ras_srv2.txt");
}

#[test]
fn ras_unavailable_reasons() {
    // replay present but no ras/ directory: family missing -> exit 5 naming the kernel version
    let o = Runner::fixture("b65-g31-k7.1").kabi().run(&["ras"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_contains(&stderr(&o), "drm-ras");
    assert_contains(&stderr(&o), "7.2");
}

#[test]
fn ras_clear_records_and_json() {
    let t = writable_copy("srv-2gpu-k7.2");
    let r = Runner::temp(&t, "srv-2gpu-k7.2").kabi();
    let o = r.run(&["-i", "0", "ras", "--clear"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let log = std::fs::read_to_string(t.tree().join("kabi/ras/clear.log")).unwrap();
    assert_contains(&log, "node 1 error 1");
    assert_contains(&log, "node 2 error 2");
    assert_not_contains(&log, "node 3");
    assert_contains(&stdout(&o), "cleared 4 counters on 0000:17:00.0");
    let o = r.run(&["ras", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"node\": \"correctable-errors\"");
    assert_contains(&s, "\"soc-internal\": 3");
}

#[test]
fn ras_fields() {
    let o = Runner::fixture("srv-2gpu-k7.2").kabi().run(&[
        "-i",
        "0",
        "query",
        "--fields",
        "ras.correctable,ras.uncorrectable",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "3, 0\n");
}
