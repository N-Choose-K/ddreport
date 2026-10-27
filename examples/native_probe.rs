//! Dumps native USB SAT SMART data as smartctl-style JSON.
//!
//! Usage:
//!   native_probe --list                 enumerate USB mass-storage devices (no device I/O)
//!   sudo native_probe <selector> [--force]   read one drive; selector is vid:pid[:serial] or bus:addr
//!
//! Refuses if the disk is mounted or open. --force only skips checks that cannot be completed.

use ddreport::native::{self, UsbSelector};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--list" => {
            for d in native::list_devices()? {
                println!("{d}");
            }
            Ok(())
        }
        [sel, rest @ ..] if !sel.starts_with('-') && (rest.is_empty() || rest == ["--force"]) => {
            let sel: UsbSelector = sel.parse()?;
            let r = native::read_smart(&sel, !rest.is_empty(), false)?;
            eprintln!("bridge: {:?}", r.bridge);
            if let Some(i) = &r.bridge_inquiry {
                eprintln!("bridge INQUIRY: {i}");
            }
            println!("{}", serde_json::to_string_pretty(&r.json)?);
            Ok(())
        }
        _ => {
            eprintln!("usage: native_probe --list");
            eprintln!("       sudo native_probe <vid:pid[:serial]|bus:addr> [--force]");
            std::process::exit(2);
        }
    }
}
