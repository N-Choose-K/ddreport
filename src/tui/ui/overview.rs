//! Overview tab.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::fmt::bytes_human;
use crate::health;
use crate::model::{DriveReport, MediaKind, Protocol};

use super::text::{fmt_capacity, fmt_count, fmt_time, trunc, wrap, yes_no};
use super::{LABEL_W, grade_color, grade_text, severity_style};

type Kv = (&'static str, String, Style);

pub(super) fn overview_lines(r: &DriveReport, width: u16) -> Vec<Line<'static>> {
    let w = width as usize;
    let g = r.verdict.grade;
    let badge = format!(" {} ", grade_text(g));
    let indent = badge.len() + 1;
    let badge_style = Style::new()
        .bg(grade_color(Some(g)))
        .fg(Color::Black)
        .bold();
    let mut out = Vec::new();
    for (i, part) in wrap(&r.verdict.summary, w.saturating_sub(indent))
        .into_iter()
        .enumerate()
    {
        let lead = if i == 0 {
            Span::styled(badge.clone(), badge_style)
        } else {
            Span::raw(" ".repeat(badge.len()))
        };
        out.push(Line::from(vec![
            lead,
            Span::raw(" "),
            Span::styled(part, Style::new().bold()),
        ]));
    }
    for fd in &r.verdict.findings {
        let (sym, style) = severity_style(fd.severity);
        for (i, part) in wrap(&fd.message, w.saturating_sub(4))
            .into_iter()
            .enumerate()
        {
            let lead = if i == 0 {
                format!("  {sym} ")
            } else {
                "    ".into()
            };
            out.push(Line::from(vec![Span::styled(lead, style), Span::raw(part)]));
        }
    }
    out.push(Line::default());

    let left = [("Identity", identity_rows(r)), ("Report", report_rows(r))];
    let right = [("Health", health_rows(r)), ("USB bridge", bridge_rows(r))];
    if w >= 68 {
        let col = (w - 2) / 2;
        let l = sections(&left, col);
        let rr = sections(&right, col);
        for i in 0..l.len().max(rr.len()) {
            let mut spans = l.get(i).cloned().map(|x| x.spans).unwrap_or_default();
            let used: usize = spans.iter().map(|s| s.width()).sum();
            spans.push(Span::raw(" ".repeat(col + 2 - used.min(col))));
            spans.extend(rr.get(i).cloned().map(|x| x.spans).unwrap_or_default());
            out.push(Line::from(spans));
        }
    } else {
        let all = [&right[..1], &left[..1], &right[1..], &left[1..]].concat();
        out.extend(sections(&all, w));
    }
    let m = &r.meta;
    let notes = m
        .collection_errors
        .iter()
        .map(|e| ("Data error", e, Style::new().fg(Color::Red).bold()))
        .chain(
            m.warnings
                .iter()
                .map(|t| ("Warning", t, Style::new().fg(Color::Yellow))),
        );
    let mut first = true;
    for (label, text, style) in notes {
        if first {
            out.push(Line::default());
            out.push(Line::from(Span::styled(
                "Notes",
                Style::new().fg(Color::Cyan).bold(),
            )));
            first = false;
        }
        for (i, part) in wrap(text, w.saturating_sub(LABEL_W))
            .into_iter()
            .enumerate()
        {
            let lead = if i == 0 {
                format!("{label:<LABEL_W$}")
            } else {
                " ".repeat(LABEL_W)
            };
            out.push(Line::from(vec![
                Span::styled(lead, Style::new().dim()),
                Span::styled(part, style),
            ]));
        }
    }
    out
}

