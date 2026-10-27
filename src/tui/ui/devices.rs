//! Drive list pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::tui::app::{App, Focus};
use crate::tui::device::Device;

use super::text::{fmt_capacity, trunc};
use super::{grade_color, pane_border, spinner};

pub(super) fn draw_devices(f: &mut Frame, area: Rect, app: &App) {
    let mut title = format!(" Drives ({}) ", app.devices.len());
    if app.scanning {
        title = format!(" Drives {} ", spinner(app));
    }
    let block = Block::bordered()
        .title(title)
        .border_style(pane_border(app, Focus::Drives));
    let inner = block.inner(area);
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
    let mut rects = app.rects.get();
    rects.list = inner;
    rects.list_offset = state.offset();
    app.rects.set(rects);
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
