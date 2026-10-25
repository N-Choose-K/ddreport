//! TestBackend smoke tests. Run with `--nocapture` to see the screens.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::model::*;

use super::TuiOptions;
use super::app::App;
use super::device::{Device, DeviceSource};
use super::ui;
use super::worker::{self, Msg};

pub fn sample_report() -> DriveReport {
    DriveReport {
        meta: ReportMeta {
            generated_at: "2026-10-25T14:03:11+02:00".into(),
            host: "bench".into(),
            os: "macos".into(),
            tool_version: "0.1.0".into(),
            backend: "smartctl 7.5".into(),
            device_path: "/dev/disk4".into(),
            device_type: "sat".into(),
            raw_sha256: "ab".repeat(32),
            warnings: vec![],
            ..Default::default()
        },
        identity: Identity {
            manufacturer: Some("Western Digital".into()),
            model_family: Some("Western Digital Red".into()),
            model: Some("WDC WD40EFRX-68N32N0".into()),
            serial: Some("WD-WCC7K1234567".into()),
            firmware: Some("82.00A82".into()),
            wwn: Some("5 0014ee 2b1234567".into()),
            capacity_bytes: Some(4_000_787_030_016),
            logical_sector_size: Some(512),
            physical_sector_size: Some(4096),
            protocol: Protocol::Ata,
            media: MediaKind::Hdd,
            rotation_rpm: Some(5400),
            form_factor: Some("3.5 inches".into()),
            sata_version: Some("SATA 3.1".into()),
            link_speed_current: Some("6.0 Gb/s".into()),
            ..Default::default()
        },
        bridge: Some(UsbBridge {
            vendor_id: 0x152d,
            product_id: 0x0578,
            vendor_name: Some("JMicron".into()),
            product_name: Some("USB to ATA/ATAPI Bridge".into()),
            chipset: Some("JMicron JMS578".into()),
            transport: Some("UAS".into()),
            link_speed: Some("5 Gb/s".into()),
            ..Default::default()
        }),
        health: Health {
            smart_supported: Some(true),
            smart_enabled: Some(true),
            overall_passed: Some(true),
            power_on_hours: Some(23_456),
            power_cycles: Some(112),
            temperature_c: Some(34),
            temperature_max_c: Some(51),
            reallocated_sectors: Some(8),
            pending_sectors: Some(0),
            offline_uncorrectable: Some(0),
            udma_crc_errors: Some(0),
            load_cycles: Some(4_211),
            ..Default::default()
        },
        attributes: vec![
            attr(1, "Raw_Read_Error_Rate", 200, 200, 51, 0, true),
            attr(5, "Reallocated_Sector_Ct", 199, 199, 140, 8, true),
            attr(9, "Power_On_Hours", 68, 68, 0, 23_456, false),
            attr(194, "Temperature_Celsius", 116, 99, 0, 34, false),
            attr(197, "Current_Pending_Sector", 200, 200, 0, 0, false),
            attr(199, "UDMA_CRC_Error_Count", 200, 200, 0, 0, false),
        ],
        self_tests: vec![SelfTest {
            kind: "Extended offline".into(),
            status: "Completed without error".into(),
            passed: Some(true),
            lifetime_hours: Some(23_400),
            lba_first_error: None,
        }],
        self_test_in_progress: Some(SelfTestProgress {
            kind: Some("Short offline".into()),
            remaining_pct: Some(40),
        }),
        self_test_status_known: true,
        statistics: vec![Stat {
            group: "General".into(),
            name: "Logical Sectors Written".into(),
            value: "123,456,789".into(),
        }],
        verdict: Verdict {
            grade: Grade::Caution,
            summary: "Usable, 8 reallocated sectors".into(),
            findings: vec![
                Finding {
                    severity: Severity::Good,
                    message: "Drive reports SMART overall-health PASSED".into(),
                },
                Finding {
                    severity: Severity::Caution,
                    message: "8 reallocated sectors: the drive has remapped bad blocks; \
                              watch for growth"
                        .into(),
                },
            ],
        },
    }
}

fn attr(id: u8, name: &str, value: u8, worst: u8, thresh: u8, raw: u64, pre: bool) -> Attribute {
    Attribute {
        id,
        name: name.into(),
        value,
        worst,
        thresh,
        raw,
        raw_string: raw.to_string(),
        prefailure: pre,
        when_failed: String::new(),
    }
}

fn app_with_report() -> App {
    let mut app = App::new(&TuiOptions {
        out_dir: "./certs".into(),
        ..Default::default()
    });
    app.start();
    let mut failing = Device::new(
        DeviceSource::Smartctl {
            path: "/dev/disk5".into(),
            scanned_type: "sat".into(),
        },
        "",
    );
    failing.error = Some("smartctl: unsupported USB bridge".into());
    app.on_msg(
        Msg::Scanned(Ok(vec![
            Device::new(
                DeviceSource::Smartctl {
                    path: "/dev/disk4".into(),
                    scanned_type: "sat".into(),
                },
                "",
            ),
            failing,
        ])),
        Instant::now(),
    );
    app.complete_load(
        "smartctl:/dev/disk4",
        Ok(Box::new(sample_report())),
        Instant::now(),
    );
    app.complete_load(
        "smartctl:/dev/disk5",
        Err("smartctl: unsupported USB bridge".into()),
        Instant::now(),
    );
    app
}

