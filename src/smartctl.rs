//! smartctl JSON backend (`smartctl -x --json`, smartmontools >= 7.0).

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::Value;

use crate::model::{
    Attribute, DriveReport, Health, Identity, MediaKind, Protocol, SelfTest, SelfTestProgress, Stat,
};
use crate::vendor;

#[derive(Debug, Clone)]
pub struct ScannedDevice {
    pub name: String,
    pub dev_type: String,
    pub protocol: String,
    pub info: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    Short,
    Long,
    Conveyance,
}

impl TestKind {
    pub fn name(self) -> &'static str {
        match self {
            TestKind::Short => "Short",
            TestKind::Long => "Extended",
            TestKind::Conveyance => "Conveyance",
        }
    }

    fn arg(self) -> &'static str {
        match self {
            TestKind::Short => "short",
            TestKind::Long => "long",
            TestKind::Conveyance => "conveyance",
        }
    }
}

/// `-d` types tried in order when auto-detection yields no SMART data.
/// Covers SAT bridges and the NVMe-over-USB bridges smartctl supports.
const PROBE_TYPES: &[&str] = &[
    "sat",
    "sat,12",
    "sntjmicron",
    "sntasmedia",
    "sntrealtek",
    "usbjmicron",
    "usbsunplus",
    "usbcypress",
];

/// Kills smartctl and anything it spawned, then reaps it.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: plain syscall; the child leads its own process group.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// smartctl exit-status bits that mean no usable data (cmdline error, open failed).
const FATAL_EXIT_BITS: i64 = 0b11;

fn smartctl() -> Command {
    Command::new(std::env::var("DDREPORT_SMARTCTL").unwrap_or_else(|_| "smartctl".into()))
}

/// Upper bound for one smartctl run; a stalled USB bridge must not hang probing.
const RUN_TIMEOUT: Duration = Duration::from_secs(90);

fn run_json(args: &[&str]) -> anyhow::Result<(String, Value)> {
    let mut cmd = smartctl();
    cmd.args(args);
    let label = format!("smartctl {}", args.join(" "));
    let raw = run_with_deadline(cmd, &label, RUN_TIMEOUT)?;
    let json: Value =
        serde_json::from_str(&raw).with_context(|| format!("{label} returned invalid JSON"))?;
    Ok((raw, json))
}

/// Runs `cmd` and returns its stdout, killing its process group at `timeout`.
fn run_with_deadline(mut cmd: Command, label: &str, timeout: Duration) -> anyhow::Result<String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .spawn()
        .context("failed to run smartctl - is smartmontools installed and on PATH?")?;
    let mut stdout = child.stdout.take().context("smartctl stdout unavailable")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = tx.send(stdout.read_to_end(&mut buf).map(|_| buf));
    });
    let deadline = Instant::now() + timeout;
    let timed_out = |child: &mut std::process::Child| {
        kill_group(child);
        anyhow::anyhow!("{label} timed out after {}s", timeout.as_secs())
    };
    // Stdout closes when smartctl and any descendants holding it exit.
    let out = match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(r) => r?,
        Err(_) => return Err(timed_out(&mut child)),
    };
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            return Err(timed_out(&mut child));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn exit_status(j: &Value) -> i64 {
    j.pointer("/smartctl/exit_status")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn messages(j: &Value) -> Vec<String> {
    j.pointer("/smartctl/messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("string")?.as_str().map(str::to_string))
        .collect()
}

fn has_smart_data(j: &Value) -> bool {
    j.get("ata_smart_attributes").is_some() || j.get("nvme_smart_health_information_log").is_some()
}

/// `smartctl --scan-open`.
pub fn scan() -> anyhow::Result<Vec<ScannedDevice>> {
    let (_, j) = run_json(&["--scan-open", "--json=c"])?;
    Ok(parse_scan(&j))
}

pub fn parse_scan(j: &Value) -> Vec<ScannedDevice> {
    j.get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|d| ScannedDevice {
            name: str_at(d, "/name").unwrap_or_default(),
            dev_type: str_at(d, "/type").unwrap_or_default(),
            protocol: str_at(d, "/protocol").unwrap_or_default(),
            info: str_at(d, "/info_name").unwrap_or_default(),
        })
        .collect()
}

/// Runs `smartctl -x --json`, probing USB bridge types when `dev_type` is None.
/// Returns (raw JSON text, device type that worked).
pub fn read(device: &str, dev_type: Option<&str>) -> anyhow::Result<(String, String)> {
    let attempt = |t: Option<&str>| {
        // `o` embeds the text output, which carries checksum warnings.
        let mut args = vec!["-x", "--json=co"];
        if let Some(t) = t {
            args.extend(["-d", t]);
        }
        args.push(device);
        run_json(&args)
    };
    let used = |j: &Value, t: Option<&str>| {
        t.map(str::to_string)
            .or_else(|| str_at(j, "/device/type"))
            .unwrap_or_else(|| "auto".into())
    };

    if let Some(t) = dev_type {
        let (raw, j) = attempt(Some(t))?;
        if exit_status(&j) & FATAL_EXIT_BITS != 0 {
            bail!("smartctl -d {t} {device}: {}", messages(&j).join("; "));
        }
        return Ok((raw, used(&j, Some(t))));
    }

    let (raw, j) = attempt(None)?;
    if exit_status(&j) & FATAL_EXIT_BITS == 0 && has_smart_data(&j) {
        let t = used(&j, None);
        return Ok((raw, t));
    }
    let mut errors = vec![format!("auto: {}", messages(&j).join("; "))];
    let mut fallback = (exit_status(&j) & FATAL_EXIT_BITS == 0).then(|| (raw, used(&j, None)));
    for t in PROBE_TYPES {
        match attempt(Some(t)) {
            Ok((raw, j)) if exit_status(&j) & FATAL_EXIT_BITS == 0 && has_smart_data(&j) => {
                return Ok((raw, (*t).to_string()));
            }
            Ok((raw, j)) => {
                errors.push(format!("{t}: {}", messages(&j).join("; ")));
                if fallback.is_none() && exit_status(&j) & FATAL_EXIT_BITS == 0 {
                    fallback = Some((raw, (*t).to_string()));
                }
            }
            Err(e) => errors.push(format!("{t}: {e:#}")),
        }
    }
    fallback.with_context(|| {
        format!(
            "no SMART data from {device}; tried:\n  {}",
            errors.join("\n  ")
        )
    })
}

/// Returns smartctl's messages (e.g. expected completion time).
pub fn start_self_test(
    device: &str,
    dev_type: Option<&str>,
    kind: TestKind,
) -> anyhow::Result<String> {
    let (_, msg) = test_command(
        device,
        dev_type,
        &["-t", kind.arg()],
        "self-test not started",
    )?;
    Ok(started_message(kind, msg))
}

/// Result of a self-test command run together with `-i`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedCommand {
    /// smartctl's messages (e.g. expected completion time).
    pub message: String,
    /// Identity printed by the same smartctl run, from the handle the command used.
    pub model: Option<String>,
    pub serial: Option<String>,
}

