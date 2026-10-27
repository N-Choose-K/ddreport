//! One-page PDF certificate rendered with an embedded Typst template.
//!
//! Rust pre-computes a display model (formatted numbers, truncated lists,
//! status levels); the template only lays it out. If the result spills onto a
//! second page, rendering retries at a denser level with shorter lists.

use std::fmt::Write as _;

use thousands::Separable;

use crate::fmt::bytes_human;
use crate::health;
use std::sync::LazyLock;

use anyhow::{Context, anyhow};
use serde::Serialize;
use typst::WorldExt;
use typst::diag::{Severity as DiagSeverity, SourceDiagnostic};
use typst::foundations::{Dict, IntoValue};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_layout::PagedDocument;

use crate::model::{
    Attribute, DriveReport, Grade, Health, Identity, MediaKind, Protocol, SelfTest, Severity,
    UsbBridge,
};

const TEMPLATE: &str = include_str!("../templates/certificate.typ");

const DISCLAIMER: &str =
    "Snapshot of drive self-reported data at the time of testing; not a warranty.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Paper {
    #[default]
    Letter,
    A4,
}

impl Paper {
    fn typst_name(self) -> &'static str {
        match self {
            Paper::Letter => "us-letter",
            Paper::A4 => "a4",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    pub paper: Paper,
}

/// Overpass and Overpass Mono (SIL OFL, see assets/fonts/OFL-Overpass*.txt).
static FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/Overpass-Regular.ttf"),
    include_bytes!("../assets/fonts/Overpass-Italic.ttf"),
    include_bytes!("../assets/fonts/Overpass-Bold.ttf"),
    include_bytes!("../assets/fonts/OverpassMono-Regular.ttf"),
    include_bytes!("../assets/fonts/OverpassMono-Bold.ttf"),
];

static ENGINE: LazyLock<TypstEngine<TypstTemplateMainFile>> = LazyLock::new(|| {
    TypstEngine::builder()
        .main_file(TEMPLATE)
        .fonts(FONTS)
        .build()
});

/// Renders a one-page US Letter certificate, locked against modification.
pub fn render(r: &DriveReport) -> anyhow::Result<Vec<u8>> {
    render_with(r, &PdfOptions::default())
}

/// Renders a one-page certificate with the given options, locked against
/// modification (see `lock`).
pub fn render_with(r: &DriveReport, opts: &PdfOptions) -> anyhow::Result<Vec<u8>> {
    crate::lock::lock(&render_unlocked(r, opts)?)
}

/// Renders the certificate without the modification lock; deterministic.
pub fn render_unlocked(r: &DriveReport, opts: &PdfOptions) -> anyhow::Result<Vec<u8>> {
    let data = serde_json::to_string(r).context("serializing report")?;
    for level in 0..LAYOUTS.len() {
        let display = build_display(r, opts.paper, level);
        let doc = compile(&data, &display)?;
        if doc.pages().len() == 1 || level == MAX_LEVEL {
            let pdf_opts = typst_pdf::PdfOptions::default();
            return typst_pdf::pdf(&doc, &pdf_opts)
                .map_err(|d| anyhow!("PDF export failed:\n{}", format_diags(&d, None)));
        }
    }
    unreachable!()
}

/// Layouts tried in order until the certificate fits one page. The last one
/// also clips the body, so output is always a single page.
const LAYOUTS: [Layout; 6] = [
    Layout::new("roomy", true, [10, 40, 6, 16, 3]),
    Layout::new("compact", true, [10, 40, 6, 16, 3]),
    Layout::new("compact", false, [8, 36, 6, 12, 2]),
    Layout::new("compact", false, [8, 32, 5, 8, 2]),
    Layout::new("dense", false, [6, 30, 4, 6, 1]),
    Layout::new("dense", false, [5, 24, 3, 0, 1]),
];
const MAX_LEVEL: usize = LAYOUTS.len() - 1;

struct Layout {
    /// Type scale: "roomy", "compact" or "dense".
    style: &'static str,
    verify_steps: bool,
    findings: usize,
    attributes: usize,
    self_tests: usize,
    stats: usize,
    warnings: usize,
}

impl Layout {
    const fn new(style: &'static str, verify_steps: bool, max: [usize; 5]) -> Self {
        let [findings, attributes, self_tests, stats, warnings] = max;
        Self {
            style,
            verify_steps,
            findings,
            attributes,
            self_tests,
            stats,
            warnings,
        }
    }
}

