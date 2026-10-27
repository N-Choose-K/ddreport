//! Grades a report into a PASS / CAUTION / FAIL verdict.
//!
//! Fail means the drive itself says so or data is unreadable right now: SMART
//! overall status FAILED, an attribute at or below the vendor's
//! threshold, a failed newest self-test, an NVMe critical warning, or sectors
//! pending reallocation on a drive that has already remapped sectors or
//! returned uncorrectable reads. Caution means a known risk signal with most
//! drives still surviving: Google's 2007 study found drives with a scan error
//! 39x more likely to fail within 60 days, yet 70% survived 8 months, and 85%
//! survived 8 months after a first reallocation. Backblaze treats a non-zero
//! raw value on 5, 187, 188, 197 or 198 as "a reason to investigate", counts
//! 197 and 198 as one correlated indicator, and schedules replacement when
//! reallocations and pending sectors both climb. CrystalDiskInfo grades 5, 197
//! and 198 Caution and reserves Bad for the vendor threshold. Grading reads
//! `Health` (counts decoded from attributes whose raw is a plain count),
//! `attributes[].when_failed` and the self-test log; packed vendor raws such
//! as Seagate 1/7/195 are never graded.
//!
//! | Check                       | Good            | Info               | Caution                      | Critical                    |
//! |-----------------------------|-----------------|--------------------|------------------------------|-----------------------------|
//! | Overall SMART status        | passed          | not readable       |                              | failed                      |
//! | Attribute thresholds        | none failed     |                    | failed in the past           | failing now                 |
//! | Pending sectors (197)       | 0               |                    | > 0                          | > 0 with 5 or 187 > 0       |
//! | Offline uncorrectable (198) | 0               |                    | > 0                          |                             |
//! | Reallocated sectors (5)     | 0               |                    | > 0                          |                             |
//! | Reported uncorrectable (187)| 0               |                    | > 0                          |                             |
//! | Command timeouts (188)      |                 | 1-99               | >= 100                       |                             |
//! | UDMA CRC errors (199)       |                 | > 0 (cable/USB)    |                              |                             |
//! | Newest conclusive self-test | passed <= 168 h | no recent extended | none, > 168 h, older failure | failed                      |
//! | NVMe critical warning       | 0               |                    |                              | != 0                        |
//! | Available spare             | >= 50%          |                    | < 50%                        | < threshold (def. 10%)      |
//! | SSD endurance used          | < 50%           | 50-79%             | 80-99%                       | >= 100%                     |
//! | NVMe media errors           | 0               |                    | > 0                          |                             |
//! | Lifetime max temperature    |                 |                    | HDD > 60 C, SSD > 80 C       |                             |
//! | Power-on time               |                 | always             |                              |                             |
//! | Unsafe shutdowns            |                 | > 0                |                              |                             |
//! | ATA error log entries       |                 | > 0                |                              |                             |
//!
//! Grade: any Critical -> Fail, else any Caution -> Caution, else Pass. Unknown
//! when SMART is unsupported, disabled or returned nothing. Self-tests that are
//! in progress, aborted or interrupted are skipped. `self_tests` is newest first.

use crate::fmt::power_on_human;
use crate::model::{
    DriveReport, Finding, Grade, Health, MediaKind, Protocol, SelfTest, Severity, Verdict,
};
use thousands::Separable;

pub const TIMEOUTS_CAUTION: u64 = 100;

/// Grading rubric printed on the certificate, worst grade first.
pub const RUBRIC: &[(&str, &str)] = &[
    (
        "FAIL",
        "SMART overall status FAILED, an attribute at or below the vendor threshold, \
         the newest self-test failed, an NVMe critical warning or spare below its threshold, \
         100% endurance used, or sectors pending reallocation on a drive that has already \
         remapped sectors or returned uncorrectable reads.",
    ),
    (
        "CAUTION",
        "any reallocated, pending or uncorrectable sector count, an attribute that failed in \
         the past, an earlier failed self-test, no self-test within 168 h, 100 or more command \
         timeouts, NVMe media errors, spare below 50%, 80-99% endurance used, a lifetime \
         temperature above 60 C (HDD) / 80 C (SSD), or incomplete SMART data.",
    ),
    ("PASS", "none of the above."),
];

/// Published evidence behind the rubric.
pub const SOURCES: &[&str] = &[
    "Pinheiro, Weber and Barroso, Failure Trends in a Large Disk Drive Population, USENIX FAST \
     2007 (scan errors: 39x failure odds within 60 days, 70% survive 8 months; reallocations: \
     14x, 85% survive 8 months)",
    "Backblaze, What SMART Stats Tell Us About Hard Drives, 2016 (a nonzero 5, 187, 188, 197 or \
     198 is a reason to investigate; 4.2% of working drives vs 76.7% of failed drives)",
    "CrystalDiskInfo health status criteria, crystalmark.info (Caution on sector counts, Bad \
     below the vendor threshold)",
    "smartmontools smartctl(8): health status and self-test log semantics",
];
pub const SELF_TEST_MAX_AGE_H: u64 = 168;
pub const ENDURANCE_INFO_PCT: u64 = 50;
pub const ENDURANCE_CAUTION_PCT: u64 = 80;
pub const ENDURANCE_FAIL_PCT: u64 = 100;
pub const SPARE_CAUTION_PCT: u64 = 50;
pub const SPARE_THRESHOLD_DEFAULT_PCT: u64 = 10;
pub const HDD_TEMP_MAX_C: i64 = 60;
pub const SSD_TEMP_MAX_C: i64 = 80;
/// Five years.
pub const HIGH_POWER_ON_H: u64 = 43_830;

