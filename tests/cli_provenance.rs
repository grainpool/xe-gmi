mod common;
use common::*;

#[test]
fn fields_show_source_and_access_columns() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["fields"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    let hdr = s.lines().next().unwrap();
    assert!(hdr.starts_with("FIELD"), "{hdr}");
    for col in ["UNIT", "AVAILABLE", "SOURCE", "ACCESS", "DESCRIPTION"] {
        assert_contains(hdr, col);
    }
    let l = s.lines().find(|l| l.starts_with("memory.used ")).unwrap();
    assert_contains(l, "fdinfo");
    let l = s.lines().find(|l| l.starts_with("power.limit ")).unwrap();
    assert_contains(l, "sysfs");
    assert_contains(l, "root");
}

#[test]
fn fields_json_provenance_and_unavailable_reasons() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["fields", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"source\": \"fdinfo\"");
    assert_contains(&s, "\"scope\": \"visible-clients\"");
    assert_contains(&s, "\"access\": \"user\"");
    assert_contains(&s, "\"quality\": \"partial\"");
    let o = Runner::fixture("b580-g21-k6.14").run(&["info", "--json"]);
    let s = stdout(&o);
    assert_contains(&s, "\"unavailable\": [");
    assert_contains(&s, "\"source\": \"sysfs\"");
}

#[test]
fn verbose_text_annotates_provenance() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["-v", "info", "--section", "memory"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(
        &stdout(&o),
        "[source: fdinfo, scope: visible-clients, quality: partial]",
    );
}
