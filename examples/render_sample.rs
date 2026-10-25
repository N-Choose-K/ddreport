//! Renders sample certificates to target/samples/ for visual review.
//!
//! cargo run --example render_sample

use std::time::Instant;

use ddreport::model::*;
use ddreport::pdf;

fn main() -> anyhow::Result<()> {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/samples");
    std::fs::create_dir_all(&out)?;
    for (name, report) in [
        ("wd-red-usb", wd_red()),
        ("seagate-failing", seagate_failing()),
        ("nvme-ssd", nvme()),
        ("stress", stress()),
    ] {
        let t = Instant::now();
        let bytes = pdf::render(&report)?;
        let path = out.join(format!("sample-{name}.pdf"));
        std::fs::write(&path, &bytes)?;
        println!(
            "{} ({} KB, {} ms)",
            path.display(),
            bytes.len() / 1024,
            t.elapsed().as_millis()
        );
    }
    Ok(())
}

fn meta(backend: &str, dev: &str, ty: &str, sha: &str) -> ReportMeta {
    ReportMeta {
        generated_at: "2026-10-25T14:32:08-07:00".into(),
        host: "workbench".into(),
        os: "macOS 26.1 (arm64)".into(),
        tool_version: "0.1.0".into(),
        backend: backend.into(),
        device_path: dev.into(),
        device_type: ty.into(),
        raw_sha256: sha.into(),
        warnings: vec![],
        collection_errors: vec![],
    }
}

#[allow(clippy::too_many_arguments)]
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

fn test(kind: &str, status: &str, passed: bool, hours: u64, lba: Option<u64>) -> SelfTest {
    SelfTest {
        kind: kind.into(),
        status: status.into(),
        passed: Some(passed),
        lifetime_hours: Some(hours),
        lba_first_error: lba,
    }
}

fn stat(group: &str, name: &str, value: &str) -> Stat {
    Stat {
        group: group.into(),
        name: name.into(),
        value: value.into(),
    }
}

fn finding(severity: Severity, message: &str) -> Finding {
    Finding {
        severity,
        message: message.into(),
    }
}

