//! Rendering. Reads `App` state only, except for recording the scroll limit.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Cell, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, Tabs, Wrap,
};

use thousands::Separable;

use crate::fmt::bytes_human;
use crate::model::{Attribute, DriveReport, Grade, MediaKind, Protocol, Severity};
use crate::smartctl::TestKind;

use super::app::{App, Confirm, Modal, Tab, kind_name};
use super::device::Device;

pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LABEL_W: usize = 13;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            "Terminal too small ({}x{}).\nNeed at least {MIN_WIDTH}x{MIN_HEIGHT}. q quits.",
            area.width, area.height
        );
        f.render_widget(Paragraph::new(msg).wrap(Wrap { trim: true }), area);
        return;
    }
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let list_w = if area.width >= 100 { 28 } else { 26 };
    let [list, detail] =
        Layout::horizontal([Constraint::Length(list_w), Constraint::Fill(1)]).areas(body);

    draw_header(f, header, app);
    draw_devices(f, list, app);
    draw_detail(f, detail, app);
    draw_status(f, status, app);
    if let Some(m) = &app.modal {
        draw_modal(f, area, m);
    }
}

fn spinner(app: &App) -> &'static str {
    SPINNER[app.spinner % SPINNER.len()]
}

fn grade_color(g: Option<Grade>) -> Color {
    match g {
        Some(Grade::Pass) => Color::Green,
        Some(Grade::Caution) => Color::Yellow,
        Some(Grade::Fail) => Color::Red,
        Some(Grade::Unknown) | None => Color::DarkGray,
    }
}

fn grade_text(g: Grade) -> &'static str {
    match g {
        Grade::Pass => "PASS",
        Grade::Caution => "CAUTION",
        Grade::Fail => "FAIL",
        Grade::Unknown => "UNKNOWN",
    }
}

