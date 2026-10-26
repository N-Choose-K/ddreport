//! Jobs out, worker messages in: scan, load, export.

use std::time::Instant;

use crate::tui::device::DeviceSource;
use crate::tui::worker::{Job, Msg, ScanOutcome, Ticket};

use super::selftest::{kind_name, schedule_after_read};
use super::{App, Modal};

impl App {
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
        // Progress reads through a bridge remount the drive themselves.
        due.into_iter()
            .filter_map(|i| {
                let remount = matches!(self.devices[i].source, DeviceSource::NativeUsb { .. });
                self.load_with(i, remount)
            })
            .collect()
    }
    pub fn on_msg(&mut self, msg: Msg, now: Instant) -> Vec<Job> {
        match msg {
            Msg::Scanned(Ok(ScanOutcome { devices, skipped })) => {
                self.scanning = false;
                self.generation += 1;
                let cancelled =
                    matches!(self.modal, Some(Modal::Confirm(..) | Modal::Remount { .. }));
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
                self.info(scan_summary(self.devices.len(), &skipped));
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

    pub(super) fn rescan(&mut self) -> Vec<Job> {
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

    pub(super) fn load(&mut self, idx: usize) -> Option<Job> {
        self.load_with(idx, false)
    }

    /// `remount`: native USB only; unmount the drive's volumes before the read.
    pub(super) fn load_with(&mut self, idx: usize, remount: bool) -> Option<Job> {
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
            remount,
        })
    }

    pub(super) fn export(&mut self) -> Option<Job> {
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
}

fn scan_summary(found: usize, skipped: &[String]) -> String {
    let names = |v: &[String]| {
        v.iter()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match skipped {
        [] => format!("Found {found} drive(s)"),
        s => format!(
            "Found {found} drive(s); skipped {} non-report file(s): {}",
            s.len(),
            names(s)
        ),
    }
}
