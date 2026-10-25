//! UI state and pure update logic: keys and worker messages in, jobs out.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::smartctl::TestKind;

use super::TuiOptions;
use super::device::{Device, TestWatch};
use super::worker::{Job, Msg, TestTarget, Ticket};

/// Reload cadence while a self-test runs.
pub const TEST_POLL: Duration = Duration::from_secs(30);
/// First reload after starting or aborting a test.
const TEST_KICK: Duration = Duration::from_secs(3);
const PAGE: u16 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Attributes,
    SelfTests,
    Statistics,
    Json,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::Attributes,
        Tab::SelfTests,
        Tab::Statistics,
        Tab::Json,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Attributes => "Attributes",
            Tab::SelfTests => "Self-tests",
            Tab::Statistics => "Statistics",
            Tab::Json => "JSON",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn step(self, delta: isize) -> Tab {
        let n = Self::ALL.len() as isize;
        Self::ALL[(self.index() as isize + delta).rem_euclid(n) as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Start(TestKind),
    Abort,
}

#[derive(Debug, Clone)]
pub enum Modal {
    Help,
    Confirm(Confirm, TestTarget),
    Message {
        title: String,
        body: String,
        error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

pub struct App {
    pub demo_files: Vec<PathBuf>,
    pub out_dir: PathBuf,
    pub device_type: Option<String>,
    pub devices: Vec<Device>,
    pub selected: usize,
    pub tab: Tab,
    pub scroll: u16,
    /// Largest useful scroll for the current tab; written by the renderer.
    pub max_scroll: Cell<u16>,
    pub modal: Option<Modal>,
    pub status: Option<Status>,
    pub scanning: bool,
    pub exporting: bool,
    /// Device keys with a start/abort command in flight.
    pub test_cmd_busy: Vec<String>,
    pub spinner: usize,
    pub should_quit: bool,
    /// Bumped on every completed scan; results from older generations are dropped.
    pub generation: u64,
    next_req: u64,
}

impl App {
    pub fn new(opts: &TuiOptions) -> Self {
        Self {
            demo_files: opts.demo_files.clone(),
            out_dir: opts.out_dir.clone(),
            device_type: opts.device_type.clone(),
            devices: Vec::new(),
            selected: 0,
            tab: Tab::Overview,
            scroll: 0,
            max_scroll: Cell::new(u16::MAX),
            modal: None,
            status: None,
            scanning: false,
            exporting: false,
            test_cmd_busy: Vec::new(),
            spinner: 0,
            should_quit: false,
            generation: 0,
            next_req: 0,
        }
    }

    pub fn demo(&self) -> bool {
        !self.demo_files.is_empty()
    }

    pub fn selected_device(&self) -> Option<&Device> {
        self.devices.get(self.selected)
    }

    pub fn busy(&self) -> bool {
        self.scanning
            || self.exporting
            || !self.test_cmd_busy.is_empty()
            || self.devices.iter().any(Device::loading)
    }

    /// Initial jobs to run at startup.
    pub fn start(&mut self) -> Vec<Job> {
        self.rescan()
    }

    pub fn on_tick(&mut self, now: Instant) -> Vec<Job> {
        self.spinner = self.spinner.wrapping_add(1);
        let due: Vec<usize> = self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.loading() && d.next_refresh.is_some_and(|t| t <= now))
            .map(|(i, _)| i)
            .collect();
        due.into_iter().filter_map(|i| self.load(i)).collect()
    }

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

    pub fn on_msg(&mut self, msg: Msg, now: Instant) -> Vec<Job> {
        match msg {
            Msg::Scanned(Ok(devices)) => {
                self.scanning = false;
                self.generation += 1;
                let cancelled = matches!(self.modal, Some(Modal::Confirm(..)));
                if cancelled {
                    self.modal = None;
                }
                // Monitoring moves to the entry at the same path; its first fresh read
                // confirms the serial or ends monitoring.
                let mut devices = devices;
                for old in self.devices.iter_mut().filter(|d| d.watch.is_some()) {
                    if let Some(new) = devices.iter_mut().find(|n| n.source == old.source) {
                        new.watch = old.watch.take();
                    }
                }
                self.devices = devices;
                self.selected = self.selected.min(self.devices.len().saturating_sub(1));
                self.scroll = 0;
                self.info(format!("Found {} drive(s)", self.devices.len()));
                if cancelled {
                    self.error("Confirmation cancelled: drive list changed");
                }
                return (0..self.devices.len())
                    .filter_map(|i| self.load(i))
                    .collect();
            }
            Msg::Scanned(Err(e)) => {
                self.scanning = false;
                self.error(format!("Scan failed: {e}"));
            }
            Msg::Loaded {
                ticket,
                key,
                result,
            } => {
                if ticket.generation != self.generation {
                    return vec![];
                }
                let Some(d) = self.device_mut(&key) else {
                    return vec![];
                };
                if d.pending != Some(ticket.req) {
                    return vec![];
                }
                d.pending = None;
                let ok = result.is_ok();
                match result {
                    Ok(r) => {
                        d.report = Some(*r);
                        d.error = None;
                    }
                    Err(e) => d.error = Some(e),
                }
                if let Some(warning) = schedule_after_read(d, ticket.req, ok, now) {
                    self.error(warning);
                }
            }
            Msg::TestStarted {
                target,
                kind,
                result,
            } => {
                self.test_cmd_busy.retain(|k| *k != target.key);
                match result {
                    Ok(text) => {
                        self.kick_refresh(&target, now);
                        self.modal = Some(Modal::Message {
                            title: format!("{} self-test started", kind_name(kind)),
                            body: text,
                            error: false,
                        });
                    }
                    Err(e) => self.popup_error("Self-test not started on the confirmed drive", e),
                }
            }
            Msg::TestAborted { target, result } => {
                self.test_cmd_busy.retain(|k| *k != target.key);
                match result {
                    Ok(()) => {
                        self.kick_refresh(&target, now);
                        self.info("Self-test aborted");
                    }
                    Err(e) => self.popup_error("Abort not confirmed", e),
                }
            }
            Msg::Exported(result) => {
                self.exporting = false;
                match result {
                    Ok(path) => self.info(format!("Saved {}", path.display())),
                    Err(e) => self.popup_error("PDF export failed", e),
                }
            }
        }
        vec![]
    }

    fn rescan(&mut self) -> Vec<Job> {
        if self.scanning {
            return vec![];
        }
        self.scanning = true;
        self.info(if self.demo() {
            "Loading demo files"
        } else {
            "Scanning for drives"
        });
        vec![Job::Scan {
            demo_files: self.demo_files.clone(),
        }]
    }

    fn load(&mut self, idx: usize) -> Option<Job> {
        let device_type = self.device_type.clone();
        let ticket = Ticket {
            generation: self.generation,
            req: self.next_req,
        };
        let d = self.devices.get_mut(idx)?;
        if d.loading() {
            return None;
        }
        self.next_req += 1;
        d.pending = Some(ticket.req);
        d.next_refresh = None;
        Some(Job::Load {
            ticket,
            source: d.source.clone(),
            device_type,
        })
    }

    /// Starts monitoring after a command. Results from before a rescan apply to the
    /// entry now at the same path unless it is known to be a different drive.
    fn kick_refresh(&mut self, target: &TestTarget, now: Instant) {
        let first_req = self.next_req;
        if let Some(d) = self.device_mut(&target.key) {
            let have = d.report.as_ref().and_then(|r| r.identity.serial.as_ref());
            if have.is_some() && have != target.serial.as_ref() {
                return;
            }
            d.watch = Some(TestWatch {
                first_req,
                backoff: TEST_KICK,
                serial: target.serial.clone(),
            });
            // A read already in flight predates the command; its result reschedules.
            if !d.loading() {
                d.next_refresh = Some(now + TEST_KICK);
            }
        }
    }

    fn confirm(&mut self, c: Confirm) {
        let Some(d) = self.selected_device() else {
            return self.error("No drive selected");
        };
        if d.source.live_path().is_none() {
            return self.error("Self-tests need a live drive (this is a saved file)");
        }
        if self.test_cmd_busy.contains(&d.source.key()) {
            return self.error("A self-test command is already in flight");
        }
        let Some(target) = self.test_target(d) else {
            return;
        };
        if target.serial.is_none() {
            return self.error("Serial unknown - re-read the drive first");
        }
        if target.model.is_none() {
            return self.error("Model unknown - re-read the drive first");
        }
        self.modal = Some(Modal::Confirm(c, target));
    }

    fn test_target(&self, d: &Device) -> Option<TestTarget> {
        let ident = d.report.as_ref().map(|r| &r.identity);
        Some(TestTarget {
            key: d.source.key(),
            path: d.source.live_path()?.to_string(),
            dev_type: self.test_dev_type(d),
            generation: self.generation,
            model: ident.and_then(|i| i.model.clone()),
            serial: ident.and_then(|i| i.serial.clone()),
        })
    }

    /// Runs the confirmed command only if its drive is still the one confirmed.
    fn run_confirmed(&mut self, c: Confirm, target: TestTarget) -> Option<Job> {
        let current = self
            .devices
            .iter()
            .find(|d| d.source.key() == target.key)
            .and_then(|d| self.test_target(d));
        if !still_confirmed(&target, current.as_ref()) {
            self.error("Cancelled: the drive changed since confirmation");
            return None;
        }
        self.test_cmd_busy.push(target.key.clone());
        Some(match c {
            Confirm::Start(kind) => {
                self.info(format!("Starting {} self-test", kind_name(kind)));
                Job::StartTest { target, kind }
            }
            Confirm::Abort => {
                self.info("Aborting self-test");
                Job::AbortTest { target }
            }
        })
    }

    /// The type that worked for the last read wins over the CLI override.
    fn test_dev_type(&self, d: &Device) -> Option<String> {
        d.report
            .as_ref()
            .map(|r| r.meta.device_type.clone())
            .filter(|t| !t.is_empty())
            .or_else(|| self.device_type.clone())
    }

    fn export(&mut self) -> Option<Job> {
        if self.exporting {
            self.error("Export already running");
            return None;
        }
        let Some(report) = self.selected_device().and_then(|d| d.report.clone()) else {
            self.error("No report loaded for this drive yet");
            return None;
        };
        self.exporting = true;
        self.info("Rendering PDF");
        Some(Job::Export {
            report: Box::new(report),
            out_dir: self.out_dir.clone(),
        })
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

    fn device_mut(&mut self, key: &str) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.source.key() == key)
    }

    fn info(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: false,
        });
    }

    fn error(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: true,
        });
    }

    fn popup_error(&mut self, title: &str, body: String) {
        self.error(format!("{title}: {}", body.lines().next().unwrap_or("")));
        self.modal = Some(Modal::Message {
            title: title.into(),
            body,
            error: true,
        });
    }
}

