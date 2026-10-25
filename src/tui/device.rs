//! Device list entries and the data sources they come from.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::model::{DriveReport, Grade};

/// Where a device's report comes from. New backends (e.g. native USB) add a variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSource {
    /// Live drive read through smartctl.
    Smartctl {
        path: String,
        /// Type reported by `smartctl --scan-open`; informational only.
        scanned_type: String,
    },
    /// Saved `smartctl -x --json` output.
    DemoFile(PathBuf),
}

impl DeviceSource {
    /// Stable identity used to match background results to list entries.
    pub fn key(&self) -> String {
        match self {
            Self::Smartctl { path, .. } => format!("smartctl:{path}"),
            Self::DemoFile(p) => format!("file:{}", p.display()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Smartctl { path, .. } => path.clone(),
            Self::DemoFile(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
        }
    }

    /// Device path for commands that act on live hardware.
    pub fn live_path(&self) -> Option<&str> {
        match self {
            Self::Smartctl { path, .. } => Some(path),
            Self::DemoFile(_) => None,
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