/// `smartctl -i -t <kind>`: one device open both identifies the drive and starts the test.
pub fn start_self_test_identified(
    device: &str,
    dev_type: Option<&str>,
    kind: TestKind,
) -> anyhow::Result<IdentifiedCommand> {
    let (j, msg) = test_command(
        device,
        dev_type,
        &["-i", "-t", kind.arg()],
        "self-test not started",
    )?;
    let (model, serial) = identity_of(&j);
    Ok(IdentifiedCommand {
        message: started_message(kind, msg),
        model,
        serial,
    })
}

/// `smartctl -i -X`: one device open both identifies the drive and aborts its test.
pub fn abort_self_test_identified(
    device: &str,
    dev_type: Option<&str>,
) -> anyhow::Result<IdentifiedCommand> {
    let (j, message) = test_command(device, dev_type, &["-i", "-X"], "abort failed")?;
    let (model, serial) = identity_of(&j);
    Ok(IdentifiedCommand {
        message,
        model,
        serial,
    })
}

fn started_message(kind: TestKind, msg: String) -> String {
    if msg.is_empty() {
        format!("{} self-test started", kind.arg())
    } else {
        msg
    }
}

/// Runs a self-test command; returns its JSON and joined messages.
fn test_command(
    device: &str,
    dev_type: Option<&str>,
    cmd: &[&str],
    failure: &str,
) -> anyhow::Result<(Value, String)> {
    let mut args = cmd.to_vec();
    args.push("--json=c");
    if let Some(t) = dev_type {
        args.extend(["-d", t]);
    }
    args.push(device);
    let (_, j) = run_json(&args)?;
    let msg = messages(&j).join("; ");
    if exit_status(&j) & 0b111 != 0 {
        bail!("{failure}: {msg}");
    }
    Ok((j, msg))
}

/// `smartctl -i`: the (model, serial) currently answering at `device`.
pub fn identify(
    device: &str,
    dev_type: Option<&str>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let mut args = vec!["-i", "--json=c"];
    if let Some(t) = dev_type {
        args.extend(["-d", t]);
    }
    args.push(device);
    let (_, j) = run_json(&args)?;
    if exit_status(&j) & FATAL_EXIT_BITS != 0 {
        bail!("cannot identify {device}: {}", messages(&j).join("; "));
    }
    Ok(identity_of(&j))
}

/// (model, serial) as `parse` reports them.
pub fn identity_of(j: &Value) -> (Option<String>, Option<String>) {
    (model_of(j), str_at(j, "/serial_number"))
}

fn model_of(j: &Value) -> Option<String> {
    str_at(j, "/model_name").or_else(|| {
        let v = str_at(j, "/scsi_vendor")?;
        Some(format!(
            "{v} {}",
            str_at(j, "/scsi_product").unwrap_or_default()
        ))
    })
}

