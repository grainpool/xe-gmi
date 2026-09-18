mod common;
use common::*;

#[test]
fn pcie_b65_golden_no_aer() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["pcie"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "pcie_b65.txt");
}

#[test]
fn pcie_server_golden_with_aer() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["pcie"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "pcie_srv2.txt");
}

#[test]
fn pcie_query_fields_and_json() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&[
        "-i",
        "0",
        "query",
        "--fields",
        "pci.aer.correctable,pci.aer.nonfatal,pci.aer.fatal,pci.aer.rootport.correctable",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "5, 1, 0, 7\n");
    let o = Runner::fixture("srv-2gpu-k7.2").run(&[
        "-i",
        "1",
        "query",
        "--fields",
        "pci.aer.correctable",
        "--no-header",
    ]);
    assert_eq!(stdout(&o), "N/A\n");
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["pcie", "--json"]);
    let s = stdout(&o);
    assert_contains(&s, "\"endpoint\": {");
    assert_contains(&s, "\"RxErr\": 2");
    assert_contains(&s, "\"root_port\": \"0000:16:01.0\"");
}
