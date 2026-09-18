mod common;
use common::*;

#[test]
fn help_lists_every_command() {
    let r = Runner::fixture("b65-g31-k7.1");
    let o = r.run(&["--help"]);
    assert_eq!(code(&o), 0);
    let s = stdout(&o);
    for name in [
        "status",
        "info",
        "list",
        "fields",
        "query",
        "processes",
        "get",
        "set",
        "reset",
        "persist",
        "doctor",
        "completions",
    ] {
        assert_contains(&s, name);
    }
    assert_not_contains(&s, "  man "); // hidden
}

#[test]
fn version_string() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["--version"]);
    assert_eq!(code(&o), 0);
    assert_eq!(
        stdout(&o).trim(),
        format!("xe-gmi {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_command_is_usage_error() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["frobnicate"]);
    assert_eq!(code(&o), 2);
}

#[test]
fn sample_ms_lower_bound() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["--sample-ms", "50", "status"]);
    assert_eq!(code(&o), 2);
    assert_contains(&stderr(&o), "--sample-ms");
}

#[test]
fn completions_bash_and_zsh_and_fish() {
    for shell in ["bash", "zsh", "fish"] {
        let o = Runner::fixture("b65-g31-k7.1").run(&["completions", shell]);
        assert_eq!(code(&o), 0, "{shell}");
        assert_contains(&stdout(&o), "xe-gmi");
    }
}

#[test]
fn set_clocks_requires_a_bound() {
    let t = writable_copy("b65-g31-k7.1");
    let o = Runner::temp(&t, "b65-g31-k7.1").run(&["set", "clocks"]);
    assert_eq!(code(&o), 2);
    assert_contains(&stderr(&o), "--min");
}
