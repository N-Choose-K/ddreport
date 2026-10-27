//! Device list entries and the data sources they come from.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::model::{DriveReport, Grade};
use crate::native::{UsbDisk, UsbSelector};

/// Where a device's report comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSource {
    /// Live drive read through smartctl.
    Smartctl {
        path: String,
        /// Type reported by `smartctl --scan-open`; informational only.
        scanned_type: String,
    },
    /// USB-SATA drive read through the native SAT backend (macOS, needs root).
    NativeUsb {
        selector: UsbSelector,
        /// Bridge product name from the USB descriptors; the selector when absent.
        bridge: String,
        /// The disk behind the bridge, as far as the OS knows without a read.
        disk: Option<UsbDisk>,
    },
    /// Saved `smartctl -x --json` output.
    DemoFile(PathBuf),
}

impl DeviceSource {
    /// Stable identity used to match background results to list entries.
    pub fn key(&self) -> String {
        match self {
            Self::Smartctl { path, .. } => format!("smartctl:{path}"),
            Self::NativeUsb { selector, .. } => format!("usb:{selector}"),
            Self::DemoFile(p) => format!("file:{}", p.display()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Smartctl { path, .. } => path.clone(),
            Self::NativeUsb { bridge, disk, .. } => disk
                .as_ref()
                .and_then(|k| k.media_name.clone())
                .unwrap_or_else(|| bridge.clone()),
            Self::DemoFile(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
        }
    }

    /// Device path for smartctl commands that act on live hardware.
    pub fn live_path(&self) -> Option<&str> {
        match self {
            Self::Smartctl { path, .. } => Some(path),
            Self::NativeUsb { .. } | Self::DemoFile(_) => None,
        }
    }

    /// Mount points of the drive's volumes, when known.
    pub fn mounts(&self) -> &[String] {
        match self {
            Self::NativeUsb { disk: Some(k), .. } => &k.mounts,
            _ => &[],
        }
    }

    /// Shown under a failed read.
    pub fn retry_hint(&self) -> &'static str {
        match self {
            Self::Smartctl { .. } => "Press r to retry, or start ddreport with -d <type>.",
            Self::NativeUsb { .. } => {
                "m remounts and reads (unmount, read, macOS mounts it again); r retries."
            }
            Self::DemoFile(_) => "Press r to retry.",
        }
    }

    /// Why self-tests cannot be started on this source; None when they can.
    pub fn no_self_test_reason(&self) -> Option<&'static str> {
        match self {
            Self::Smartctl { .. } | Self::NativeUsb { .. } => None,
            Self::DemoFile(_) => Some("this is a saved file"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub source: DeviceSource,
    /// Extra scan info shown in the list.
    pub info: String,
    /// Last good report; kept while a refresh is in flight.
    pub report: Option<DriveReport>,
    /// Request ID of the read in flight; results with any other ID are stale.
    pub pending: Option<u64>,
    pub error: Option<String>,
    /// When to reload automatically (self-test progress polling).
    pub next_refresh: Option<Instant>,
    /// Present while a self-test runs or a start/abort awaits a fresh read.
    pub watch: Option<TestWatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestWatch {
    /// Reads with a request ID at or above this are fresh (issued after the command).
    pub first_req: u64,
    /// Delay before the next retry; doubles on each failed read.
    pub backoff: Duration,
    /// Serial of the commanded drive; None = unverified (path match only).
    pub serial: Option<String>,
    /// Consecutive fresh reads without self-test state.
    pub unknown_reads: u8,
}

impl Device {
    pub fn new(source: DeviceSource, info: impl Into<String>) -> Self {
        Self {
            source,
            info: info.into(),
            report: None,
            pending: None,
            error: None,
            next_refresh: None,
            watch: None,
        }
    }

    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn grade(&self) -> Option<Grade> {
        self.report.as_ref().map(|r| r.verdict.grade)
    }

    pub fn test_running(&self) -> bool {
        self.report
            .as_ref()
            .is_some_and(|r| r.self_test_in_progress.is_some())
    }
}
