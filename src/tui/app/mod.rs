//! UI state and pure update logic: keys and worker messages in, jobs out.

mod eject;
mod keys;
mod selftest;
#[cfg(test)]
mod tests;
mod worker_msgs;

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Duration;

use crate::smartctl::TestKind;

use super::TuiOptions;
use super::device::Device;
use super::worker::TestTarget;

pub use selftest::kind_name;

/// Fresh reads without self-test state tolerated before monitoring gives up.
pub const MAX_UNKNOWN_READS: u8 = 5;
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
    /// Eject a USB drive's volumes and read it through the native backend.
    Eject {
        key: String,
        label: String,
        bridge: String,
        mounts: Vec<String>,
    },
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
