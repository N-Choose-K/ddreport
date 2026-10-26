//! Renders collected data as a subset of `smartctl -x --json` output.

use serde_json::{Map, Value, json};

use super::ata::{self, SelfTestLog};
use super::attr_names;
use super::collect::Collected;

fn self_test_log_json(log: &SelfTestLog, extended: bool) -> Value {
    let table: Vec<Value> = log
        .entries
        .iter()
        .map(|e| {
            let nibble = ata::self_test_status_nibble(e.status);
            let mut status = json!({
                "value": e.status,
                "string": ata::self_test_log_status_string(nibble),
            });
            if let Some(p) = ata::self_test_passed(nibble) {
                status["passed"] = json!(p);
            }
            if let Some(pct) = ata::self_test_remaining_percent(e.status) {
                status["remaining_percent"] = json!(pct);
            }
            let mut row = json!({
                "type": { "value": e.kind, "string": ata::self_test_type_string(e.kind) },
                "status": status,
                "lifetime_hours": e.hours,
            });
            let no_lba = if extended {
                0xffff_ffff_ffff
            } else {
                0xffff_ffff
            };
            if (3..=8).contains(&nibble) && e.lba != no_lba {
                row["lba"] = json!(e.lba);
            }
            row
        })
        .collect();
    let errors = log
        .entries
        .iter()
        .filter(|e| ata::self_test_passed(ata::self_test_status_nibble(e.status)) == Some(false))
        .count();
    let mut v = json!({
        "revision": log.revision,
        "table": table,
        "count": log.entries.len(),
        "error_count_total": errors,
        "error_count_outdated": 0,
    });
    if extended {
        v["sectors"] = json!(log.sectors);
    }
    v
}

fn insert(m: &mut Map<String, Value>, k: &str, v: Value) {
    m.insert(k.to_owned(), v);
}

