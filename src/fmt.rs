//! Human-readable formatting shared by the CLI, PDF and TUI.

use humansize::{DECIMAL, FormatSizeOptions, format_size};
use thousands::Separable;

/// Byte count in decimal (SI) units with two decimals, matching drive labels: `"4.00 TB"`.
pub fn bytes_human(bytes: u64) -> String {
    let opts = FormatSizeOptions::from(DECIMAL)
        .decimal_places(2)
        .decimal_zeroes(2);
    format_size(bytes, opts)
}

/// Power-on time as calendar time plus raw hours, e.g. `"2 years 7 months (23,140 h)"`.
/// Uses 365.25-day years and 730.5-hour months.
pub fn power_on_human(hours: u64) -> String {
    let raw = format!("{} h", hours.separate_with_commas());
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
    fn bytes() {
        assert_eq!(bytes_human(4_000_787_030_016), "4.00 TB");
        assert_eq!(bytes_human(31_900_000_000_000), "31.90 TB");
        assert_eq!(bytes_human(1_000), "1.00 kB");
    }

    #[test]
    fn power_on() {
        assert_eq!(power_on_human(5), "5 h");
        assert_eq!(power_on_human(50), "2 days (50 h)");
        assert_eq!(power_on_human(23_140), "2 years 7 months (23,140 h)");
        assert_eq!(power_on_human(8_766), "1 year (8,766 h)");
    }
}
