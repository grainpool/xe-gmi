mod common;
use common::*;

#[test]
fn doctor_bundle_writes_directory_with_redaction() {
    let t = writable_copy("srv-2gpu-k7.2");
    let out = t.root.join("bundle");
    let o = Runner::temp(&t, "srv-2gpu-k7.2").run(&["doctor", "--bundle", out.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    for f in [
        "doctor.txt",
        "info.json",
        "fields.json",
        "topology.txt",
        "pcie.txt",
        "crash.txt",
        "cgroups.txt",
        "sriov.txt",
        "processes.json",
        "kmsg-xe.txt",
        "MANIFEST.txt",
    ] {
        assert!(out.join(f).exists(), "missing {f}");
    }
    let procs = std::fs::read_to_string(out.join("processes.json")).unwrap();
    assert_not_contains(&procs, "llama-server"); // names redacted by default
    assert_contains(&procs, "\"name\": \"<redacted>\"");
    let cg = std::fs::read_to_string(out.join("cgroups.txt")).unwrap();
    assert_not_contains(&cg, "llama.service");
    let man = std::fs::read_to_string(out.join("MANIFEST.txt")).unwrap();
    assert_contains(&man, "redaction: on");
    assert_contains(&stdout(&o), &format!("bundle written to {}", out.display()));
}

#[test]
fn doctor_bundle_no_redact() {
    let t = writable_copy("srv-2gpu-k7.2");
    let out = t.root.join("bundle");
    let o = Runner::temp(&t, "srv-2gpu-k7.2").run(&[
        "doctor",
        "--bundle",
        out.to_str().unwrap(),
        "--no-redact",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_contains(
        &std::fs::read_to_string(out.join("processes.json")).unwrap(),
        "llama-server",
    );
    assert_contains(
        &std::fs::read_to_string(out.join("MANIFEST.txt")).unwrap(),
        "redaction: off",
    );
}
