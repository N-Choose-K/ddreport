//! Background jobs. Each job runs on its own thread and reports back with one `Msg`.

use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use crate::model::DriveReport;
use crate::native;
use crate::report::{self, CollectOptions};
use crate::smartctl::{self, TestKind};

use super::device::{Device, DeviceSource};
use super::export;

pub const WORKER_THREAD: &str = "ddreport-worker";

/// Identifies one read: the scan generation it belongs to and a unique request ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u64,
    pub req: u64,
}

/// The drive a self-test command was confirmed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestTarget {
    pub key: String,
    pub path: String,
    pub dev_type: Option<String>,
    pub generation: u64,
    pub model: Option<String>,
    pub serial: Option<String>,
}

impl TestTarget {
    /// Ok when the identity read right before the command matches the confirmed drive.
    pub fn verify(&self, model: Option<&str>, serial: Option<&str>) -> Result<(), String> {
        check_field("serial", self.serial.as_deref(), serial)?;
        check_field("model", self.model.as_deref(), model)
    }
}

fn check_field(name: &str, expected: Option<&str>, actual: Option<&str>) -> Result<(), String> {
    match (expected, actual) {
        (None, _) => Err(format!("{name} of the confirmed drive is unknown")),
        (Some(_), None) => Err(format!("drive did not report a {name}")),
        (Some(e), Some(a)) if e != a => Err(format!("{name} is now {a}, confirmed {e}")),
        _ => Ok(()),
    }
}

/// Checks the identity printed by the command run itself. `Err` means the command
/// may have reached another drive, or its target cannot be confirmed.
pub fn check_reached(
    target: &TestTarget,
    abort: bool,
    model: Option<&str>,
    serial: Option<&str>,
) -> Result<(), String> {
    let (Some(m), Some(s)) = (model, serial) else {
        return Err(format!(
            "smartctl did not report which drive at {} received the command; check the drive \
             and press r",
            target.path
        ));
    };
    if target.verify(model, serial).is_ok() {
        return Ok(());
    }
    Err(if abort {
        format!(
            "Abort reached a different drive (model {m}, serial {s}); the confirmed drive may \
             still be testing"
        )
    } else {
        format!(
            "Command reached a different drive (model {m}, serial {s}) - SMART self-tests are \
             read-only; press a on that drive to abort if unwanted"
        )
    })
}

/// Starts (`kind` set) or aborts a self-test on the confirmed drive.
///
/// smartctl addresses drives by path, so a different drive can appear at the path
/// between the `-i` pre-check and the command run. The command run uses `-i` too, so
/// smartctl reports the identity from the same open handle that received the command
/// and a swap in that window is reported to the user. The residual risk is acceptable:
/// self-tests and aborts are non-destructive (a self-test only reads the media).
fn self_test_command(target: &TestTarget, kind: Option<TestKind>) -> anyhow::Result<String> {
    verify_identity(target)?;
    let dev_type = target.dev_type.as_deref();
    let out = match kind {
        Some(k) => smartctl::start_self_test_identified(&target.path, dev_type, k)?,
        None => smartctl::abort_self_test_identified(&target.path, dev_type)?,
    };
    check_reached(
        target,
        kind.is_none(),
        out.model.as_deref(),
        out.serial.as_deref(),
    )
    .map_err(anyhow::Error::msg)?;
    Ok(out.message)
}

/// Re-identifies the drive at the target path; refuses if it is not the confirmed one.
fn verify_identity(target: &TestTarget) -> anyhow::Result<()> {
    let (model, serial) = smartctl::identify(&target.path, target.dev_type.as_deref())?;
    target
        .verify(model.as_deref(), serial.as_deref())
        .map_err(|e| anyhow::anyhow!("refused, {} may be a different drive: {e}", target.path))
}

