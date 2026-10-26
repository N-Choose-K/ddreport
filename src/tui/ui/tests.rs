use crate::model::{Attribute, Severity};

use super::tables::attr_severity;
use super::text::{fmt_capacity, trunc, wrap};

#[test]
fn formatting() {
    assert_eq!(fmt_capacity(4_000_787_030_016), "4 TB");
    assert_eq!(fmt_capacity(1_920_383_410_176), "1.92 TB");
    assert_eq!(fmt_capacity(500_107_862_016), "500 GB");
    assert_eq!(fmt_capacity(100), "100 B");
    assert_eq!(fmt_capacity(10_000_831_348_736), "10 TB");
    assert_eq!(trunc("abcdef", 4), "abc…");
    assert_eq!(trunc("abc", 4), "abc");
}

#[test]
fn wrapping() {
    assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
    assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
    assert_eq!(wrap("", 3), vec![""]);
}

#[test]
fn attribute_highlighting() {
    let a = |id, value, thresh, raw, when: &str| Attribute {
        id,
        value,
        thresh,
        raw,
        when_failed: when.into(),
        ..Default::default()
    };
    assert_eq!(attr_severity(&a(5, 100, 10, 0, "")), None);
    assert_eq!(
        attr_severity(&a(5, 100, 10, 8, "")),
        Some(Severity::Caution)
    );
    assert_eq!(attr_severity(&a(1, 5, 6, 0, "")), Some(Severity::Critical));
    assert_eq!(
        attr_severity(&a(1, 100, 6, 0, "now")),
        Some(Severity::Critical)
    );
    assert_eq!(
        attr_severity(&a(194, 30, 0, 30, "past")),
        Some(Severity::Caution)
    );
    assert_eq!(attr_severity(&a(199, 200, 0, 3, "")), Some(Severity::Info));
    assert_eq!(attr_severity(&a(9, 90, 0, 9000, "")), None);
}
