mod confirm;
mod monitoring;

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::DriveReport;
use crate::tui::TuiOptions;
use crate::tui::device::DeviceSource;
use crate::tui::worker::{Job, Msg, ScanOutcome, Ticket};

use super::*;

impl App {
    /// Completes the read in flight for `key`, starting one first if needed.
    pub(crate) fn complete_load(
        &mut self,
        key: &str,
        result: Result<Box<crate::model::DriveReport>, String>,
        now: Instant,
    ) -> Vec<Job> {
        let idx = self
            .devices
            .iter()
            .position(|d| d.source.key() == key)
            .expect("unknown device");
        if !self.devices[idx].loading() {
            self.load(idx);
        }
        let ticket = Ticket {
            generation: self.generation,
            req: self.devices[idx].pending.expect("read in flight"),
        };
        let key = key.to_string();
        self.on_msg(
            Msg::Loaded {
                ticket,
                key,
                result,
            },
            now,
        )
    }
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn live(path: &str) -> Device {
    Device::new(
        DeviceSource::Smartctl {
            path: path.into(),
            scanned_type: "sat".into(),
        },
        "",
    )
}

fn app() -> App {
    App::new(&TuiOptions {
        demo_files: vec![],
        out_dir: "/tmp/out".into(),
        device_type: Some("sat".into()),
    })
}

fn scanned(app: &mut App, devs: Vec<Device>) -> Vec<Job> {
    app.start();
    app.on_msg(Msg::scanned(devs), Instant::now())
}

fn loaded(app: &mut App, key: &str, r: DriveReport, now: Instant) {
    app.complete_load(key, Ok(Box::new(r)), now);
}

fn start_target(jobs: Vec<Job>) -> TestTarget {
    match jobs.as_slice() {
        [Job::StartTest { target, .. }] | [Job::AbortTest { target }] => target.clone(),
        other => panic!("expected a test command, got {other:?}"),
    }
}

/// A read that reports the drive idle (no self-test running).
fn idle() -> DriveReport {
    DriveReport {
        self_test_status_known: true,
        ..Default::default()
    }
}

fn with_serial(serial: &str) -> DriveReport {
    let mut r = idle();
    r.identity.model = Some("WDC WD40EFRX".into());
    r.identity.serial = Some(serial.into());
    r
}

#[test]
fn start_scans_then_loads_every_device() {
    let mut a = app();
    assert!(matches!(a.start().as_slice(), [Job::Scan { .. }]));
    assert!(a.start().is_empty(), "no duplicate scan while scanning");
    let jobs = a.on_msg(
        Msg::scanned(vec![live("/dev/a"), live("/dev/b")]),
        Instant::now(),
    );
    assert_eq!(jobs.len(), 2);
    assert!(a.devices.iter().all(Device::loading));
    assert!(a.busy());
    assert!(a.on_key(key('r')).is_empty(), "no refresh while loading");
}

#[test]
fn navigation_tabs_and_scroll() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a"), live("/dev/b")]);
    a.on_key(code(KeyCode::Down));
    a.on_key(key('j'));
    assert_eq!(a.selected, 1);
    a.on_key(key('k'));
    assert_eq!(a.selected, 0);
    a.on_key(key('3'));
    assert_eq!(a.tab, Tab::SelfTests);
    a.on_key(code(KeyCode::Tab));
    a.on_key(code(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Json);
    a.on_key(code(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Overview);
    a.on_key(code(KeyCode::BackTab));
    assert_eq!(a.tab, Tab::Json);
    a.max_scroll.set(15);
    a.on_key(code(KeyCode::PageDown));
    a.on_key(code(KeyCode::PageDown));
    assert_eq!(a.scroll, 15);
    a.on_key(code(KeyCode::PageUp));
    assert_eq!(a.scroll, 5);
    a.on_key(key('1'));
    assert_eq!(a.scroll, 0, "tab change resets scroll");
}

#[test]
fn quit_keys() {
    for k in [key('q'), code(KeyCode::Esc)] {
        let mut a = app();
        a.on_key(k);
        assert!(a.should_quit);
    }
    let mut a = app();
    a.on_key(key('?'));
    a.on_key(code(KeyCode::Esc));
    assert!(!a.should_quit, "Esc closes the overlay first");
    a.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(a.should_quit);
}

#[test]
fn errors_surface_in_popup() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let target = a.test_target(&a.devices[0]).unwrap();
    a.on_msg(
        Msg::TestStarted {
            target,
            kind: TestKind::Short,
            result: Err("Permission denied\nrun as root".into()),
        },
        Instant::now(),
    );
    assert!(matches!(a.modal, Some(Modal::Message { error: true, .. })));
    assert_eq!(
        a.status.as_ref().unwrap().text,
        "Self-test not started on the confirmed drive: Permission denied"
    );
    a.on_key(code(KeyCode::Enter));
    assert!(a.modal.is_none());
}

#[test]
fn load_error_keeps_previous_report() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    loaded(
        &mut a,
        "smartctl:/dev/a",
        DriveReport::default(),
        Instant::now(),
    );
    a.on_key(key('r'));
    a.complete_load(
        "smartctl:/dev/a",
        Err("bridge timeout".into()),
        Instant::now(),
    );
    let d = &a.devices[0];
    assert!(d.report.is_some());
    assert_eq!(d.error.as_deref(), Some("bridge timeout"));
    assert!(!d.loading());
}

