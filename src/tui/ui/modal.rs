//! Popups: help, confirmation and messages.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::smartctl::TestKind;
use crate::tui::app::{Confirm, Modal, kind_name};

use super::text::wrap_all;

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

pub(super) fn draw_modal(f: &mut Frame, area: Rect, m: &Modal) {
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
        Modal::Eject {
            label,
            bridge,
            mounts,
            ..
        } => {
            let mut text = vec![
                Line::from(format!("Eject and read {label}?")).bold(),
                Line::from(format!("via {bridge}")).dim(),
                Line::default(),
            ];
            if mounts.is_empty() {
                text.push(Line::from("No volumes mounted."));
            } else {
                text.push(Line::from("Volumes to eject:"));
                text.extend(mounts.iter().map(|m| Line::from(format!("  {m}"))));
            }
            text.push(Line::default());
            text.push(Line::from(
                "The drive leaves macOS for the read and is remounted afterwards. \
                 Nothing may be using it.",
            ));
            text.push(Line::default());
            text.push(Line::from(vec![
                Span::styled("y", Style::new().fg(Color::Green).bold()),
                Span::raw(" yes   "),
                Span::styled("n", Style::new().fg(Color::Red).bold()),
                Span::raw(" no"),
            ]));
            let h = text.len() as u16 + 3;
            let inner = popup(f, area, 60, h, "Eject and read", Color::Yellow);
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
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

const HELP: [(&str, &str); 15] = [
    ("↑/↓  k/j", "select drive"),
    ("1-5  Tab ←/→", "switch tab"),
    ("PgUp/PgDn J/K", "scroll details"),
    ("Home/End", "scroll to top / bottom"),
    ("r", "re-read selected drive"),
    ("R", "rescan for drives"),
    ("e", "eject a USB drive's volumes and read it"),
    ("s", "start short self-test"),
    ("l", "start extended self-test"),
    ("a", "abort running self-test"),
    ("p", "export PDF report"),
    ("?", "this help"),
    ("q  Esc", "quit (Esc closes popups)"),
    ("Ctrl-C", "quit immediately"),
    ("", "Self-tests auto-refresh every 30 s"),
];
