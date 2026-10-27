//! Rendering. Reads `App` state only, except for recording the scroll limit.

mod devices;
mod modal;
mod overview;
mod tables;
#[cfg(test)]
mod tests;
mod text;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Tabs, Wrap};

use crate::model::{Grade, Severity};

use super::app::{App, Tab};
use devices::draw_devices;
use modal::draw_modal;
use overview::overview_lines;
use tables::{draw_attributes, draw_self_tests, draw_statistics};
pub use text::fmt_capacity;

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
            (Some(e), false) => {
                let mut lines = vec![Line::from("Could not read this drive:").red().bold()];
                lines.extend(e.lines().map(Line::from));
                lines.push(Line::default());
                lines.push(Line::from(d.source.retry_hint()).dim());
                Paragraph::new(lines)
            }
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
        " r refresh  m remount+read  s/l/a self-test  p PDF  ? help  q quit ",
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