pub fn abort_self_test(device: &str, dev_type: Option<&str>) -> anyhow::Result<()> {
    test_command(device, dev_type, &["-X"], "abort failed").map(|_| ())
}

/// Fills identity, health, attributes, self-tests and statistics from smartctl JSON.
pub fn parse(j: &Value, r: &mut DriveReport) {
    r.attributes = parse_attributes(j);
    r.identity = parse_identity(j);
    r.statistics = parse_statistics(j);
    r.health = parse_health(j, &r.identity, &r.attributes, &r.statistics);
    r.self_tests = parse_self_tests(j);
    r.self_test_in_progress = parse_progress(j);
    r.self_test_status_known = self_test_status_known(j);
    r.meta.backend = format!(
        "smartctl {}",
        j.pointer("/smartctl/version")
            .and_then(Value::as_array)
            .map(|v| v
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("."))
            .unwrap_or_else(|| "?".into())
    );
    if let Some(t) = str_at(j, "/device/type") {
        r.meta.device_type = t;
    }
    r.meta.collection_errors = collection_errors(j);
    for &(id, meaning) in GRADED_IDS {
        let Some(a) = r.attributes.iter().find(|a| a.id == id) else {
            continue;
        };
        let graded = match id {
            5 => r.health.reallocated_sectors.is_some(),
            187 => r.health.reported_uncorrectable.is_some(),
            197 => r.health.pending_sectors.is_some(),
            _ => r.health.offline_uncorrectable.is_some(),
        };
        if !graded {
            r.meta.warnings.push(format!(
                "Attribute {id} is {} on this drive (raw {}); not graded as {meaning}",
                a.name, a.raw_string
            ));
        }
    }
    r.meta.warnings.extend(
        j.pointer("/smartctl/messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|m| m.get("severity").and_then(Value::as_str) != Some("information"))
            .filter_map(|m| m.get("string")?.as_str().map(str::to_string)),
    );
}

/// Problems that make the SMART evidence itself unreliable or incomplete.
fn collection_errors(j: &Value) -> Vec<String> {
    let text = j
        .pointer("/smartctl/output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_string));
    let mut out: Vec<String> = messages(j)
        .into_iter()
        .chain(text)
        .filter(|m| m.to_ascii_lowercase().contains("checksum"))
        .map(|m| m.trim().to_string())
        .collect();
    out.dedup();
    let available = bool_at(j, "/smart_support/available") != Some(false);
    let enabled = bool_at(j, "/smart_support/enabled") != Some(false);
    if available && enabled {
        if !has_smart_data(j) {
            out.push("SMART attributes / health log could not be read".into());
        }
        if j.pointer("/smart_status/passed").is_none() {
            out.push("SMART overall-health status could not be read".into());
        }
    }
    out
}

fn str_at(j: &Value, p: &str) -> Option<String> {
    j.pointer(p)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn u64_at(j: &Value, p: &str) -> Option<u64> {
    let v = j.pointer(p)?;
    v.as_u64()
        .or_else(|| v.as_i64().filter(|&n| n >= 0).map(|n| n as u64))
}

fn i64_at(j: &Value, p: &str) -> Option<i64> {
    j.pointer(p)?.as_i64()
}

fn bool_at(j: &Value, p: &str) -> Option<bool> {
    j.pointer(p)?.as_bool()
}

fn parse_identity(j: &Value) -> Identity {
    let protocol = match str_at(j, "/device/protocol").as_deref() {
        Some("ATA") => Protocol::Ata,
        Some("NVMe") => Protocol::Nvme,
        Some("SCSI") => Protocol::Scsi,
        _ => Protocol::Unknown,
    };
    let model = model_of(j);
    let family = str_at(j, "/model_family");
    let rotation = u64_at(j, "/rotation_rate").map(|r| r as u32);
    let media = match (protocol, rotation) {
        (Protocol::Nvme, _) => MediaKind::Ssd,
        (_, Some(0)) => MediaKind::Ssd,
        (_, Some(_)) => MediaKind::Hdd,
        _ => MediaKind::Unknown,
    };

    let wwn = match (
        u64_at(j, "/wwn/naa"),
        u64_at(j, "/wwn/oui"),
        u64_at(j, "/wwn/id"),
    ) {
        (Some(naa), Some(oui), Some(id)) => Some(format!("{naa:x} {oui:06x} {id:09x}")),
        _ => j.pointer("/nvme_namespaces/0/eui64").and_then(|e| {
            Some(format!(
                "{:06x} {:010x}",
                e.get("oui")?.as_u64()?,
                e.get("ext_id")?.as_u64()?
            ))
        }),
    };

    let manufacturer = model
        .as_deref()
        .and_then(|m| vendor::from_model(m, family.as_deref()))
        .or_else(|| u64_at(j, "/wwn/oui").and_then(vendor::from_oui))
        .or_else(|| u64_at(j, "/nvme_pci_vendor/id").and_then(vendor::from_pci_vendor))
        .map(str::to_string)
        .or_else(|| str_at(j, "/scsi_vendor"));

    Identity {
        manufacturer,
        model_family: family,
        model,
        serial: str_at(j, "/serial_number"),
        firmware: str_at(j, "/firmware_version").or_else(|| str_at(j, "/scsi_revision")),
        wwn,
        capacity_bytes: u64_at(j, "/user_capacity/bytes")
            .or_else(|| u64_at(j, "/nvme_total_capacity"))
            .filter(|&b| b > 0),
        logical_sector_size: u64_at(j, "/logical_block_size")
            .or_else(|| u64_at(j, "/nvme_namespaces/0/formatted_lba_size"))
            .map(|v| v as u32),
        physical_sector_size: u64_at(j, "/physical_block_size").map(|v| v as u32),
        protocol,
        media,
        rotation_rpm: rotation.filter(|&r| r > 0),
        form_factor: str_at(j, "/form_factor/name"),
        ata_version: str_at(j, "/ata_version/string"),
        sata_version: str_at(j, "/sata_version/string"),
        nvme_version: str_at(j, "/nvme_version/string"),
        link_speed_max: str_at(j, "/interface_speed/max/string"),
        link_speed_current: str_at(j, "/interface_speed/current/string"),
        trim_supported: bool_at(j, "/trim/supported"),
        in_drive_database: bool_at(j, "/in_smartctl_database"),
    }
}

fn parse_attributes(j: &Value) -> Vec<Attribute> {
    j.pointer("/ata_smart_attributes/table")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Attribute {
                id: u64_at(a, "/id")? as u8,
                name: str_at(a, "/name").unwrap_or_else(|| "Unknown_Attribute".into()),
                value: u64_at(a, "/value").unwrap_or(0) as u8,
                worst: u64_at(a, "/worst").unwrap_or(0) as u8,
                thresh: u64_at(a, "/thresh").unwrap_or(0) as u8,
                raw: u64_at(a, "/raw/value").unwrap_or(0),
                raw_string: str_at(a, "/raw/string").unwrap_or_default(),
                prefailure: bool_at(a, "/flags/prefailure").unwrap_or(false),
                when_failed: str_at(a, "/when_failed").unwrap_or_default(),
            })
        })
        .collect()
}

