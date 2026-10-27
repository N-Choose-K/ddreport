//! Mouse: click selects a drive or a tab and focuses that pane; the wheel
//! moves the selection over the list and scrolls over the details.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::{App, Focus, Tab};

/// Wheel step over the details pane.
const WHEEL: i32 = 3;

impl App {
    pub fn on_mouse(&mut self, m: MouseEvent) {
        // Popups are keyboard-driven.
        if self.modal.is_some() {
            return;
        }
        let r = self.rects.get();
        let at = Position::new(m.column, m.row);
        let (list, tabs, detail) = (
            r.list.contains(at),
            r.tabs.contains(at),
            r.detail.contains(at),
        );
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if list => {
                self.focus = Focus::Drives;
                // Two rows per device; a trailing odd row shows nothing.
                let row = usize::from(m.row - r.list.y) / 2;
                let idx = r.list_offset + row;
                if row < usize::from(r.list.height) / 2 && idx < self.devices.len() {
                    self.select(idx);
                }
            }
            MouseEventKind::Down(MouseButton::Left) if tabs => {
                self.focus = Focus::Details;
                if let Some(i) = r
                    .tab_spans
                    .iter()
                    .position(|(a, b)| (*a..*b).contains(&m.column))
                {
                    self.set_tab(Tab::ALL[i]);
                }
            }
            MouseEventKind::Down(MouseButton::Left) if detail => self.focus = Focus::Details,
            MouseEventKind::ScrollDown if list => self.select(self.selected + 1),
            MouseEventKind::ScrollUp if list => self.select(self.selected.saturating_sub(1)),
            MouseEventKind::ScrollDown if detail => self.scroll_by(WHEEL),
            MouseEventKind::ScrollUp if detail => self.scroll_by(-WHEEL),
            _ => {}
        }
    }
}