fn render(app: &App, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| ui::draw(f, app)).unwrap();
    t.backend().buffer().clone()
}

fn text(buf: &Buffer) -> String {
    let a = buf.area;
    (0..a.height)
        .map(|y| {
            let line: String = (0..a.width).map(|x| buf[(x, y)].symbol()).collect();
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn screen(app: &App, w: u16, h: u16) -> String {
    let s = text(&render(app, w, h));
    println!("---- {w}x{h} ----\n{s}\n");
    s
}

fn press(app: &mut App, c: KeyCode) {
    app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
}

#[test]
fn overview_renders_at_common_sizes() {
    let app = app_with_report();
    for (w, h) in [(100, 30), (80, 24)] {
        let s = screen(&app, w, h);
        assert!(s.contains("CAUTION"), "{w}x{h}");
        assert!(s.contains("WDC WD40EFRX"), "{w}x{h}");
        assert!(s.contains("Reallocated"), "{w}x{h}");
        assert!(s.contains("/dev/disk5"), "{w}x{h}");
        assert!(s.contains("read failed"), "{w}x{h}");
    }
    // Two-column layout at 120 wide.
    let s = screen(&app, 120, 30);
    assert!(
        s.lines()
            .any(|l| l.contains("Model") && l.contains("SMART status"))
    );
}

#[test]
fn verdict_badge_is_colored() {
    let app = app_with_report();
    let buf = render(&app, 100, 30);
    let pos = (0..30)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .find(|&(x, y)| buf[(x, y)].symbol() == "C" && buf[(x + 1, y)].symbol() == "A")
        .expect("CAUTION badge");
    assert_eq!(buf[pos].bg, ratatui::style::Color::Yellow);
}

#[test]
fn every_tab_and_modal_renders() {
    let mut app = app_with_report();
    for c in ['2', '3', '4', '5'] {
        press(&mut app, KeyCode::Char(c));
        screen(&app, 80, 24);
    }
    press(&mut app, KeyCode::Char('3'));
    assert!(screen(&app, 80, 24).contains("40% remaining"));
    press(&mut app, KeyCode::Char('2'));
    assert!(screen(&app, 80, 24).contains("Reallocated Sector Ct"));

    press(&mut app, KeyCode::Char('?'));
    assert!(screen(&app, 80, 24).contains("export PDF report"));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('l'));
    assert!(screen(&app, 80, 24).contains("extended self-test on /dev/disk4"));
    press(&mut app, KeyCode::Char('n'));
    press(&mut app, KeyCode::Char('e'));
    assert!(app.modal.is_none(), "e is unbound");

    press(&mut app, KeyCode::Down);
    assert!(screen(&app, 80, 24).contains("unsupported USB bridge"));
}

#[test]
fn json_tab_scrolls() {
    let mut app = app_with_report();
    press(&mut app, KeyCode::Char('5'));
    let first = screen(&app, 80, 24);
    assert!(app.max_scroll.get() > 0);
    press(&mut app, KeyCode::PageDown);
    assert_ne!(first, screen(&app, 80, 24));
    press(&mut app, KeyCode::End);
    let end = screen(&app, 80, 24);
    assert!(end.contains('}'));
}

#[test]
fn tiny_terminal_shows_notice() {
    let app = app_with_report();
    assert!(screen(&app, 50, 12).contains("Terminal too small"));
}

#[test]
fn empty_state() {
    let mut app = App::new(&TuiOptions::default());
    app.start();
    app.on_msg(Msg::Scanned(Ok(vec![])), Instant::now());
    let s = screen(&app, 80, 24);
    assert!(s.contains("No drives found"));
}

/// Loads every fixture through the demo path and renders all tabs.
#[test]
fn fixtures_render_on_every_tab() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "json")
                && !p.file_name().unwrap().to_string_lossy().starts_with("scan")
        })
        .collect();
    files.sort();
    assert!(!files.is_empty());

    let mut app = App::new(&TuiOptions::default());
    app.start();
    let devices = files
        .iter()
        .map(|p| Device::new(DeviceSource::DemoFile(p.clone()), ""))
        .collect();
    let jobs = app.on_msg(Msg::Scanned(Ok(devices)), Instant::now());
    for job in jobs {
        let msg = worker::run(job);
        assert!(matches!(&msg, Msg::Loaded { result: Ok(_), .. }), "{msg:?}");
        app.on_msg(msg, Instant::now());
    }
    for i in 0..files.len() {
        app.selected = i;
        for tab in ['1', '2', '3', '4', '5'] {
            press(&mut app, KeyCode::Char(tab));
            let s = screen(&app, 80, 24);
            assert!(s.contains("Drives"));
        }
        screen(&app, 100, 30);
        press(&mut app, KeyCode::Char('1'));
        screen(&app, 100, 30);
    }
}
