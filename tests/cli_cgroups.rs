mod common;
use common::*;

#[test]
fn cgroups_b65_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["cgroups"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "cgroups_b65.txt");
}

#[test]
fn cgroups_absent_controller() {
    // the b580 tree has no sys/fs/cgroup at all
    let o = Runner::fixture("b580-g21-k6.14").run(&["cgroups"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_contains(&stderr(&o), "dmem");
}

#[test]
fn cgroups_json_and_processes_by_cgroup() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["cgroups", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "\"path\": \"/system.slice/llama.service\"");
    assert_contains(&s, "\"current_bytes\": 1073741824");
    assert_contains(&s, "\"max_bytes\": 21474836480");
    assert_contains(&s, "\"max_bytes\": null"); // "max" limit
    let o = Runner::fixture("b65-g31-k7.1").run(&["processes", "--group-by", "cgroup"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "processes_cgroup_b65.txt");
}

#[test]
fn cgroups_multi_device_sums_per_region() {
    let o = Runner::fixture("srv-2gpu-k7.2").run(&["cgroups"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "/system.slice/llama.service");
    assert_contains(&s, "8192 MiB");
    assert_contains(&s, "16384 MiB");
    assert_contains(&s, "Capacity: 32768 MiB (drm/0000:17:00.0/vram0)");
}