fn sections(secs: &[(&'static str, Vec<Kv>)], w: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (title, rows) in secs.iter().filter(|(_, rows)| !rows.is_empty()) {
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.push(Line::from(Span::styled(
            *title,
            Style::new().fg(Color::Cyan).bold(),
        )));
        for (label, value, style) in rows {
            out.push(Line::from(vec![
                Span::styled(format!("{label:<LABEL_W$}"), Style::new().dim()),
                Span::styled(trunc(value, w.saturating_sub(LABEL_W)), *style),
            ]));
        }
    }
    out
}

fn kv(label: &'static str, v: Option<String>) -> Option<Kv> {
    v.filter(|s| !s.is_empty())
        .map(|s| (label, s, Style::new()))
}

/// Counter that is bad when nonzero.
fn bad_count(label: &'static str, v: Option<u64>, bad: Color) -> Option<Kv> {
    v.map(|n| {
        let style = if n > 0 {
            Style::new().fg(bad).bold()
        } else {
            Style::new().fg(Color::Green)
        };
        (label, fmt_count(n), style)
    })
}

fn identity_rows(r: &DriveReport) -> Vec<Kv> {
    let i = &r.identity;
    let media = match (i.media, i.rotation_rpm) {
        (MediaKind::Hdd, Some(rpm)) => Some(format!("HDD, {rpm} rpm")),
        (MediaKind::Hdd, None) => Some("HDD".into()),
        (MediaKind::Ssd, _) => Some("SSD".into()),
        (MediaKind::Unknown, _) => None,
    };
    let proto = match i.protocol {
        Protocol::Ata => Some("ATA"),
        Protocol::Nvme => Some("NVMe"),
        Protocol::Scsi => Some("SCSI"),
        Protocol::Unknown => None,
    };
    let iface = [
        i.sata_version
            .clone()
            .or(i.nvme_version.as_ref().map(|v| format!("NVMe {v}")))
            .or(proto.map(String::from)),
        i.link_speed_current.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ");
    let sectors = match (i.logical_sector_size, i.physical_sector_size) {
        (Some(l), Some(p)) if l != p => Some(format!("{l} / {p} B")),
        (Some(l), _) => Some(format!("{l} B")),
        _ => None,
    };
    [
        kv("Model", i.model.clone()),
        kv("Family", i.model_family.clone()),
        kv("Serial", i.serial.clone()),
        kv("Firmware", i.firmware.clone()),
        kv("Capacity", i.capacity_bytes.map(fmt_capacity)),
        kv("Bytes", i.capacity_bytes.map(fmt_count)),
        kv("Media", media),
        kv("Form factor", i.form_factor.clone()),
        kv("Interface", Some(iface)),
        kv("Sectors", sectors),
        kv("WWN", i.wwn.clone()),
        kv("TRIM", i.trim_supported.map(yes_no)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn health_rows(r: &DriveReport) -> Vec<Kv> {
    let h = &r.health;
    let smart = match (h.smart_supported, h.smart_enabled, h.overall_passed) {
        (Some(false), _, _) => Some(("unsupported".to_string(), Color::Yellow)),
        (_, Some(false), _) => Some(("disabled".to_string(), Color::Yellow)),
        (_, _, Some(true)) => Some(("PASSED".to_string(), Color::Green)),
        (_, _, Some(false)) => Some(("FAILED".to_string(), Color::Red)),
        _ => None,
    }
    .map(|(s, c)| ("SMART status", s, Style::new().fg(c).bold()));
    let temp = h.temperature_c.map(|t| match h.temperature_max_c {
        Some(m) => format!("{t} °C (max {m} °C)"),
        None => format!("{t} °C"),
    });
    let spare = h
        .available_spare_pct
        .map(|s| match h.available_spare_threshold_pct {
            Some(t) => format!("{s}% (threshold {t}%)"),
            None => format!("{s}%"),
        });
    let pending_color = if health::pending_escalated(h) {
        Color::Red
    } else {
        Color::Yellow
    };
    let wear = h.endurance_used_pct.map(|p| {
        let c = match p {
            0..=49 => Color::Green,
            50..=89 => Color::Yellow,
            _ => Color::Red,
        };
        ("Wear used", format!("{p}%"), Style::new().fg(c))
    });
    [
        smart,
        kv(
            "Power-on",
            h.power_on_hours
                .map(|n| format!("{} h ({:.1} years)", fmt_count(n), n as f64 / 8766.0)),
        ),
        kv("Power cycles", h.power_cycles.map(fmt_count)),
        kv("Temperature", temp),
        bad_count("Reallocated", h.reallocated_sectors, Color::Yellow),
        bad_count("Pending", h.pending_sectors, pending_color),
        bad_count("Offline unc", h.offline_uncorrectable, Color::Yellow),
        bad_count("Reported unc", h.reported_uncorrectable, Color::Yellow),
        bad_count("CRC errors", h.udma_crc_errors, Color::Yellow),
        bad_count("Cmd timeouts", h.command_timeouts, Color::Yellow),
        bad_count("Media errors", h.nvme_media_errors, Color::Red),
        wear,
        kv("Spare", spare),
        kv("Written", h.bytes_written.map(bytes_human)),
        kv("Read", h.bytes_read.map(bytes_human)),
        kv("Start/stop", h.start_stop_count.map(fmt_count)),
        kv("Load cycles", h.load_cycles.map(fmt_count)),
        kv("Unsafe off", h.unsafe_shutdowns.map(fmt_count)),
        kv("Error log", h.error_log_count.map(fmt_count)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn bridge_rows(r: &DriveReport) -> Vec<Kv> {
    let Some(b) = &r.bridge else {
        return vec![];
    };
    let ids = format!("{:04x}:{:04x}", b.vendor_id, b.product_id);
    [
        kv("Chipset", b.chipset.clone()),
        kv("Product", b.product_name.clone()),
        kv("Vendor", b.vendor_name.clone()),
        kv("USB ID", Some(ids)),
        kv("Transport", b.transport.clone()),
        kv("Link", b.link_speed.clone()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn report_rows(r: &DriveReport) -> Vec<Kv> {
    let m = &r.meta;
    [
        kv("Backend", Some(m.backend.clone())),
        kv("Device", Some(m.device_path.clone())),
        kv("Type", Some(m.device_type.clone())),
        kv("Generated", Some(fmt_time(&m.generated_at))),
    ]
    .into_iter()
    .flatten()
    .collect()
}