fn wd_red() -> DriveReport {
    let poh = 21_384;
    let mut temp = attr(194, "Temperature_Celsius", 116, 101, 0, 34, false);
    temp.raw_string = "34 (Min/Max 18/49)".into();
    DriveReport {
        meta: meta(
            "smartctl 7.5",
            "/dev/disk4",
            "sat",
            "7f3a91c2e4b05d6187a2c9f0b3e1d4a6c8f2e0b7a5d3c1e9f7b5a3d1c9e7f5a3",
        ),
        identity: Identity {
            manufacturer: Some("Western Digital".into()),
            model_family: Some("Western Digital Red (CMR)".into()),
            model: Some("WDC WD40EFRX-68N32N0".into()),
            serial: Some("WD-WCC7K4XJ2N1R".into()),
            firmware: Some("82.00A82".into()),
            wwn: Some("5 0014ee 2b9c4f1a7".into()),
            capacity_bytes: Some(4_000_787_030_016),
            logical_sector_size: Some(512),
            physical_sector_size: Some(4096),
            protocol: Protocol::Ata,
            media: MediaKind::Hdd,
            rotation_rpm: Some(5400),
            form_factor: Some("3.5 inches".into()),
            ata_version: Some("ACS-3 T13/2161-D revision 5".into()),
            sata_version: Some("SATA 3.1".into()),
            nvme_version: None,
            link_speed_max: Some("6.0 Gb/s".into()),
            link_speed_current: Some("6.0 Gb/s".into()),
            trim_supported: Some(false),
            in_drive_database: Some(true),
        },
        bridge: Some(UsbBridge {
            vendor_id: 0x152d,
            product_id: 0x0578,
            vendor_name: Some("JMicron".into()),
            product_name: Some("USB to ATA/ATAPI Bridge".into()),
            serial: Some("0123456789ABCDEF".into()),
            chipset: Some("JMicron JMS578".into()),
            transport: Some("UAS".into()),
            link_speed: Some("USB 3.2 Gen 1 (5 Gb/s)".into()),
        }),
        health: Health {
            smart_supported: Some(true),
            smart_enabled: Some(true),
            overall_passed: Some(true),
            power_on_hours: Some(poh),
            power_cycles: Some(118),
            start_stop_count: Some(131),
            load_cycles: Some(1_204),
            temperature_c: Some(34),
            temperature_max_c: Some(49),
            reallocated_sectors: Some(0),
            reallocation_events: Some(0),
            pending_sectors: Some(0),
            offline_uncorrectable: Some(0),
            udma_crc_errors: Some(0),
            bytes_read: Some(48_213_884_190_720),
            bytes_written: Some(31_906_417_065_984),
            error_log_count: Some(0),
            ..Default::default()
        },
        attributes: vec![
            attr(1, "Raw_Read_Error_Rate", 200, 200, 51, 0, true),
            attr(3, "Spin_Up_Time", 177, 172, 21, 8133, true),
            attr(4, "Start_Stop_Count", 100, 100, 0, 131, false),
            attr(5, "Reallocated_Sector_Ct", 200, 200, 140, 0, true),
            attr(7, "Seek_Error_Rate", 200, 200, 0, 0, false),
            attr(9, "Power_On_Hours", 71, 71, 0, poh, false),
            attr(10, "Spin_Retry_Count", 100, 253, 0, 0, false),
            attr(11, "Calibration_Retry_Count", 100, 253, 0, 0, false),
            attr(12, "Power_Cycle_Count", 100, 100, 0, 118, false),
            attr(192, "Power-Off_Retract_Count", 200, 200, 0, 61, false),
            attr(193, "Load_Cycle_Count", 200, 200, 0, 1_204, false),
            temp,
            attr(196, "Reallocated_Event_Count", 200, 200, 0, 0, false),
            attr(197, "Current_Pending_Sector", 200, 200, 0, 0, false),
            attr(198, "Offline_Uncorrectable", 100, 253, 0, 0, false),
            attr(199, "UDMA_CRC_Error_Count", 200, 200, 0, 0, false),
            attr(200, "Multi_Zone_Error_Rate", 200, 200, 0, 0, false),
        ],
        self_tests: vec![
            test(
                "Extended offline",
                "Completed without error",
                true,
                21_371,
                None,
            ),
            test(
                "Short offline",
                "Completed without error",
                true,
                21_362,
                None,
            ),
            test(
                "Short offline",
                "Completed without error",
                true,
                20_618,
                None,
            ),
            test(
                "Extended offline",
                "Completed without error",
                true,
                17_045,
                None,
            ),
        ],
        self_test_in_progress: None,
        self_test_status_known: true,
        statistics: vec![
            stat("General", "Lifetime Power-On Resets", "118"),
            stat("General", "Head Flying Hours", "21,107"),
            stat("General", "Logical Sectors Written", "62,317,220,832"),
            stat("General", "Logical Sectors Read", "94,167,742,560"),
            stat("Rotating Media", "Spindle Motor Power-on Hours", "21,380"),
            stat("Rotating Media", "Head Load Events", "1,204"),
            stat(
                "Rotating Media",
                "Number of Reallocated Logical Sectors",
                "0",
            ),
            stat("Temperature", "Current Temperature", "34 °C"),
            stat("Temperature", "Highest Temperature", "49 °C"),
            stat("Transport", "Number of Hardware Resets", "142"),
            stat("Transport", "Number of Interface CRC Errors", "0"),
        ],
        verdict: Verdict {
            grade: Grade::Pass,
            summary: "No signs of media or mechanical wear; all self-tests passed.".into(),
            findings: vec![
                finding(Severity::Good, "Drive reports overall SMART health PASSED"),
                finding(
                    Severity::Good,
                    "0 reallocated, pending or uncorrectable sectors",
                ),
                finding(
                    Severity::Good,
                    "Latest extended self-test completed without error",
                ),
                finding(
                    Severity::Good,
                    "No interface CRC errors (cable and bridge OK)",
                ),
                finding(
                    Severity::Info,
                    "21,384 power-on hours (2.4 years) - typical NAS duty",
                ),
                finding(Severity::Good, "Peak temperature 49 °C, within spec"),
            ],
        },
    }
}

