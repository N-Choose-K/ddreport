//! PDF export and output file naming.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{Local, NaiveDateTime};

use crate::model::DriveReport;
use crate::pdf;

/// Renders the PDF report and writes it to `out_dir`, returning the file path.
pub fn write_pdf(report: &DriveReport, out_dir: &Path) -> anyhow::Result<PathBuf> {
    let bytes = pdf::render(report)?;
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    write_new(
        out_dir,
        &export_stem(report, Local::now().naive_local()),
        &bytes,
    )
}

/// Writes `<stem>.pdf`, or `<stem>-2.pdf`, `-3`, ... if taken. Never overwrites
/// and never follows an existing symlink (`create_new` is O_EXCL).
pub fn write_new(dir: &Path, stem: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    for n in 1..=999 {
        let name = match n {
            1 => format!("{stem}.pdf"),
            n => format!("{stem}-{n}.pdf"),
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(bytes)
                    .with_context(|| format!("writing {}", path.display()))?;
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
    anyhow::bail!(
        "too many existing exports named {stem}*.pdf in {}",
        dir.display()
    )
}

/// `<model>_<serial>_<YYYYMMDD-HHMMSS>.pdf` for an export at local time `at`.
pub fn export_filename(report: &DriveReport, at: NaiveDateTime) -> String {
    format!("{}.pdf", export_stem(report, at))
}

pub fn export_stem(report: &DriveReport, at: NaiveDateTime) -> String {
    let part = |s: Option<&String>, fallback: &str| {
        let clean = sanitize(s.map(String::as_str).unwrap_or(""));
        if clean.is_empty() {
            fallback.to_string()
        } else {
            clean
        }
    };
    format!(
        "{}_{}_{}",
        part(report.identity.model.as_ref(), "drive"),
        part(report.identity.serial.as_ref(), "unknown"),
        at.format("%Y%m%d-%H%M%S")
    )
}

/// Keeps `[A-Za-z0-9.-]`, folds everything else into single `_`, caps the length.
pub fn sanitize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
            c
        } else {
            '_'
        };
        if c == '_' && (out.is_empty() || out.ends_with('_')) {
            continue;
        }
        out.push(c);
    }
    out.truncate(48);
    out.trim_matches(|c| c == '_' || c == '.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize("WDC WD40EFRX-68N32N0"), "WDC_WD40EFRX-68N32N0");
        assert_eq!(sanitize("  a/b\\c:d  "), "a_b_c_d");
        assert_eq!(sanitize("../../etc/passwd"), "etc_passwd");
        assert_eq!(
            sanitize("Samsung SSD 870 EVO 1TB"),
            "Samsung_SSD_870_EVO_1TB"
        );
        assert_eq!(sanitize("日本語"), "");
        assert_eq!(sanitize(&"x".repeat(100)).len(), 48);
    }

    #[test]
    fn filename_uses_local_timestamp_and_fallbacks() {
        let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(14, 3, 9)
            .unwrap();
        let mut r = DriveReport::default();
        assert_eq!(export_filename(&r, at), "drive_unknown_20261025-140309.pdf");
        r.identity.model = Some("ST4000DM004-2CV104".into());
        r.identity.serial = Some("ZFN 0/AB".into());
        assert_eq!(
            export_filename(&r, at),
            "ST4000DM004-2CV104_ZFN_0_AB_20261025-140309.pdf"
        );
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ddreport-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn write_new_never_overwrites() {
        let d = scratch_dir("export-collide");
        let a = write_new(&d, "x", b"one").unwrap();
        let b = write_new(&d, "x", b"two").unwrap();
        let c = write_new(&d, "x", b"three").unwrap();
        assert_eq!(a, d.join("x.pdf"));
        assert_eq!(b, d.join("x-2.pdf"));
        assert_eq!(c, d.join("x-3.pdf"));
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn write_new_does_not_follow_symlinks() {
        let d = scratch_dir("export-symlink");
        let victim = d.join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, d.join("x.pdf")).unwrap();
        let p = write_new(&d, "x", b"pdf").unwrap();
        assert_eq!(p, d.join("x-2.pdf"));
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