#[derive(Debug, Clone)]
pub enum Job {
    Scan {
        demo_files: Vec<PathBuf>,
    },
    Load {
        ticket: Ticket,
        source: DeviceSource,
        device_type: Option<String>,
    },
    StartTest {
        target: TestTarget,
        kind: TestKind,
    },
    AbortTest {
        target: TestTarget,
    },
    Export {
        report: Box<DriveReport>,
        out_dir: PathBuf,
    },
}

/// Devices found by a scan, plus demo files skipped as not being drive reports.
#[derive(Debug, Default)]
pub struct ScanOutcome {
    pub devices: Vec<Device>,
    pub skipped: Vec<String>,
}

impl From<Vec<Device>> for ScanOutcome {
    fn from(devices: Vec<Device>) -> Self {
        Self {
            devices,
            skipped: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum Msg {
    Scanned(Result<ScanOutcome, String>),
    Loaded {
        ticket: Ticket,
        key: String,
        result: Result<Box<DriveReport>, String>,
    },
    TestStarted {
        target: TestTarget,
        kind: TestKind,
        result: Result<String, String>,
    },
    TestAborted {
        target: TestTarget,
        result: Result<(), String>,
    },
    Exported(Result<PathBuf, String>),
}

#[cfg(test)]
impl Msg {
    pub fn scanned(devices: Vec<Device>) -> Self {
        Msg::Scanned(Ok(devices.into()))
    }
}

pub fn spawn(job: Job, tx: Sender<Msg>) {
    let res = thread::Builder::new()
        .name(WORKER_THREAD.into())
        .spawn(move || {
            // The UI may have quit already; nothing to report to then.
            let _ = tx.send(run(job));
        });
    debug_assert!(res.is_ok(), "failed to spawn worker thread");
}

pub fn is_worker_thread() -> bool {
    thread::current().name() == Some(WORKER_THREAD)
}

/// Executes a job synchronously.
pub fn run(job: Job) -> Msg {
    match job {
        Job::Scan { demo_files } => Msg::Scanned(guarded(|| scan(demo_files))),
        Job::Load {
            ticket,
            source,
            device_type,
        } => Msg::Loaded {
            ticket,
            key: source.key(),
            result: guarded(|| load(&source, device_type).map(Box::new)),
        },
        Job::StartTest { target, kind } => Msg::TestStarted {
            result: guarded(|| self_test_command(&target, Some(kind))),
            target,
            kind,
        },
        Job::AbortTest { target } => Msg::TestAborted {
            result: guarded(|| self_test_command(&target, None).map(|_| ())),
            target,
        },
        Job::Export { report, out_dir } => {
            Msg::Exported(guarded(|| export::write_pdf(&report, &out_dir)))
        }
    }
}

fn scan(demo_files: Vec<PathBuf>) -> anyhow::Result<ScanOutcome> {
    if !demo_files.is_empty() {
        let (reports, skipped): (Vec<_>, Vec<_>) =
            demo_files.into_iter().partition(|p| !is_non_report(p));
        return Ok(ScanOutcome {
            devices: reports
                .into_iter()
                .map(|p| Device::new(DeviceSource::DemoFile(p), "demo file"))
                .collect(),
            skipped: skipped.iter().map(|p| p.display().to_string()).collect(),
        });
    }
    let mut devices: Vec<Device> = smartctl::scan()?
        .into_iter()
        .map(|d| {
            let info = format!("{} {}", d.protocol, d.info).trim().to_string();
            Device::new(
                DeviceSource::Smartctl {
                    path: d.name,
                    scanned_type: d.dev_type,
                },
                info,
            )
        })
        .collect();
    // macOS smartctl cannot open USB bridges; list them for the native backend.
    if cfg!(target_os = "macos") {
        let usb = native::list_devices()?;
        devices.extend(usb.iter().map(|d| usb_device(d, &usb)));
    }
    Ok(devices.into())
}

fn usb_device(d: &native::UsbStorageDevice, all: &[native::UsbStorageDevice]) -> Device {
    let selector = d.unique_selector(all);
    let name = d
        .product
        .clone()
        .or_else(|| d.manufacturer.clone())
        .unwrap_or_else(|| selector.to_string());
    let mut info = format!("USB {selector} [{}]", d.transport.as_str());
    if let Some(s) = &d.speed {
        info.push(' ');
        info.push_str(s);
    }
    Device::new(DeviceSource::NativeUsb { selector, name }, info)
}

/// True for JSON that parses but is no single-drive report, e.g. `--scan-open`
/// output. Unreadable files stay in the list so their error is shown.
fn is_non_report(p: &std::path::Path) -> bool {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|j| !j.get("device").is_some_and(serde_json::Value::is_object))
}

fn load(source: &DeviceSource, device_type: Option<String>) -> anyhow::Result<DriveReport> {
    match source {
        DeviceSource::Smartctl { path, .. } => report::collect(&CollectOptions {
            device: path.clone(),
            device_type,
        }),
        DeviceSource::NativeUsb { selector, .. } => {
            if cfg!(target_os = "macos") && !native::is_root() {
                anyhow::bail!("needs root to read USB drives: run `sudo ddreport`");
            }
            report::from_native(selector, false).map(|(r, _)| r)
        }
        DeviceSource::DemoFile(p) => {
            let raw = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", p.display()))?;
            report::from_smartctl_json(&raw, &p.display().to_string())
        }
    }
}

/// Runs `f`, turning both errors and panics into a message string.
fn guarded<T>(f: impl FnOnce() -> anyhow::Result<T>) -> Result<T, String> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            Err(format!("internal error: {what}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_maps_errors_and_panics() {
        assert_eq!(guarded(|| Ok(1)), Ok(1));
        assert_eq!(
            guarded::<()>(|| Err(anyhow::anyhow!("boom"))),
            Err("boom".into())
        );
        let r = guarded::<()>(|| panic!("kaput"));
        assert_eq!(r, Err("internal error: kaput".into()));
    }

    fn target(model: Option<&str>, serial: Option<&str>) -> TestTarget {
        TestTarget {
            key: "smartctl:/dev/a".into(),
            path: "/dev/a".into(),
            dev_type: Some("sat".into()),
            generation: 1,
            model: model.map(Into::into),
            serial: serial.map(Into::into),
        }
    }

    #[test]
    fn verify_requires_matching_model_and_serial() {
        let t = target(Some("WDC X"), Some("S1"));
        assert_eq!(t.verify(Some("WDC X"), Some("S1")), Ok(()));
        assert!(
            t.verify(Some("WDC X"), Some("S2"))
                .unwrap_err()
                .contains("serial is now S2")
        );
        assert!(
            t.verify(Some("ST Y"), Some("S1"))
                .unwrap_err()
                .contains("model is now ST Y")
        );
        assert!(t.verify(Some("WDC X"), None).is_err());
        assert!(t.verify(None, Some("S1")).is_err());
        assert!(
            target(None, None).verify(None, None).is_err(),
            "unknown never matches"
        );
        assert!(
            target(Some("WDC X"), None)
                .verify(Some("WDC X"), None)
                .is_err()
        );
    }

    #[test]
    fn check_reached_compares_the_command_runs_identity() {
        let t = target(Some("WDC X"), Some("S1"));
        assert_eq!(check_reached(&t, false, Some("WDC X"), Some("S1")), Ok(()));
        let e = check_reached(&t, false, Some("ST Y"), Some("Z9")).unwrap_err();
        assert!(e.starts_with("Command reached a different drive (model ST Y, serial Z9)"));
        assert!(e.contains("press a on that drive"));
        let e = check_reached(&t, true, Some("WDC X"), Some("Z9")).unwrap_err();
        assert!(e.starts_with("Abort reached a different drive"));
        for (m, s) in [(None, Some("S1")), (Some("WDC X"), None), (None, None)] {
            let e = check_reached(&t, false, m, s).unwrap_err();
            assert!(e.contains("did not report which drive"), "{e}");
        }
    }

    #[test]
    fn demo_scan_lists_files_without_reading() {
        let Msg::Scanned(Ok(ScanOutcome { devices: devs, .. })) = run(Job::Scan {
            demo_files: vec!["/nonexistent/a.json".into()],
        }) else {
            panic!("expected scan result");
        };
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].source.label(), "a");
    }