fn seagate_failing() -> DriveReport {
    let poh = 38_906;
    let mut attrs = vec![
        attr(1, "Raw_Read_Error_Rate", 72, 63, 44, 17_461_392, true),
        attr(3, "Spin_Up_Time", 91, 90, 0, 0, true),
        attr(4, "Start_Stop_Count", 99, 99, 20, 1_417, false),
        attr(5, "Reallocated_Sector_Ct", 85, 85, 10, 3_976, true),
        attr(7, "Seek_Error_Rate", 82, 60, 45, 172_384_118, true),
        attr(9, "Power_On_Hours", 56, 56, 0, poh, false),
        attr(10, "Spin_Retry_Count", 100, 100, 97, 0, true),
        attr(12, "Power_Cycle_Count", 99, 99, 20, 1_398, false),
        attr(183, "Runtime_Bad_Block", 100, 100, 0, 0, false),
        attr(184, "End-to-End_Error", 100, 100, 99, 0, true),
        attr(187, "Reported_Uncorrect", 1, 1, 0, 412, false),
        attr(188, "Command_Timeout", 100, 97, 0, 12_885_098_501, false),
        attr(189, "High_Fly_Writes", 98, 98, 0, 2, false),
        attr(190, "Airflow_Temperature_Cel", 58, 41, 40, 42, false),
        attr(191, "G-Sense_Error_Rate", 100, 100, 0, 19, false),
        attr(192, "Power-Off_Retract_Count", 100, 100, 0, 611, false),
        attr(193, "Load_Cycle_Count", 63, 63, 0, 74_208, false),
        attr(194, "Temperature_Celsius", 42, 59, 0, 42, false),
        attr(195, "Hardware_ECC_Recovered", 72, 64, 0, 17_461_392, false),
        attr(197, "Current_Pending_Sector", 96, 96, 0, 328, false),
        attr(198, "Offline_Uncorrectable", 96, 96, 0, 328, false),
        attr(199, "UDMA_CRC_Error_Count", 200, 200, 0, 0, false),
        attr(240, "Head_Flying_Hours", 100, 253, 0, 37_102, false),
        attr(
            241,
            "Total_LBAs_Written",
            100,
            253,
            0,
            58_224_391_402,
            false,
        ),
        attr(242, "Total_LBAs_Read", 100, 253, 0, 141_908_224_811, false),
    ];
    attrs[4].when_failed = "past".into();
    DriveReport {
        meta: meta(
            "smartctl 7.5",
            "/dev/sdb",
            "sat",
            "c41e0b77d2a9f3e8b1c6a4d7e0f9b2c5a8d1e4f7b0c3a6d9e2f5b8c1a4d7e0f3",
        ),
        identity: Identity {
            manufacturer: Some("Seagate".into()),
            model_family: Some("Seagate Barracuda 7200.14 (AF)".into()),
            model: Some("ST3000DM001-1CH166".into()),
            serial: Some("Z1F4K9QX".into()),
            firmware: Some("CC27".into()),
            wwn: Some("5 000c50 05c8f2e1d".into()),
            capacity_bytes: Some(3_000_592_982_016),
            logical_sector_size: Some(512),
            physical_sector_size: Some(4096),
            protocol: Protocol::Ata,
            media: MediaKind::Hdd,
            rotation_rpm: Some(7200),
            form_factor: Some("3.5 inches".into()),
            ata_version: Some("ACS-2, ACS-3 T13/2161-D revision 3b".into()),
            sata_version: Some("SATA 3.1".into()),
            link_speed_max: Some("6.0 Gb/s".into()),
            link_speed_current: Some("3.0 Gb/s".into()),
            in_drive_database: Some(true),
            ..Default::default()
        },
        bridge: None,
        health: Health {
            smart_supported: Some(true),
            smart_enabled: Some(true),
            overall_passed: Some(true),
            power_on_hours: Some(poh),
            power_cycles: Some(1_398),
            start_stop_count: Some(1_417),
            load_cycles: Some(74_208),
            temperature_c: Some(42),
            temperature_max_c: Some(59),
            reallocated_sectors: Some(3_976),
            pending_sectors: Some(328),
            offline_uncorrectable: Some(328),
            reported_uncorrectable: Some(412),
            udma_crc_errors: Some(0),
            command_timeouts: Some(5),
            bytes_read: Some(141_908_224_811 * 512),
            bytes_written: Some(58_224_391_402 * 512),
            ..Default::default()
        },
        attributes: attrs,
        self_tests: vec![
            test(
                "Extended offline",
                "Completed: read failure (90% remaining)",
                false,
                38_901,
                Some(1_841_220_416),
            ),
            test(
                "Short offline",
                "Completed: read failure (10% remaining)",
                false,
                38_899,
                Some(1_841_220_416),
            ),
            test(
                "Short offline",
                "Completed without error",
                true,
                36_114,
                None,
            ),
            test(
                "Extended offline",
                "Completed without error",
                true,
                30_022,
                None,
            ),
            test(
                "Short offline",
                "Completed without error",
                true,
                29_998,
                None,
            ),
            test(
                "Short offline",
                "Completed without error",
                true,
                25_410,
                None,
            ),
            test(
                "Short offline",
                "Completed without error",
                true,
                21_002,
                None,
            ),
            test(
                "Short offline",
                "Completed without error",
                true,
                18_552,
                None,
            ),
        ],
        self_test_in_progress: None,
        self_test_status_known: true,
        statistics: vec![],
        verdict: Verdict {
            grade: Grade::Fail,
            summary:
                "Active media failure: pending and reallocated sectors with failed self-tests."
                    .into(),
            findings: vec![
                finding(
                    Severity::Critical,
                    "328 sectors pending reallocation (unreadable data)",
                ),
                finding(Severity::Critical, "3,976 sectors already reallocated"),
                finding(
                    Severity::Critical,
                    "Latest extended self-test failed: read failure at LBA 1841220416",
                ),
                finding(
                    Severity::Critical,
                    "412 uncorrectable errors reported to the host",
                ),
                finding(
                    Severity::Caution,
                    "Seek error rate fell below threshold in the past",
                ),
                finding(
                    Severity::Caution,
                    "Link negotiated at 3.0 Gb/s (max 6.0 Gb/s)",
                ),
                finding(
                    Severity::Info,
                    "Drive's own SMART status still reads PASSED",
                ),
                finding(Severity::Info, "38,906 power-on hours (4.4 years)"),
            ],
        },
    }
}