#[test]
fn export_requires_report_and_reports_path() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    assert!(a.on_key(key('p')).is_empty());
    loaded(
        &mut a,
        "smartctl:/dev/a",
        DriveReport::default(),
        Instant::now(),
    );
    let jobs = a.on_key(key('p'));
    assert!(
        matches!(&jobs[..], [Job::Export { out_dir, .. }] if out_dir == &PathBuf::from("/tmp/out"))
    );
    assert!(a.on_key(key('p')).is_empty(), "one export at a time");
    a.on_msg(Msg::Exported(Ok("/tmp/out/x.pdf".into())), Instant::now());
    assert_eq!(a.status.as_ref().unwrap().text, "Saved /tmp/out/x.pdf");
    assert!(!a.exporting);
}

#[test]
fn stale_reads_are_dropped() {
    let mut a = app();
    let jobs = scanned(&mut a, vec![live("/dev/a")]);
    let [Job::Load { ticket: old, .. }] = jobs.as_slice() else {
        panic!("expected one load");
    };
    let old = *old;
    let stale = |dev_type: &str| {
        let mut r = DriveReport::default();
        r.meta.device_type = dev_type.into();
        Msg::Loaded {
            ticket: old,
            key: "smartctl:/dev/a".into(),
            result: Ok(Box::new(r)),
        }
    };

    // Rescan: a different drive now sits at /dev/a.
    a.on_key(key('R'));
    let jobs = a.on_msg(Msg::scanned(vec![live("/dev/a")]), Instant::now());
    let [Job::Load { ticket: new, .. }] = jobs.as_slice() else {
        panic!("expected one load");
    };
    assert_ne!(old.generation, new.generation);
    a.on_msg(stale("sat"), Instant::now());
    assert!(a.devices[0].report.is_none(), "old generation dropped");
    assert!(a.devices[0].loading(), "new read still pending");

    let mut r = DriveReport::default();
    r.meta.device_type = "nvme".into();
    a.complete_load("smartctl:/dev/a", Ok(Box::new(r)), Instant::now());
    a.on_msg(stale("sat"), Instant::now());
    let d = &a.devices[0];
    assert_eq!(d.report.as_ref().unwrap().meta.device_type, "nvme");

    // Same generation, superseded request ID.
    a.on_key(key('r'));
    let wrong = Msg::Loaded {
        ticket: Ticket {
            generation: a.generation,
            req: a.devices[0].pending.unwrap() + 1,
        },
        key: "smartctl:/dev/a".into(),
        result: Err("late".into()),
    };
    a.on_msg(wrong, Instant::now());
    assert!(a.devices[0].error.is_none());
    assert!(a.devices[0].loading());
}

#[test]
fn scan_status_names_skipped_files() {
    let mut a = app();
    a.start();
    a.on_msg(
        Msg::Scanned(Ok(ScanOutcome {
            devices: vec![live("/dev/a")],
            skipped: vec!["fixtures/scan_open.json".into()],
        })),
        Instant::now(),
    );
    assert_eq!(
        a.status.as_ref().unwrap().text,
        "Found 1 drive(s); skipped 1 non-report file(s): scan_open.json"
    );
}

#[test]
fn rescan_keeps_selection_in_range() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a"), live("/dev/b")]);
    a.on_key(key('j'));
    a.on_key(key('R'));
    a.on_msg(Msg::scanned(vec![live("/dev/a")]), Instant::now());
    assert_eq!(a.selected, 0);
    a.on_msg(Msg::scanned(vec![]), Instant::now());
    assert!(a.selected_device().is_none());
    assert!(a.on_key(key('r')).is_empty());
}