/// Leading integer of the raw string ("31 (Min/Max 20/45)" -> 31), which
/// avoids vendor-packed high bits in `raw.value`.
fn attr_count(attrs: &[Attribute], id: u8) -> Option<u64> {
    let a = attrs.iter().find(|a| a.id == id)?;
    let lead: String = a
        .raw_string
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    lead.parse().ok().or(Some(a.raw))
}

/// Normalized-value attributes meaning "life remaining, 100 = new", as named
/// by smartctl's drive database. Attribute IDs alone are vendor-specific.
const LIFE_REMAINING: &[&str] = &[
    "Wear_Leveling_Count",
    "SSD_Life_Left",
    "Media_Wearout_Indicator",
    "Percent_Lifetime_Remain",
    "Remaining_Lifetime_Perc",
    "Percent_Life_Remaining",
    "Lifetime_Remaining",
];

/// Attribute IDs whose usual meaning is graded; a different vendor name is
/// reported as ungraded so a reader can judge it from the attribute table.
const GRADED_IDS: &[(u8, &str)] = &[
    (5, "reallocated sectors"),
    (187, "reported uncorrectable errors"),
    (197, "pending sectors"),
    (198, "offline uncorrectable sectors"),
];

fn has_prefix(name: &str, prefixes: &[&str]) -> bool {
    let n = name.to_ascii_lowercase();
    prefixes
        .iter()
        .any(|p| n.starts_with(&p.to_ascii_lowercase()))
}

/// Count from attribute `id` only when its smartctl name carries the expected
/// meaning; Crucial MX500 uses 197 for `Current_Pending_ECC_Cnt`.
fn named(attrs: &[Attribute], id: u8, names: &[&str]) -> Option<u64> {
    attrs
        .iter()
        .find(|a| a.id == id && has_prefix(&a.name, names))
        .and(attr_count(attrs, id))
}

fn stat_value(stats: &[Stat], name: &str) -> Option<u64> {
    stats
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))?
        .value
        .parse()
        .ok()
}

/// Bytes from a lifetime read/write attribute, using the unit in its name.
fn lifetime_bytes(attrs: &[Attribute], id: u8, sector: u64) -> Option<u64> {
    let a = attrs.iter().find(|a| a.id == id)?;
    let n = attr_count(attrs, id)?;
    let name = a.name.to_ascii_lowercase();
    let unit = if name.contains("32mib") {
        32 << 20
    } else if name.contains("gib") {
        1 << 30
    } else if name.contains("_gb") {
        1_000_000_000
    } else if name.contains("lba") || name.contains("sector") {
        sector
    } else {
        return None;
    };
    n.checked_mul(unit)
}