/// Builds the smartctl-shaped JSON document.
pub fn build(c: &Collected, name: &str, info_name: &str) -> Value {
    let id = &c.identify;
    let mut m = Map::new();
    let mut exit_status = 0u8;

    insert(&mut m, "json_format_version", json!([1, 0]));
    insert(
        &mut m,
        "device",
        json!({ "name": name, "info_name": info_name, "type": "sat", "protocol": "ATA" }),
    );
    insert(&mut m, "model_name", json!(id.model));
    insert(&mut m, "serial_number", json!(id.serial));
    insert(&mut m, "firmware_version", json!(id.firmware));
    if let Some(w) = &id.wwn {
        insert(
            &mut m,
            "wwn",
            json!({ "naa": w.naa, "oui": w.oui, "id": w.id }),
        );
    }
    insert(&mut m, "in_smartctl_database", json!(false));
    insert(
        &mut m,
        "user_capacity",
        json!({ "blocks": id.sectors, "bytes": id.sectors * id.logical_sector as u64 }),
    );
    insert(&mut m, "logical_block_size", json!(id.logical_sector));
    insert(&mut m, "physical_block_size", json!(id.physical_sector));
    if let Some(r) = id.rotation {
        insert(&mut m, "rotation_rate", json!(if r == 1 { 0 } else { r }));
    }
    if let Some(f) = id.form_factor {
        let mut ff = json!({ "ata_value": f });
        if let Some(n) = ata::form_factor_name(f) {
            ff["name"] = json!(n);
        }
        insert(&mut m, "form_factor", ff);
    }
    if let Some(t) = id.trim {
        let mut trim = json!({ "supported": t });
        if t {
            trim["deterministic"] = json!(id.trim_deterministic);
            trim["zeroed"] = json!(id.trim_zeroed);
        }
        insert(&mut m, "trim", trim);
    }
    if let Some(s) = ata::ata_version_string(id.ata_major, id.ata_minor) {
        insert(
            &mut m,
            "ata_version",
            json!({ "string": s, "major_value": id.ata_major, "minor_value": id.ata_minor }),
        );
    }
    if let Some(w) = id.sata_version
        && let Some(s) = ata::sata_version_name(w)
    {
        insert(
            &mut m,
            "sata_version",
            json!({ "string": s, "value": w & 0x0fff }),
        );
    }
    if let Some(caps) = id.sata_caps {
        let speed = |g: u8| {
            ata::sata_speed(g).map(|(s, units)| {
                json!({ "sata_value": g, "string": s, "units_per_second": units, "bits_per_unit": 100_000_000 })
            })
        };
        let mut is = Map::new();
        if let Some(v) = ata::sata_max_generation(caps).and_then(speed) {
            insert(&mut is, "max", v);
        }
        if let Some(v) = id.sata_current.and_then(speed) {
            insert(&mut is, "current", v);
        }
        if !is.is_empty() {
            insert(&mut m, "interface_speed", Value::Object(is));
        }
    }

    insert(
        &mut m,
        "smart_support",
        json!({ "available": id.smart_supported, "enabled": id.smart_enabled }),
    );
    if let Some(p) = c.smart_status {
        if !p {
            exit_status |= 1 << 3;
        }
        insert(&mut m, "smart_status", json!({ "passed": p }));
    }

    let mut temp = Map::new();
    if let Some(s) = &c.sct {
        let mut t = Map::new();
        for (k, v) in [
            ("current", s.current),
            ("power_cycle_min", s.power_cycle_min),
            ("power_cycle_max", s.power_cycle_max),
            ("lifetime_min", s.lifetime_min),
            ("lifetime_max", s.lifetime_max),
        ] {
            if let Some(v) = v {
                insert(&mut t, k, json!(v));
                insert(&mut temp, k, json!(v));
            }
        }
        insert(
            &mut m,
            "ata_sct_status",
            json!({
                "format_version": s.format_version,
                "sct_version": s.sct_version,
                "device_state": { "value": s.device_state },
                "temperature": t,
            }),
        );
    }

    if let Some(sd) = &c.smart {
        let st = sd.self_test_status;
        let mut status = json!({ "value": st, "string": ata::self_test_exec_string(st) });
        if let Some(p) = ata::self_test_passed(ata::self_test_status_nibble(st)) {
            status["passed"] = json!(p);
        }
        if let Some(pct) = ata::self_test_remaining_percent(st) {
            status["remaining_percent"] = json!(pct);
        }
        let mut polling = json!({ "short": sd.short_minutes, "extended": sd.extended_minutes });
        if sd.conveyance_supported() {
            polling["conveyance"] = json!(sd.conveyance_minutes);
        }
        let mut off = json!({
            "status": { "value": sd.offline_status, "string": ata::offline_status_string(sd.offline_status) },
            "completion_seconds": sd.offline_seconds,
        });
        if sd.offline_status & 0x7f == 0x02 {
            off["status"]["passed"] = json!(true);
        }
        insert(
            &mut m,
            "ata_smart_data",
            json!({
                "offline_data_collection": off,
                "self_test": { "status": status, "polling_minutes": polling },
                "capabilities": {
                    "values": [sd.offline_caps, sd.smart_caps],
                    "self_tests_supported": sd.self_tests_supported(),
                    "conveyance_self_test_supported": sd.conveyance_supported(),
                    "selective_self_test_supported": sd.selective_supported(),
                    "attribute_autosave_enabled": sd.smart_caps & 0x02 != 0,
                    "error_logging_supported": sd.error_logging_supported(),
                    "gp_logging_supported": id.gpl_supported,
                },
            }),
        );

        let th = c.thresholds.as_deref().unwrap_or(&[]);
        let mut table = Vec::new();
        for a in &sd.attrs {
            let thresh = th.iter().find(|t| t.0 == a.id).map_or(0, |t| t.1);
            let (name, fmt) = attr_names::lookup(a.id);
            let wf = ata::when_failed(a.value, a.worst, thresh);
            let prefail = a.flags & 1 != 0;
            match wf {
                "now" if prefail => exit_status |= 1 << 4,
                "now" | "past" => exit_status |= 1 << 5,
                _ => {}
            }
            let f = a.flags;
            table.push(json!({
                "id": a.id,
                "name": name,
                "value": a.value,
                "worst": a.worst,
                "thresh": thresh,
                "when_failed": wf,
                "flags": {
                    "value": f,
                    "string": ata::attr_flags_string(f),
                    "prefailure": prefail,
                    "updated_online": f & 0x02 != 0,
                    "performance": f & 0x04 != 0,
                    "error_rate": f & 0x08 != 0,
                    "event_count": f & 0x10 != 0,
                    "auto_keep": f & 0x20 != 0,
                },
                "raw": { "value": attr_names::raw48(&a.raw), "string": attr_names::format_raw(fmt, &a.raw) },
            }));
            let primary = attr_names::primary_value(fmt, &a.raw);
            match a.id {
                9 => insert(&mut m, "power_on_time", json!({ "hours": primary })),
                12 => insert(&mut m, "power_cycle_count", json!(primary)),
                194 if !temp.contains_key("current") => {
                    insert(&mut temp, "current", json!(primary))
                }
                _ => {}
            }
        }
        if !temp.contains_key("current")
            && let Some(a) = sd.attrs.iter().find(|a| a.id == 190)
        {
            insert(&mut temp, "current", json!(a.raw[0]));
        }
        insert(
            &mut m,
            "ata_smart_attributes",
            json!({ "revision": sd.revision, "table": table }),
        );
    }

    let devstat = |page: u8, off: u16| {
        c.devstats
            .iter()
            .filter(|p| p.number == page)
            .flat_map(|p| &p.stats)
            .find(|s| s.offset == off)
            .and_then(|s| s.value)
    };
    if !m.contains_key("power_on_time")
        && let Some(h) = devstat(1, 16)
    {
        insert(&mut m, "power_on_time", json!({ "hours": h }));
    }
    if !temp.contains_key("current")
        && let Some(t) = devstat(5, 8)
    {
        insert(&mut temp, "current", json!(t));
    }
    if !temp.contains_key("lifetime_max")
        && let Some(t) = devstat(5, 32)
    {
        insert(&mut temp, "lifetime_max", json!(t));
    }
    if !temp.is_empty() {
        insert(&mut m, "temperature", Value::Object(temp));
    }

    let mut stl = Map::new();
    if let Some(log) = &c.selftest {
        insert(&mut stl, "standard", self_test_log_json(log, false));
    }
    if let Some(log) = &c.ext_selftest {
        insert(&mut stl, "extended", self_test_log_json(log, true));
    }
    let newest_failed = [&c.selftest, &c.ext_selftest]
        .iter()
        .filter_map(|l| l.as_ref()?.entries.first())
        .any(|e| ata::self_test_passed(ata::self_test_status_nibble(e.status)) == Some(false));
    if newest_failed {
        exit_status |= 1 << 7;
    }
    if !stl.is_empty() {
        insert(&mut m, "ata_smart_self_test_log", Value::Object(stl));
    }

    if let Some((rev, count)) = c.error_log {
        if count > 0 {
            exit_status |= 1 << 6;
        }
        insert(
            &mut m,
            "ata_smart_error_log",
            json!({ "summary": { "revision": rev, "count": count } }),
        );
    }

    if !c.devstats.is_empty() {
        let pages: Vec<Value> = c
            .devstats
            .iter()
            .map(|p| {
                let table: Vec<Value> = p
                    .stats
                    .iter()
                    .map(|s| {
                        let mut row = json!({
                            "offset": s.offset,
                            "name": s.info.name,
                            "size": s.info.size.unsigned_abs(),
                            "flags": {
                                "value": s.flags,
                                "string": s.flags_string(),
                                "valid": s.valid(),
                                "normalized": s.normalized(),
                                "supports_dsn": s.supports_dsn(),
                                "monitored_condition_met": s.condition_met(),
                            },
                        });
                        if let Some(v) = s.value {
                            row["value"] = json!(v);
                        }
                        row
                    })
                    .collect();
                json!({
                    "number": p.number,
                    "name": ata::devstat_page_name(p.number),
                    "revision": p.revision,
                    "table": table,
                })
            })
            .collect();
        insert(&mut m, "ata_device_statistics", json!({ "pages": pages }));
    }

    if c.messages.iter().any(|msg| msg.severity != "information") {
        exit_status |= 1 << 2;
    }
    let messages: Vec<Value> = c
        .messages
        .iter()
        .map(|msg| json!({ "string": msg.text, "severity": msg.severity }))
        .collect();
    let mut sc =
        json!({ "version": [0, 0], "argv": ["ddreport-native"], "exit_status": exit_status });
    if !messages.is_empty() {
        sc["messages"] = json!(messages);
    }
    insert(&mut m, "smartctl", sc);
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::collect::collect;
    use crate::native::sat::Sat;
    use crate::native::testutil::*;

    fn doc(drive: MockDrive) -> Value {
        let c = collect(&mut Sat::new(drive)).unwrap();
        build(&c, "1f75:0611", "1f75:0611 [Innostor IS611] [SAT native]")
    }

    #[test]
    fn smartctl_key_shapes() {
        let j = doc(MockDrive::default());
        assert_eq!(j["device"]["type"], "sat");
        assert_eq!(j["device"]["protocol"], "ATA");
        assert_eq!(j["device"]["name"], "1f75:0611");
        assert!(j.get("model_family").is_none());
        assert_eq!(j["model_name"], "WDC WD80EFAX-68KNBN0");
        assert_eq!(j["serial_number"], "VAGABCDE");
        assert_eq!(j["firmware_version"], "81.00A81");
        assert_eq!(j["wwn"]["naa"], 5);
        assert_eq!(j["wwn"]["oui"], 0x0014ee);
        assert_eq!(j["wwn"]["id"], 0x2b1234567u64);
        assert_eq!(j["user_capacity"]["blocks"], 15_628_053_168u64);
        assert_eq!(j["user_capacity"]["bytes"], 8_001_563_222_016u64);
        assert_eq!(j["logical_block_size"], 512);
        assert_eq!(j["physical_block_size"], 4096);
        assert_eq!(j["rotation_rate"], 5400);
        assert_eq!(j["form_factor"]["ata_value"], 2);
        assert_eq!(j["form_factor"]["name"], "3.5 inches");
        assert_eq!(j["trim"]["supported"], false);
        assert_eq!(j["ata_version"]["string"], "ACS-3 T13/2161-D revision 5");
        assert_eq!(j["ata_version"]["major_value"], 0x07f0);
        assert_eq!(j["sata_version"]["string"], "SATA 3.1");
        assert!(j["sata_version"]["value"].is_u64());
        assert_eq!(j["interface_speed"]["max"]["string"], "6.0 Gb/s");
        assert_eq!(j["interface_speed"]["current"]["string"], "6.0 Gb/s");
        assert_eq!(j["smart_support"]["available"], true);
        assert_eq!(j["smart_support"]["enabled"], true);
        assert_eq!(j["smart_status"]["passed"], true);

        let st = &j["ata_smart_data"]["self_test"];
        assert_eq!(st["status"]["value"], 0);
        assert_eq!(st["status"]["string"], "completed without error");
        assert_eq!(st["status"]["passed"], true);
        assert_eq!(st["polling_minutes"]["short"], 2);
        assert_eq!(st["polling_minutes"]["extended"], 1000);

        let attrs = &j["ata_smart_attributes"];
        assert_eq!(attrs["revision"], 16);
        let t = attrs["table"].as_array().unwrap();
        assert_eq!(t.len(), 7);
        let a1 = &t[0];
        for k in [
            "id",
            "name",
            "value",
            "worst",
            "thresh",
            "when_failed",
            "flags",
            "raw",
        ] {
            assert!(a1.get(k).is_some(), "attribute missing {k}");
        }
        assert_eq!(a1["name"], "Raw_Read_Error_Rate");
        assert_eq!(a1["thresh"], 51);
        assert_eq!(a1["when_failed"], "");
        for k in [
            "value",
            "string",
            "prefailure",
            "updated_online",
            "performance",
            "error_rate",
            "event_count",
            "auto_keep",
        ] {
            assert!(a1["flags"].get(k).is_some(), "flags missing {k}");
        }
        assert_eq!(a1["flags"]["string"], "POSR-K ");
        assert_eq!(a1["flags"]["prefailure"], true);
        let temp = t.iter().find(|a| a["id"] == 194).unwrap();
        assert_eq!(temp["raw"]["string"], "34 (Min/Max 18/45)");
        assert_eq!(temp["raw"]["value"], 0x002d_0012_0022u64);

        assert_eq!(j["power_on_time"]["hours"], 12_345);
        assert_eq!(j["power_cycle_count"], 42);
        assert_eq!(j["temperature"]["current"], 34);
        assert_eq!(j["temperature"]["lifetime_max"], 52);

        let std = &j["ata_smart_self_test_log"]["standard"];
        assert_eq!(std["count"], 2);
        assert_eq!(std["revision"], 1);
        let row = &std["table"][0];
        assert_eq!(row["type"]["value"], 2);
        assert_eq!(row["type"]["string"], "Extended offline");
        assert_eq!(row["status"]["value"], 0);
        assert_eq!(row["status"]["string"], "Completed without error");
        assert_eq!(row["status"]["passed"], true);
        assert_eq!(row["lifetime_hours"], 12_300);
        assert!(row.get("lba").is_none());
        assert_eq!(j["ata_smart_self_test_log"]["extended"]["count"], 2);

        let pages = j["ata_device_statistics"]["pages"].as_array().unwrap();
        assert_eq!(pages[0]["number"], 1);
        assert_eq!(pages[0]["name"], "General Statistics");
        assert!(pages[0]["revision"].is_u64());
        let s = &pages[0]["table"][1];
        assert_eq!(s["offset"], 16);
        assert_eq!(s["name"], "Power-on Hours");
        assert_eq!(s["size"], 4);
        assert_eq!(s["value"], 12_345);
        assert_eq!(s["flags"]["valid"], true);
        assert_eq!(s["flags"]["normalized"], false);

        assert_eq!(j["ata_smart_error_log"]["summary"]["count"], 0);
        assert_eq!(j["smartctl"]["argv"][0], "ddreport-native");
        assert_eq!(j["smartctl"]["version"], json!([0, 0]));
        assert_eq!(j["smartctl"]["exit_status"], 0);
    }

    #[test]
    fn failures_set_exit_bits_and_lba() {
        let mut attrs = sample_attrs();
        attrs[1] = (5, 0x0033, 100, 100, 140, 8); // prefail value <= thresh
        let drive = MockDrive {
            smart_data: smart_data_sector(&attrs, 0xf3),
            thresholds: thresholds_sector(&attrs),
            failing: true,
            selftest_log: selftest_log_sector(&[(1, 0x02, 0x74, 500, 0x00ab_cdef)], 1),
            ..Default::default()
        };
        let j = doc(drive);
        assert_eq!(j["smart_status"]["passed"], false);
        let t = j["ata_smart_attributes"]["table"].as_array().unwrap();
        assert_eq!(t[1]["when_failed"], "now");
        let st = &j["ata_smart_data"]["self_test"]["status"];
        assert_eq!(st["remaining_percent"], 30);
        assert!(st.get("passed").is_none());
        let row = &j["ata_smart_self_test_log"]["standard"]["table"][0];
        assert_eq!(row["status"]["passed"], false);
        assert_eq!(row["lba"], 0x00ab_cdef);
        assert_eq!(
            j["ata_smart_self_test_log"]["standard"]["error_count_total"],
            1
        );
        let exit = j["smartctl"]["exit_status"].as_u64().unwrap();
        assert_eq!(exit & (1 << 3 | 1 << 4 | 1 << 7), 1 << 3 | 1 << 4 | 1 << 7);
    }

    #[test]
    fn smartctl_parser_reads_native_json() {
        let raw = serde_json::to_string(&doc(MockDrive::default())).unwrap();
        let r = crate::report::from_smartctl_json(&raw, "1f75:0611").unwrap();
        assert_eq!(r.identity.model.as_deref(), Some("WDC WD80EFAX-68KNBN0"));
        assert_eq!(r.identity.serial.as_deref(), Some("VAGABCDE"));
        assert_eq!(r.identity.capacity_bytes, Some(8_001_563_222_016));
        assert_eq!(r.health.overall_passed, Some(true));
        assert_eq!(r.health.power_on_hours, Some(12_345));
        assert_eq!(r.health.udma_crc_errors, Some(3));
        assert_eq!(r.attributes.len(), 7);
        assert_eq!(
            r.self_tests.len(),
            2,
            "standard log; extended duplicates it"
        );
        assert!(r.meta.warnings.is_empty(), "{:?}", r.meta.warnings);
    }

    #[test]
    fn ssd_rotation_zero_and_trim() {
        let spec = IdentifySpec {
            rotation: 1,
            trim: true,
            form_factor: 3,
            ..Default::default()
        };
        let j = doc(MockDrive {
            identify: identify_sector(&spec),
            ..Default::default()
        });
        assert_eq!(j["rotation_rate"], 0);
        assert_eq!(j["trim"]["supported"], true);
        assert_eq!(j["form_factor"]["name"], "2.5 inches");
    }
}