fn compile(data: &str, display: &Display) -> anyhow::Result<PagedDocument> {
    let display = serde_json::to_string(display).context("serializing display model")?;
    let mut inputs = Dict::new();
    inputs.insert("data".into(), data.into_value());
    inputs.insert("display".into(), display.as_str().into_value());

    let world = ENGINE
        .world_builder()
        .with_inputs(inputs)
        .build()
        .map_err(|e| anyhow!("setting up Typst world: {e}"))?;
    let warned = typst::compile::<PagedDocument>(&world);
    typst::comemo::evict(10);
    warned.output.map_err(|errs| {
        let mut all: Vec<SourceDiagnostic> = errs.to_vec();
        all.extend(warned.warnings.iter().cloned());
        anyhow!(
            "certificate template failed to compile:\n{}",
            format_diags(&all, Some(&world))
        )
    })
}

fn format_diags(diags: &[SourceDiagnostic], world: Option<&dyn typst::World>) -> String {
    let mut out = String::new();
    for d in diags {
        let kind = match d.severity {
            DiagSeverity::Error => "error",
            DiagSeverity::Warning => "warning",
        };
        let loc = world
            .and_then(|w| w.range(d.span))
            .map(|r| {
                let line = TEMPLATE[..r.start.min(TEMPLATE.len())]
                    .lines()
                    .count()
                    .max(1);
                let snippet = TEMPLATE.get(r).unwrap_or("").lines().next().unwrap_or("");
                format!(
                    " at certificate.typ:{line}: `{}`",
                    truncate(snippet.trim(), 60)
                )
            })
            .unwrap_or_default();
        let _ = writeln!(out, "  {kind}{loc}: {}", d.message);
        for h in &d.hints {
            let _ = writeln!(out, "    hint: {}", h.v);
        }
    }
    out
}

// ---------- display model ----------

#[derive(Serialize)]
struct Display {
    paper: &'static str,
    style: &'static str,
    verify_steps: bool,
    clip: bool,
    grade: &'static str,
    summary: String,
    generated: String,
    host: String,
    source: String,
    tool_version: String,
    model: String,
    vendor_line: String,
    serial: String,
    poh: Option<String>,
    identity: Vec<Kv>,
    bridge: Vec<Kv>,
    tiles: Vec<Tile>,
    findings: Listed<FindingRow>,
    attributes: Listed<AttrRow>,
    attr_note: Option<String>,
    self_tests: Listed<TestRow>,
    self_test_note: Option<String>,
    stats: Listed<Kv>,
    warnings: Listed<String>,
    sha256: String,
    rubric: Vec<Kv>,
    sources: Vec<&'static str>,
    disclaimer: &'static str,
}

#[derive(Serialize)]
struct Kv {
    k: String,
    v: String,
}

fn kv(k: &str, v: impl Into<String>) -> Kv {
    Kv {
        k: k.into(),
        v: truncate(&v.into(), 80),
    }
}

#[derive(Serialize)]
struct Listed<T> {
    items: Vec<T>,
    more: usize,
}

fn listed<T>(mut items: Vec<T>, max: usize) -> Listed<T> {
    let more = items.len().saturating_sub(max);
    items.truncate(max);
    Listed { items, more }
}