fn parse_health(j: &Value, id: &Identity, attrs: &[Attribute], stats: &[Stat]) -> Health {
    let nv = |k: &str| u64_at(j, &format!("/nvme_smart_health_information_log/{k}"));
    let sector = id.logical_sector_size.unwrap_or(512) as u64;

    let endurance_used_pct = nv("percentage_used")
        .or_else(|| u64_at(j, "/endurance_used/current_percent"))
        .or_else(|| stat_value(stats, "Percentage Used Endurance Indicator"))
        .or_else(|| {
            if id.media != MediaKind::Ssd {
                return None;
            }
            attrs
                .iter()
                .find(|a| has_prefix(&a.name, LIFE_REMAINING))
                .map(|a| 100u64.saturating_sub(a.value as u64))
        });

    let data_unit = |k: &str| nv(k).and_then(|n| n.checked_mul(512_000));

    Health {
        smart_supported: bool_at(j, "/smart_support/available"),
        smart_enabled: bool_at(j, "/smart_support/enabled"),
        overall_passed: bool_at(j, "/smart_status/passed"),
        power_on_hours: u64_at(j, "/power_on_time/hours")
            .or_else(|| nv("power_on_hours"))
            .or_else(|| named(attrs, 9, &["Power_On_Hours"])),
        power_cycles: u64_at(j, "/power_cycle_count")
            .or_else(|| nv("power_cycles"))
            .or_else(|| named(attrs, 12, &["Power_Cycle_Count"])),
        start_stop_count: named(attrs, 4, &["Start_Stop_Count"]),
        load_cycles: named(attrs, 193, &["Load_Cycle_Count"])
            .or_else(|| named(attrs, 225, &["Load_Cycle_Count"])),
        temperature_c: i64_at(j, "/temperature/current"),
        temperature_max_c: i64_at(j, "/temperature/lifetime_max")
            .or_else(|| stat_value(stats, "Highest Temperature").map(|v| v as i64)),
        reallocated_sectors: named(
            attrs,
            5,
            &[
                "Reallocated_Sector",
                "Reallocated_Block_Count",
                "Reallocate_NAND_Blk",
                "Reallocated_NAND_Blk",
                "Retired_Block_Count",
            ],
        ),
        reallocation_events: named(attrs, 196, &["Reallocated_Event_Count"]),
        pending_sectors: named(
            attrs,
            197,
            &["Current_Pending_Sector", "Total_Pending_Sectors"],
        ),
        offline_uncorrectable: named(
            attrs,
            198,
            &[
                "Offline_Uncorrectable",
                "Total_Offl_Uncorrectabl",
                "Uncor_Read_Error_Ct",
            ],
        ),
        reported_uncorrectable: named(
            attrs,
            187,
            &[
                "Reported_Uncorrect",
                "Uncorrectable_Error_Cnt",
                "Uncorrectable_ECC_Ct",
                "Reported_UE",
            ],
        ),
        udma_crc_errors: named(
            attrs,
            199,
            &["UDMA_CRC_Error_Count", "CRC_Error_Count", "SATA_CRC_Error"],
        ),
        command_timeouts: named(attrs, 188, &["Command_Timeout"]),
        bytes_read: data_unit("data_units_read")
            .or_else(|| stat_value(stats, "Logical Sectors Read").map(|n| n * sector))
            .or_else(|| lifetime_bytes(attrs, 242, sector)),
        bytes_written: data_unit("data_units_written")
            .or_else(|| stat_value(stats, "Logical Sectors Written").map(|n| n * sector))
            .or_else(|| lifetime_bytes(attrs, 241, sector)),
        endurance_used_pct,
        available_spare_pct: nv("available_spare"),
        available_spare_threshold_pct: nv("available_spare_threshold"),
        nvme_critical_warning: nv("critical_warning"),
        nvme_media_errors: nv("media_errors"),
        unsafe_shutdowns: nv("unsafe_shutdowns"),
        error_log_count: u64_at(j, "/ata_smart_error_log/extended/count")
            .or_else(|| u64_at(j, "/ata_smart_error_log/summary/count"))
            .or_else(|| nv("num_err_log_entries")),
    }
}

fn parse_self_tests(j: &Value) -> Vec<SelfTest> {
    let ata = [
        "/ata_smart_self_test_log/extended/table",
        "/ata_smart_self_test_log/standard/table",
    ]
    .iter()
    .find_map(|p| {
        j.pointer(p)
            .and_then(Value::as_array)
            .filter(|t| !t.is_empty())
    });
    if let Some(table) = ata {
        return table
            .iter()
            .map(|t| SelfTest {
                kind: str_at(t, "/type/string").unwrap_or_default(),
                status: str_at(t, "/status/string").unwrap_or_default(),
                passed: bool_at(t, "/status/passed"),
                lifetime_hours: u64_at(t, "/lifetime_hours"),
                lba_first_error: u64_at(t, "/lba"),
            })
            .collect();
    }
    j.pointer("/nvme_self_test_log/table")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| {
            let result = u64_at(t, "/self_test_result/value");
            SelfTest {
                kind: str_at(t, "/self_test_code/string").unwrap_or_default(),
                status: str_at(t, "/self_test_result/string").unwrap_or_default(),
                // NVMe self-test result: 0 = no error; 5-7 = failed; others = aborted.
                passed: match result {
                    Some(0) => Some(true),
                    Some(5..=7) => Some(false),
                    _ => None,
                },
                lifetime_hours: u64_at(t, "/power_on_hours"),
                lba_first_error: u64_at(t, "/lba"),
            }
        })
        .collect()
}

