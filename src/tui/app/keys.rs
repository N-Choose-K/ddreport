//! Key handling, navigation and scrolling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::smartctl::TestKind;
use crate::tui::worker::Job;

use super::{App, Confirm, Modal, PAGE, Tab};

impl App {
    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Job> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return vec![];
        }
        if let Some(modal) = self.modal.take() {
            return self.on_modal_key(modal, key);
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') => self.modal = Some(Modal::Help),
            KeyCode::Up | KeyCode::Char('k') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.select(self.selected + 1),
            KeyCode::PageDown | KeyCode::Char('J') => self.scroll_by(PAGE as i32),
            KeyCode::Char('d') if ctrl => self.scroll_by(PAGE as i32),
            KeyCode::PageUp | KeyCode::Char('K') => self.scroll_by(-(PAGE as i32)),
            KeyCode::Char('u') if ctrl => self.scroll_by(-(PAGE as i32)),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll.get(),
            KeyCode::Tab | KeyCode::Right => self.set_tab(self.tab.step(1)),
            KeyCode::BackTab | KeyCode::Left => self.set_tab(self.tab.step(-1)),
            KeyCode::Char(c @ '1'..='5') => {
                self.set_tab(Tab::ALL[c as usize - '1' as usize]);
            }
            KeyCode::Char('r') => return self.load(self.selected).into_iter().collect(),
            KeyCode::Char('R') => return self.rescan(),
            KeyCode::Char('s') => self.confirm(Confirm::Start(TestKind::Short)),
            KeyCode::Char('l') => self.confirm(Confirm::Start(TestKind::Long)),
            KeyCode::Char('a') => self.confirm(Confirm::Abort),
            KeyCode::Char('p') => return self.export().into_iter().collect(),
            _ => {}
        }
        vec![]
    }

    fn on_modal_key(&mut self, modal: Modal, key: KeyEvent) -> Vec<Job> {
        match modal {
            Modal::Help | Modal::Message { .. } => {}
            Modal::Confirm(c, target) => match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    return self.run_confirmed(c, target).into_iter().collect();
                }
                KeyCode::Char('n' | 'N' | 'q') | KeyCode::Esc => {}
                _ => self.modal = Some(Modal::Confirm(c, target)),
            },
        }
        vec![]
    }
    fn select(&mut self, idx: usize) {
        let idx = idx.min(self.devices.len().saturating_sub(1));
        if idx != self.selected {
            self.selected = idx;
            self.scroll = 0;
        }
    }

    fn set_tab(&mut self, tab: Tab) {
        if tab != self.tab {
            self.tab = tab;
            self.scroll = 0;
        }
    }

    fn scroll_by(&mut self, delta: i32) {
        let max = i32::from(self.max_scroll.get());
        self.scroll = (i32::from(self.scroll) + delta).clamp(0, max) as u16;
    }
}
