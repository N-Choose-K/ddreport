# ddreport

One-page drive health report (PDF) for hard drives and SSDs. It reads the
drive's identity and SMART data, grades it PASS / CAUTION / FAIL, and renders a
printable certificate.

![Sample certificate](docs/sample-wd-red-usb.png)

## What the certificate shows

- Identity: manufacturer, model family, model, serial, firmware, capacity, WWN,
  sector sizes, rpm or SSD, form factor, interface and link speed
- USB bridge details (chipset, VID:PID, BOT/UAS) for USB-attached drives
- Key metrics: power-on hours, power cycles, temperature, reallocated / pending /
  uncorrectable sectors, CRC errors, lifetime reads/writes, SSD wear
- Full SMART attribute table, self-test log, device statistics
- Verdict with plain-English findings and the SHA-256 of the raw data

## Requirements

- Rust 1.92+ (edition 2024)
- [smartmontools](https://www.smartmontools.org/) 7.0+ (`smartctl` on PATH, or set
  `DDREPORT_SMARTCTL`). Most commands need root.

## Usage

```sh
ddreport scan                                  # list drives
sudo ddreport test /dev/sdb long               # start an extended self-test
sudo ddreport report /dev/sdb -o wd-red.pdf
ddreport report --from-json saved.json         # from saved `smartctl -x --json`
```

`report` probes common USB bridge types (`sat`, `sat,12`, `sntjmicron`,
`sntasmedia`, `sntrealtek`, `usbjmicron`, ...) when `-d` is omitted, and records
which one worked. `--save-raw` keeps the raw smartctl JSON alongside the PDF.

## Platform notes

- Linux: full support, including USB-SATA and USB-NVMe bridges.
- macOS: smartctl reaches internal and Thunderbolt drives only; macOS has no
  SCSI passthrough for USB mass storage.

## Development

```sh
cargo test
cargo run -- report --from-json fixtures/sata_hdd_wd_red_usb.json -o /tmp/wd.pdf
```

`fixtures/` holds smartctl JSON samples; see `fixtures/README.md`.

Font files under `assets/fonts/` are stored with Git LFS; install `git-lfs` before cloning.