/// Whether the read included the drive's current self-test state.
fn self_test_status_known(j: &Value) -> bool {
    j.pointer("/ata_smart_data/self_test/status/value")
        .is_some()
        || j.pointer("/nvme_self_test_log/current_self_test_operation/value")
            .is_some()
}

fn parse_progress(j: &Value) -> Option<SelfTestProgress> {
    if let Some(v) = u64_at(j, "/ata_smart_data/self_test/status/value") {
        if v >> 4 == 0xF {
            return Some(SelfTestProgress {
                kind: None,
                remaining_pct: u64_at(j, "/ata_smart_data/self_test/status/remaining_percent")
                    .or(Some((v & 0xF) * 10))
                    .map(|p| p as u8),
            });
        }
        return None;
    }
    let op = u64_at(j, "/nvme_self_test_log/current_self_test_operation/value")?;
    (op != 0).then(|| SelfTestProgress {
        kind: str_at(j, "/nvme_self_test_log/current_self_test_operation/string"),
        remaining_pct: u64_at(
            j,
            "/nvme_self_test_log/current_self_test_completion_percent",
        )
        .map(|done| 100u64.saturating_sub(done) as u8),
    })
}

fn parse_statistics(j: &Value) -> Vec<Stat> {
    let mut out = Vec::new();
    for page in j
        .pointer("/ata_device_statistics/pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let group = str_at(page, "/name").unwrap_or_else(|| "Device Statistics".into());
        for e in page
            .get("table")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if bool_at(e, "/flags/valid") == Some(false) {
                continue;
            }
            let (Some(name), Some(v)) = (str_at(e, "/name"), e.get("value")) else {
                continue;
            };
            out.push(Stat {
                group: group.clone(),
                name,
                value: v.to_string(),
            });
        }
    }
    if let Some(log) = j
        .get("nvme_smart_health_information_log")
        .and_then(Value::as_object)
    {
        const NVME: &[(&str, &str)] = &[
            ("data_units_read", "Data Units Read (512,000 B)"),
            ("data_units_written", "Data Units Written (512,000 B)"),
            ("host_reads", "Host Read Commands"),
            ("host_writes", "Host Write Commands"),
            ("controller_busy_time", "Controller Busy Time (min)"),
            ("unsafe_shutdowns", "Unsafe Shutdowns"),
            ("num_err_log_entries", "Error Log Entries"),
            ("warning_temp_time", "Warning Temp Time (min)"),
            ("critical_comp_time", "Critical Temp Time (min)"),
        ];
        for (k, label) in NVME {
            if let Some(v) = log.get(*k).filter(|v| v.is_number()) {
                out.push(Stat {
                    group: "NVMe Health Log".into(),
                    name: (*label).into(),
                    value: v.to_string(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> (Value, DriveReport) {
        let raw =
            std::fs::read_to_string(format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR")))
                .unwrap();
        let j: Value = serde_json::from_str(&raw).unwrap();
        let mut r = DriveReport::default();
        parse(&j, &mut r);
        (j, r)
    }

    #[test]
    fn identity_of_matches_parse() {
        for f in ["sata_hdd_wd_red_usb.json", "nvme_usb_jmicron.json"] {
            let (j, r) = load(f);
            assert_eq!(identity_of(&j), (r.identity.model, r.identity.serial));
        }
        assert_eq!(identity_of(&serde_json::json!({})), (None, None));
    }

    #[test]
    fn nvme_apple() {
        let (_, r) = load("nvme_apple.json");
        assert_eq!(r.identity.protocol, Protocol::Nvme);
        assert_eq!(r.identity.media, MediaKind::Ssd);
        assert_eq!(r.identity.manufacturer.as_deref(), Some("Apple"));
        assert_eq!(r.health.power_on_hours, Some(4321));
        assert_eq!(r.health.endurance_used_pct, Some(7));
        assert_eq!(r.health.bytes_written, Some(401_234_567 * 512_000));
        assert_eq!(r.health.overall_passed, Some(true));
        assert!(
            r.statistics
                .iter()
                .any(|s| s.name == "Unsafe Shutdowns" && s.value == "17")
        );
        assert!(r.meta.backend.starts_with("smartctl 7.5"));
        assert!(!r.meta.warnings.is_empty());
    }

    #[test]
    fn wd_red_usb() {
        let (_, r) = load("sata_hdd_wd_red_usb.json");
        let id = &r.identity;
        assert_eq!(id.manufacturer.as_deref(), Some("Western Digital"));
        assert_eq!(id.media, MediaKind::Hdd);
        assert_eq!(id.rotation_rpm, Some(5400));
        assert_eq!(id.serial.as_deref(), Some("WD-WCC7K0FAKE01"));
        assert!(id.wwn.as_deref().unwrap().starts_with("5 0014ee "));
        assert_eq!(r.health.power_on_hours, Some(23012));
        assert_eq!(r.health.reallocated_sectors, Some(0));
        assert!(r.health.bytes_written.is_some());
        assert!(r.health.temperature_max_c.is_some());
        assert_eq!(r.self_tests.len(), 2);
        assert!(r.statistics.iter().any(|s| s.group == "General Statistics"));
        assert_eq!(r.meta.device_type, "sat");
    }

    #[test]
    fn seagate_failing() {
        let (_, r) = load("sata_hdd_seagate_failing.json");
        let h = &r.health;
        assert_eq!(h.reallocated_sectors, Some(1248));
        assert_eq!(h.pending_sectors, Some(16));
        assert_eq!(h.offline_uncorrectable, Some(16));
        assert_eq!(h.reported_uncorrectable, Some(45));
        assert_eq!(h.udma_crc_errors, Some(3));
        assert_eq!(h.command_timeouts, Some(0));
        assert_eq!(r.self_tests[0].passed, Some(false));
        assert!(r.self_tests[0].lba_first_error.is_some());
        assert_eq!(h.bytes_written, Some(31_207_614_118 * 512));
    }

    #[test]
    fn hgst_failed() {
        let (_, r) = load("sata_hdd_failed_smart.json");
        assert_eq!(r.health.overall_passed, Some(false));
        assert!(r.attributes.iter().any(|a| a.when_failed == "now"));
        assert_eq!(
            r.identity.manufacturer.as_deref(),
            Some("HGST (Western Digital)")
        );
    }

    #[test]
    fn samsung_ssd() {
        let (_, r) = load("sata_ssd_samsung_870.json");
        assert_eq!(r.identity.media, MediaKind::Ssd);
        assert_eq!(r.identity.rotation_rpm, None);
        assert_eq!(r.identity.trim_supported, Some(true));
        assert!(r.health.endurance_used_pct.is_some_and(|p| p < 10));
        assert_eq!(r.identity.manufacturer.as_deref(), Some("Samsung"));
    }

    #[test]
    fn nvme_jmicron() {
        let (_, r) = load("nvme_usb_jmicron.json");
        assert_eq!(r.identity.protocol, Protocol::Nvme);
        assert_eq!(r.meta.device_type, "sntjmicron");
        assert_eq!(r.self_tests.len(), 1);
        assert_eq!(r.self_tests[0].passed, Some(true));
        assert_eq!(r.self_tests[0].lifetime_hours, Some(2208));
        assert!(r.self_test_in_progress.is_none());
        assert!(r.self_test_status_known);
    }

    #[test]
    fn scan_fixture() {
        let raw = std::fs::read_to_string(format!(
            "{}/fixtures/scan_open.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let d = parse_scan(&serde_json::from_str(&raw).unwrap());
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn attr_count_uses_leading_raw_integer() {
        let attrs = vec![Attribute {
            id: 194,
            raw: 0x2d_0014_001f,
            raw_string: "31 (Min/Max 20/45)".into(),
            ..Default::default()
        }];
        assert_eq!(attr_count(&attrs, 194), Some(31));
    }

    #[test]
    fn lifetime_bytes_units() {
        let a = |name: &str, raw: u64| Attribute {
            id: 241,
            name: name.into(),
            raw,
            raw_string: raw.to_string(),
            ..Default::default()
        };
        assert_eq!(
            lifetime_bytes(&[a("Total_LBAs_Written", 1000)], 241, 512),
            Some(512_000)
        );
        assert_eq!(
            lifetime_bytes(&[a("Host_Writes_32MiB", 2)], 241, 512),
            Some(64 << 20)
        );
        assert_eq!(
            lifetime_bytes(&[a("Lifetime_Writes_GiB", 3)], 241, 512),
            Some(3 << 30)
        );
        assert_eq!(
            lifetime_bytes(&[a("Total_Writes_GB", 3)], 241, 512),
            Some(3_000_000_000)
        );
        assert_eq!(lifetime_bytes(&[a("Unknown_Attribute", 3)], 241, 512), None);
    }

    #[test]
    fn checksum_error_is_collection_error() {
        let j = serde_json::json!({
            "smartctl": {"exit_status": 4, "messages": [
                {"string": "Warning! SMART Attribute Data Structure error: invalid SMART checksum.", "severity": "warning"},
                {"string": "Read SCT Status failed", "severity": "error"}
            ]},
            "device": {"protocol": "ATA"},
            "smart_support": {"available": true, "enabled": true},
            "smart_status": {"passed": true},
            "ata_smart_attributes": {"table": []}
        });
        let errs = collection_errors(&j);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("checksum"));
    }

    #[cfg(unix)]
    #[test]
    fn deadline_covers_descendants_holding_stdout() {
        let mut ok = Command::new("sh");
        ok.args(["-c", "echo '{}'"]);
        assert_eq!(
            run_with_deadline(ok, "ok", Duration::from_secs(5))
                .unwrap()
                .trim(),
            "{}"
        );

        // The shell exits at once but a background child keeps stdout open.
        let mut hung = Command::new("sh");
        hung.args(["-c", "sleep 30 & echo '{}'"]);
        let t = Instant::now();
        let err = run_with_deadline(hung, "hung", Duration::from_secs(1)).unwrap_err();
        assert!(err.to_string().contains("timed out"));
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn checksum_warning_in_text_output_is_collection_error() {
        // Default `-b warn` prints the warning but leaves exit status 0.
        let j = serde_json::json!({
            "smartctl": {"exit_status": 0, "output": [
                "=== START OF READ SMART DATA SECTION ===",
                "Warning! SMART Attribute Data Structure error: invalid SMART checksum."
            ]},
            "smart_support": {"available": true, "enabled": true},
            "smart_status": {"passed": true},
            "ata_smart_attributes": {"table": []}
        });
        let errs = collection_errors(&j);
        assert_eq!(errs.len(), 1, "{errs:?}");
        let mut r = DriveReport::default();
        parse(&j, &mut r);
        let v = crate::health::evaluate(&r);
        assert_ne!(v.grade, crate::model::Grade::Pass);
    }

    #[test]
    fn documented_counter_aliases() {
        let attr = |id: u8, name: &str, raw: u64| Attribute {
            id,
            name: name.into(),
            raw,
            raw_string: raw.to_string(),
            ..Default::default()
        };
        let j = serde_json::json!({});
        let attrs = [
            attr(5, "Reallocated_Block_Count", 7),
            attr(187, "Uncorrectable_ECC_Ct", 2),
            attr(198, "Uncor_Read_Error_Ct", 3),
            attr(177, "Lifetime_Remaining%", 0),
        ];
        let id = Identity {
            media: MediaKind::Ssd,
            ..Default::default()
        };
        let h = parse_health(&j, &id, &attrs, &[]);
        assert_eq!(h.reallocated_sectors, Some(7));
        assert_eq!(h.reported_uncorrectable, Some(2));
        assert_eq!(h.offline_uncorrectable, Some(3));
        assert_eq!(h.endurance_used_pct, Some(100));
    }

    #[test]
    fn unrecognized_counter_name_is_reported() {
        let j = serde_json::json!({"ata_smart_attributes": {"table": [
            {"id": 197, "name": "Current_Pending_ECC_Cnt", "value": 100, "worst": 100,
             "thresh": 0, "raw": {"value": 1, "string": "1"}}
        ]}});
        let mut r = DriveReport::default();
        parse(&j, &mut r);
        assert_eq!(r.health.pending_sectors, None);
        assert!(
            r.meta
                .warnings
                .iter()
                .any(|w| w.contains("Current_Pending_ECC_Cnt"))
        );
    }

    #[test]
    fn missing_smart_data_is_collection_error() {
        let j = serde_json::json!({"smart_support": {"available": true, "enabled": true}});
        assert_eq!(collection_errors(&j).len(), 2);
        let off = serde_json::json!({"smart_support": {"available": false}});
        assert!(collection_errors(&off).is_empty());
    }

    #[test]
    fn vendor_attribute_meanings() {
        let attr = |id: u8, name: &str, value: u8, raw: u64| Attribute {
            id,
            name: name.into(),
            value,
            raw,
            raw_string: raw.to_string(),
            ..Default::default()
        };
        // MX500: 197 is an ECC counter.
        let mx = [attr(197, "Current_Pending_ECC_Cnt", 100, 1)];
        assert_eq!(named(&mx, 197, &["Current_Pending_Sector"]), None);
        // Lexar: 177 is a retry counter; wear comes from 231.
        let lexar = [
            attr(177, "Read_Retry_Cnt", 100, 0),
            attr(231, "SSD_Life_Left", 3, 3),
        ];
        let wear = lexar
            .iter()
            .find(|a| has_prefix(&a.name, LIFE_REMAINING))
            .map(|a| 100 - a.value as u64);
        assert_eq!(wear, Some(97));
    }

    #[test]
    fn scan_parse() {
        let j = serde_json::json!({"devices":[{"name":"/dev/sdb","info_name":"/dev/sdb [SAT]","type":"sat","protocol":"ATA"}]});
        let d = parse_scan(&j);
        assert_eq!(d[0].dev_type, "sat");
        assert_eq!(d[0].info, "/dev/sdb [SAT]");
    }
}