pub fn evaluate(r: &DriveReport) -> Verdict {
    if let Some(reason) = smart_unavailable(r) {
        return Verdict {
            grade: Grade::Unknown,
            summary: "SMART data could not be read, so this drive's health cannot be verified."
                .into(),
            findings: vec![Finding {
                severity: Severity::Caution,
                message: reason.into(),
            }],
        };
    }

    let mut f = Findings::default();
    for e in &r.meta.collection_errors {
        f.push(
            Severity::Caution,
            format!("Incomplete or unreliable SMART data: {e}; re-read the drive before relying on this report"),
        );
    }
    overall(&r.health, &mut f);
    attributes(r, &mut f);
    sectors(&r.health, &mut f);
    interface(&r.health, &mut f);
    self_tests(r, &mut f);
    nvme(&r.health, &mut f);
    endurance(&r.health, &mut f);
    temperature(r, &mut f);
    usage(r, &mut f);

    let mut findings = f.0;
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    let count = |s| findings.iter().filter(|f| f.severity == s).count() as u64;
    let (grade, summary) = match (count(Severity::Critical), count(Severity::Caution)) {
        (0, 0) => (
            Grade::Pass,
            "No problems found: SMART data and a recent self-test indicate a healthy drive."
                .to_string(),
        ),
        (0, 1) => (
            Grade::Caution,
            "No critical problems, but 1 item needs attention before relying on this drive."
                .to_string(),
        ),
        (0, n) => (
            Grade::Caution,
            format!(
                "No critical problems, but {n} items need attention before relying on this drive."
            ),
        ),
        (n, _) => (
            Grade::Fail,
            format!(
                "{} found; this drive is not recommended for storing data.",
                plural(n, "critical problem")
            ),
        ),
    };
    Verdict {
        grade,
        summary,
        findings,
    }
}

#[derive(Default)]
struct Findings(Vec<Finding>);

impl Findings {
    fn push(&mut self, severity: Severity, message: impl Into<String>) {
        self.0.push(Finding {
            severity,
            message: message.into(),
        });
    }
}

fn smart_unavailable(r: &DriveReport) -> Option<&'static str> {
    let h = &r.health;
    if h.smart_supported == Some(false) {
        return Some("Drive does not report SMART health data - its condition cannot be verified");
    }
    if h.smart_enabled == Some(false) {
        return Some("SMART is disabled on this drive - its condition cannot be verified");
    }
    let any_data = h.overall_passed.is_some()
        || !r.attributes.is_empty()
        || !r.self_tests.is_empty()
        || h.power_on_hours.is_some()
        || h.reallocated_sectors.is_some()
        || h.pending_sectors.is_some()
        || h.nvme_critical_warning.is_some()
        || h.endurance_used_pct.is_some()
        || h.available_spare_pct.is_some();
    (!any_data)
        .then_some("No SMART health data could be read (the USB adapter may not pass it through)")
}

fn overall(h: &Health, f: &mut Findings) {
    match (h.overall_passed, h.overall_by_attributes) {
        (Some(true), true) => f.push(
            Severity::Good,
            "SMART status PASSED by attribute check; this bridge cannot relay the drive's own verdict",
        ),
        (Some(false), true) => f.push(
            Severity::Critical,
            "SMART status FAILED by attribute check - an attribute is at or below its failure threshold",
        ),
        (Some(true), false) => f.push(
            Severity::Good,
            "Drive's built-in SMART self-assessment: PASSED",
        ),
        (Some(false), false) => f.push(
            Severity::Critical,
            "Drive's built-in SMART self-assessment: FAILED - the drive predicts its own failure",
        ),
        (None, _) => f.push(
            Severity::Info,
            "Drive's overall SMART self-assessment could not be read (common through USB \
             adapters); graded on individual values",
        ),
    }
}

fn attributes(r: &DriveReport, f: &mut Findings) {
    let mut any_failed = false;
    for a in &r.attributes {
        let name = a.name.replace('_', " ");
        let (severity, when) = match a.when_failed.as_str() {
            "now" => (Severity::Critical, "is"),
            "past" => (Severity::Caution, "fell"),
            _ => continue,
        };
        let past = if when == "fell" { " in the past" } else { "" };
        f.push(
            severity,
            format!(
                "SMART attribute {} ({name}) {when} below the manufacturer's failure threshold{past}",
                a.id
            ),
        );
        any_failed = true;
    }
    if !any_failed && !r.attributes.is_empty() {
        f.push(
            Severity::Good,
            "All SMART attributes are within manufacturer limits",
        );
    }
}

/// Pending sectors on a drive that has already remapped sectors (5) or
/// returned uncorrectable reads (187): Backblaze's replacement case.
pub fn pending_escalated(h: &Health) -> bool {
    let nonzero = |v: Option<u64>| v.is_some_and(|n| n > 0);
    nonzero(h.pending_sectors)
        && (nonzero(h.reallocated_sectors) || nonzero(h.reported_uncorrectable))
}

