mod common;
use common::*;

const FIELDS: &str = "index,name,pci.address,temp.pkg,power.draw,power.limit,utilization.gt,memory.used,memory.total,clock.cur,clock.max,gt1.clock.cur,throttle.reasons,fan.rpm,pcie.link.gen.current,power.profile";

#[test]
fn query_csv_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["query", "--fields", FIELDS]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "query_b65.csv");
}

#[test]
fn query_no_header_no_units() {
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "query",
        "--fields",
        "temp.pkg,power.draw",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "41, 12.40\n");
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "query",
        "--fields",
        "temp.pkg,power.draw",
        "--no-units",
    ]);
    assert_eq!(stdout(&o), "temp.pkg, power.draw\n41, 12.40\n");
}

#[test]
fn query_unknown_field_is_usage_error() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["query", "--fields", "temp.pkg,bogus.field"]);
    assert_eq!(code(&o), 2);
    assert_contains(&stderr(&o), "bogus.field");
    assert_contains(&stderr(&o), "xe-gmi fields");
}

#[test]
fn query_na_fields_print_na() {
    let o = Runner::fixture("b580-g21-k6.14").run(&[
        "query",
        "--fields",
        "temp.pkg,power.limit,fan.rpm,power.profile,clock.mem",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "N/A, N/A, N/A, N/A, N/A\n");
}

#[test]
fn query_json_is_object_with_schema() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["query", "--fields", "index,temp.pkg", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"schema_version\": 1");
    assert_contains(&s, "\"temp.pkg\": 41");
}

#[test]
fn query_update_loop_repeats_rows_without_repeating_header() {
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "-u",
        "0.05",
        "--count",
        "3",
        "query",
        "--fields",
        "index,clock.cur",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_eq!(s.lines().count(), 4, "{s}");
    assert_eq!(s.lines().filter(|l| l.starts_with("index")).count(), 1);
}

#[test]
fn query_energy_and_pkg_fields() {
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "query",
        "--fields",
        "energy.card,energy.pkg,power.draw.pkg,temp.vram,memory.free",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "123481.589, 98785.432, 10.00, 38, 31680\n");
}