#[cfg(test)]
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

/// Updates test monitoring after a read and schedules the next one. Monitoring
/// lasts while a test runs and ends on a fresh read showing no test or a different
/// drive (returned as a warning). Failed or pre-command reads retry with backoff.
fn schedule_after_read(d: &mut Device, req: u64, ok: bool, now: Instant) -> Option<String> {
    let have = d.report.as_ref().and_then(|r| r.identity.serial.clone());
    let running = d.test_running();
    if ok && d.watch.is_none() && running {
        // A running test is monitored even when another tool started it.
        d.watch = Some(TestWatch {
            first_req: req,
            backoff: TEST_KICK,
            serial: have.clone(),
        });
    }
    let Some(w) = d.watch.as_mut() else {
        d.next_refresh = None;
        return None;
    };
    if !(ok && req >= w.first_req) {
        let delay = w.backoff;
        if !ok {
            w.backoff = (w.backoff * 2).min(TEST_POLL);
        }
        d.next_refresh = Some(now + delay);
        return None;
    }
    if w.serial.is_some() && have.is_some() && w.serial != have {
        d.watch = None;
        d.next_refresh = None;
        let path = d.source.label();
        return Some(format!(
            "{path} is now a different drive; test monitoring stopped"
        ));
    }
    if !running {
        d.watch = None;
        d.next_refresh = None;
        return None;
    }
    w.serial = w.serial.take().or(have);
    w.backoff = TEST_KICK;
    d.next_refresh = Some(now + TEST_POLL);
    None
}

