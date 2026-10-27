//! Builds a complete, graded `DriveReport` from a backend.

use anyhow::Context;
use sha2::{Digest, Sha256};

use crate::model::DriveReport;
use crate::native::{self, UsbSelector};
use crate::{bridge, health, smartctl};

#[derive(Debug, Clone, Default)]
pub struct CollectOptions {
    pub device: String,
    /// smartctl `-d` type; None = auto-probe.
    pub device_type: Option<String>,
}

/// Reads a live drive via smartctl, parses and grades it.
pub fn collect(opts: &CollectOptions) -> anyhow::Result<DriveReport> {
    let (raw, used_type) = smartctl::read(&opts.device, opts.device_type.as_deref())?;
    let mut r = from_smartctl_json(&raw, &opts.device)?;
    r.meta.device_type = used_type;
    r.bridge = r.bridge.or_else(|| bridge::lookup(&opts.device));
    Ok(r)
}

/// Builds a graded report from saved `smartctl -x --json` output.
pub fn from_smartctl_json(raw: &str, device_path: &str) -> anyhow::Result<DriveReport> {
    let json: serde_json::Value = serde_json::from_str(raw).context("invalid smartctl JSON")?;
    let mut r = DriveReport::default();
    smartctl::parse(&json, &mut r);
    stamp(&mut r, raw.as_bytes(), device_path);
    r.verdict = health::evaluate(&r);
    Ok(r)
}

/// Reads a USB-SATA drive through the native SAT backend and grades it.
/// Returns the report and the raw smartctl-shaped JSON it was built from.
/// `unmount` unmounts the drive's volumes first; `force` only skips busy-disk
/// checks that cannot be completed.
pub fn from_native(
    sel: &UsbSelector,
    force: bool,
    unmount: bool,
) -> anyhow::Result<(DriveReport, String)> {
    let res = native::read_smart(sel, force, unmount)?;
    let raw = serde_json::to_string_pretty(&res.json)?;
    let mut r = from_smartctl_json(&raw, &sel.to_string())?;
    r.bridge = Some(res.bridge);
    r.meta.backend = "ddreport native USB (SAT)".into();
    r.meta.device_type = "native-sat".into();
    r.verdict = health::evaluate(&r);
    Ok((r, raw))
}

/// Fills report metadata; the SHA-256 ties the PDF to the exact raw data.
pub fn stamp(r: &mut DriveReport, raw: &[u8], device_path: &str) {
    let now = chrono::Local::now();
    let raw_sha = hex(&Sha256::digest(raw));
    r.meta.generated_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    r.meta.host = gethostname::gethostname().to_string_lossy().into_owned();
    r.meta.os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    r.meta.tool_version = format!("ddreport {}", env!("CARGO_PKG_VERSION"));
    r.meta.device_path = device_path.to_string();
    r.meta.raw_sha256 = raw_sha;
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
