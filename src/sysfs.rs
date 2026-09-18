//! sysfs read helpers: everything returns `Avail`, nothing panics.

use crate::avail::{Avail, Reason};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Trimmed file contents. `NotFound` → `Missing`, every other error → `Unreadable`.
pub fn read_string(p: &Path) -> Avail<String> {
    match fs::read_to_string(p) {
        Ok(s) => Avail::Value(s.trim().to_string()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Avail::NotAvailable(Reason::Missing(p.to_path_buf()))
        }
        Err(e) => Avail::NotAvailable(Reason::Unreadable(p.to_path_buf(), e.kind())),
    }
}

pub fn read_u64(p: &Path) -> Avail<u64> {
    read_num(p)
}

pub fn read_i64(p: &Path) -> Avail<i64> {
    read_num(p)
}

fn read_num<T: FromStr>(p: &Path) -> Avail<T> {
    match read_string(p) {
        Avail::Value(s) => match s.parse() {
            Ok(v) => Avail::Value(v),
            Err(_) => Avail::NotAvailable(Reason::Unparseable(p.to_path_buf(), s)),
        },
        Avail::NotAvailable(r) => Avail::NotAvailable(r),
    }
}

/// Target of a symlink, last component only (e.g. `driver` → `xe`).
pub fn link_basename(p: &Path) -> Avail<String> {
    match fs::read_link(p) {
        Ok(target) => match target.file_name().map(|n| n.to_string_lossy().into_owned()) {
            Some(name) => Avail::Value(name),
            None => Avail::NotAvailable(Reason::Unparseable(
                p.to_path_buf(),
                target.to_string_lossy().into_owned(),
            )),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Avail::NotAvailable(Reason::Missing(p.to_path_buf()))
        }
        Err(e) => Avail::NotAvailable(Reason::Unreadable(p.to_path_buf(), e.kind())),
    }
}

/// Entries of `p` whose name passes `pattern`, sorted by name with natural order for numeric
/// runs (so `gt2` sorts before `gt10`, ). Empty list when the dir is absent.
pub fn list_dir(p: &Path, pattern: fn(&str) -> bool) -> Vec<PathBuf> {
    let mut names: Vec<String> = match fs::read_dir(p) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| pattern(n))
            .collect(),
        Err(_) => return Vec::new(),
    };
    names.sort_by(|a, b| natural_cmp(a, b));
    names.into_iter().map(|n| p.join(n)).collect()
}

pub fn canonical(p: &Path) -> Avail<PathBuf> {
    match fs::canonicalize(p) {
        Ok(c) => Avail::Value(c),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Avail::NotAvailable(Reason::Missing(p.to_path_buf()))
        }
        Err(e) => Avail::NotAvailable(Reason::Unreadable(p.to_path_buf(), e.kind())),
    }
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    while i < ab.len() && j < bb.len() {
        if ab[i].is_ascii_digit() && bb[j].is_ascii_digit() {
            let (ie, je) = (digit_run(ab, i), digit_run(bb, j));
            let (na, nb) = (
                a[i..ie].trim_start_matches('0'),
                b[j..je].trim_start_matches('0'),
            );
            match na.len().cmp(&nb.len()).then_with(|| na.cmp(nb)) {
                std::cmp::Ordering::Equal => (i, j) = (ie, je),
                o => return o,
            }
        } else {
            match ab[i].cmp(&bb[j]) {
                std::cmp::Ordering::Equal => (i, j) = (i + 1, j + 1),
                o => return o,
            }
        }
    }
    (ab.len() - i).cmp(&(bb.len() - j))
}

fn digit_run(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_sorts_numeric_runs() {
        assert!(natural_cmp("gt2", "gt10") == std::cmp::Ordering::Less);
        assert!(natural_cmp("tile0", "tile10") == std::cmp::Ordering::Less);
        assert!(natural_cmp("temp1_input", "temp2_input") == std::cmp::Ordering::Less);
        assert!(natural_cmp("gt0", "gt0") == std::cmp::Ordering::Equal);
    }
}
