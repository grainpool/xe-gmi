//! Half B: DRM device queries through src/kabi with the replay seam (XE_GMI_KABI_REPLAY).
mod common;
use common::*;

#[test]
fn firmware_golden() {
    let o = Runner::fixture("b65-g31-k7.1").kabi().run(&["firmware"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "firmware_b65.txt");
}

#[test]
fn firmware_without_kabi_is_na_with_reason() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["firmware"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "GuC (submission)         : N/A");
    let o = Runner::fixture("b65-g31-k7.1").run(&["-v", "firmware"]);
    assert_contains(&stdout(&o), "render node");
}

#[test]
fn memory_from_kernel_regions() {
    let o = Runner::fixture("b65-g31-k7.1").kabi().run(&["query", "--fields", "memory.total,memory.used,memory.free,memory.total.source,memory.used.source,memory.cpu_visible.total,memory.cpu_visible.used", "--no-header"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "32768, 9412, 23356, drm-uapi, drm-uapi, 32768, 9412\n"
    );
    let o = Runner::fixture("b65-g31-k7.1")
        .kabi()
        .run(&["info", "--section", "memory"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "info_b65_memory_kabi.txt");
    // without the replay, 0.1.x behaviour is unchanged
    let o = Runner::fixture("b65-g31-k7.1").run(&[
        "query",
        "--fields",
        "memory.total,memory.used,memory.total.source,memory.used.source",
        "--no-header",
    ]);
    assert_eq!(stdout(&o), "32768, 1088, bar, fdinfo\n");
}

#[test]
fn status_uses_kernel_memory_when_available() {
    let o = Runner::fixture("b65-g31-k7.1").kabi().run(&["status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(&stdout(&o), "| 9412 / 32768   |");
    assert_contains(
        &stdout(&o),
        "Memory used = kernel allocator (every allocation on the device).",
    );
}

#[test]
fn topology_hardware_golden_and_json() {
    let o = Runner::fixture("b65-g31-k7.1")
        .kabi()
        .run(&["topology", "--hardware"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "topology_hw_b65.txt");
    let o = Runner::fixture("b65-g31-k7.1")
        .kabi()
        .run(&["topology", "--hardware", "--json"]);
    let s = stdout(&o);
    assert_contains(&s, "\"engines\": [");
    assert_contains(&s, "\"class\": \"ccs\"");
    assert_contains(&s, "\"reference_clock_hz\": 19200000");
    assert_contains(&s, "\"geometry_dss\": 20");
    assert_contains(&s, "\"ip_version\": \"20.1.0\"");
    let o = Runner::fixture("b65-g31-k7.1").run(&["topology", "--hardware"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_contains(&stderr(&o), "render node");
}

#[test]
fn config_query_in_info_device_section() {
    let o = Runner::fixture("b65-g31-k7.1")
        .kabi()
        .run(&["info", "--section", "device"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "        VA bits                 : 48");
    assert_contains(&s, "        Min alignment           : 64 KiB");
}

#[test]
fn doctor_reports_kabi_capabilities() {
    let o = Runner::fixture("b65-g31-k7.1").kabi().run(&["doctor"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(
        &s,
        "    kernel memory regions       : available (render node)",
    );
    assert_contains(
        &s,
        "    firmware versions           : GuC 70.44.1, HuC 9.4.13",
    );
}
