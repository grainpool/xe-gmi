mod common;
use common::*;

#[test]
fn cgroup_set_vram_max_replaces_region_line() {
    let t = writable_copy("srv-2gpu-k7.2");
    let r = Runner::temp(&t, "srv-2gpu-k7.2");
    let o = r.run(&[
        "-i",
        "0",
        "cgroup",
        "set",
        "/system.slice/llama.service",
        "--vram-max",
        "8G",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let f = t.read_abs("sys/fs/cgroup/system.slice/llama.service/dmem.max");
    assert_contains(&f, "drm/0000:17:00.0/vram0 8589934592");
    assert_contains(&f, "drm/0000:19:00.0/vram0 max"); // other regions untouched
    assert_contains(
        &stdout(&o),
        "cgroup /system.slice/llama.service: dmem.max drm/0000:17:00.0/vram0 = 8192 MiB",
    );
    let o = r.run(&[
        "-i",
        "0",
        "cgroup",
        "set",
        "/system.slice/llama.service",
        "--vram-max",
        "max",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(
        &t.read_abs("sys/fs/cgroup/system.slice/llama.service/dmem.max"),
        "drm/0000:17:00.0/vram0 max",
    );
}

#[test]
fn cgroup_set_validation() {
    let t = writable_copy("srv-2gpu-k7.2");
    let r = Runner::temp(&t, "srv-2gpu-k7.2");
    let o = r.run(&[
        "-i",
        "0",
        "cgroup",
        "set",
        "/nonexistent",
        "--vram-max",
        "8G",
    ]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let o = r.run(&[
        "-i",
        "0",
        "cgroup",
        "set",
        "/system.slice/llama.service",
        "--vram-max",
        "40G",
    ]);
    assert_eq!(code(&o), 2, "{}", stderr(&o)); // above capacity
    let o = r.run(&[
        "-i",
        "0",
        "cgroup",
        "set",
        "/system.slice/llama.service",
        "--vram-max",
        "-1",
    ]);
    assert_eq!(code(&o), 2);
    let o = r.run(&[
        "cgroup",
        "set",
        "/system.slice/llama.service",
        "--vram-max",
        "8G",
    ]);
    assert_eq!(code(&o), 2, "{}", stderr(&o)); // several devices, no -i
    assert_contains(&stderr(&o), "-i");
}

#[test]
fn cgroup_show() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["cgroup", "show", "/system.slice/llama.service"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "drm/0000:e3:00.0/vram0");
    assert_contains(&s, "current                  : 1024 MiB");
    assert_contains(&s, "max                      : 20480 MiB");
    assert_contains(&s, "clients                  : 1 (llama-server)");
}
