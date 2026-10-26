//! Self-test confirmation, commands and monitoring.

use std::time::{Duration, Instant};

use crate::smartctl::TestKind;
use crate::tui::device::DeviceSource;
use crate::tui::device::{Device, TestWatch};
use crate::tui::worker::{Job, TestTarget, TestVia};

use super::{App, Confirm, MAX_UNKNOWN_READS, Modal, TEST_KICK, TEST_POLL, USB_TEST_POLL};

impl App {
    /// Starts monitoring after a command. Results from before a rescan apply to the
    /// entry now at the same path unless it is known to be a different drive.
    pub(super) fn kick_refresh(&mut self, target: &TestTarget, now: Instant) {
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
                unknown_reads: 0,
            });
            // A read already in flight predates the command; its result reschedules.
            if !d.loading() {
                d.next_refresh = Some(now + TEST_KICK);
            }
        }
    }

    pub(super) fn confirm(&mut self, c: Confirm) {
        let Some(d) = self.selected_device() else {
            return self.error("No drive selected");
        };
        if let Some(why) = d.source.no_self_test_reason() {
            return self.error(format!("Self-tests need a live drive ({why})"));
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

    pub(super) fn test_target(&self, d: &Device) -> Option<TestTarget> {
        let ident = d.report.as_ref().map(|r| &r.identity);
        let via = match &d.source {
            DeviceSource::Smartctl { path, .. } => TestVia::Smartctl {
                path: path.clone(),
                dev_type: self.test_dev_type(d),
            },
            DeviceSource::NativeUsb { selector, .. } => TestVia::NativeUsb {
                selector: selector.clone(),
            },
            DeviceSource::DemoFile(_) => return None,
        };
        Some(TestTarget {
            key: d.source.key(),
            label: d.source.label(),
            via,
            generation: self.generation,
            model: ident.and_then(|i| i.model.clone()),
            serial: ident.and_then(|i| i.serial.clone()),
        })
    }

    /// Runs the confirmed command only if its drive is still the one confirmed.
    pub(super) fn run_confirmed(&mut self, c: Confirm, target: TestTarget) -> Option<Job> {
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
}

/// Updates test monitoring after a read and schedules the next one. Monitoring
/// lasts while a test runs and ends on a fresh read that reports the drive idle or
/// shows a different drive (returned as a warning). Failed, pre-command and
/// status-less reads retry with backoff; `MAX_UNKNOWN_READS` status-less reads in a
/// row end monitoring with a warning.
pub(super) fn schedule_after_read(
    d: &mut Device,
    req: u64,
    ok: bool,
    now: Instant,
) -> Option<String> {
    let have = d.report.as_ref().and_then(|r| r.identity.serial.clone());
    let running = d.test_running();
    let status_known = d.report.as_ref().is_some_and(|r| r.self_test_status_known);
    if ok && d.watch.is_none() && running {
        // A running test is monitored even when another tool started it.
        d.watch = Some(TestWatch {
            first_req: req,
            backoff: TEST_KICK,
            serial: have.clone(),
            unknown_reads: 0,
        });
    }
    let poll = poll_interval(d);
    let Some(w) = d.watch.as_mut() else {
        d.next_refresh = None;
        return None;
    };
    if !(ok && req >= w.first_req) {
        let delay = w.backoff;
        if !ok {
            w.backoff = (w.backoff * 2).min(poll);
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
    if !running && status_known {
        d.watch = None;
        d.next_refresh = None;
        return None;
    }
    if !running {
        // Self-test state missing from this read: the test may still run.
        w.unknown_reads += 1;
        if w.unknown_reads >= MAX_UNKNOWN_READS {
            d.watch = None;
            d.next_refresh = None;
            return Some("Drive does not report self-test status; stopped monitoring".into());
        }
        let delay = w.backoff;
        w.backoff = (w.backoff * 2).min(poll);
        d.next_refresh = Some(now + delay);
        return None;
    }
    w.unknown_reads = 0;
    w.serial = w.serial.take().or(have);
    w.backoff = TEST_KICK;
    d.next_refresh = Some(now + poll);
    None
}

/// Progress reads through a USB bridge unmount and remount the drive each
/// time, so they run less often.
pub fn poll_interval(d: &Device) -> Duration {
    match d.source {
        DeviceSource::NativeUsb { .. } => USB_TEST_POLL,
        _ => TEST_POLL,
    }
}

/// The confirmed drive is still the one at its path: every captured field is
/// unchanged and the identity is known.
pub(super) fn still_confirmed(confirmed: &TestTarget, current: Option<&TestTarget>) -> bool {
    confirmed.serial.is_some() && confirmed.model.is_some() && current == Some(confirmed)
}

pub fn kind_name(k: TestKind) -> &'static str {
    k.name()
}