fn sectors(h: &Health, f: &mut Findings) {
    if let Some(n @ 1..) = h.pending_sectors {
        if pending_escalated(h) {
            f.push(
                Severity::Critical,
                format!(
                    "{} pending reallocation on a drive that has already remapped sectors or \
                     returned uncorrectable reads - copy the data off and replace it",
                    plural(n, "sector")
                ),
            );
        } else {
            f.push(
                Severity::Caution,
                format!(
                    "{} pending reallocation - data there may be unreadable; back up now and \
                     re-check for growth",
                    plural(n, "sector")
                ),
            );
        }
    }
    if let Some(n @ 1..) = h.offline_uncorrectable {
        f.push(
            Severity::Caution,
            format!(
                "{} the drive's own surface scan could not read - watch for growth",
                plural(n, "sector")
            ),
        );
    }
    if let Some(n @ 1..) = h.reallocated_sectors {
        f.push(
            Severity::Caution,
            format!(
                "{} reallocated to spare areas - the drive has had bad sectors; watch for growth",
                plural(n, "sector")
            ),
        );
    }
    if let Some(n @ 1..) = h.reported_uncorrectable {
        f.push(
            Severity::Caution,
            format!("{} the drive could not correct", plural(n, "read error")),
        );
    }

    let unc = [h.offline_uncorrectable, h.reported_uncorrectable];
    let clean: Vec<&str> = [
        ("reallocated", h.reallocated_sectors == Some(0)),
        ("pending", h.pending_sectors == Some(0)),
        (
            "uncorrectable",
            unc.iter().any(Option::is_some) && unc.iter().all(|v| v.unwrap_or(0) == 0),
        ),
    ]
    .iter()
    .filter(|c| c.1)
    .map(|c| c.0)
    .collect();
    if !clean.is_empty() {
        f.push(Severity::Good, format!("No {} sectors", join_or(&clean)));
    }
}

