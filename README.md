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

- Rust 1.99+ (edition 2024; `rust-toolchain.toml` selects stable)
- [smartmontools](https://www.smartmontools.org/) 7.0+ (`smartctl` on PATH, or set
  `DDREPORT_SMARTCTL`). Most commands need root.

## Usage

```sh
ddreport scan                                  # list drives
sudo ddreport test /dev/sdb long               # start an extended self-test
sudo ddreport test --usb 1f75:0611 short --remount  # same over USB (selector as for report)
sudo ddreport report /dev/sdb -o wd-red.pdf
ddreport report --from-json saved.json         # from saved `smartctl -x --json`
ddreport scan --usb                            # list USB mass-storage devices
sudo ddreport report --usb 1f75:0611           # USB-SATA drive via native SAT backend
sudo ddreport report --usb 1f75:0611 --remount # unmount, read, let macOS mount it again
```

`report` probes common USB bridge types (`sat`, `sat,12`, `sntjmicron`,
`sntasmedia`, `sntrealtek`, `usbjmicron`, ...) when `-d` is omitted, and records
which one worked. `--save-raw` keeps the raw smartctl JSON alongside the PDF.

## Platform notes

- Linux: full support, including USB-SATA and USB-NVMe bridges.
- macOS: smartctl reaches internal and Thunderbolt drives only; macOS has no
  SCSI passthrough for USB mass storage. For USB-SATA drives use the native
  backend: `sudo ddreport report --usb 1f75:0611` (selector is `vid:pid`,
  `vid:pid:serial` or `bus:addr` from `ddreport scan --usb`). It needs root:
  libusb takes the device from macOS's storage driver, so the drive disappears
  from macOS while it is read and is re-enumerated (remounted) afterwards.
  The TUI lists USB drives too (name, disk and size come from the OS without
  a read) and reads them this way under `sudo`; `m` remounts a mounted drive:
  unmount, read, and macOS mounts it again. `--remount` does the same on the
  CLI.
- `--usb` refuses to run if the drive is mounted, used as swap, or open in any
  process (e.g. a wipe in progress); unmount it and stop such tools first.
  `--force` only skips checks that can't be completed (disk not found in the
  IORegistry or sysfs, `lsof` missing or failing, or no root privileges); a
  detected mount or open file always stops the run.
- The native backend sends read commands (IDENTIFY, SMART READ
  DATA/THRESHOLDS/LOG, SMART RETURN STATUS, READ LOG EXT) through SAT
  ATA PASS-THROUGH over USB Bulk-Only Transport, plus SMART EXECUTE OFF-LINE
  IMMEDIATE when you start or abort a self-test. It works on Linux too, where
  it detaches and reattaches `usb-storage`/`uas`.
- Self-tests through a bridge: the TUI's `s`/`l`/`a` keys work on USB entries;
  each command and each progress read (once a minute while a test runs)
  unmounts the drive briefly and macOS mounts it again.

## Install

```sh
cargo install --git ssh://git@github.com/nortagawa/ddreport
```

Update with `cargo install --force --git ssh://git@github.com/nortagawa/ddreport`
(cargo skips a reinstall of the same version without `--force`). `smartctl`
(smartmontools) must be on PATH for the smartctl backend. On macOS with a nix or Homebrew `cc` first on PATH,
point cargo at Apple's toolchain in `~/.cargo/config.toml` (the repo's own
`.cargo/config.toml` does not apply to `cargo install --git`):

```toml
[target.aarch64-apple-darwin]
linker = "/usr/bin/cc"

[target.x86_64-apple-darwin]
linker = "/usr/bin/cc"

[env]
CC_aarch64_apple_darwin = "/usr/bin/cc"
CC_x86_64_apple_darwin = "/usr/bin/cc"
```

## Development

```sh
cargo test
cargo run -- report --from-json fixtures/sata_hdd_wd_red_usb.json -o /tmp/wd.pdf
```

`fixtures/` holds smartctl JSON samples; see `fixtures/README.md`.

Sample images under `docs/` are stored with Git LFS; the fonts under
`assets/fonts/` are plain blobs so `cargo install --git` can embed them.
