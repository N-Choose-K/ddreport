//! Human-readable formatting shared by the PDF and TUI.

/// Hours per year (365.25 days).
const HOURS_PER_YEAR: u64 = 8766;

/// `23140` -> `"23,140"`.
pub fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Power-on time as calendar time plus raw hours, e.g. `"2 years 7 months (23,140 h)"`.
/// Uses 365.25-day years and 730.5-hour months.
pub fn power_on_human(hours: u64) -> String {
    let raw = format!("{} h", group_thousands(hours));
    let months = hours * 2 / 1461;
    let (y, m) = (months / 12, months % 12);
    let span = match (y, m) {
        (0, 0) => match hours / 24 {
            0 => return raw,
            d => plural(d, "day"),
        },
        (0, m) => plural(m, "month"),
        (y, 0) => plural(y, "year"),
        (y, m) => format!("{} {}", plural(y, "year"), plural(m, "month")),
    };
    format!("{span} ({raw})")
}

/// Power-on hours as fractional years, e.g. `5.3`.
pub fn years(hours: u64) -> f64 {
    hours as f64 / HOURS_PER_YEAR as f64
}

/// Byte count in decimal (SI) units with two decimals, matching drive labels: `"4.00 TB"`.
pub fn bytes_human(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut i = 0;
    // Bump on 999.995+ so rounding never prints "1000.00".
    while v >= 999.995 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    format!("{v:.2} {}", UNITS[i])
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(23140), "23,140");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }

    #[test]
    fn power_on() {
        assert_eq!(power_on_human(23140), "2 years 7 months (23,140 h)");
        assert_eq!(power_on_human(0), "0 h");
        assert_eq!(power_on_human(23), "23 h");
        assert_eq!(power_on_human(24), "1 day (24 h)");
        assert_eq!(power_on_human(300), "12 days (300 h)");
        assert_eq!(power_on_human(731), "1 month (731 h)");
        assert_eq!(power_on_human(8766), "1 year (8,766 h)");
        assert_eq!(power_on_human(8766 + 1461), "1 year 2 months (10,227 h)");
        assert_eq!(power_on_human(43830), "5 years (43,830 h)");
    }

    #[test]
    fn bytes() {
        assert_eq!(bytes_human(0), "0 B");
        assert_eq!(bytes_human(999), "999 B");
        assert_eq!(bytes_human(1000), "1.00 kB");
        assert_eq!(bytes_human(4_000_787_030_016), "4.00 TB");
        assert_eq!(bytes_human(500_107_862_016), "500.11 GB");
        assert_eq!(bytes_human(999_999_999_999), "1.00 TB");
        assert_eq!(bytes_human(u64::MAX), "18.45 EB");
    }

    #[test]
    fn years_fraction() {
        assert!((years(8766) - 1.0).abs() < 1e-9);
    }
}
