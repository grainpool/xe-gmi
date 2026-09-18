mod common;
use common::*;

#[test]
fn topology_b65_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["topology"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "topology_b65.txt");
}

#[test]
fn topology_server_matrix_golden() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["topology"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "topology_srv2.txt");
}

#[test]
fn topology_never_lists_other_vendors() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["topology"]);
    let s = stdout(&o);
    assert_not_contains(&s, "0000:01:00.0");
    assert_not_contains(&s, "0000:81:00.0");
}

#[test]
fn topology_json_and_query_fields() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["topology", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"numa_node\": 1");
    assert_contains(&s, "\"iommu_group\": 32");
    assert_contains(&s, "\"root_port\": \"0000:64:01.0\"");
    assert_contains(&s, "\"affinity\"");
    assert_contains(&s, "\"PIX\"");
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "query",
        "--fields",
        "pci.numa_node,pci.local_cpus,pci.iommu_group,pci.root_port,sriov.total_vfs,sriov.num_vfs",
        "--no-header",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "0, 0-31, 42, N/A, 7, 0\n");
}

#[test]
fn topology_missing_files_degrade() {
    // no-gpu has no devices: exit 3 as every read command
    let o = Runner::fixture("no-gpu").run(&["topology"]);
    assert_eq!(code(&o), 3);
    // dg2 tree: NUMA/IOMMU present, no SR-IOV files at all
    let o = Runner::fixture("dg2-a770-k6.12-forced").run(&["topology"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "SR-IOV                   : N/A");
}
