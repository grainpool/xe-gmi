//! Civil dates without a calendar library: Howard Hinnant's `civil_from_days`.

/// Days since 1970-01-01 → (year, month, day). Correct across era boundaries and leap years.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = div_floor(z, 146097);
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso8601(epoch: u64) -> String {
    let days = (epoch / 86400) as i64;
    let secs = epoch % 86400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn div_floor(a: i64, b: i64) -> i64 {
    let q = a / b;
    if a % b != 0 && (a < 0) != (b < 0) {
        q - 1
    } else {
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_cases() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        // Golden clock of the test harness (tests/common/mod.rs NOW).
        assert_eq!(iso8601(1789381351), "2026-09-14T10:22:31Z");
        assert_eq!(civil_from_days(1789381351 / 86400), (2026, 9, 14));
        assert_eq!(iso8601(951782400), "2000-02-29T00:00:00Z");
        assert_eq!(civil_from_days(11016), (2000, 2, 29));
        assert_eq!(iso8601(4102444800), "2100-01-01T00:00:00Z");
        assert_eq!(civil_from_days(4102444800 / 86400), (2100, 1, 1));
    }
}