/// Like `listed`, but keeps the worst rows (in original order) when truncating.
fn listed_worst<T>(items: Vec<T>, max: usize, level: impl Fn(&T) -> Level) -> Listed<T> {
    if items.len() <= max {
        return listed(items, max);
    }
    let mut ranked: Vec<(usize, u8)> = items
        .iter()
        .enumerate()
        .map(|(i, t)| (i, level(t).rank()))
        .collect();
    ranked.sort_by_key(|&(i, rank)| (std::cmp::Reverse(rank), i));
    let mut keep: Vec<usize> = ranked.into_iter().take(max).map(|(i, _)| i).collect();
    keep.sort_unstable();
    let more = items.len() - max;
    let mut items: Vec<Option<T>> = items.into_iter().map(Some).collect();
    Listed {
        items: keep.into_iter().filter_map(|i| items[i].take()).collect(),
        more,
    }
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum Level {
    Ok,
    Info,
    Warn,
    Bad,
    Na,
}

impl Level {
    fn rank(self) -> u8 {
        match self {
            Level::Bad => 3,
            Level::Warn => 2,
            Level::Info | Level::Na => 1,
            Level::Ok => 0,
        }
    }
}

#[derive(Serialize)]
struct Tile {
    label: &'static str,
    value: String,
    sub: String,
    level: Level,
}

#[derive(Serialize)]
struct FindingRow {
    level: Level,
    text: String,
}

#[derive(Serialize)]
struct AttrRow {
    id: String,
    name: String,
    value: String,
    worst: String,
    thresh: String,
    raw: String,
    status: &'static str,
    level: Level,
}

#[derive(Serialize)]
struct TestRow {
    kind: String,
    status: String,
    hours: String,
    lba: String,
    level: Level,
}

fn build_display(r: &DriveReport, paper: Paper, level: usize) -> Display {
    let lim = &LAYOUTS[level];
    let id = &r.identity;

    let attr_rows: Vec<AttrRow> = r.attributes.iter().map(attr_row).collect();
    // NVMe has no attribute table; its health log is already shown as tiles.
    let attr_note = (r.attributes.is_empty() && id.protocol != Protocol::Nvme)
        .then(|| "The drive did not report a SMART attribute table.".to_string());

    let self_test_note = match &r.self_test_in_progress {
        Some(p) => Some(format!(
            "{} test in progress{}.",
            p.kind.as_deref().unwrap_or("Self"),
            p.remaining_pct
                .map(|n| format!(", {n}% remaining"))
                .unwrap_or_default()
        )),
        None if r.self_tests.is_empty() => Some("No self-tests logged.".into()),
        None => None,
    };

    Display {
        paper: paper.typst_name(),
        style: lim.style,
        verify_steps: lim.verify_steps,
        clip: level == MAX_LEVEL,
        grade: grade_label(r.verdict.grade),
        summary: truncate(&r.verdict.summary, 120),
        generated: format_timestamp(&r.meta.generated_at),
        host: truncate(&join_nonempty(&[&r.meta.host, &r.meta.os], " - "), 48),
        source: truncate(
            &join_nonempty(
                &[
                    &r.meta.backend,
                    &r.meta.device_path,
                    &paren(&r.meta.device_type),
                ],
                " ",
            ),
            56,
        ),
        tool_version: match r.meta.tool_version.trim() {
            "" => "ddreport".into(),
            v if v.starts_with("ddreport") => truncate(v, 24),
            v => truncate(&format!("ddreport {v}"), 24),
        },
        model: truncate(id.model.as_deref().unwrap_or("Unknown model"), 44),
        vendor_line: truncate(
            &[id.manufacturer.as_deref(), id.model_family.as_deref()]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" - "),
            70,
        ),
        serial: clip(id.serial.as_deref().unwrap_or("Not reported"), 40),
        poh: r.health.power_on_hours.map(count),
        identity: identity_rows(id, &r.health),
        bridge: r.bridge.as_ref().map(bridge_rows).unwrap_or_default(),
        tiles: tiles(id, &r.health),
        findings: listed_worst(
            r.verdict
                .findings
                .iter()
                .map(|f| FindingRow {
                    level: match f.severity {
                        Severity::Good => Level::Ok,
                        Severity::Info => Level::Info,
                        Severity::Caution => Level::Warn,
                        Severity::Critical => Level::Bad,
                    },
                    text: truncate(&f.message, 100),
                })
                .collect(),
            lim.findings,
            |f| f.level,
        ),
        attributes: listed_worst(attr_rows, lim.attributes, |a| a.level),
        attr_note,
        self_tests: listed(r.self_tests.iter().map(test_row).collect(), lim.self_tests),
        self_test_note,
        stats: listed(
            r.statistics
                .iter()
                .map(|s| {
                    let v = s.value.trim();
                    let v = v.parse::<u64>().map_or_else(|_| v.to_string(), count);
                    kv(&truncate(&s.name, 34), truncate(&v, 22))
                })
                .collect(),
            lim.stats,
        ),
        warnings: listed(
            r.meta.warnings.iter().map(|w| truncate(w, 120)).collect(),
            lim.warnings,
        ),
        sha256: r.meta.raw_sha256.clone(),
        rubric: health::RUBRIC
            .iter()
            .map(|(g, t)| Kv {
                k: (*g).into(),
                v: (*t).into(),
            })
            .collect(),
        sources: health::SOURCES.to_vec(),
        disclaimer: DISCLAIMER,
    }
}

