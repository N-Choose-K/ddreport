use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::*;

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

const CLICK: MouseEventKind = MouseEventKind::Down(MouseButton::Left);

fn laid_out() -> App {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a"), live("/dev/b"), live("/dev/c")]);
    a.rects.set(Rects {
        list: Rect::new(1, 2, 24, 10),
        list_offset: 1,
        tabs: Rect::new(30, 2, 60, 1),
        tab_spans: [(30, 42), (42, 56), (56, 70), (70, 84), (84, 92)],
        detail: Rect::new(29, 1, 70, 20),
    });
    a.max_scroll.set(20);
    a
}

#[test]
fn click_selects_a_drive_and_focuses_the_list() {
    let mut a = laid_out();
    a.focus = Focus::Details;
    a.on_mouse(mouse(CLICK, 5, 4));
    assert_eq!(
        (a.selected, a.focus),
        (2, Focus::Drives),
        "row 1 of 2 past offset 1"
    );
    a.on_mouse(mouse(CLICK, 5, 9));
    assert_eq!(a.selected, 2, "click below the last entry is ignored");
    a.on_mouse(mouse(CLICK, 0, 4));
    assert_eq!(a.selected, 2, "the border is not the list");
}

#[test]
fn click_switches_tab_or_focuses_details() {
    let mut a = laid_out();
    a.on_mouse(mouse(CLICK, 57, 2));
    assert_eq!((a.tab, a.focus), (Tab::SelfTests, Focus::Details));
    a.on_mouse(mouse(CLICK, 95, 2));
    assert_eq!(a.tab, Tab::SelfTests, "past the titles: tab unchanged");
    a.focus = Focus::Drives;
    a.on_mouse(mouse(CLICK, 50, 10));
    assert_eq!(a.focus, Focus::Details);
}

#[test]
fn wheel_moves_selection_over_the_list_and_scrolls_details() {
    let mut a = laid_out();
    a.on_mouse(mouse(MouseEventKind::ScrollDown, 5, 3));
    a.on_mouse(mouse(MouseEventKind::ScrollDown, 5, 3));
    assert_eq!(a.selected, 2);
    a.on_mouse(mouse(MouseEventKind::ScrollUp, 5, 3));
    assert_eq!(a.selected, 1);
    a.on_mouse(mouse(MouseEventKind::ScrollDown, 50, 10));
    assert_eq!(a.scroll, 3);
    a.on_mouse(mouse(MouseEventKind::ScrollUp, 50, 10));
    assert_eq!(a.scroll, 0);
}

#[test]
fn popups_ignore_the_mouse() {
    let mut a = laid_out();
    a.on_key(key('?'));
    a.on_mouse(mouse(CLICK, 5, 4));
    assert!(matches!(a.modal, Some(Modal::Help)));
    assert_eq!(a.selected, 0);
}
