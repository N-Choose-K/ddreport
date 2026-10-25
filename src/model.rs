//! Backend-neutral drive report. Every data source (smartctl JSON, native USB
//! passthrough) produces a `DriveReport`; the PDF and TUI only read this type.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DriveReport {
    pub meta: ReportMeta,
    pub identity: Identity,
    pub bridge: Option<UsbBridge>,
    pub health: Health,
    pub attributes: Vec<Attribute>,
    pub self_tests: Vec<SelfTest>,
    pub self_test_in_progress: Option<SelfTestProgress>,
    /// The drive reported its current self-test state, so a missing
    /// `self_test_in_progress` means idle (false: unknown, e.g. a partial read).
    #[serde(default)]
    pub self_test_status_known: bool,
    /// Device statistics (ATA GP log 0x04) and other "label: value" facts.
    pub statistics: Vec<Stat>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReportMeta {
    /// RFC 3339 timestamp.
    pub generated_at: String,
    pub host: String,
    pub os: String,
    pub tool_version: String,
    /// e.g. "smartctl 7.5" or "ddreport native USB (SAT)".
    pub backend: String,
    pub device_path: String,
    /// smartctl `-d` type actually used, e.g. "sat", "nvme", "sntjmicron".
    pub device_type: String,
    /// SHA-256 of the raw backend output, hex.
    pub raw_sha256: String,
    /// Non-fatal problems hit while collecting data.
    pub warnings: Vec<String>,
    /// Essential data that was missing or corrupt (e.g. SMART checksum error).
    /// A report with any of these cannot grade PASS.
    pub collection_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum Protocol {
    Ata,
    Nvme,
    Scsi,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum MediaKind {
    Hdd,
    Ssd,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Identity {
    pub manufacturer: Option<String>,
    pub model_family: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    /// "5 0014ee 2b1234567" style NAA/WWN or NVMe EUI-64.
    pub wwn: Option<String>,
    pub capacity_bytes: Option<u64>,
    pub logical_sector_size: Option<u32>,
    pub physical_sector_size: Option<u32>,
    pub protocol: Protocol,
    pub media: MediaKind,
    pub rotation_rpm: Option<u32>,
    pub form_factor: Option<String>,
    pub ata_version: Option<String>,
    pub sata_version: Option<String>,
    pub nvme_version: Option<String>,
    pub link_speed_max: Option<String>,
    pub link_speed_current: Option<String>,
    pub trim_supported: Option<bool>,
    pub in_drive_database: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsbBridge {
    pub vendor_id: u16,
    pub product_id: u16,
    pub vendor_name: Option<String>,
    pub product_name: Option<String>,
    pub serial: Option<String>,
    /// Known bridge chipset, e.g. "JMicron JMS578".
    pub chipset: Option<String>,
    /// "BOT" or "UAS".
    pub transport: Option<String>,
    pub link_speed: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Health {
    pub smart_supported: Option<bool>,
    pub smart_enabled: Option<bool>,
    /// Drive's own overall assessment (SMART RETURN STATUS / NVMe critical warning).
    pub overall_passed: Option<bool>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub start_stop_count: Option<u64>,
    pub load_cycles: Option<u64>,
    pub temperature_c: Option<i64>,
    pub temperature_max_c: Option<i64>,
    pub reallocated_sectors: Option<u64>,
    pub reallocation_events: Option<u64>,
    pub pending_sectors: Option<u64>,
    pub offline_uncorrectable: Option<u64>,
    pub reported_uncorrectable: Option<u64>,
    pub udma_crc_errors: Option<u64>,
    pub command_timeouts: Option<u64>,
    pub bytes_read: Option<u64>,
    pub bytes_written: Option<u64>,
    /// SSD wear: percentage of rated endurance used (0-100+).
    pub endurance_used_pct: Option<u64>,
    pub available_spare_pct: Option<u64>,
    pub available_spare_threshold_pct: Option<u64>,
    pub nvme_critical_warning: Option<u64>,
    pub nvme_media_errors: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    pub error_log_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Attribute {
    pub id: u8,
    pub name: String,
    pub value: u8,
    pub worst: u8,
    pub thresh: u8,
    pub raw: u64,
    pub raw_string: String,
    pub prefailure: bool,
    /// "", "now", "past" (smartctl semantics).
    pub when_failed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SelfTest {
    pub kind: String,
    pub status: String,
    pub passed: Option<bool>,
    pub lifetime_hours: Option<u64>,
    pub lba_first_error: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SelfTestProgress {
    pub kind: Option<String>,
    pub remaining_pct: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stat {
    pub group: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grade {
    Pass,
    Caution,
    Fail,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Good,
    Info,
    Caution,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Verdict {
    pub grade: Grade,
    pub summary: String,
    pub findings: Vec<Finding>,
}