/// The confirmed drive is still the one at its path: every captured field is
/// unchanged and the identity is known.
fn still_confirmed(confirmed: &TestTarget, current: Option<&TestTarget>) -> bool {
    confirmed.serial.is_some() && confirmed.model.is_some() && current == Some(confirmed)
}

pub fn kind_name(k: TestKind) -> &'static str {
    match k {
        TestKind::Short => "Short",
        TestKind::Long => "Extended",
        TestKind::Conveyance => "Conveyance",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DriveReport, SelfTestProgress};
    use crate::tui::device::DeviceSource;

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
        app.on_msg(Msg::Scanned(Ok(devs)), Instant::now())
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

    fn with_serial(serial: &str) -> DriveReport {
        let mut r = DriveReport::default();
        r.identity.model = Some("WDC WD40EFRX".into());
        r.identity.serial = Some(serial.into());
        r
    }

    #[test]
    fn confirmation_needs_known_identity() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        a.on_key(key('s'));
        assert!(a.modal.is_none(), "no report yet");
        loaded(
            &mut a,
            "smartctl:/dev/a",
            DriveReport::default(),
            Instant::now(),
        );
        a.on_key(key('s'));
        assert!(a.modal.is_none());
        assert_eq!(
            a.status.as_ref().unwrap().text,
            "Serial unknown - re-read the drive first"
        );
    }

    #[test]
    fn still_confirmed_compares_every_field() {
        let t = TestTarget {
            key: "smartctl:/dev/a".into(),
            path: "/dev/a".into(),
            dev_type: Some("sat".into()),
            generation: 2,
            model: Some("M".into()),
            serial: Some("S".into()),
        };
        assert!(still_confirmed(&t, Some(&t.clone())));
        assert!(!still_confirmed(&t, None));
        let changed: [fn(&mut TestTarget); 5] = [
            |t| t.path = "/dev/b".into(),
            |t| t.dev_type = Some("nvme".into()),
            |t| t.generation = 3,
            |t| t.model = Some("Other".into()),
            |t| t.serial = Some("S2".into()),
        ];
        for change in changed {
            let mut c = t.clone();
            change(&mut c);
            assert!(!still_confirmed(&t, Some(&c)), "{c:?}");
        }
        let unknown = TestTarget {
            serial: None,
            ..t.clone()
        };
        assert!(!still_confirmed(&unknown, Some(&unknown.clone())));
    }

    #[test]
    fn confirmation_cancelled_when_drive_list_changes() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a"), live("/dev/b")]);
        loaded(&mut a, "smartctl:/dev/a", with_serial("A1"), Instant::now());
        loaded(&mut a, "smartctl:/dev/b", with_serial("B1"), Instant::now());
        a.on_key(key('j'));
        // R then s: the scan lands while the confirmation is open.
        a.on_key(key('R'));
        a.on_key(key('s'));
        let Some(Modal::Confirm(_, t)) = &a.modal else {
            panic!("expected confirmation");
        };
        assert_eq!(t.path, "/dev/b");
        assert_eq!(t.serial.as_deref(), Some("B1"));
        a.on_msg(
            Msg::Scanned(Ok(vec![live("/dev/b"), live("/dev/a")])),
            Instant::now(),
        );
        assert!(a.modal.is_none());
        assert!(a.status.as_ref().unwrap().error);
        assert!(a.on_key(key('y')).is_empty(), "nothing left to confirm");
    }

    #[test]
    fn confirmation_cancelled_when_serial_changes() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        loaded(&mut a, "smartctl:/dev/a", with_serial("A1"), Instant::now());
        a.on_key(key('l'));
        // A refresh reveals a different drive behind the same path.
        loaded(&mut a, "smartctl:/dev/a", with_serial("Z9"), Instant::now());
        assert!(a.on_key(key('y')).is_empty());
        assert!(a.test_cmd_busy.is_empty());
        assert_eq!(
            a.status.as_ref().unwrap().text,
            "Cancelled: the drive changed since confirmation"
        );

        a.on_key(key('l'));
        let target = start_target(a.on_key(key('y')));
        assert_eq!(target.serial.as_deref(), Some("Z9"));
        assert_eq!(target.generation, a.generation);
    }

    #[test]
    fn start_scans_then_loads_every_device() {
        let mut a = app();
        assert!(matches!(a.start().as_slice(), [Job::Scan { .. }]));
        assert!(a.start().is_empty(), "no duplicate scan while scanning");
        let jobs = a.on_msg(
            Msg::Scanned(Ok(vec![live("/dev/a"), live("/dev/b")])),
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
    fn self_test_needs_confirmation() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let mut r = with_serial("A1");
        r.meta.device_type = "sntjmicron".into();
        loaded(&mut a, "smartctl:/dev/a", r, Instant::now());

        assert!(a.on_key(key('s')).is_empty());
        assert!(matches!(
            a.modal,
            Some(Modal::Confirm(Confirm::Start(TestKind::Short), _))
        ));
        assert!(a.on_key(key('n')).is_empty());
        assert!(a.modal.is_none());

        a.on_key(key('l'));
        let jobs = a.on_key(key('y'));
        let [Job::StartTest { target, kind }] = jobs.as_slice() else {
            panic!("expected StartTest, got {jobs:?}");
        };
        assert_eq!(target.path, "/dev/a");
        assert_eq!(target.dev_type.as_deref(), Some("sntjmicron"));
        assert_eq!(*kind, TestKind::Long);

        a.on_key(key('a'));
        assert!(a.modal.is_none(), "blocked while a command is in flight");
        assert!(a.status.as_ref().unwrap().error);
    }

    #[test]
    fn self_test_refused_for_demo_files() {
        let mut a = app();
        scanned(
            &mut a,
            vec![Device::new(DeviceSource::DemoFile("x.json".into()), "")],
        );
        a.on_key(key('s'));
        assert!(a.modal.is_none());
        assert!(a.status.as_ref().unwrap().error);
    }

    #[test]
    fn test_started_shows_message_and_polls() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, "smartctl:/dev/a", with_serial("A1"), t0);
        a.on_key(key('s'));
        let target = start_target(a.on_key(key('y')));
        a.on_msg(
            Msg::TestStarted {
                target,
                kind: TestKind::Short,
                result: Ok("Please wait 2 minutes".into()),
            },
            t0,
        );
        assert!(
            matches!(&a.modal, Some(Modal::Message { body, error: false, .. }) if body.contains("2 minutes"))
        );
        assert!(a.test_cmd_busy.is_empty());

        assert!(a.on_tick(t0 + Duration::from_secs(1)).is_empty());
        let jobs = a.on_tick(t0 + TEST_KICK);
        assert!(matches!(jobs.as_slice(), [Job::Load { .. }]));

        let r = DriveReport {
            self_test_in_progress: Some(SelfTestProgress {
                kind: Some("Short".into()),
                remaining_pct: Some(60),
            }),
            ..Default::default()
        };
        let t1 = t0 + TEST_KICK;
        loaded(&mut a, "smartctl:/dev/a", r, t1);
        assert!(
            a.on_tick(t1 + TEST_POLL - Duration::from_secs(1))
                .is_empty()
        );
        assert_eq!(a.on_tick(t1 + TEST_POLL).len(), 1);

        loaded(
            &mut a,
            "smartctl:/dev/a",
            DriveReport::default(),
            t1 + TEST_POLL,
        );
        assert!(
            a.on_tick(t1 + TEST_POLL * 10).is_empty(),
            "polling stops once the test is done"
        );
    }

    fn started(a: &mut App, now: Instant) {
        a.on_key(key('s'));
        let target = start_target(a.on_key(key('y')));
        a.on_msg(
            Msg::TestStarted {
                target,
                kind: TestKind::Short,
                result: Ok("ok".into()),
            },
            now,
        );
        a.modal = None;
    }

    fn running() -> DriveReport {
        DriveReport {
            self_test_in_progress: Some(SelfTestProgress {
                kind: Some("Short".into()),
                remaining_pct: Some(90),
            }),
            ..Default::default()
        }
    }

    const KEY_A: &str = "smartctl:/dev/a";

    #[test]
    fn polling_survives_failed_reads_after_start() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, KEY_A, with_serial("A1"), t0);
        started(&mut a, t0);

        // Reads keep failing: retry with doubling delays capped at TEST_POLL.
        let mut t = t0 + TEST_KICK;
        for secs in [3, 6, 12, 24, 30, 30] {
            assert_eq!(a.on_tick(t).len(), 1, "retry due");
            a.complete_load(KEY_A, Err("bridge reset".into()), t);
            let next = a.devices[0].next_refresh.expect("still polling");
            assert_eq!(next - t, Duration::from_secs(secs));
            t = next;
        }

        // First fresh read shows the test running: normal 30 s polling.
        assert_eq!(a.on_tick(t).len(), 1);
        a.complete_load(KEY_A, Ok(Box::new(running())), t);
        assert!(
            a.devices[0].watch.is_some(),
            "monitored while the test runs"
        );
        assert_eq!(a.devices[0].next_refresh, Some(t + TEST_POLL));
    }

    #[test]
    fn read_from_before_the_command_does_not_stop_polling() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, KEY_A, with_serial("A1"), t0);
        a.on_key(key('r'));
        let old_req = a.devices[0].pending.unwrap();
        started(&mut a, t0);
        assert!(a.devices[0].watch.is_some());

        // The pre-command read lands showing no test: keep watching.
        a.complete_load(KEY_A, Ok(Box::new(DriveReport::default())), t0);
        assert!(old_req < a.devices[0].watch.as_ref().unwrap().first_req);
        assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK));

        // A fresh read showing no test ends polling.
        a.on_tick(t0 + TEST_KICK);
        a.complete_load(KEY_A, Ok(Box::new(DriveReport::default())), t0 + TEST_KICK);
        assert!(a.devices[0].watch.is_none());
        assert_eq!(a.devices[0].next_refresh, None);
    }

    fn start_result(target: TestTarget) -> Msg {
        Msg::TestStarted {
            target,
            kind: TestKind::Short,
            result: Ok("ok".into()),
        }
    }

    fn confirmed_start(a: &mut App) -> TestTarget {
        a.on_key(key('s'));
        start_target(a.on_key(key('y')))
    }

    fn rescan_same(a: &mut App, now: Instant) {
        a.on_key(key('R'));
        a.on_msg(Msg::Scanned(Ok(vec![live("/dev/a")])), now);
        assert!(a.devices[0].loading());
    }

    #[test]
    fn monitoring_survives_rescan_before_command_result() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, KEY_A, with_serial("A1"), t0);
        let target = confirmed_start(&mut a);
        rescan_same(&mut a, t0);
        a.on_msg(start_result(target), t0);
        assert_eq!(
            a.devices[0].watch.as_ref().unwrap().serial.as_deref(),
            Some("A1")
        );
        a.complete_load(KEY_A, Err("bridge reset".into()), t0);
        assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK));
    }

    #[test]
    fn monitoring_survives_rescan_after_command_result() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, KEY_A, with_serial("A1"), t0);
        let target = confirmed_start(&mut a);
        a.on_msg(start_result(target), t0);
        a.modal = None;
        rescan_same(&mut a, t0);
        assert!(a.devices[0].watch.is_some(), "carried to the new entry");
        a.complete_load(KEY_A, Err("bridge reset".into()), t0);
        assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK));
        a.complete_load(KEY_A, Ok(Box::new(running())), t0 + TEST_KICK);
        assert!(a.devices[0].watch.is_some());
        assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK + TEST_POLL));
    }

    fn running_on(serial: &str) -> DriveReport {
        let mut r = running();
        r.identity = with_serial(serial).identity;
        r
    }

    #[test]
    fn established_monitoring_survives_rescan_and_failed_read() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, KEY_A, with_serial("A1"), t0);
        started(&mut a, t0);
        a.on_tick(t0 + TEST_KICK);
        let t1 = t0 + TEST_KICK;
        a.complete_load(KEY_A, Ok(Box::new(running_on("A1"))), t1);
        assert_eq!(a.devices[0].next_refresh, Some(t1 + TEST_POLL));

        rescan_same(&mut a, t1);
        assert!(a.devices[0].report.is_none(), "new entry has no report yet");
        a.complete_load(KEY_A, Err("bridge reset".into()), t1);
        assert_eq!(
            a.devices[0].next_refresh,
            Some(t1 + TEST_KICK),
            "still scheduled"
        );

        let t2 = t1 + TEST_KICK;
        assert_eq!(a.on_tick(t2).len(), 1);
        a.complete_load(KEY_A, Ok(Box::new(running_on("A1"))), t2);
        assert_eq!(a.devices[0].next_refresh, Some(t2 + TEST_POLL));

        // Test completed: monitoring ends.
        let t3 = t2 + TEST_POLL;
        assert_eq!(a.on_tick(t3).len(), 1);
        a.complete_load(KEY_A, Ok(Box::new(with_serial("A1"))), t3);
        assert!(a.devices[0].watch.is_none());
        assert_eq!(a.devices[0].next_refresh, None);
        assert!(a.on_tick(t3 + TEST_POLL * 4).is_empty());
    }

    #[test]
    fn externally_started_test_is_monitored() {
        let mut d = live("/dev/a");
        let now = Instant::now();
        d.report = Some(running_on("A1"));
        assert_eq!(schedule_after_read(&mut d, 7, true, now), None);
        let w = d.watch.clone().expect("monitoring started");
        assert_eq!((w.first_req, w.serial.as_deref()), (7, Some("A1")));
        assert_eq!(d.next_refresh, Some(now + TEST_POLL));

        // Read error while monitored: quick retry.
        assert_eq!(schedule_after_read(&mut d, 8, false, now), None);
        assert_eq!(d.next_refresh, Some(now + TEST_KICK));

        // Fresh read from a different drive at the path ends monitoring.
        d.report = Some(running_on("Z9"));
        let warn = schedule_after_read(&mut d, 9, true, now).unwrap();
        assert!(warn.contains("different drive"));
        assert!(d.watch.is_none() && d.next_refresh.is_none());
    }

    #[test]
    fn late_command_result_skips_a_different_drive() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, KEY_A, with_serial("A1"), t0);
        let target = confirmed_start(&mut a);
        rescan_same(&mut a, t0);
        a.complete_load(KEY_A, Ok(Box::new(with_serial("Z9"))), t0);
        a.on_msg(start_result(target), t0);
        assert!(a.devices[0].watch.is_none());
        assert_eq!(a.devices[0].next_refresh, None);
    }

    #[test]
    fn carried_monitoring_stops_when_serial_differs() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a")]);
        let t0 = Instant::now();
        loaded(&mut a, KEY_A, with_serial("A1"), t0);
        let target = confirmed_start(&mut a);
        a.on_msg(start_result(target), t0);
        rescan_same(&mut a, t0);
        a.complete_load(KEY_A, Ok(Box::new(with_serial("Z9"))), t0);
        assert!(a.devices[0].watch.is_none());
        assert_eq!(a.devices[0].next_refresh, None);
        assert!(a.status.as_ref().unwrap().text.contains("different drive"));
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
        let jobs = a.on_msg(Msg::Scanned(Ok(vec![live("/dev/a")])), Instant::now());
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
    fn rescan_keeps_selection_in_range() {
        let mut a = app();
        scanned(&mut a, vec![live("/dev/a"), live("/dev/b")]);
        a.on_key(key('j'));
        a.on_key(key('R'));
        a.on_msg(Msg::Scanned(Ok(vec![live("/dev/a")])), Instant::now());
        assert_eq!(a.selected, 0);
        a.on_msg(Msg::Scanned(Ok(vec![])), Instant::now());
        assert!(a.selected_device().is_none());
        assert!(a.on_key(key('r')).is_empty());
    }
}
