# Fixtures

`smartctl --json` samples for parser and grading tests. All are synthetic (invented serials) except `nvme_apple.json` (real, scrubbed).
- `sata_hdd_wd_red_usb.json` - healthy 4 TB WD Red via USB-SAT, full `-x` data
- `sata_hdd_seagate_failing.json` - ST3000DM001, SMART passed but reallocated/pending sectors, read-failure self-test, error log; no device statistics
- `sata_ssd_samsung_870.json` - 870 EVO 1 TB via USB-SAT, TRIM, wear attributes, endurance statistic
- `sata_hdd_failed_smart.json` - HGST, `smart_status.passed=false`, attribute failing now; `-a` output (no device statistics or SCT)
- `nvme_usb_jmicron.json` - WD Blue SN570 behind a JMicron bridge, NVMe self-test log
- `scan_open.json` - `--scan-open` device list
