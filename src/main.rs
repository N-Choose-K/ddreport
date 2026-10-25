use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};

use ddreport::fmt::{bytes_human, power_on_human};
use ddreport::model::DriveReport;
use ddreport::native::{self, UsbSelector};
use ddreport::report::{self, CollectOptions};
use ddreport::smartctl::{self, TestKind};
use ddreport::tui::{self, TuiOptions};
use ddreport::{bridge, pdf};

#[derive(Parser)]
#[command(version, about = "One-page drive health report PDFs from SMART data")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interactive terminal UI (default).
    Tui(TuiArgs),
    /// List drives smartctl can open.
    Scan {
        /// List USB mass-storage devices for --usb instead (enumeration only).
        #[arg(long)]
        usb: bool,
    },
    /// Read a drive and write the PDF certificate.
    Report(Box<ReportArgs>),
    /// Start or abort a SMART self-test.
    Test {
        device: String,
        #[arg(value_enum, default_value = "short")]
        kind: Kind,
        /// smartctl -d type (e.g. sat, sntjmicron).
        #[arg(short = 'd', long)]
        device_type: Option<String>,
    },
}

#[derive(Args, Default)]
struct TuiArgs {
    /// Browse saved `smartctl -x --json` files (no drive access).
    #[arg(long, num_args = 1.., value_name = "FILES")]
    demo: Vec<PathBuf>,
    /// Directory for exported PDFs.
    #[arg(short, long, default_value = ".")]
    out_dir: PathBuf,
    /// smartctl -d type; probes USB bridge types when omitted.
    #[arg(short = 'd', long)]
    device_type: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Short,
    Long,
    Conveyance,
    Abort,
}

#[derive(Args)]
struct ReportArgs {
    /// Device path, e.g. /dev/sdb or /dev/disk4.
    #[arg(required_unless_present_any = ["from_json", "usb"])]
    device: Option<String>,
    /// smartctl -d type; probes USB bridge types when omitted.
    #[arg(short = 'd', long)]
    device_type: Option<String>,
    /// Build from saved `smartctl -x --json` output (no drive access).
    #[arg(long, conflicts_with = "device")]
    from_json: Option<PathBuf>,
    /// Read a USB-SATA drive directly over USB (no smartctl): vid:pid,
    /// vid:pid:serial or bus:addr from `scan --usb`. Needs root on macOS.
    #[arg(long, value_name = "SELECTOR", conflicts_with_all = ["device", "from_json", "device_type"])]
    usb: Option<String>,
    /// With --usb: continue when the busy-disk check can't be completed.
    /// Never overrides a detected mount or open file.
    #[arg(long, requires = "usb", conflicts_with_all = ["device", "from_json"])]
    force: bool,
    /// PDF path (default: <model>_<serial>.pdf).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Also write the parsed report as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Also save the raw smartctl(-shaped) JSON (live drives only).
    #[arg(long, conflicts_with = "from_json")]
    save_raw: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        None => run_tui(TuiArgs {
            out_dir: ".".into(),
            ..Default::default()
        }),
        Some(Cmd::Tui(a)) => run_tui(a),
        Some(Cmd::Scan { usb: true }) => {
            for d in native::list_devices()? {
                println!("{:<32} {d}", d.selector());
            }
            Ok(())
        }
        Some(Cmd::Scan { usb: false }) => {
            for d in smartctl::scan()? {
                println!(
                    "{:<24} {:<12} {:<6} {}",
                    d.name, d.dev_type, d.protocol, d.info
                );
            }
            Ok(())
        }
        Some(Cmd::Test {
            device,
            kind,
            device_type,
        }) => {
            let t = device_type.as_deref();
            let kind = match kind {
                Kind::Abort => {
                    smartctl::abort_self_test(&device, t)?;
                    println!("Self-test aborted");
                    return Ok(());
                }
                Kind::Short => TestKind::Short,
                Kind::Long => TestKind::Long,
                Kind::Conveyance => TestKind::Conveyance,
            };
            println!("{}", smartctl::start_self_test(&device, t, kind)?);
            Ok(())
        }
        Some(Cmd::Report(a)) => run_report(*a),
    }
}

