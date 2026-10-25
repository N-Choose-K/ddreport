use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};

use ddreport::fmt::{bytes_human, power_on_human};
use ddreport::model::DriveReport;
use ddreport::report::{self, CollectOptions};
use ddreport::smartctl::{self, TestKind};
use ddreport::{bridge, pdf};

#[derive(Parser)]
#[command(version, about = "One-page drive health report PDFs from SMART data")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List drives smartctl can open.
    Scan,
    /// Read a drive and write the PDF certificate.
    Report(ReportArgs),
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
    #[arg(required_unless_present = "from_json")]
    device: Option<String>,
    /// smartctl -d type; probes USB bridge types when omitted.
    #[arg(short = 'd', long)]
    device_type: Option<String>,
    /// Build from saved `smartctl -x --json` output (no drive access).
    #[arg(long, conflicts_with = "device")]
    from_json: Option<PathBuf>,
    /// PDF path (default: <model>_<serial>.pdf).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Also write the parsed report as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Also save the raw smartctl JSON (live drives only).
    #[arg(long, conflicts_with = "from_json")]
    save_raw: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        None => {
            use clap::CommandFactory;
            Cli::command().print_help()?;
            Ok(())
        }
        Some(Cmd::Scan) => {
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
        Some(Cmd::Report(a)) => run_report(a),
    }
}

fn run_report(a: ReportArgs) -> anyhow::Result<()> {
    let r = match (&a.from_json, &a.save_raw) {
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