fn nvme() -> DriveReport {
    DriveReport {
        meta: ReportMeta {
            warnings: vec!["Read 1 entries from Error Information Log failed".into()],
            ..meta(
                "smartctl 7.5",
                "/dev/nvme0",
                "nvme",
                "09b45e2d8c1f7a3e6b0d4c9f2a5e8b1d7c3f6a0e9b2d5c8f1a4e7b0d3c6f9a2e",
            )
        },
        identity: Identity {
            manufacturer: Some("Samsung".into()),
            model_family: None,
            model: Some("Samsung SSD 980 PRO 2TB".into()),
            serial: Some("S6B0NL0W712345K".into()),
            firmware: Some("5B2QGXA7".into()),
            wwn: Some("002538 b911a2c3d4".into()),
            capacity_bytes: Some(2_000_398_934_016),
            logical_sector_size: Some(512),
            physical_sector_size: None,
            protocol: Protocol::Nvme,
            media: MediaKind::Ssd,
            form_factor: Some("M.2 2280".into()),
            nvme_version: Some("1.3".into()),
            link_speed_max: Some("PCIe 4.0 x4".into()),
            link_speed_current: Some("PCIe 4.0 x4".into()),
            trim_supported: Some(true),
            ..Default::default()
        },
        bridge: None,
        health: Health {
            smart_supported: Some(true),
            smart_enabled: Some(true),
            overall_passed: Some(true),
            power_on_hours: Some(6_412),
            power_cycles: Some(1_874),
            temperature_c: Some(41),
            temperature_max_c: Some(78),
            bytes_read: Some(91_337_000_000_000),
            bytes_written: Some(68_214_000_000_000),
            endurance_used_pct: Some(5),
            available_spare_pct: Some(100),
            available_spare_threshold_pct: Some(10),
            nvme_critical_warning: Some(0),
            nvme_media_errors: Some(0),
            unsafe_shutdowns: Some(57),
            error_log_count: Some(2_311),
            ..Default::default()
        },
        attributes: vec![],
        self_tests: vec![
            test("Short", "Completed without error", true, 6_410, None),
            test("Extended", "Completed without error", true, 6_211, None),
        ],
        self_test_in_progress: None,
        self_test_status_known: true,
        statistics: vec![
            stat("NVMe", "Controller Busy Time", "1,203 min"),
            stat("NVMe", "Host Read Commands", "1,402,331,876"),
            stat("NVMe", "Host Write Commands", "1,987,220,413"),
            stat("NVMe", "Warning Temp Time", "0 min"),
            stat("NVMe", "Critical Temp Time", "0 min"),
            stat("NVMe", "Temperature Sensor 1", "41 °C"),
            stat("NVMe", "Temperature Sensor 2", "46 °C"),
        ],
        verdict: Verdict {
            grade: Grade::Pass,
            summary: "Lightly used NVMe SSD with 95% of rated endurance remaining.".into(),
            findings: vec![
                finding(
                    Severity::Good,
                    "No critical warnings; 0 media and data integrity errors",
                ),
                finding(Severity::Good, "5% of rated endurance used, spare at 100%"),
                finding(Severity::Info, "68.2 TB written over 6,412 power-on hours"),
                finding(
                    Severity::Info,
                    "2,311 error log entries (benign host aborts are common)",
                ),
                finding(
                    Severity::Caution,
                    "Peak temperature 78 °C reached at some point",
                ),
            ],
        },
    }
}

/// Worst case: oversized strings and lists, to check truncation and fit.
fn stress() -> DriveReport {
    let mut r = seagate_failing();
    let long = "Extremely Long Vendor Supplied String That Keeps Going On";
    r.meta.warnings = vec![long.repeat(2); 5];
    r.identity.model = Some(long.repeat(2));
    r.identity.model_family = Some(long.into());
    r.identity.serial = Some("SN-0123456789-ABCDEFGHIJKLMNOPQRSTUV".into());
    r.bridge = wd_red().bridge;
    for i in 0..20u8 {
        r.attributes.push(attr(
            200 + i,
            "Vendor_Specific_Attribute_Long_Name",
            1,
            1,
            10,
            u64::MAX,
            true,
        ));
    }
    r.self_tests = r.self_tests.iter().cycle().take(21).cloned().collect();
    r.statistics = (0..60)
        .map(|i| {
            stat(
                "General",
                &format!("Statistic number {i} with a long name"),
                "123,456,789",
            )
        })
        .collect();
    r.verdict.findings = r
        .verdict
        .findings
        .iter()
        .cycle()
        .take(30)
        .cloned()
        .collect();
    r
}