    #[test]
    fn demo_scan_skips_non_reports() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let files = vec![
            dir.join("sata_hdd_wd_red_usb.json"),
            dir.join("scan_open.json"),
        ];
        let Msg::Scanned(Ok(out)) = run(Job::Scan { demo_files: files }) else {
            panic!("expected scan result");
        };
        assert_eq!(out.devices.len(), 1);
        assert_eq!(out.devices[0].source.label(), "sata_hdd_wd_red_usb");
        assert_eq!(out.skipped.len(), 1);
        assert!(out.skipped[0].ends_with("scan_open.json"));
    }

    #[test]
    fn usb_device_names_and_selects_the_bridge() {
        let dev = native::UsbStorageDevice {
            bus: 0,
            address: 13,
            vendor_id: 0x1f75,
            product_id: 0x0611,
            manufacturer: Some("CalDigit, Inc".into()),
            product: Some("TS4 HUB".into()),
            serial: Some("2015".into()),
            transport: native::Transport::Bot,
            speed: Some("USB 3.x SuperSpeed (5 Gb/s)".into()),
            interface: 0,
        };
        let d = usb_device(&dev, std::slice::from_ref(&dev));
        assert_eq!(d.source.label(), "TS4 HUB");
        assert_eq!(d.source.key(), "usb:1f75:0611:2015");
        assert_eq!(
            d.info,
            "USB 1f75:0611:2015 [BOT] USB 3.x SuperSpeed (5 Gb/s)"
        );
        assert!(d.source.live_path().is_none());
        assert!(d.source.no_self_test_reason().is_some());

        // Serials are kept verbatim so the selector matches the descriptor.
        let odd = native::UsbStorageDevice {
            serial: Some("a:b ".into()),
            ..dev.clone()
        };
        let d = usb_device(&odd, std::slice::from_ref(&odd));
        assert_eq!(d.source.key(), "usb:1f75:0611:a:b ");

        let bare = native::UsbStorageDevice {
            manufacturer: None,
            product: None,
            serial: None,
            speed: None,
            ..dev
        };
        let d = usb_device(&bare, std::slice::from_ref(&bare));
        assert_eq!(d.source.label(), "1f75:0611");
        assert_eq!(d.info, "USB 1f75:0611 [BOT]");

        // Two bridges with the same IDs and no serial: keyed by bus:addr.
        let twin = native::UsbStorageDevice {
            address: 14,
            ..bare.clone()
        };
        let all = vec![bare, twin];
        assert_eq!(usb_device(&all[0], &all).source.key(), "usb:0:13");
        assert_eq!(usb_device(&all[1], &all).source.key(), "usb:0:14");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn usb_load_without_root_says_so() {
        if native::is_root() {
            return;
        }
        let Msg::Loaded { result, .. } = run(Job::Load {
            ticket: Ticket {
                generation: 0,
                req: 0,
            },
            source: DeviceSource::NativeUsb {
                selector: "ffff:ffff".parse().unwrap(),
                name: "x".into(),
            },
            device_type: None,
        }) else {
            panic!("expected load result");
        };
        assert!(result.unwrap_err().contains("sudo ddreport"));
    }

    #[test]
    fn demo_load_reports_missing_file() {
        let Msg::Loaded { result, .. } = run(Job::Load {
            ticket: Ticket {
                generation: 0,
                req: 0,
            },
            source: DeviceSource::DemoFile("/nonexistent/a.json".into()),
            device_type: None,
        }) else {
            panic!("expected load result");
        };
        assert!(result.unwrap_err().contains("reading /nonexistent/a.json"));
    }
}
