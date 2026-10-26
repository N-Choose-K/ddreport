//! Attributes, self-test and statistics tables.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Gauge, Paragraph, Row, Table, Wrap};

use crate::model::{Attribute, DriveReport, Protocol, Severity};
use crate::tui::app::App;

use super::text::fmt_count;

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

pub(super) fn draw_attributes(f: &mut Frame, area: Rect, app: &App, r: &DriveReport) {
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

pub(super) fn draw_self_tests(f: &mut Frame, area: Rect, app: &App, r: &DriveReport) {
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

pub(super) fn draw_statistics(f: &mut Frame, area: Rect, app: &App, r: &DriveReport) {
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
