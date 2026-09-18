//! The only directory allowed to contain `unsafe` is src/kabi/. Everything else is checked textually,
//! which catches `unsafe` blocks, `unsafe fn`, `unsafe impl` and `#![allow(unsafe_code)]` alike.
use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

#[test]
fn no_unsafe_outside_kabi() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![];
    walk(&src, &mut files);
    assert!(files.len() > 10, "unexpectedly few source files");
    let mut offenders = vec![];
    for f in files {
        let rel = f.strip_prefix(&src).unwrap();
        if rel.starts_with("kabi") {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let l = line.trim_start();
            if l.starts_with("//") {
                continue;
            }
            if l.contains("unsafe")
                && !l.contains("deny(unsafe_code)")
                && !l.contains("forbid(unsafe_code)")
            {
                offenders.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "unsafe outside src/kabi:\n{}",
        offenders.join("\n")
    );
    let kabi = src.join("kabi");
    assert!(
        kabi.join("ioctl.rs").exists(),
        "src/kabi/ioctl.rs must exist"
    );
}