fn run_tui(a: TuiArgs) -> anyhow::Result<()> {
    tui::run(TuiOptions {
        demo_files: a.demo,
        out_dir: a.out_dir,
        device_type: a.device_type,
    })
}

fn run_report(a: ReportArgs) -> anyhow::Result<()> {
    let r = match (&a.from_json, &a.save_raw) {
        _ if a.usb.is_some() => {
            let sel: UsbSelector = a.usb.as_deref().unwrap_or_default().parse()?;
            let (r, raw) = report::from_native(&sel, a.force)?;
            if let Some(p) = &a.save_raw {
                std::fs::write(p, &raw).with_context(|| format!("writing {}", p.display()))?;
            }
            r
        }
        (Some(path), _) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            report::from_smartctl_json(&raw, &path.display().to_string())?
        }
        (None, Some(raw_path)) => {
            let device = a.device.clone().unwrap_or_default();
            let (raw, used) = smartctl::read(&device, a.device_type.as_deref())?;
            std::fs::write(raw_path, &raw)
                .with_context(|| format!("writing {}", raw_path.display()))?;
            let mut r = report::from_smartctl_json(&raw, &device)?;
            r.meta.device_type = used;
            r.bridge = bridge::lookup(&device);
            r
        }
        (None, None) => report::collect(&CollectOptions {
            device: a.device.clone().unwrap_or_default(),
            device_type: a.device_type.clone(),
        })?,
    };

    print_summary(&r);
    let out = a
        .output
        .unwrap_or_else(|| PathBuf::from(default_file_name(&r)));
    std::fs::write(&out, pdf::render(&r)?).with_context(|| format!("writing {}", out.display()))?;
    println!("PDF: {}", out.display());
    if let Some(p) = &a.json {
        write_json(p, &r)?;
        println!("JSON: {}", p.display());
    }
    Ok(())
}

fn write_json(p: &Path, r: &DriveReport) -> anyhow::Result<()> {
    std::fs::write(p, serde_json::to_string_pretty(r)?)
        .with_context(|| format!("writing {}", p.display()))
}

/// `<model>_<serial>.pdf`, restricted to filename-safe characters.
fn default_file_name(r: &DriveReport) -> String {
    let id = &r.identity;
    let stem = format!(
        "{}_{}",
        id.model.as_deref().unwrap_or("drive"),
        id.serial.as_deref().unwrap_or("unknown")
    );
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{safe}.pdf")
}

fn print_summary(r: &DriveReport) {
    let id = &r.identity;
    let h = &r.health;
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    println!("{}", or_dash(id.model.clone()));
    println!("  Maker:      {}", or_dash(id.manufacturer.clone()));
    println!("  Serial:     {}", or_dash(id.serial.clone()));
    println!(
        "  Capacity:   {}",
        or_dash(id.capacity_bytes.map(bytes_human))
    );
    println!(
        "  Power-on:   {}",
        or_dash(h.power_on_hours.map(power_on_human))
    );
    println!(
        "  Verdict:    {:?} - {}",
        r.verdict.grade, r.verdict.summary
    );
    for f in &r.verdict.findings {
        println!("    [{:?}] {}", f.severity, f.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("ddreport").chain(args.iter().copied()))
    }

    #[test]
    fn usb_flag_conflicts() {
        assert!(parse(&["report", "--usb", "1f75:0611"]).is_ok());
        assert!(parse(&["report", "--usb", "1f75:0611", "--force"]).is_ok());
        assert!(parse(&["report", "--usb", "1f75:0611", "/dev/disk6"]).is_err());
        assert!(parse(&["report", "--usb", "1f75:0611", "--from-json", "x.json"]).is_err());
        assert!(parse(&["report", "--usb", "1f75:0611", "-d", "sat"]).is_err());
        assert!(parse(&["report", "/dev/disk6", "--force"]).is_err());
        assert!(parse(&["report", "--from-json", "x.json", "--force"]).is_err());
        assert!(parse(&["report"]).is_err());
        assert!(matches!(
            parse(&["scan", "--usb"]).unwrap().cmd,
            Some(Cmd::Scan { usb: true })
        ));
    }
}