fn severity_style(s: Severity) -> (&'static str, Style) {
    match s {
        Severity::Good => ("✓", Style::new().fg(Color::Green)),
        Severity::Info => ("•", Style::new().fg(Color::Cyan)),
        Severity::Caution => ("!", Style::new().fg(Color::Yellow)),
        Severity::Critical => ("✗", Style::new().fg(Color::Red).bold()),
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let mode = if app.demo() {
        format!("demo: {} file(s)", app.demo_files.len())
    } else {
        "live".into()
    };
    let out = if app.out_dir.as_os_str().is_empty() {
        ".".into()
    } else {
        app.out_dir.display().to_string()
    };
    let right = format!("{mode} | out: {out} ");
    let left = Line::from(vec![
        Span::styled(" ddreport ", Style::new().bold().reversed()),
        Span::raw(" drive health report"),
    ]);
    f.render_widget(left, area);
    let rw = (right.chars().count() as u16).min(area.width / 2);
    let r = Rect {
        x: area.right() - rw,
        width: rw,
        ..area
    };
    f.render_widget(Line::from(right).dim().right_aligned(), r);
}

fn draw_devices(f: &mut Frame, area: Rect, app: &App) {
    let mut title = format!(" Drives ({}) ", app.devices.len());
    if app.scanning {
        title = format!(" Drives {} ", spinner(app));
    }
    let block = Block::bordered().title(title);
    if app.devices.is_empty() {
        let msg = if app.scanning {
            "Scanning..."
        } else {
            "No drives found.\n\nConnect a drive and press R to rescan."
        };
        f.render_widget(
            Paragraph::new(msg).wrap(Wrap { trim: true }).block(block),
            area,
        );
        return;
    }
    let inner_w = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .devices
        .iter()
        .map(|d| device_item(d, app, inner_w))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(Color::DarkGray))
        .highlight_symbol("▌");
    let mut state = ListState::default().with_selected(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn device_item<'a>(d: &Device, app: &App, w: usize) -> ListItem<'a> {
    let marker = if d.loading() {
        Span::styled(spinner(app), Style::new().fg(Color::Cyan))
    } else if d.report.is_none() && d.error.is_some() {
        Span::styled("!", Style::new().fg(Color::Red).bold())
    } else {
        Span::styled("●", Style::new().fg(grade_color(d.grade())))
    };
    let top = Line::from(vec![
        marker,
        Span::raw(" "),
        Span::styled(
            trunc(&d.source.label(), w.saturating_sub(2)),
            Style::new().bold(),
        ),
    ]);
    let sub = match &d.report {
        Some(r) => {
            let model = r
                .identity
                .model
                .clone()
                .unwrap_or_else(|| "unknown model".into());
            let cap = r
                .identity
                .capacity_bytes
                .map(fmt_capacity)
                .unwrap_or_default();
            let room = w.saturating_sub(cap.len() + 3);
            format!("{} {cap}", trunc(&model, room))
        }
        None if d.error.is_some() => "read failed".into(),
        None if d.loading() => "reading...".into(),
        None => d.info.clone(),
    };
    let sub_style = if d.report.is_none() && d.error.is_some() {
        Style::new().fg(Color::Red)
    } else {
        Style::new().dim()
    };
    let sub = Line::from(Span::styled(
        format!("  {}", trunc(&sub, w.saturating_sub(2))),
        sub_style,
    ));
    ListItem::new(Text::from(vec![top, sub]))
}

fn draw_detail(f: &mut Frame, area: Rect, app: &App) {
    let d = app.selected_device();
    let mut title = d
        .map(|d| format!(" {} ", d.source.label()))
        .unwrap_or_default();
    if d.is_some_and(|d| d.loading()) {
        title.push_str(spinner(app));
        title.push(' ');
    }
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [tabs_area, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let wide = inner.width >= 66;
    let titles = Tab::ALL.iter().enumerate().map(|(i, t)| {
        let name = match (wide, t) {
            (false, Tab::Attributes) => "Attrs",
            (false, Tab::SelfTests) => "Tests",
            (false, Tab::Statistics) => "Stats",
            _ => t.title(),
        };
        format!("{} {name}", i + 1)
    });
    f.render_widget(
        Tabs::new(titles)
            .select(app.tab.index())
            .highlight_style(Style::new().fg(Color::Cyan).bold().underlined()),
        tabs_area,
    );

    app.max_scroll.set(0);
    let Some(d) = d else {
        let msg = if app.scanning {
            "Looking for drives..."
        } else {
            "No drive selected."
        };
        f.render_widget(Paragraph::new(msg).dim(), content);
        return;
    };
    let Some(r) = &d.report else {
        let p = match (&d.error, d.loading()) {
            (_, true) => Paragraph::new(format!(
                "{} Reading SMART data from {} (can take ~10 s)",
                spinner(app),
                d.source.label()
            )),
            (Some(e), false) => Paragraph::new(vec![
                Line::from("Could not read this drive:").red().bold(),
                Line::from(e.as_str()),
                Line::default(),
                Line::from("Press r to retry, or start ddreport with -d <type>.").dim(),
            ]),
            (None, false) => Paragraph::new("Not loaded yet. Press r.").dim(),
        };
        f.render_widget(p.wrap(Wrap { trim: false }), content);
        return;
    };
    let content = match &d.error {
        Some(e) => {
            let [warn, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(content);
            f.render_widget(
                Line::from(format!("Refresh failed: {e}")).fg(Color::Red),
                warn,
            );
            rest
        }
        None => content,
    };
    match app.tab {
        Tab::Overview => scroll_lines(f, content, app, overview_lines(r, content.width)),
        Tab::Attributes => draw_attributes(f, content, app, r),
        Tab::SelfTests => draw_self_tests(f, content, app, r),
        Tab::Statistics => draw_statistics(f, content, app, r),
        Tab::Json => {
            let json = serde_json::to_string_pretty(r).unwrap_or_else(|e| e.to_string());
            scroll_lines(
                f,
                content,
                app,
                json.lines().map(|l| Line::from(l.to_string())).collect(),
            );
        }
    }
}

/// Renders pre-wrapped lines with the app's scroll offset and records the limit.
fn scroll_lines(f: &mut Frame, area: Rect, app: &App, lines: Vec<Line>) {
    let max = (lines.len() as u16).saturating_sub(area.height);
    app.max_scroll.set(max);
    f.render_widget(Paragraph::new(lines).scroll((app.scroll.min(max), 0)), area);
}

// ---- Overview ----

type Kv = (&'static str, String, Style);

fn overview_lines(r: &DriveReport, width: u16) -> Vec<Line<'static>> {
    let w = width as usize;
    let g = r.verdict.grade;
    let badge = format!(" {} ", grade_text(g));
    let indent = badge.len() + 1;
    let badge_style = Style::new()
        .bg(grade_color(Some(g)))
        .fg(Color::Black)
        .bold();
    let mut out = Vec::new();
    for (i, part) in wrap(&r.verdict.summary, w.saturating_sub(indent))
        .into_iter()
        .enumerate()
    {
        let lead = if i == 0 {
            Span::styled(badge.clone(), badge_style)
        } else {
            Span::raw(" ".repeat(badge.len()))
        };
        out.push(Line::from(vec![
            lead,
            Span::raw(" "),
            Span::styled(part, Style::new().bold()),
        ]));
    }
    for fd in &r.verdict.findings {
        let (sym, style) = severity_style(fd.severity);
        for (i, part) in wrap(&fd.message, w.saturating_sub(4))
            .into_iter()
            .enumerate()
        {
            let lead = if i == 0 {
                format!("  {sym} ")
            } else {
                "    ".into()
            };
            out.push(Line::from(vec![Span::styled(lead, style), Span::raw(part)]));
        }
    }
    out.push(Line::default());

    let left = [("Identity", identity_rows(r)), ("Report", report_rows(r))];
    let right = [("Health", health_rows(r)), ("USB bridge", bridge_rows(r))];
    if w >= 68 {
        let col = (w - 2) / 2;
        let l = sections(&left, col);
        let rr = sections(&right, col);
        for i in 0..l.len().max(rr.len()) {
            let mut spans = l.get(i).cloned().map(|x| x.spans).unwrap_or_default();
            let used: usize = spans.iter().map(|s| s.width()).sum();
            spans.push(Span::raw(" ".repeat(col + 2 - used.min(col))));
            spans.extend(rr.get(i).cloned().map(|x| x.spans).unwrap_or_default());
            out.push(Line::from(spans));
        }
    } else {
        let all = [&right[..1], &left[..1], &right[1..], &left[1..]].concat();
        out.extend(sections(&all, w));
    }
    out
}

fn sections(secs: &[(&'static str, Vec<Kv>)], w: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (title, rows) in secs.iter().filter(|(_, rows)| !rows.is_empty()) {
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.push(Line::from(Span::styled(
            *title,
            Style::new().fg(Color::Cyan).bold(),
        )));
        for (label, value, style) in rows {
            out.push(Line::from(vec![
                Span::styled(format!("{label:<LABEL_W$}"), Style::new().dim()),
                Span::styled(trunc(value, w.saturating_sub(LABEL_W)), *style),
            ]));
        }
    }
    out
}

fn kv(label: &'static str, v: Option<String>) -> Option<Kv> {
    v.filter(|s| !s.is_empty())
        .map(|s| (label, s, Style::new()))
}

/// Counter that is bad when nonzero.
fn bad_count(label: &'static str, v: Option<u64>, bad: Color) -> Option<Kv> {
    v.map(|n| {
        let style = if n > 0 {
            Style::new().fg(bad).bold()
        } else {
            Style::new().fg(Color::Green)
        };
        (label, fmt_count(n), style)
    })
}

fn identity_rows(r: &DriveReport) -> Vec<Kv> {
    let i = &r.identity;
    let media = match (i.media, i.rotation_rpm) {
        (MediaKind::Hdd, Some(rpm)) => Some(format!("HDD, {rpm} rpm")),
        (MediaKind::Hdd, None) => Some("HDD".into()),
        (MediaKind::Ssd, _) => Some("SSD".into()),
        (MediaKind::Unknown, _) => None,
    };
    let proto = match i.protocol {
        Protocol::Ata => Some("ATA"),
        Protocol::Nvme => Some("NVMe"),
        Protocol::Scsi => Some("SCSI"),
        Protocol::Unknown => None,
    };
    let iface = [
        i.sata_version
            .clone()
            .or(i.nvme_version.as_ref().map(|v| format!("NVMe {v}")))
            .or(proto.map(String::from)),
        i.link_speed_current.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ");
    let sectors = match (i.logical_sector_size, i.physical_sector_size) {
        (Some(l), Some(p)) if l != p => Some(format!("{l} / {p} B")),
        (Some(l), _) => Some(format!("{l} B")),
        _ => None,
    };
    [
        kv("Model", i.model.clone()),
        kv("Family", i.model_family.clone()),
        kv("Serial", i.serial.clone()),
        kv("Firmware", i.firmware.clone()),
        kv("Capacity", i.capacity_bytes.map(fmt_capacity)),
        kv("Bytes", i.capacity_bytes.map(fmt_count)),
        kv("Media", media),
        kv("Form factor", i.form_factor.clone()),
        kv("Interface", Some(iface)),
        kv("Sectors", sectors),
        kv("WWN", i.wwn.clone()),
        kv("TRIM", i.trim_supported.map(yes_no)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn health_rows(r: &DriveReport) -> Vec<Kv> {
    let h = &r.health;
    let smart = match (h.smart_supported, h.smart_enabled, h.overall_passed) {
        (Some(false), _, _) => Some(("unsupported".to_string(), Color::Yellow)),
        (_, Some(false), _) => Some(("disabled".to_string(), Color::Yellow)),
        (_, _, Some(true)) => Some(("PASSED".to_string(), Color::Green)),
        (_, _, Some(false)) => Some(("FAILED".to_string(), Color::Red)),
        _ => None,
    }
    .map(|(s, c)| ("SMART status", s, Style::new().fg(c).bold()));
    let temp = h.temperature_c.map(|t| match h.temperature_max_c {
        Some(m) => format!("{t} °C (max {m} °C)"),
        None => format!("{t} °C"),
    });
    let spare = h
        .available_spare_pct
        .map(|s| match h.available_spare_threshold_pct {
            Some(t) => format!("{s}% (threshold {t}%)"),
            None => format!("{s}%"),
        });
    let wear = h.endurance_used_pct.map(|p| {
        let c = match p {
            0..=49 => Color::Green,
            50..=89 => Color::Yellow,
            _ => Color::Red,
        };
        ("Wear used", format!("{p}%"), Style::new().fg(c))
    });
    [
        smart,
        kv(
            "Power-on",
            h.power_on_hours
                .map(|n| format!("{} h ({:.1} years)", fmt_count(n), n as f64 / 8766.0)),
        ),
        kv("Power cycles", h.power_cycles.map(fmt_count)),
        kv("Temperature", temp),
        bad_count("Reallocated", h.reallocated_sectors, Color::Yellow),
        bad_count("Pending", h.pending_sectors, Color::Red),
        bad_count("Offline unc", h.offline_uncorrectable, Color::Red),
        bad_count("Reported unc", h.reported_uncorrectable, Color::Yellow),
        bad_count("CRC errors", h.udma_crc_errors, Color::Yellow),
        bad_count("Cmd timeouts", h.command_timeouts, Color::Yellow),
        bad_count("Media errors", h.nvme_media_errors, Color::Red),
        wear,
        kv("Spare", spare),
        kv("Written", h.bytes_written.map(bytes_human)),
        kv("Read", h.bytes_read.map(bytes_human)),
        kv("Start/stop", h.start_stop_count.map(fmt_count)),
        kv("Load cycles", h.load_cycles.map(fmt_count)),
        kv("Unsafe off", h.unsafe_shutdowns.map(fmt_count)),
        kv("Error log", h.error_log_count.map(fmt_count)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn bridge_rows(r: &DriveReport) -> Vec<Kv> {
    let Some(b) = &r.bridge else {
        return vec![];
    };
    let ids = format!("{:04x}:{:04x}", b.vendor_id, b.product_id);
    [
        kv("Chipset", b.chipset.clone()),
        kv("Product", b.product_name.clone()),
        kv("Vendor", b.vendor_name.clone()),
        kv("USB ID", Some(ids)),
        kv("Transport", b.transport.clone()),
        kv("Link", b.link_speed.clone()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn report_rows(r: &DriveReport) -> Vec<Kv> {
    let m = &r.meta;
    let mut rows: Vec<Kv> = [
        kv("Backend", Some(m.backend.clone())),
        kv("Device", Some(m.device_path.clone())),
        kv("Type", Some(m.device_type.clone())),
        kv("Generated", Some(fmt_time(&m.generated_at))),
    ]
    .into_iter()
    .flatten()
    .collect();
    rows.extend(
        m.collection_errors
            .iter()
            .map(|e| ("Data error", e.clone(), Style::new().fg(Color::Red).bold())),
    );
    rows.extend(
        m.warnings
            .iter()
            .map(|w| ("Warning", w.clone(), Style::new().fg(Color::Yellow))),
    );
    rows
}

// ---- Tables ----

/// Attribute IDs whose nonzero raw value signals media trouble.
const CRITICAL_IDS: [u8; 9] = [5, 10, 184, 187, 188, 196, 197, 198, 201];

pub fn attr_severity(a: &Attribute) -> Option<Severity> {
    if a.when_failed == "now" || (a.thresh > 0 && a.value <= a.thresh) {
        Some(Severity::Critical)
    } else if a.when_failed == "past" || (CRITICAL_IDS.contains(&a.id) && a.raw > 0) {
        Some(Severity::Caution)
    } else if a.id == 199 && a.raw > 0 {
        Some(Severity::Info)
    } else {
        None
    }
}

fn header(cells: &[&'static str]) -> Row<'static> {
    Row::new(cells.iter().copied()).style(Style::new().bold().add_modifier(Modifier::UNDERLINED))
}

/// Visible window of rows plus the scroll limit for a table with a header line.
fn window<T>(rows: &[T], area: Rect, app: &App) -> std::ops::Range<usize> {
    let visible = area.height.saturating_sub(1) as usize;
    let max = rows.len().saturating_sub(visible);
    app.max_scroll.set(max as u16);
    let start = (app.scroll as usize).min(max);
    start..(start + visible).min(rows.len())
}

fn draw_attributes(f: &mut Frame, area: Rect, app: &App, r: &DriveReport) {
    if r.attributes.is_empty() {
        let msg = if r.identity.protocol == Protocol::Nvme {
            "NVMe drives have no ATA attributes; the health log is on the Overview tab."
        } else {
            "No SMART attributes reported."
        };
        f.render_widget(Paragraph::new(msg).dim().wrap(Wrap { trim: true }), area);
        return;
    }
    let [table_area, legend] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    // The "Fail" column only appears when some attribute has failed.
    let any_failed = r.attributes.iter().any(|a| !a.when_failed.is_empty());
    let rows: Vec<Row> = r.attributes[window(&r.attributes, table_area, app)]
        .iter()
        .map(|a| {
            let style = match attr_severity(a) {
                Some(Severity::Critical) => Style::new().fg(Color::Red).bold(),
                Some(Severity::Caution) => Style::new().fg(Color::Yellow),
                Some(Severity::Info) => Style::new().fg(Color::Cyan),
                _ => Style::new(),
            };
            let mut cells = vec![
                Cell::from(format!("{:>3}", a.id)),
                Cell::from(a.name.replace('_', " ")),
                Cell::from(format!("{:>3}", a.value)),
                Cell::from(format!("{:>3}", a.worst)),
                Cell::from(format!("{:>3}", a.thresh)),
                Cell::from(if a.prefailure { "P" } else { "" }),
                Cell::from(a.raw_string.clone()),
            ];
            if any_failed {
                cells.push(Cell::from(match a.when_failed.as_str() {
                    "now" => "NOW",
                    other => other,
                }));
            }
            Row::new(cells).style(style)
        })
        .collect();
    let mut widths = vec![
        Constraint::Length(3),
        Constraint::Min(21),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Fill(1),
    ];
    let mut titles = vec!["ID", "Attribute", "Val", "Wst", "Thr", "P", "Raw"];
    if any_failed {
        widths.push(Constraint::Length(4));
        titles.push("Fail");
    }
    let t = Table::new(rows, widths).header(header(&titles));
    f.render_widget(t, table_area);
    f.render_widget(
        Line::from(vec![
            Span::styled("red", Style::new().fg(Color::Red)),
            Span::raw(" failed  "),
            Span::styled("yellow", Style::new().fg(Color::Yellow)),
            Span::raw(" sectors  "),
            Span::styled("cyan", Style::new().fg(Color::Cyan)),
            Span::raw(" CRC  P prefail"),
        ])
        .dim(),
        legend,
    );
}

fn draw_self_tests(f: &mut Frame, area: Rect, app: &App, r: &DriveReport) {
    let area = match &r.self_test_in_progress {
        Some(p) => {
            let [g, rest] =
                Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);
            let remaining = p.remaining_pct.unwrap_or(100).min(100);
            let kind = p.kind.clone().unwrap_or_else(|| "Self-test".into());
            let gauge = Gauge::default()
                .block(Block::bordered().title(format!(" {kind} in progress, auto-refresh 30 s ")))
                .gauge_style(Style::new().fg(Color::Cyan))
                .ratio(f64::from(100 - remaining) / 100.0)
                .label(format!("{remaining}% remaining"));
            f.render_widget(gauge, g);
            rest
        }
        None => area,
    };
    if r.self_tests.is_empty() {
        f.render_widget(
            Paragraph::new(
                "No self-tests logged.\n\ns  short test (~2 min)\nl  extended test (hours)",
            )
            .dim(),
            area,
        );
        return;
    }
    let rows: Vec<Row> = r.self_tests[window(&r.self_tests, area, app)]
        .iter()
        .map(|t| {
            let style = match t.passed {
                Some(true) => Style::new().fg(Color::Green),
                Some(false) => Style::new().fg(Color::Red).bold(),
                None => Style::new(),
            };
            Row::new(vec![
                Cell::from(t.kind.replace(" offline", "")),
                Cell::from(t.status.clone()).style(style),
                Cell::from(t.lifetime_hours.map(fmt_count).unwrap_or_default()),
                Cell::from(
                    t.lba_first_error
                        .map(|l| l.to_string())
                        .unwrap_or("-".into()),
                ),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(10),
        Constraint::Fill(1),
        Constraint::Length(6),
        Constraint::Length(9),
    ];
    let t = Table::new(rows, widths).header(header(&["Type", "Status", "Hours", "Error LBA"]));
    f.render_widget(t, area);
}

fn draw_statistics(f: &mut Frame, area: Rect, app: &App, r: &DriveReport) {
    if r.statistics.is_empty() {
        f.render_widget(Paragraph::new("No device statistics reported.").dim(), area);
        return;
    }
    let range = window(&r.statistics, area, app);
    let rows: Vec<Row> = range
        .clone()
        .map(|i| {
            let s = &r.statistics[i];
            let new_group = i == range.start || r.statistics[i - 1].group != s.group;
            Row::new(vec![
                Cell::from(if new_group {
                    s.group.clone()
                } else {
                    String::new()
                })
                .style(Style::new().fg(Color::Cyan)),
                Cell::from(s.name.clone()),
                Cell::from(s.value.clone()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Fill(1),
        Constraint::Fill(2),
        Constraint::Fill(1),
    ];
    f.render_widget(
        Table::new(rows, widths).header(header(&["Group", "Statistic", "Value"])),
        area,
    );
}

// ---- Status bar and modals ----

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = Vec::new();
    if app.busy() {
        spans.push(Span::styled(
            format!("{} ", spinner(app)),
            Style::new().fg(Color::Cyan),
        ));
    }
    match &app.status {
        Some(s) if s.error => spans.push(Span::styled(s.text.clone(), Style::new().fg(Color::Red))),
        Some(s) => spans.push(Span::raw(s.text.clone())),
        None => {}
    }
    // Longest hint set that leaves room for the status text.
    let used: usize = spans.iter().map(|s| s.width()).sum();
    let room = (area.width as usize).saturating_sub(used + 2);
    let hints = [
        " r refresh  s/l/a self-test  p PDF  ? help  q quit ",
        " s/l/a test  p PDF  ? help  q quit ",
        " p PDF  ? help  q quit ",
        " ? help ",
    ]
    .into_iter()
    .find(|h| h.chars().count() <= room)
    .unwrap_or("");
    let [left, right] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(hints.chars().count() as u16),
    ])
    .areas(area);
    f.render_widget(Line::from(spans), left);
    f.render_widget(Line::from(hints).reversed().right_aligned(), right);
}

fn popup(f: &mut Frame, area: Rect, w: u16, h: u16, title: &str, border: Color) -> Rect {
    let r = area.centered(
        Constraint::Length(w.min(area.width)),
        Constraint::Length(h.min(area.height)),
    );
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(Style::new().fg(border));
    let inner = block.inner(r);
    f.render_widget(block, r);
    inner
}

fn draw_modal(f: &mut Frame, area: Rect, m: &Modal) {
    match m {
        Modal::Help => {
            let lines: Vec<Line> = HELP
                .iter()
                .map(|(k, d)| {
                    Line::from(vec![
                        Span::styled(format!("{k:<14}"), Style::new().fg(Color::Cyan)),
                        Span::raw(*d),
                    ])
                })
                .collect();
            let inner = popup(f, area, 58, lines.len() as u16 + 2, "Keys", Color::Cyan);
            f.render_widget(Paragraph::new(lines), inner);
        }
        Modal::Confirm(c, target) => {
            let label = &target.path;
            let ident = [target.model.clone(), target.serial.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("  S/N ");
            let (question, note) = match c {
                Confirm::Start(k) => (
                    format!(
                        "Start {} self-test on {label}?",
                        kind_name(*k).to_lowercase()
                    ),
                    match k {
                        TestKind::Long => {
                            "Runs for hours inside the drive. Keep it powered and connected; \
                             some USB enclosures sleep and abort it."
                        }
                        _ => "Runs inside the drive for about 2 minutes. The drive stays usable.",
                    },
                ),
                Confirm::Abort => (
                    format!("Abort the running self-test on {label}?"),
                    "The log will record the test as aborted by host.",
                ),
            };
            let text = vec![
                Line::from(question).bold(),
                Line::from(ident).dim(),
                Line::default(),
                Line::from(note),
                Line::default(),
                Line::from(vec![
                    Span::styled("y", Style::new().fg(Color::Green).bold()),
                    Span::raw(" yes   "),
                    Span::styled("n", Style::new().fg(Color::Red).bold()),
                    Span::raw(" no"),
                ]),
            ];
            let inner = popup(f, area, 60, 10, "Confirm", Color::Yellow);
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
        }
        Modal::Message { title, body, error } => {
            let color = if *error { Color::Red } else { Color::Green };
            let w = 64u16.min(area.width);
            let lines = wrap_all(body, w.saturating_sub(2) as usize);
            let h = (lines.len() as u16 + 4).min(area.height);
            let inner = popup(f, area, w, h, title, color);
            let mut text: Vec<Line> = lines.into_iter().map(Line::from).collect();
            text.push(Line::default());
            text.push(Line::from("Press any key to close").dim());
            f.render_widget(Paragraph::new(text), inner);
        }
    }
}

const HELP: [(&str, &str); 14] = [
    ("↑/↓  k/j", "select drive"),
    ("1-5  Tab ←/→", "switch tab"),
    ("PgUp/PgDn J/K", "scroll details"),
    ("Home/End", "scroll to top / bottom"),
    ("r", "re-read selected drive"),
    ("R", "rescan for drives"),
    ("s", "start short self-test"),
    ("l", "start extended self-test"),
    ("a", "abort running self-test"),
    ("p", "export PDF report"),
    ("?", "this help"),
    ("q  Esc", "quit (Esc closes popups)"),
    ("Ctrl-C", "quit immediately"),
    ("", "Self-tests auto-refresh every 30 s"),
];

// ---- Formatting helpers ----

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

fn wrap_all(s: &str, w: usize) -> Vec<String> {
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
fn fmt_time(s: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| s.to_string())
}

fn fmt_count(n: u64) -> String {
    n.separate_with_commas()
}

fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
