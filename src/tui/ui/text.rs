//! Text formatting helpers.

use thousands::Separable;

pub fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(w - 1).collect();
    out.push('…');
    out
}

/// Greedy word wrap; words longer than `w` are hard-split.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > w {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            let head: String = word.chars().take(w).collect();
            word = word.chars().skip(w).collect();
            lines.push(head);
        }
        let need = cur.chars().count() + usize::from(!cur.is_empty()) + word.chars().count();
        if need > w && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

pub(super) fn wrap_all(s: &str, w: usize) -> Vec<String> {
    s.lines().flat_map(|l| wrap(l, w)).collect()
}

/// Short capacity for lists: "4 TB", "500 GB", "1.92 TB".
pub fn fmt_capacity(b: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    let decimals = match v {
        v if v >= 100.0 => 0,
        v if v >= 10.0 => 1,
        _ => 2,
    };
    let mut num = format!("{v:.decimals$}");
    if num.contains('.') {
        num = num.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    format!("{num} {}", UNITS[u])
}

/// RFC 3339 to "YYYY-MM-DD HH:MM" in the original offset; passes other text through.
pub(super) fn fmt_time(s: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| s.to_string())
}

pub(super) fn fmt_count(n: u64) -> String {
    n.separate_with_commas()
}

pub(super) fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.into()
}