fn interface(h: &Health, f: &mut Findings) {
    match h.command_timeouts {
        Some(n) if n >= TIMEOUTS_CAUTION => f.push(
            Severity::Caution,
            format!(
                "{} - the drive has repeatedly failed to respond in time",
                plural(n, "command timeout")
            ),
        ),
        Some(n @ 1..) => f.push(
            Severity::Info,
            format!(
                "{} (often caused by cabling, power or the USB adapter)",
                plural(n, "command timeout")
            ),
        ),
        _ => {}
    }
    if let Some(n @ 1..) = h.udma_crc_errors {
        f.push(
            Severity::Info,
            format!(
                "{} - usually a cable or USB adapter fault that does not affect the disk's media",
                plural(n, "interface (CRC) error")
            ),
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Inconclusive,
}

fn outcome(t: &SelfTest) -> Outcome {
    let s = t.status.to_ascii_lowercase();
    if s.contains("progress") || s.contains("abort") || s.contains("interrupt") {
        Outcome::Inconclusive
    } else if s.contains("fail") || t.passed == Some(false) {
        Outcome::Failed
    } else if t.passed == Some(true) || s.contains("without error") {
        Outcome::Passed
    } else if s.contains("error") {
        Outcome::Failed
    } else {
        Outcome::Inconclusive
    }
}

fn is_extended(t: &SelfTest) -> bool {
    let k = t.kind.to_ascii_lowercase();
    k.contains("extended") || k.contains("long")
}

fn kind_label(t: &SelfTest) -> &'static str {
    let k = t.kind.to_ascii_lowercase();
    ["Short", "Conveyance", "Selective"]
        .into_iter()
        .find(|l| k.contains(&l.to_ascii_lowercase()))
        .unwrap_or("Most recent")
}

/// Hours since a self-test ran. ATA logs store a 16-bit hour stamp that wraps.
fn test_age(r: &DriveReport, t: &SelfTest) -> Option<u64> {
    let (now, at) = (r.health.power_on_hours?, t.lifetime_hours?);
    if r.identity.protocol != Protocol::Nvme && now > 0xFFFF && at <= 0xFFFF {
        Some((now - at) % 0x1_0000)
    } else {
        now.checked_sub(at)
    }
}

fn ago(hours: u64) -> String {
    match hours {
        0 => "within the last power-on hour".into(),
        1 => "1 power-on hour ago".into(),
        n => format!("{} power-on hours ago", n.separate_with_commas()),
    }
}

fn self_tests(r: &DriveReport, f: &mut Findings) {
    if let Some(p) = &r.self_test_in_progress {
        let test = p
            .kind
            .as_deref()
            .map_or("Self-test".into(), |k| format!("{} self-test", k.trim()));
        let left = p
            .remaining_pct
            .map(|n| format!(" ({n}% remaining)"))
            .unwrap_or_default();
        f.push(
            Severity::Info,
            format!("{test} in progress{left}; its result is not included"),
        );
    }

    let done: Vec<(&SelfTest, Outcome)> = r
        .self_tests
        .iter()
        .map(|t| (t, outcome(t)))
        .filter(|(_, o)| *o != Outcome::Inconclusive)
        .collect();
    let Some(&(newest, newest_outcome)) = done.first() else {
        f.push(
            Severity::Caution,
            "No completed self-test on record - run an extended self-test before relying on \
             this drive",
        );
        return;
    };

    if newest_outcome == Outcome::Failed {
        let lba = newest
            .lba_first_error
            .map(|l| format!(", first bad block at LBA {}", l.separate_with_commas()))
            .unwrap_or_default();
        f.push(
            Severity::Critical,
            format!(
                "Most recent self-test failed ({}{lba})",
                newest.status.trim()
            ),
        );
        return;
    }

    if let Some((old, _)) = done.iter().find(|(_, o)| *o == Outcome::Failed) {
        let when = old
            .lifetime_hours
            .map(|h| format!(" at hour {}", h.separate_with_commas()))
            .unwrap_or_default();
        f.push(
            Severity::Caution,
            format!(
                "An earlier self-test failed{when} ({}); later tests passed",
                old.status.trim()
            ),
        );
    }

    let recent = |t: &SelfTest| test_age(r, t).filter(|&a| a <= SELF_TEST_MAX_AGE_H);
    let Some(age) = recent(newest) else {
        let when = test_age(r, newest).map_or("at an unknown time".into(), ago);
        f.push(
            Severity::Caution,
            format!("Last self-test ran {when} - run an extended self-test for a current result"),
        );
        return;
    };

    let extended = done
        .iter()
        .take_while(|(_, o)| *o == Outcome::Passed)
        .find_map(|(t, _)| is_extended(t).then(|| recent(t)).flatten());
    match extended {
        Some(a) => f.push(
            Severity::Good,
            format!("Extended self-test (full surface scan) passed {}", ago(a)),
        ),
        None => {
            f.push(
                Severity::Good,
                format!("{} self-test passed {}", kind_label(newest), ago(age)),
            );
            f.push(
                Severity::Info,
                "No recent extended self-test; shorter tests check only part of the drive",
            );
        }
    }
}

fn nvme(h: &Health, f: &mut Findings) {
    match h.nvme_critical_warning {
        Some(0) => f.push(Severity::Good, "No NVMe critical warnings"),
        Some(w) => f.push(
            Severity::Critical,
            format!("NVMe critical warning: {}", decode_nvme_warning(w)),
        ),
        None => {}
    }
    if let Some(spare) = h.available_spare_pct {
        let threshold = h
            .available_spare_threshold_pct
            .unwrap_or(SPARE_THRESHOLD_DEFAULT_PCT);
        if spare < threshold {
            f.push(
                Severity::Critical,
                format!(
                    "Spare capacity at {spare}%, below the drive's {threshold}% safety threshold"
                ),
            );
        } else if spare < SPARE_CAUTION_PCT {
            f.push(
                Severity::Caution,
                format!("Spare capacity down to {spare}% - many worn blocks have been retired"),
            );
        } else {
            f.push(Severity::Good, format!("Spare capacity at {spare}%"));
        }
    }
    match h.nvme_media_errors {
        Some(0) => f.push(Severity::Good, "No media or data integrity errors"),
        Some(n) => f.push(
            Severity::Caution,
            format!(
                "{} recorded - the drive failed to read or verify stored data",
                plural(n, "media or data integrity error")
            ),
        ),
        None => {}
    }
}

fn decode_nvme_warning(w: u64) -> String {
    const BITS: [(u64, &str); 6] = [
        (0x01, "spare capacity below threshold"),
        (0x02, "temperature outside safe range"),
        (0x04, "reliability degraded by media or internal errors"),
        (0x08, "drive has switched to read-only"),
        (0x10, "volatile memory backup failed"),
        (0x20, "persistent memory region is read-only"),
    ];
    let mut parts: Vec<String> = BITS
        .iter()
        .filter(|(bit, _)| w & bit != 0)
        .map(|(_, s)| s.to_string())
        .collect();
    let unknown = w & !0x3F;
    if unknown != 0 {
        parts.push(format!("unknown warning 0x{unknown:02X}"));
    }
    parts.join("; ")
}

fn endurance(h: &Health, f: &mut Findings) {
    let Some(p) = h.endurance_used_pct else {
        return;
    };
    let (severity, note) = match p {
        _ if p >= ENDURANCE_FAIL_PCT => (Severity::Critical, " - past its rated lifespan"),
        _ if p >= ENDURANCE_CAUTION_PCT => (Severity::Caution, " - nearing end of rated lifespan"),
        _ if p >= ENDURANCE_INFO_PCT => (Severity::Info, ""),
        _ => (Severity::Good, ""),
    };
    let lead = if severity == Severity::Good {
        "Low wear: "
    } else {
        ""
    };
    f.push(
        severity,
        format!("{lead}{p}% of rated write endurance used{note}"),
    );
}

fn temperature(r: &DriveReport, f: &mut Findings) {
    let Some(max) = r.health.temperature_max_c else {
        return;
    };
    let ssd = r.identity.media == MediaKind::Ssd || r.identity.protocol == Protocol::Nvme;
    let (limit, kind) = if ssd {
        (SSD_TEMP_MAX_C, "SSDs")
    } else {
        (HDD_TEMP_MAX_C, "hard drives")
    };
    if max > limit {
        f.push(
            Severity::Caution,
            format!(
                "Highest recorded temperature {max} C - above the typical {limit} C rated \
                 maximum for {kind}"
            ),
        );
    }
}

fn usage(r: &DriveReport, f: &mut Findings) {
    let h = &r.health;
    if let Some(hours) = h.power_on_hours {
        let heavy = if hours >= HIGH_POWER_ON_H {
            " - high for a used drive"
        } else {
            ""
        };
        f.push(
            Severity::Info,
            format!("Powered on for {}{heavy}", power_on_human(hours)),
        );
    }
    if let Some(n @ 1..) = h.unsafe_shutdowns {
        f.push(
            Severity::Info,
            format!(
                "{} (power lost without a clean shutdown) - common and usually harmless",
                plural(n, "unsafe shutdown")
            ),
        );
    }
    if r.identity.protocol != Protocol::Nvme
        && let Some(n @ 1..) = h.error_log_count
    {
        f.push(
            Severity::Info,
            format!(
                "{} in the drive's error log (historical; may stem from past cabling or host \
                 problems)",
                plural(n, "entry")
            ),
        );
    }
}

/// `plural(16, "sector")` -> `"16 sectors"`; a trailing "y" becomes "ies".
fn plural(n: u64, noun: &str) -> String {
    let count = n.separate_with_commas();
    match (n, noun.strip_suffix('y')) {
        (1, _) => format!("{count} {noun}"),
        (_, Some(stem)) => format!("{count} {stem}ies"),
        _ => format!("{count} {noun}s"),
    }
}

/// `["a", "b", "c"]` -> `"a, b or c"`.
fn join_or(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attribute, Identity, SelfTestProgress};

    fn attr(id: u8, name: &str, raw: u64, when_failed: &str) -> Attribute {
        Attribute {
            id,
            name: name.into(),
            value: 100,
            worst: 100,
            thresh: 10,
            raw,
            raw_string: raw.to_string(),
            prefailure: true,
            when_failed: when_failed.into(),
        }
    }

    fn test(kind: &str, status: &str, passed: Option<bool>, at: u64) -> SelfTest {
        SelfTest {
            kind: kind.into(),
            status: status.into(),
            passed,
            lifetime_hours: Some(at),
            lba_first_error: None,
        }
    }

    fn healthy_hdd() -> DriveReport {
        DriveReport {
            identity: Identity {
                protocol: Protocol::Ata,
                media: MediaKind::Hdd,
                ..Default::default()
            },
            health: Health {
                smart_supported: Some(true),
                smart_enabled: Some(true),
                overall_passed: Some(true),
                power_on_hours: Some(23_140),
                temperature_c: Some(34),
                temperature_max_c: Some(45),
                reallocated_sectors: Some(0),
                pending_sectors: Some(0),
                offline_uncorrectable: Some(0),
                reported_uncorrectable: Some(0),
                udma_crc_errors: Some(0),
                command_timeouts: Some(0),
                ..Default::default()
            },
            attributes: vec![
                attr(1, "Raw_Read_Error_Rate", 0, ""),
                attr(5, "Reallocated_Sector_Ct", 0, ""),
                attr(197, "Current_Pending_Sector", 0, ""),
            ],
            self_tests: vec![test(
                "Extended offline",
                "Completed without error",
                Some(true),
                23_100,
            )],
            ..Default::default()
        }
    }

    fn healthy_nvme() -> DriveReport {
        DriveReport {
            identity: Identity {
                protocol: Protocol::Nvme,
                media: MediaKind::Ssd,
                ..Default::default()
            },
            health: Health {
                smart_supported: Some(true),
                overall_passed: Some(true),
                power_on_hours: Some(5_000),
                temperature_max_c: Some(70),
                endurance_used_pct: Some(3),
                available_spare_pct: Some(100),
                available_spare_threshold_pct: Some(10),
                nvme_critical_warning: Some(0),
                nvme_media_errors: Some(0),
                unsafe_shutdowns: Some(12),
                error_log_count: Some(40),
                ..Default::default()
            },
            self_tests: vec![test(
                "Extended",
                "Completed without error",
                Some(true),
                4_990,
            )],
            ..Default::default()
        }
    }

    fn has(v: &Verdict, sev: Severity, needle: &str) -> bool {
        v.findings
            .iter()
            .any(|f| f.severity == sev && f.message.contains(needle))
    }

    fn worst(v: &Verdict) -> Severity {
        v.findings.iter().map(|f| f.severity).max().unwrap()
    }

    #[test]
    fn healthy_hdd_passes() {
        let v = evaluate(&healthy_hdd());
        assert_eq!(v.grade, Grade::Pass, "{v:#?}");
        assert!(worst(&v) <= Severity::Info);
        assert!(has(
            &v,
            Severity::Good,
            "No reallocated, pending or uncorrectable sectors"
        ));
        assert!(has(&v, Severity::Good, "SMART self-assessment: PASSED"));
        assert!(has(
            &v,
            Severity::Good,
            "Extended self-test (full surface scan) passed 40 power-on hours ago"
        ));
        assert!(has(&v, Severity::Good, "within manufacturer limits"));
        assert!(has(&v, Severity::Info, "2 years 7 months (23,140 h)"));
        assert!(v.summary.starts_with("No problems found"));
    }

    #[test]
    fn findings_sorted_by_severity_desc() {
        let mut r = healthy_hdd();
        r.health.pending_sectors = Some(3);
        r.health.udma_crc_errors = Some(2);
        r.health.reallocated_sectors = Some(4);
        let v = evaluate(&r);
        let sevs: Vec<Severity> = v.findings.iter().map(|f| f.severity).collect();
        assert!(sevs.windows(2).all(|w| w[0] >= w[1]), "{sevs:?}");
        assert_eq!(sevs[0], Severity::Critical);
        assert_eq!(*sevs.last().unwrap(), Severity::Good);
    }

    #[test]
    fn pending_sectors_alone_caution() {
        let mut r = healthy_hdd();
        r.health.pending_sectors = Some(16);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(
            &v,
            Severity::Caution,
            "16 sectors pending reallocation - data there may be unreadable"
        ));
        assert!(has(
            &v,
            Severity::Good,
            "No reallocated or uncorrectable sectors"
        ));
        assert_eq!(
            v.summary,
            "No critical problems, but 1 item needs attention before relying on this drive."
        );
    }

    #[test]
    fn pending_with_reallocated_or_reported_fails() {
        for field in ["reallocated", "reported"] {
            let mut r = healthy_hdd();
            r.health.pending_sectors = Some(5);
            r.health.offline_uncorrectable = None;
            match field {
                "reallocated" => r.health.reallocated_sectors = Some(10),
                _ => r.health.reported_uncorrectable = Some(1),
            }
            let v = evaluate(&r);
            assert_eq!(v.grade, Grade::Fail, "{field}");
            assert!(has(
                &v,
                Severity::Critical,
                "5 sectors pending reallocation on a drive that has already remapped"
            ));
            assert_eq!(
                v.summary,
                "1 critical problem found; this drive is not recommended for storing data."
            );
        }
    }

    #[test]
    fn pending_with_offline_uncorrectable_only_cautions() {
        let mut r = healthy_hdd();
        r.health.pending_sectors = Some(5);
        r.health.offline_uncorrectable = Some(20);
        r.health.reallocated_sectors = None;
        r.health.reported_uncorrectable = None;
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(&v, Severity::Caution, "5 sectors pending reallocation"));
        assert!(has(
            &v,
            Severity::Caution,
            "20 sectors the drive's own surface scan"
        ));
    }

    #[test]
    fn huge_counts_do_not_overflow() {
        let mut r = healthy_hdd();
        r.health.reallocated_sectors = Some(u64::MAX);
        r.health.offline_uncorrectable = Some(u64::MAX);
        r.health.reported_uncorrectable = Some(u64::MAX);
        assert_eq!(evaluate(&r).grade, Grade::Caution);
        r.health.pending_sectors = Some(u64::MAX);
        assert_eq!(evaluate(&r).grade, Grade::Fail);
    }

    #[test]
    fn ssd_without_pending_attribute_stays_caution() {
        let mut r = healthy_hdd();
        r.identity.media = MediaKind::Ssd;
        r.health.pending_sectors = None;
        r.health.reallocated_sectors = Some(2);
        r.health.offline_uncorrectable = Some(2);
        r.health.reported_uncorrectable = Some(0);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(!pending_escalated(&r.health));
    }

    #[test]
    fn offline_uncorrectable_cautions() {
        let mut r = healthy_hdd();
        r.health.offline_uncorrectable = Some(1);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(
            &v,
            Severity::Caution,
            "1 sector the drive's own surface scan could not read"
        ));
        assert!(has(&v, Severity::Good, "No reallocated or pending sectors"));
    }

    #[test]
    fn remapped_sectors_without_pending_caution() {
        let mut r = healthy_hdd();
        r.health.reallocated_sectors = Some(2);
        r.health.offline_uncorrectable = Some(2);
        r.health.reported_uncorrectable = Some(3);
        r.health.pending_sectors = Some(0);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(&v, Severity::Good, "No pending sectors"));
    }

    #[test]
    fn overall_failed_fails() {
        let mut r = healthy_hdd();
        r.health.overall_passed = Some(false);
        assert_eq!(evaluate(&r).grade, Grade::Fail);
    }

    #[test]
    fn overall_by_attributes_names_the_check() {
        let mut r = healthy_hdd();
        r.health.overall_by_attributes = true;
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass);
        assert!(has(&v, Severity::Good, "PASSED by attribute check"));
        r.health.overall_passed = Some(false);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Fail);
        assert!(has(&v, Severity::Critical, "FAILED by attribute check"));
    }

    #[test]
    fn overall_unreadable_is_info() {
        let mut r = healthy_hdd();
        r.health.overall_passed = None;
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass);
        assert!(has(&v, Severity::Info, "could not be read"));
    }

    #[test]
    fn reallocated_any_count_cautions() {
        let mut r = healthy_hdd();
        r.health.reallocated_sectors = Some(8);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(&v, Severity::Caution, "8 sectors reallocated"));
        assert!(has(
            &v,
            Severity::Good,
            "No pending or uncorrectable sectors"
        ));

        r.health.reallocated_sectors = Some(5000);
        assert_eq!(evaluate(&r).grade, Grade::Caution);
    }

    #[test]
    fn reported_uncorrectable_cautions() {
        let mut r = healthy_hdd();
        r.health.reported_uncorrectable = Some(2);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(
            &v,
            Severity::Caution,
            "2 read errors the drive could not correct"
        ));
        assert!(has(&v, Severity::Good, "No reallocated or pending sectors"));
    }

    #[test]
    fn attribute_failed_in_past_cautions() {
        let mut r = healthy_hdd();
        r.attributes
            .push(attr(190, "Airflow_Temperature_Cel", 40, "past"));
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(
            &v,
            Severity::Caution,
            "SMART attribute 190 (Airflow Temperature Cel) fell below the manufacturer's failure threshold in the past"
        ));
        assert!(!has(&v, Severity::Good, "within manufacturer limits"));
    }

    #[test]
    fn attribute_failing_now_fails() {
        let mut r = healthy_hdd();
        r.attributes.push(attr(3, "Spin_Up_Time", 0, "now"));
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Fail);
        assert!(has(
            &v,
            Severity::Critical,
            "attribute 3 (Spin Up Time) is below"
        ));
    }

    #[test]
    fn seagate_packed_raw_values_ignored() {
        let mut r = healthy_hdd();
        r.attributes = vec![
            attr(1, "Raw_Read_Error_Rate", 0x0000_0A3F_1C2B_44E1, ""),
            attr(7, "Seek_Error_Rate", 0x0000_0002_0B1F_E2A0, ""),
            attr(195, "Hardware_ECC_Recovered", 0x0000_0000_0B1F_0042, ""),
        ];
        assert_eq!(evaluate(&r).grade, Grade::Pass);
    }

    #[test]
    fn crc_only_is_info() {
        let mut r = healthy_hdd();
        r.health.udma_crc_errors = Some(27);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass);
        assert!(has(
            &v,
            Severity::Info,
            "27 interface (CRC) errors - usually a cable or USB adapter fault"
        ));
    }

    #[test]
    fn command_timeouts() {
        let mut r = healthy_hdd();
        r.health.command_timeouts = Some(5);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass);
        assert!(has(&v, Severity::Info, "5 command timeouts"));

        r.health.command_timeouts = Some(TIMEOUTS_CAUTION);
        assert_eq!(evaluate(&r).grade, Grade::Caution);
    }

    #[test]
    fn no_self_test_cautions() {
        let mut r = healthy_hdd();
        r.self_tests.clear();
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(
            &v,
            Severity::Caution,
            "No completed self-test on record - run an extended self-test"
        ));
        assert_eq!(
            v.summary,
            "No critical problems, but 1 item needs attention before relying on this drive."
        );
    }

    #[test]
    fn stale_self_test_cautions() {
        let mut r = healthy_hdd();
        r.self_tests[0].lifetime_hours = Some(23_140 - SELF_TEST_MAX_AGE_H - 1);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(
            &v,
            Severity::Caution,
            "Last self-test ran 169 power-on hours ago - run an extended"
        ));

        r.self_tests[0].lifetime_hours = Some(23_140 - SELF_TEST_MAX_AGE_H);
        assert_eq!(evaluate(&r).grade, Grade::Pass);
    }

    #[test]
    fn self_test_age_unknown_cautions() {
        let mut r = healthy_hdd();
        r.self_tests[0].lifetime_hours = None;
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(&v, Severity::Caution, "at an unknown time"));
    }

    #[test]
    fn ata_self_test_hour_stamp_wraps() {
        let mut r = healthy_hdd();
        r.health.power_on_hours = Some(70_000);
        r.self_tests[0].lifetime_hours = Some(70_000 - 2 - 0x1_0000);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass, "{v:#?}");
        assert!(has(&v, Severity::Good, "passed 2 power-on hours ago"));
        assert!(has(&v, Severity::Info, "high for a used drive"));
    }

    #[test]
    fn short_test_only_adds_info() {
        let mut r = healthy_hdd();
        r.self_tests = vec![test(
            "Short offline",
            "Completed without error",
            Some(true),
            23_139,
        )];
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass);
        assert!(has(
            &v,
            Severity::Good,
            "Short self-test passed 1 power-on hour ago"
        ));
        assert!(has(&v, Severity::Info, "No recent extended self-test"));
    }

    #[test]
    fn short_after_extended_credits_extended() {
        let mut r = healthy_hdd();
        r.self_tests.insert(
            0,
            test(
                "Short offline",
                "Completed without error",
                Some(true),
                23_140,
            ),
        );
        let v = evaluate(&r);
        assert!(has(
            &v,
            Severity::Good,
            "Extended self-test (full surface scan) passed 40"
        ));
        assert!(!has(&v, Severity::Info, "No recent extended"));
    }

    #[test]
    fn self_test_in_progress_is_not_failure() {
        let mut r = healthy_hdd();
        r.self_tests.insert(
            0,
            test(
                "Extended offline",
                "Self-test routine in progress",
                Some(false),
                23_140,
            ),
        );
        r.self_test_in_progress = Some(SelfTestProgress {
            kind: Some("Extended".into()),
            remaining_pct: Some(60),
        });
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass, "{v:#?}");
        assert!(has(
            &v,
            Severity::Info,
            "Extended self-test in progress (60% remaining)"
        ));
    }

    #[test]
    fn aborted_and_interrupted_are_not_failures() {
        let mut r = healthy_hdd();
        r.self_tests.insert(
            0,
            test("Short offline", "Aborted by host", Some(false), 23_139),
        );
        r.self_tests.insert(
            0,
            test(
                "Extended offline",
                "Interrupted (host reset)",
                Some(false),
                23_140,
            ),
        );
        assert_eq!(evaluate(&r).grade, Grade::Pass);

        r.self_tests = vec![test(
            "Short offline",
            "Aborted by host",
            Some(false),
            23_139,
        )];
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(&v, Severity::Caution, "No completed self-test"));
    }

    #[test]
    fn recent_self_test_failure_fails() {
        let mut r = healthy_hdd();
        let mut t = test(
            "Extended offline",
            "Completed: read failure",
            Some(false),
            23_120,
        );
        t.lba_first_error = Some(1_234_567);
        r.self_tests.insert(0, t);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Fail);
        assert!(has(
            &v,
            Severity::Critical,
            "Most recent self-test failed (Completed: read failure, first bad block at LBA 1,234,567)"
        ));
    }

    #[test]
    fn fatal_error_status_fails() {
        let mut r = healthy_hdd();
        r.self_tests = vec![test(
            "Short offline",
            "Fatal or unknown error",
            None,
            23_139,
        )];
        assert_eq!(evaluate(&r).grade, Grade::Fail);
    }

    #[test]
    fn older_self_test_failure_cautions() {
        let mut r = healthy_hdd();
        r.self_tests.push(test(
            "Short offline",
            "Completed: electrical failure",
            Some(false),
            9_000,
        ));
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(
            &v,
            Severity::Caution,
            "An earlier self-test failed at hour 9,000"
        ));
    }

    #[test]
    fn hot_hdd_cautions() {
        let mut r = healthy_hdd();
        r.health.temperature_max_c = Some(HDD_TEMP_MAX_C + 1);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Caution);
        assert!(has(&v, Severity::Caution, "61 C - above the typical 60 C"));
    }

    #[test]
    fn healthy_nvme_passes() {
        let v = evaluate(&healthy_nvme());
        assert_eq!(v.grade, Grade::Pass, "{v:#?}");
        assert!(has(&v, Severity::Good, "No NVMe critical warnings"));
        assert!(has(
            &v,
            Severity::Good,
            "Low wear: 3% of rated write endurance used"
        ));
        assert!(has(&v, Severity::Good, "Spare capacity at 100%"));
        assert!(has(&v, Severity::Good, "No media or data integrity errors"));
        assert!(has(&v, Severity::Info, "12 unsafe shutdowns"));
        assert!(!has(&v, Severity::Info, "error log"));
        assert!(!has(&v, Severity::Good, "sectors"));
    }

    #[test]
    fn nvme_endurance_tiers() {
        let grade = |pct| {
            let mut r = healthy_nvme();
            r.health.endurance_used_pct = Some(pct);
            evaluate(&r).grade
        };
        assert_eq!(grade(ENDURANCE_INFO_PCT), Grade::Pass);
        assert_eq!(grade(ENDURANCE_CAUTION_PCT), Grade::Caution);
        assert_eq!(grade(ENDURANCE_FAIL_PCT), Grade::Fail);
        assert_eq!(grade(140), Grade::Fail);
    }

    #[test]
    fn nvme_worn_out() {
        let mut r = healthy_nvme();
        r.health.endurance_used_pct = Some(100);
        r.health.available_spare_pct = Some(5);
        r.health.nvme_critical_warning = Some(0x05);
        r.health.nvme_media_errors = Some(3);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Fail);
        assert!(has(
            &v,
            Severity::Critical,
            "100% of rated write endurance used - past its rated lifespan"
        ));
        assert!(has(
            &v,
            Severity::Critical,
            "Spare capacity at 5%, below the drive's 10% safety threshold"
        ));
        assert!(has(
            &v,
            Severity::Critical,
            "NVMe critical warning: spare capacity below threshold; reliability degraded"
        ));
        assert!(has(
            &v,
            Severity::Caution,
            "3 media or data integrity errors"
        ));
        assert!(v.summary.starts_with("3 critical problems found"));
    }

    #[test]
    fn nvme_low_spare_cautions() {
        let mut r = healthy_nvme();
        r.health.available_spare_pct = Some(40);
        assert_eq!(evaluate(&r).grade, Grade::Caution);
    }

    #[test]
    fn nvme_unknown_warning_bits() {
        assert_eq!(decode_nvme_warning(0x80), "unknown warning 0x80");
        assert_eq!(decode_nvme_warning(0x08), "drive has switched to read-only");
    }

    #[test]
    fn ata_error_log_is_info() {
        let mut r = healthy_hdd();
        r.health.error_log_count = Some(1);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Pass);
        assert!(has(&v, Severity::Info, "1 entry in the drive's error log"));
    }

    #[test]
    fn no_smart_data_is_unknown() {
        let v = evaluate(&DriveReport::default());
        assert_eq!(v.grade, Grade::Unknown);
        assert!(has(
            &v,
            Severity::Caution,
            "No SMART health data could be read"
        ));

        let mut r = healthy_hdd();
        r.health.smart_supported = Some(false);
        assert_eq!(evaluate(&r).grade, Grade::Unknown);

        let mut r = healthy_hdd();
        r.health.smart_enabled = Some(false);
        let v = evaluate(&r);
        assert_eq!(v.grade, Grade::Unknown);
        assert!(has(&v, Severity::Caution, "SMART is disabled"));
    }

    #[test]
    fn helpers() {
        assert_eq!(plural(1, "sector"), "1 sector");
        assert_eq!(plural(1_500, "sector"), "1,500 sectors");
        assert_eq!(plural(2, "entry"), "2 entries");
        assert_eq!(join_or(&["a"]), "a");
        assert_eq!(join_or(&["a", "b"]), "a or b");
        assert_eq!(join_or(&["a", "b", "c"]), "a, b or c");
    }
}