fn identity_rows(id: &Identity, h: &Health) -> Vec<Kv> {
    let mut rows = Vec::new();
    if let Some(v) = &id.firmware {
        rows.push(kv("Firmware", truncate(v, 30)));
    }
    rows.push(kv(
        "Capacity",
        match id.capacity_bytes {
            Some(b) => format!("{} ({} bytes)", bytes_human(b), b.separate_with_commas()),
            None => "Not reported".into(),
        },
    ));
    match (id.logical_sector_size, id.physical_sector_size) {
        (Some(l), Some(p)) if l != p => {
            rows.push(kv("Sector size", format!("{l} B logical / {p} B physical")))
        }
        (Some(l), _) | (None, Some(l)) => rows.push(kv("Sector size", format!("{l} B"))),
        _ => {}
    }
    let media = match (id.media, id.rotation_rpm) {
        (MediaKind::Hdd, Some(rpm)) if rpm > 0 => {
            format!("Hard disk, {} rpm", u64::from(rpm).separate_with_commas())
        }
        (MediaKind::Hdd, _) => "Hard disk".into(),
        (MediaKind::Ssd, _) if id.protocol == Protocol::Nvme => "NVMe solid-state drive".into(),
        (MediaKind::Ssd, _) => "Solid-state drive".into(),
        (MediaKind::Unknown, _) => String::new(),
    };
    let media = [Some(media), id.form_factor.clone()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    if !media.is_empty() {
        rows.push(kv("Type", truncate(&media, 48)));
    }
    let iface: Vec<String> = match id.protocol {
        Protocol::Nvme => vec![format!(
            "NVMe{}",
            id.nvme_version
                .as_deref()
                .map(|v| format!(" {v}"))
                .unwrap_or_default()
        )],
        Protocol::Ata => [id.sata_version.clone(), id.ata_version.clone()]
            .into_iter()
            .flatten()
            .collect(),
        Protocol::Scsi => vec!["SCSI / SAS".into()],
        Protocol::Unknown => vec![],
    };
    if !iface.is_empty() {
        rows.push(kv("Interface", truncate(&iface.join("; "), 60)));
    }
    match (&id.link_speed_max, &id.link_speed_current) {
        (Some(m), Some(c)) if m != c => rows.push(kv("Link speed", format!("{c} (max {m})"))),
        (Some(s), _) | (None, Some(s)) => rows.push(kv("Link speed", s.clone())),
        _ => {}
    }
    if let Some(v) = &id.wwn {
        rows.push(kv(
            if id.protocol == Protocol::Nvme {
                "EUI-64 / WWN"
            } else {
                "WWN"
            },
            truncate(v, 40),
        ));
    }
    if let Some(t) = id.trim_supported {
        rows.push(kv("TRIM", if t { "Supported" } else { "Not supported" }));
    }
    let smart = match (h.smart_supported, h.smart_enabled) {
        (Some(false), _) => Some("Not supported"),
        (_, Some(true)) => Some("Supported, enabled"),
        (_, Some(false)) => Some("Supported, disabled"),
        (Some(true), None) => Some("Supported"),
        (None, None) => None,
    };
    if let Some(s) = smart {
        rows.push(kv("SMART", s));
    }
    if let Some(db) = id.in_drive_database {
        rows.push(kv(
            "Drive database",
            if db { "Known model" } else { "Not in database" },
        ));
    }
    rows
}

fn bridge_rows(b: &UsbBridge) -> Vec<Kv> {
    let mut rows = Vec::new();
    let name = [b.vendor_name.as_deref(), b.product_name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    rows.push(kv(
        "Chipset",
        truncate(
            b.chipset
                .as_deref()
                .or((!name.is_empty()).then_some(name.as_str()))
                .unwrap_or("Unknown"),
            40,
        ),
    ));
    rows.push(kv(
        "USB ID",
        format!("{:04x}:{:04x}", b.vendor_id, b.product_id),
    ));
    if b.chipset.is_some() && !name.is_empty() {
        rows.push(kv("Product", truncate(&name, 40)));
    }
    if let Some(t) = &b.transport {
        rows.push(kv("Transport", t.clone()));
    }
    if let Some(s) = &b.link_speed {
        rows.push(kv("USB speed", truncate(s, 40)));
    }
    rows
}

fn tiles(id: &Identity, h: &Health) -> Vec<Tile> {
    let nvme = id.protocol == Protocol::Nvme;
    let ssd = id.media == MediaKind::Ssd || nvme;
    let mut t = Vec::new();

    t.push(match h.power_on_hours {
        Some(hrs) => tile(
            "Power-on time",
            format!("{} h", count(hrs)),
            years(hrs),
            Level::Info,
        ),
        None => na("Power-on time"),
    });
    t.push(count_tile("Power cycles", h.power_cycles, None));
    t.push(match h.temperature_c {
        Some(c) => tile(
            "Temperature",
            format!("{c} °C"),
            h.temperature_max_c
                .map(|m| format!("max {m} °C"))
                .unwrap_or_else(|| "current".into()),
            if c >= 65 {
                Level::Bad
            } else if c >= 55 {
                Level::Warn
            } else {
                Level::Ok
            },
        ),
        None => na("Temperature"),
    });

    if nvme {
        t.push(endurance_tile(h));
        t.push(spare_tile(h));
        t.push(zero_good_tile(
            "Media errors",
            h.nvme_media_errors,
            Level::Bad,
        ));
        t.push(match h.nvme_critical_warning {
            Some(0) => tile("Critical warning", "None", "", Level::Ok),
            Some(w) => tile(
                "Critical warning",
                format!("0x{w:02x}"),
                "flags set",
                Level::Bad,
            ),
            None => na("Critical warning"),
        });
        t.push(count_tile("Unsafe shutdowns", h.unsafe_shutdowns, None));
        t.push(count_tile("Error log entries", h.error_log_count, None));
    } else {
        t.push(zero_good_tile(
            "Reallocated sectors",
            h.reallocated_sectors,
            Level::Warn,
        ));
        t.push(zero_good_tile(
            "Pending sectors",
            h.pending_sectors,
            if health::pending_escalated(h) {
                Level::Bad
            } else {
                Level::Warn
            },
        ));
        t.push(uncorrectable_tile(h));
        t.push(zero_good_tile(
            "Interface CRC errors",
            h.udma_crc_errors,
            Level::Warn,
        ));
        if ssd {
            t.push(endurance_tile(h));
            if h.available_spare_pct.is_some() {
                t.push(spare_tile(h));
            }
        } else {
            t.push(count_tile("Load cycles", h.load_cycles, None));
        }
    }

    t.push(bytes_tile("Data written", h.bytes_written));
    t.push(bytes_tile("Data read", h.bytes_read));
    if t.len() < 12 && !nvme && h.command_timeouts.is_some() {
        t.push(count_tile("Command timeouts", h.command_timeouts, None));
    }
    if t.len() < 12 && !ssd && h.start_stop_count.is_some() {
        t.push(count_tile("Start/stop count", h.start_stop_count, None));
    }
    t.truncate(12);
    t
}

fn tile(
    label: &'static str,
    value: impl Into<String>,
    sub: impl Into<String>,
    level: Level,
) -> Tile {
    Tile {
        label,
        value: value.into(),
        sub: sub.into(),
        level,
    }
}

fn na(label: &'static str) -> Tile {
    tile(label, "n/a", "not reported", Level::Na)
}

fn count_tile(label: &'static str, v: Option<u64>, sub: Option<String>) -> Tile {
    match v {
        Some(n) => tile(label, count(n), sub.unwrap_or_default(), Level::Info),
        None => na(label),
    }
}

fn zero_good_tile(label: &'static str, v: Option<u64>, bad: Level) -> Tile {
    match v {
        Some(0) => tile(label, "0", "", Level::Ok),
        Some(n) => tile(label, count(n), "", bad),
        None => na(label),
    }
}

/// Combines offline (198) and reported (187) uncorrectable counts; the
/// larger one is the headline so a nonzero count is never hidden.
fn uncorrectable_tile(h: &Health) -> Tile {
    const LABEL: &str = "Uncorrectable";
    let (offline, reported) = (h.offline_uncorrectable, h.reported_uncorrectable);
    let Some(worst) = offline.max(reported) else {
        return na(LABEL);
    };
    let sub = match (offline, reported) {
        (Some(o), Some(r)) => format!("offline {} / reported {}", count(o), count(r)),
        (Some(_), None) => "offline scan".into(),
        _ => "reported to host".into(),
    };
    let level = if worst == 0 { Level::Ok } else { Level::Warn };
    tile(LABEL, count(worst), sub, level)
}

fn bytes_tile(label: &'static str, v: Option<u64>) -> Tile {
    match v {
        Some(b) => tile(label, bytes_human(b), "", Level::Info),
        None => na(label),
    }
}

fn endurance_tile(h: &Health) -> Tile {
    match h.endurance_used_pct {
        Some(p) => tile(
            "Endurance used",
            format!("{p}%"),
            format!("{}% remaining", 100u64.saturating_sub(p)),
            if p >= 100 {
                Level::Bad
            } else if p >= 80 {
                Level::Warn
            } else {
                Level::Ok
            },
        ),
        None => na("Endurance used"),
    }
}

fn spare_tile(h: &Health) -> Tile {
    match h.available_spare_pct {
        Some(p) => {
            let thr = h.available_spare_threshold_pct;
            tile(
                "Available spare",
                format!("{p}%"),
                thr.map(|t| format!("threshold {t}%")).unwrap_or_default(),
                match thr {
                    Some(t) if p <= t => Level::Bad,
                    _ if p < 50 => Level::Warn,
                    _ => Level::Ok,
                },
            )
        }
        None => na("Available spare"),
    }
}

/// ATA attributes where any nonzero raw count means media or link trouble.
const CRITICAL_IDS: [u8; 8] = [5, 10, 184, 187, 196, 197, 198, 199];

fn attr_row(a: &Attribute) -> AttrRow {
    let (status, level) = if a.when_failed == "now" || (a.thresh > 0 && a.value <= a.thresh) {
        ("FAILING", Level::Bad)
    } else if a.when_failed == "past" {
        ("Prior fail", Level::Warn)
    } else if CRITICAL_IDS.contains(&a.id) && a.raw > 0 {
        ("Check", Level::Warn)
    } else {
        ("OK", Level::Ok)
    };
    // Keep the leading value only: "34 (Min/Max 18/49)" -> "34".
    let lead = a.raw_string.split_whitespace().next().unwrap_or("");
    let raw = match lead.parse::<u64>() {
        Ok(n) => count(n),
        Err(_) if lead.is_empty() => count(a.raw),
        Err(_) => lead.to_string(),
    };
    AttrRow {
        id: a.id.to_string(),
        name: truncate(&a.name.replace('_', " "), 23),
        value: format!("{:03}", a.value),
        worst: format!("{:03}", a.worst),
        thresh: format!("{:03}", a.thresh),
        raw: truncate(&raw, 16),
        status,
        level,
    }
}

fn test_row(t: &SelfTest) -> TestRow {
    TestRow {
        kind: truncate(&t.kind, 22),
        status: truncate(&t.status, 40),
        hours: t.lifetime_hours.map(count).unwrap_or_else(|| "-".into()),
        lba: t
            .lba_first_error
            .map(|l| l.to_string())
            .unwrap_or_else(|| "-".into()),
        level: match t.passed {
            Some(true) => Level::Ok,
            Some(false) => Level::Bad,
            None => Level::Info,
        },
    }
}

// ---------- formatting helpers ----------

/// Thousands-separated, or scientific for implausibly large values.
fn count(n: u64) -> String {
    if n >= 1_000_000_000_000 {
        format!("{:.2e}", n as f64)
    } else {
        n.separate_with_commas()
    }
}

fn grade_label(g: Grade) -> &'static str {
    match g {
        Grade::Pass => "PASS",
        Grade::Caution => "CAUTION",
        Grade::Fail => "FAIL",
        Grade::Unknown => "UNKNOWN",
    }
}

fn paren(s: &str) -> String {
    if s.trim().is_empty() {
        String::new()
    } else {
        format!("(-d {})", s.trim())
    }
}

fn join_nonempty(parts: &[&str], sep: &str) -> String {
    parts
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(sep)
}

/// Clips to `max` chars and lets long unbroken runs wrap, so no cell can
/// overflow sideways.
fn truncate(s: &str, max: usize) -> String {
    breakable(&clip(s, max))
}

fn breakable(s: &str) -> String {
    const RUN: usize = 14;
    let mut out = String::with_capacity(s.len());
    let mut run = 0;
    for c in s.chars() {
        if c.is_whitespace() || matches!(c, '-' | '/' | '\u{200B}') {
            run = 0;
        } else if run == RUN {
            out.push('\u{200B}');
            run = 0;
        }
        run += usize::from(!c.is_whitespace());
        out.push(c);
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn years(hours: u64) -> String {
    if hours > 1_000_000 {
        return "implausible value".into();
    }
    let days = hours as f64 / 24.0;
    if days < 60.0 {
        format!("{days:.0} days")
    } else if days < 730.0 {
        format!("{:.1} months", days / 30.44)
    } else {
        format!("{:.1} years", days / 365.25)
    }
}

fn format_timestamp(ts: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(t) => t.format("%Y-%m-%d %H:%M %:z").to_string(),
        Err(_) if ts.is_empty() => "Unknown".into(),
        Err(_) => truncate(ts, 32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn page_count(pdf: &[u8]) -> usize {
        let s = String::from_utf8_lossy(pdf);
        s.matches("/Type/Page").count() - s.matches("/Type/Pages").count()
    }

    fn maximal() -> DriveReport {
        let long = "X".repeat(200);
        DriveReport {
            meta: ReportMeta {
                generated_at: "2026-10-25T12:34:56Z".into(),
                host: long.clone(),
                os: long.clone(),
                tool_version: "0.1.0".into(),
                backend: long.clone(),
                device_path: long.clone(),
                device_type: long.clone(),
                raw_sha256: "a".repeat(64),
                warnings: vec![long.clone(); 10],
                collection_errors: vec![],
            },
            identity: Identity {
                manufacturer: Some(long.clone()),
                model_family: Some(long.clone()),
                model: Some(long.clone()),
                serial: Some(long.clone()),
                firmware: Some(long.clone()),
                wwn: Some(long.clone()),
                capacity_bytes: Some(u64::MAX),
                logical_sector_size: Some(512),
                physical_sector_size: Some(4096),
                protocol: Protocol::Ata,
                media: MediaKind::Ssd,
                rotation_rpm: Some(7200),
                form_factor: Some(long.clone()),
                ata_version: Some(long.clone()),
                sata_version: Some(long.clone()),
                nvme_version: Some(long.clone()),
                link_speed_max: Some(long.clone()),
                link_speed_current: Some("1.5 Gb/s".into()),
                trim_supported: Some(true),
                in_drive_database: Some(false),
            },
            bridge: Some(UsbBridge {
                vendor_id: 0x152d,
                product_id: 0x0578,
                vendor_name: Some(long.clone()),
                product_name: Some(long.clone()),
                serial: Some(long.clone()),
                chipset: Some(long.clone()),
                transport: Some(long.clone()),
                link_speed: Some(long.clone()),
            }),
            health: Health {
                smart_supported: Some(true),
                smart_enabled: Some(true),
                overall_passed: Some(false),
                power_on_hours: Some(u64::MAX),
                power_cycles: Some(u64::MAX),
                start_stop_count: Some(u64::MAX),
                load_cycles: Some(u64::MAX),
                temperature_c: Some(99),
                temperature_max_c: Some(120),
                reallocated_sectors: Some(u64::MAX),
                reallocation_events: Some(u64::MAX),
                pending_sectors: Some(u64::MAX),
                offline_uncorrectable: Some(u64::MAX),
                reported_uncorrectable: Some(u64::MAX),
                udma_crc_errors: Some(u64::MAX),
                command_timeouts: Some(u64::MAX),
                bytes_read: Some(u64::MAX),
                bytes_written: Some(u64::MAX),
                endurance_used_pct: Some(250),
                available_spare_pct: Some(1),
                available_spare_threshold_pct: Some(10),
                ..Default::default()
            },
            attributes: (0..48)
                .map(|i| Attribute {
                    id: i as u8,
                    name: long.clone(),
                    value: 1,
                    worst: 1,
                    thresh: 10,
                    raw: u64::MAX,
                    raw_string: long.clone(),
                    prefailure: true,
                    when_failed: "now".into(),
                })
                .collect(),
            self_tests: vec![
                SelfTest {
                    kind: long.clone(),
                    status: long.clone(),
                    passed: Some(false),
                    lifetime_hours: Some(u64::MAX),
                    lba_first_error: Some(u64::MAX),
                };
                21
            ],
            self_test_in_progress: Some(SelfTestProgress {
                kind: Some(long.clone()),
                remaining_pct: Some(90),
            }),
            self_test_status_known: true,
            statistics: vec![
                Stat {
                    group: long.clone(),
                    name: long.clone(),
                    value: long.clone(),
                };
                60
            ],
            verdict: Verdict {
                grade: Grade::Fail,
                summary: long.repeat(3),
                findings: vec![
                    Finding {
                        severity: Severity::Critical,
                        message: long.clone(),
                    };
                    30
                ],
            },
        }
    }

    #[test]
    fn maximal_report_fits_one_page() {
        for paper in [Paper::Letter, Paper::A4] {
            let pdf = render_with(&maximal(), &PdfOptions { paper }).unwrap();
            assert!(pdf.starts_with(b"%PDF"));
            assert_eq!(page_count(&pdf), 1, "{paper:?}");
        }
    }

    #[test]
    fn empty_report_renders() {
        let pdf = render(&DriveReport::default()).unwrap();
        assert_eq!(page_count(&pdf), 1);
    }

    #[test]
    fn realistic_worst_case_fits_without_clipping() {
        let r = realistic_worst();
        let data = serde_json::to_string(&r).unwrap();
        let fits = (0..MAX_LEVEL).any(|level| {
            let d = build_display(&r, Paper::Letter, level);
            compile(&data, &d).unwrap().pages().len() == 1
        });
        assert!(fits, "only the clipping fallback fits");
    }

    /// Long but plausible values: what a cluttered real drive can produce.
    fn realistic_worst() -> DriveReport {
        let mut r = maximal();
        let words = "Vendor Specific Attribute Data Value";
        r.meta.host = "build-server-07.example.internal".into();
        r.meta.os = "Ubuntu 24.04.1 LTS (x86_64)".into();
        r.meta.backend = "smartctl 7.5".into();
        r.meta.device_path = "/dev/disk/by-id/usb-JMicron_Generic_0123456789AB-0:0".into();
        r.meta.device_type = "sat,auto".into();
        r.meta.warnings = vec![format!("{words} could not be read from the device"); 5];
        let id = &mut r.identity;
        id.manufacturer = Some("Western Digital".into());
        id.model_family = Some("Western Digital Ultrastar DC HC550 (SATA, 18TB)".into());
        id.model = Some("WDC WUH721818ALE6L4 (Ultrastar DC HC550)".into());
        id.serial = Some("2BJK3XYZ0123456789AB".into());
        id.firmware = Some("PCGNW232".into());
        id.wwn = Some("5 000cca 2b9c4f1a7".into());
        id.form_factor = Some("3.5 inches".into());
        id.ata_version = Some("ACS-4 published, ANSI INCITS 529-2018".into());
        id.sata_version = Some("SATA 3.3".into());
        id.link_speed_max = Some("6.0 Gb/s".into());
        let b = r.bridge.as_mut().unwrap();
        b.chipset = Some("JMicron JMS583 (USB 3.2 Gen 2)".into());
        b.vendor_name = Some("JMicron Technology Corp.".into());
        b.product_name = Some("JMS583 Gen 2 to PCIe Gen3x2 Bridge".into());
        b.transport = Some("UAS".into());
        b.link_speed = Some("USB 3.2 Gen 2 (10 Gb/s)".into());
        for (i, a) in r.attributes.iter_mut().enumerate() {
            a.name = "Vendor_Specific_Attribute_Name".into();
            a.raw = 12_885_098_501 + i as u64;
            a.raw_string = a.raw.to_string();
        }
        for t in &mut r.self_tests {
            t.kind = "Extended offline".into();
            t.status = "Completed: read failure (90% remaining)".into();
            t.lifetime_hours = Some(38_901);
            t.lba_first_error = Some(1_841_220_416);
        }
        r.self_test_in_progress = None;
        for (i, st) in r.statistics.iter_mut().enumerate() {
            st.name = format!("Number of Reallocated Logical Sectors {i}");
            st.value = "62,317,220,832".into();
        }
        r.verdict.summary =
            "Active media failure: pending and reallocated sectors with failed self-tests.".into();
        for f in &mut r.verdict.findings {
            f.message =
                format!("{words} reported by the drive exceeds the safe limit (LBA 1841220416)");
        }
        r
    }

    #[test]
    fn deterministic_output() {
        let r = maximal();
        let opts = PdfOptions::default();
        assert_eq!(
            render_unlocked(&r, &opts).unwrap(),
            render_unlocked(&r, &opts).unwrap()
        );
    }

    #[test]
    fn uncorrectable_tile_shows_worse_counter() {
        let h = Health {
            offline_uncorrectable: Some(0),
            reported_uncorrectable: Some(12),
            ..Default::default()
        };
        let t = uncorrectable_tile(&h);
        assert_eq!((t.value.as_str(), t.level), ("12", Level::Warn));
        assert_eq!(t.sub, "offline 0 / reported 12");

        let h = Health {
            offline_uncorrectable: Some(0),
            reported_uncorrectable: Some(0),
            ..Default::default()
        };
        assert_eq!(uncorrectable_tile(&h).level, Level::Ok);
        assert_eq!(uncorrectable_tile(&Health::default()).level, Level::Na);

        let h = Health {
            reported_uncorrectable: Some(3),
            ..Default::default()
        };
        let t = uncorrectable_tile(&h);
        assert_eq!((t.value.as_str(), t.level), ("3", Level::Warn));
    }

    #[test]
    fn helpers() {
        assert_eq!(years(24 * 365 * 3), "3.0 years");
        assert_eq!(truncate("abcdef", 4), "abc…");
        let temp = Attribute {
            raw: 34,
            raw_string: "34 (Min/Max 18/49)".into(),
            ..Default::default()
        };
        assert_eq!(attr_row(&temp).raw, "34");
        assert_eq!(
            format_timestamp("2026-10-25T12:34:56Z"),
            "2026-10-25 12:34 +00:00"
        );
    }
}
