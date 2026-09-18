//! Output rendering. Shared pieces live here; each format is a submodule.

pub mod csv;
pub mod fields;
pub mod hw;
pub mod info;
pub mod json;
pub mod proc;
pub mod report;
pub mod table;

/// Text placeholder for `Avail::NotAvailable`.
pub const NA: &str = "N/A";

/// Names and command lines are truncated to 28 columns: 28 unchanged, 29+ → 25 + `...`.
pub fn truncate_28(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 28 {
        s.to_string()
    } else {
        let mut out: String = chars[..25].iter().collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_28;

    #[test]
    fn truncate_28_rules() {
        let exact = "Battlemage G31 [Arc Pro B65]";
        assert_eq!(exact.chars().count(), 28);
        assert_eq!(truncate_28(exact), exact);
        let short = "DG2 [Arc A770]";
        assert_eq!(truncate_28(short), short);
        let long = "Battlemage G31 [Arc Pro B65X]"; // 29 chars -> first 25 + "..."
        assert_eq!(truncate_28(long), "Battlemage G31 [Arc Pro B...");
        assert_eq!(truncate_28(long).chars().count(), 28);
        assert_eq!(truncate_28(""), "");
        // multi-byte characters count as one column each
        assert_eq!(truncate_28(&"é".repeat(40)).chars().count(), 28);
    }
}
