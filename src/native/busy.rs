//! Busy-disk guard: refuses to capture a USB device whose disk is mounted,
//! used as swap, held by device-mapper, or open in any process (e.g. a wipe
//! in progress). Detected use always refuses; `force` only skips checks that
//! could not be completed (unmapped disk, missing lsof, no root).

use anyhow::{Result, bail};

use super::UsbStorageDevice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    pub pid: u32,
    pub command: String,
    pub path: String,
}

/// Parses BSD/macOS `mount` output: "SRC on TARGET (opts)".
pub fn parse_bsd_mount(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|l| {
            let (source, rest) = l.split_once(" on ")?;
            let target = rest.rsplit_once(" (").map_or(rest, |(t, _)| t);
            Some(Mount {
                source: source.to_owned(),
                target: target.to_owned(),
            })
        })
        .collect()
}

/// Decodes the octal escapes (`\040`) used in /proc/mounts and /proc/swaps.
fn unescape_octal(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Block device number (major, minor).
pub type DevNum = (u32, u32);

/// Splits a Linux `dev_t` (glibc encoding) into (major, minor).
pub fn linux_dev_split(dev: u64) -> DevNum {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major as u32, minor as u32)
}

/// Parses "major:minor" (sysfs `dev` files, mountinfo field 3).
pub fn parse_dev_numbers(s: &str) -> Option<DevNum> {
    let (a, b) = s.trim().split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Mount or swap entry with the device number it lives on, if known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevMount {
    pub dev: Option<DevNum>,
    pub mount: Mount,
}

/// Parses /proc/self/mountinfo:
/// "ID PARENT MAJ:MIN ROOT MOUNTPOINT OPTS [TAGS...] - FSTYPE SOURCE SUPEROPTS".
pub fn parse_mountinfo(text: &str) -> Vec<DevMount> {
    text.lines()
        .filter_map(|l| {
            let (pre, post) = l.split_once(" - ")?;
            let f: Vec<&str> = pre.split_whitespace().collect();
            let source = post.split_whitespace().nth(1)?;
            Some(DevMount {
                dev: parse_dev_numbers(f.get(2)?),
                mount: Mount {
                    source: unescape_octal(source),
                    target: unescape_octal(f.get(4)?),
                },
            })
        })
        .collect()
}

/// Swap device/file paths from /proc/swaps (header skipped).
pub fn parse_swaps(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next().map(unescape_octal))
        .collect()
}

/// Open file with the block device number it refers to, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevOpen {
    pub dev: Option<DevNum>,
    pub file: OpenFile,
}

/// Linux blockers: entries on one of `devs` (by device number, so alternate
/// device nodes match) or whose path names one of `disks`.
pub fn linux_blockers(
    disks: &[String],
    devs: &[DevNum],
    mounts: &[DevMount],
    open: &[DevOpen],
) -> Vec<String> {
    let hit = |dev: Option<DevNum>, path: &str| {
        dev.is_some_and(|d| devs.contains(&d)) || linux_belongs(path, disks)
    };
    let m = mounts
        .iter()
        .filter(|m| hit(m.dev, &m.mount.source))
        .map(|m| format!("{} is mounted on {}", m.mount.source, m.mount.target));
    let o = open.iter().filter(|o| hit(o.dev, &o.file.path)).map(|o| {
        format!(
            "{} is open by pid {} ({})",
            o.file.path, o.file.pid, o.file.command
        )
    });
    m.chain(o).collect()
}

/// Parses `lsof -F pcn` output (p=pid, c=command, n=name lines).
pub fn parse_lsof(text: &str) -> Vec<OpenFile> {
    let (mut pid, mut command) = (0u32, String::new());
    let mut out = Vec::new();
    for l in text.lines() {
        let Some(tag) = l.chars().next() else {
            continue;
        };
        let v = &l[tag.len_utf8()..];
        match tag {
            'p' => {
                pid = v.parse().unwrap_or(0);
                command.clear();
            }
            'c' => command = v.to_owned(),
            'n' => out.push(OpenFile {
                pid,
                command: command.clone(),
                path: v.to_owned(),
            }),
            _ => {}
        }
    }
    out
}

/// Interprets an lsof run. Only exit 0 with parsed records, or exit 1 with
/// empty stdout and stderr (no match), count as a completed check; anything
/// else returns an error. Records found are returned either way.
pub fn interpret_lsof(
    code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> (Vec<OpenFile>, Option<String>) {
    let open = parse_lsof(stdout);
    let clean = match code {
        Some(0) => !open.is_empty(),
        Some(1) => stdout.trim().is_empty() && stderr.trim().is_empty(),
        _ => false,
    };
    let err = (!clean).then(|| {
        let code = code.map_or("signal".to_owned(), |c| c.to_string());
        let detail = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no diagnostics");
        format!("lsof did not complete cleanly (exit {code}): {detail}")
    });
    (open, err)
}

fn plist_uint(d: &plist::Dictionary, k: &str) -> Option<u64> {
    let v = d.get(k)?;
    v.as_unsigned_integer()
        .or_else(|| v.as_signed_integer().map(|x| x as u64))
}

fn children(d: &plist::Dictionary) -> impl Iterator<Item = &plist::Dictionary> {
    d.get("IORegistryEntryChildren")
        .and_then(plist::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(plist::Value::as_dictionary)
}

fn collect_bsd_names(d: &plist::Dictionary, out: &mut Vec<String>) {
    if let Some(n) = d.get("BSD Name").and_then(plist::Value::as_string) {
        out.push(n.to_owned());
    }
    for c in children(d) {
        collect_bsd_names(c, out);
    }
}

fn find_usb_device<'a>(
    d: &'a plist::Dictionary,
    dev: &UsbStorageDevice,
) -> Option<&'a plist::Dictionary> {
    let is_match = d.get("IOObjectClass").and_then(plist::Value::as_string)
        == Some("IOUSBHostDevice")
        && plist_uint(d, "idVendor") == Some(dev.vendor_id as u64)
        && plist_uint(d, "idProduct") == Some(dev.product_id as u64)
        && plist_uint(d, "USB Address") == Some(dev.address as u64)
        && plist_uint(d, "locationID").map(|l| (l >> 24) as u8) == Some(dev.bus);
    if is_match {
        return Some(d);
    }
    children(d).find_map(|c| find_usb_device(c, dev))
}

/// BSD names (whole disks, slices, synthesized APFS disks) below the USB
/// device in `ioreg -a -r -c IOUSBHostDevice -l` output. None if the device
/// isn't in the registry dump.
pub fn ioreg_bsd_names(xml: &[u8], dev: &UsbStorageDevice) -> Result<Option<Vec<String>>> {
    let root = plist::Value::from_reader_xml(xml)?;
    let tops: Vec<&plist::Dictionary> = match &root {
        plist::Value::Array(a) => a.iter().filter_map(plist::Value::as_dictionary).collect(),
        plist::Value::Dictionary(d) => vec![d],
        _ => Vec::new(),
    };
    Ok(tops
        .into_iter()
        .find_map(|d| find_usb_device(d, dev))
        .map(|d| {
            let mut names = Vec::new();
            collect_bsd_names(d, &mut names);
            names
        }))
}

/// macOS: "/dev/disk6", "/dev/rdisk6s1", "disk6s2" belong to disk set {disk6}.
pub fn macos_belongs(path: &str, disks: &[String]) -> bool {
    let n = path.strip_prefix("/dev/").unwrap_or(path);
    let n = n
        .strip_prefix('r')
        .filter(|x| x.starts_with("disk"))
        .unwrap_or(n);
    let slice = |r: &str| {
        r.strip_prefix('s')
            .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit() || c == 's'))
    };
    disks
        .iter()
        .any(|d| n == d || n.strip_prefix(d.as_str()).is_some_and(slice))
}

/// Linux: "/dev/sdb", "/dev/sdb1" belong to {sdb}; "/dev/sdba" does not.
pub fn linux_belongs(path: &str, disks: &[String]) -> bool {
    let n = path.strip_prefix("/dev/").unwrap_or(path);
    let part = |r: &str| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit());
    disks
        .iter()
        .any(|d| n == d || n.strip_prefix(d.as_str()).is_some_and(part))
}

/// Human-readable reasons the disks are in use.
pub fn blockers(
    disks: &[String],
    mounts: &[Mount],
    open: &[OpenFile],
    belongs: fn(&str, &[String]) -> bool,
) -> Vec<String> {
    let m = mounts
        .iter()
        .filter(|m| belongs(&m.source, disks))
        .map(|m| format!("{} is mounted on {}", m.source, m.target));
    let o = open
        .iter()
        .filter(|f| belongs(&f.path, disks))
        .map(|f| format!("{} is open by pid {} ({})", f.path, f.pid, f.command));
    m.chain(o).collect()
}

/// Refuses on any blocker; refuses on unverified checks unless `force`.
/// Returns notes to record in the report.
pub fn decide(blockers: &[String], unverified: &[String], force: bool) -> Result<Vec<String>> {
    if !blockers.is_empty() {
        bail!(
            "refusing to capture the USB device: its disk is in use:\n  - {}\nStop that process or unmount the disk first.",
            blockers.join("\n  - ")
        );
    }
    if unverified.is_empty() {
        return Ok(Vec::new());
    }
    if !force {
        bail!(
            "refusing to capture the USB device: cannot verify its disk is idle:\n  - {}\nPass --force to continue anyway.",
            unverified.join("\n  - ")
        );
    }
    Ok(unverified
        .iter()
        .map(|u| format!("busy check incomplete, continued with --force: {u}"))
        .collect())
}

#[cfg(target_os = "macos")]
fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Checks the device's disk is idle before its driver is detached.
#[cfg(target_os = "macos")]
pub fn ensure_idle(dev: &UsbStorageDevice, force: bool) -> Result<Vec<String>> {
    use std::process::Command;

    let mut unverified = Vec::new();
    let disks = match Command::new("/usr/sbin/ioreg")
        .args(["-a", "-r", "-c", "IOUSBHostDevice", "-l"])
        .output()
    {
        Ok(o) if o.status.success() => match ioreg_bsd_names(&o.stdout, dev) {
            Ok(Some(d)) if !d.is_empty() => d,
            Ok(_) => {
                return decide(
                    &[],
                    &["no BSD disk found for this USB device in the IORegistry".into()],
                    force,
                );
            }
            Err(e) => return decide(&[], &[format!("cannot parse ioreg output: {e}")], force),
        },
        Ok(o) => return decide(&[], &[format!("ioreg failed: {}", o.status)], force),
        Err(e) => return decide(&[], &[format!("cannot run ioreg: {e}")], force),
    };

    let mounts = match Command::new("/sbin/mount").output() {
        Ok(o) if o.status.success() => parse_bsd_mount(&String::from_utf8_lossy(&o.stdout)),
        _ => {
            unverified.push("cannot run mount".into());
            Vec::new()
        }
    };

    let paths: Vec<String> = disks
        .iter()
        .flat_map(|d| [format!("/dev/{d}"), format!("/dev/r{d}")])
        .collect();
    let lsof = ["/usr/sbin/lsof", "lsof"].iter().find_map(|bin| {
        Command::new(bin)
            .args(["-w", "-F", "pcn", "--"])
            .args(&paths)
            .output()
            .ok()
    });
    let open = match lsof {
        Some(o) => {
            let (open, err) = interpret_lsof(
                o.status.code(),
                &String::from_utf8_lossy(&o.stdout),
                &String::from_utf8_lossy(&o.stderr),
            );
            unverified.extend(err);
            open
        }
        None => {
            unverified.push("lsof not found; cannot check for processes using the disk".into());
            Vec::new()
        }
    };
    if !is_root() {
        unverified.push("not running as root; lsof cannot see other users' processes".into());
    }
    decide(
        &blockers(&disks, &mounts, &open, macos_belongs),
        &unverified,
        force,
    )
}

/// Reports a failed read as an unverified check. NotFound (a process or
/// holder vanishing mid-scan) is ignored.
pub fn note_io<T>(r: std::io::Result<T>, what: &str, unverified: &mut Vec<String>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            unverified.push(format!("cannot read {what}: {e}"));
            None
        }
    }
}

/// Scans the fd table of every process and every thread under `proc_root`
/// (normally "/proc"); a process whose main thread exited keeps its open
/// files only in /proc/<pid>/task/<tid>/fd. `hit` selects open files of
/// interest. Returns (matches, failed reads). ENOENT is ignored only when the
/// process or thread itself is gone.
#[cfg(unix)]
pub fn scan_proc_fds(
    proc_root: &std::path::Path,
    skip_pid: u32,
    hit: impl Fn(Option<DevNum>, &str) -> bool,
) -> (Vec<DevOpen>, Vec<String>) {
    use std::fs;
    use std::io::ErrorKind::NotFound;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let mut open: Vec<DevOpen> = Vec::new();
    let mut failed = Vec::new();
    let entries = match fs::read_dir(proc_root) {
        Ok(e) => e,
        Err(e) => return (open, vec![format!("{}: {e}", proc_root.display())]),
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                failed.push(format!("{}: {e}", proc_root.display()));
                continue;
            }
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == skip_pid {
            continue;
        }
        let pdir = entry.path();
        let alive = |p: &std::path::Path| fs::symlink_metadata(p).is_ok();

        let mut fd_dirs = vec![(pdir.join("fd"), pdir.clone())];
        match fs::read_dir(pdir.join("task")) {
            Ok(tasks) => {
                for t in tasks {
                    match t {
                        Ok(t) => fd_dirs.push((t.path().join("fd"), t.path())),
                        Err(e) => failed.push(format!("{}/task: {e}", pdir.display())),
                    }
                }
            }
            Err(e) if e.kind() == NotFound && !alive(&pdir) => continue,
            Err(e) => failed.push(format!("{}/task: {e}", pdir.display())),
        }

        for (dir, owner) in fd_dirs {
            let fds = match fs::read_dir(&dir) {
                Ok(f) => f,
                Err(e) if e.kind() == NotFound && !alive(&owner) => continue,
                Err(e) => {
                    failed.push(format!("{}: {e}", dir.display()));
                    continue;
                }
            };
            for fd in fds {
                let fd = match fd {
                    Ok(fd) => fd,
                    Err(e) => {
                        failed.push(format!("{}: {e}", dir.display()));
                        continue;
                    }
                };
                let target = match fs::read_link(fd.path()) {
                    Ok(t) => t,
                    // The fd closed mid-scan.
                    Err(e) if e.kind() == NotFound => continue,
                    Err(e) => {
                        failed.push(format!("{}: {e}", fd.path().display()));
                        continue;
                    }
                };
                let path = target.to_string_lossy().into_owned();
                // stat() through the fd link: the open inode's st_rdev, however it was named.
                let dev = fs::metadata(fd.path())
                    .ok()
                    .filter(|m| m.file_type().is_block_device())
                    .map(|m| linux_dev_split(m.rdev()));
                let dup = open
                    .iter()
                    .any(|o| o.file.pid == pid && o.file.path == path);
                if hit(dev, &path) && !dup {
                    let command = fs::read_to_string(pdir.join("comm"))
                        .unwrap_or_default()
                        .trim()
                        .to_owned();
                    open.push(DevOpen {
                        dev,
                        file: OpenFile { pid, command, path },
                    });
                }
            }
        }
    }
    (open, failed)
}

/// Checks the device's disk is idle before its driver is detached.
#[cfg(target_os = "linux")]
pub fn ensure_idle(dev: &UsbStorageDevice, force: bool) -> Result<Vec<String>> {
    use std::fs;
    use std::path::Path;

    let mut unverified = Vec::new();
    let read_num = |p: &Path, f: &str| {
        fs::read_to_string(p.join(f))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    };
    let mut disks = Vec::new();
    let mut devs = Vec::new();
    let mut holders = Vec::new();
    let block = note_io(fs::read_dir("/sys/block"), "/sys/block", &mut unverified);
    for entry in block.into_iter().flatten() {
        let Some(entry) = note_io(entry, "/sys/block entry", &mut unverified) else {
            continue;
        };
        let Some(real) = note_io(
            fs::canonicalize(entry.path()),
            "/sys/block link",
            &mut unverified,
        ) else {
            continue;
        };
        let owned = real.ancestors().any(|a| {
            read_num(a, "busnum") == Some(dev.bus as u32)
                && read_num(a, "devnum") == Some(dev.address as u32)
        });
        if !owned {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Holders and device numbers of the disk and its partitions.
        let mut dirs = vec![real.clone()];
        let what = format!("partitions of /dev/{name}");
        for e in note_io(fs::read_dir(&real), &what, &mut unverified)
            .into_iter()
            .flatten()
        {
            if let Some(e) = note_io(e, &what, &mut unverified)
                && e.path().join("partition").exists()
            {
                dirs.push(e.path());
            }
        }
        for d in dirs {
            let what = format!("device number of {}", d.display());
            let dev = note_io(fs::read_to_string(d.join("dev")), &what, &mut unverified);
            match dev.as_deref().and_then(parse_dev_numbers) {
                Some(n) => devs.push(n),
                None => unverified.push(format!("cannot read {what}")),
            }
            let what = format!("holders of {}", d.display());
            for h in note_io(fs::read_dir(d.join("holders")), &what, &mut unverified)
                .into_iter()
                .flatten()
            {
                if let Some(h) = note_io(h, &what, &mut unverified) {
                    holders.push(format!(
                        "/dev/{name} is held by {}",
                        h.file_name().to_string_lossy()
                    ));
                }
            }
        }
        disks.push(name);
    }
    if disks.is_empty() {
        unverified.push("no block device found for this USB device in /sys/block".into());
        return decide(&[], &unverified, force);
    }

    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let block_dev = |m: fs::Metadata| {
        m.file_type()
            .is_block_device()
            .then(|| linux_dev_split(m.rdev()))
    };

    let info = note_io(
        fs::read_to_string("/proc/self/mountinfo"),
        "/proc/self/mountinfo",
        &mut unverified,
    );
    let mut mounts = parse_mountinfo(info.as_deref().unwrap_or_default());
    let swaps = note_io(
        fs::read_to_string("/proc/swaps"),
        "/proc/swaps",
        &mut unverified,
    );
    for path in parse_swaps(swaps.as_deref().unwrap_or_default()) {
        let dev = fs::metadata(&path).ok().and_then(block_dev);
        mounts.push(DevMount {
            dev,
            mount: Mount {
                source: path,
                target: "[swap]".into(),
            },
        });
    }

    let (open, unreadable) = scan_proc_fds(Path::new("/proc"), std::process::id(), |dev, path| {
        dev.is_some_and(|d| devs.contains(&d)) || linux_belongs(path, &disks)
    });
    if let Some(first) = unreadable.first() {
        unverified.push(format!(
            "{} open-file reads failed (run as root), first: {first}",
            unreadable.len()
        ));
    }
    let mut b = linux_blockers(&disks, &devs, &mounts, &open);
    b.extend(holders);
    decide(&b, &unverified, force)
}

/// No busy check on this platform.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn ensure_idle(_dev: &UsbStorageDevice, force: bool) -> Result<Vec<String>> {
    decide(
        &[],
        &["busy-disk check is not implemented on this platform".into()],
        force,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::Transport;

    fn dev() -> UsbStorageDevice {
        UsbStorageDevice {
            bus: 0,
            address: 13,
            vendor_id: 0x1f75,
            product_id: 0x0611,
            manufacturer: None,
            product: None,
            serial: None,
            transport: Transport::Bot,
            speed: None,
            interface: 0,
        }
    }

    const IOREG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
  <dict>
    <key>IOObjectClass</key><string>IOUSBHostDevice</string>
    <key>idVendor</key><integer>1452</integer>
    <key>idProduct</key><integer>4097</integer>
    <key>USB Address</key><integer>1</integer>
    <key>locationID</key><integer>2097152</integer>
    <key>IORegistryEntryChildren</key>
    <array>
      <dict>
        <key>IOObjectClass</key><string>IOUSBHostDevice</string>
        <key>idVendor</key><integer>8053</integer>
        <key>idProduct</key><integer>1553</integer>
        <key>USB Address</key><integer>13</integer>
        <key>locationID</key><integer>2371584</integer>
        <key>USB Serial Number</key><string>SANITIZED</string>
        <key>IORegistryEntryChildren</key>
        <array>
          <dict>
            <key>IOObjectClass</key><string>IOUSBHostInterface</string>
            <key>IORegistryEntryChildren</key>
            <array>
              <dict>
                <key>IOObjectClass</key><string>IOMedia</string>
                <key>Whole</key><true/>
                <key>BSD Name</key><string>disk6</string>
                <key>IORegistryEntryChildren</key>
                <array>
                  <dict>
                    <key>IOObjectClass</key><string>IOMedia</string>
                    <key>Whole</key><false/>
                    <key>BSD Name</key><string>disk6s1</string>
                  </dict>
                  <dict>
                    <key>IOObjectClass</key><string>IOMedia</string>
                    <key>BSD Name</key><string>disk6s2</string>
                    <key>IORegistryEntryChildren</key>
                    <array>
                      <dict>
                        <key>IOObjectClass</key><string>AppleAPFSContainer</string>
                        <key>IORegistryEntryChildren</key>
                        <array>
                          <dict>
                            <key>IOObjectClass</key><string>IOMedia</string>
                            <key>BSD Name</key><string>disk7</string>
                          </dict>
                        </array>
                      </dict>
                    </array>
                  </dict>
                </array>
              </dict>
            </array>
          </dict>
        </array>
      </dict>
    </array>
  </dict>
</array>
</plist>"#;

    #[test]
    fn ioreg_maps_device_to_disks() {
        let names = ioreg_bsd_names(IOREG.as_bytes(), &dev()).unwrap().unwrap();
        assert_eq!(names, ["disk6", "disk6s1", "disk6s2", "disk7"]);
        let other = UsbStorageDevice {
            address: 14,
            ..dev()
        };
        assert_eq!(ioreg_bsd_names(IOREG.as_bytes(), &other).unwrap(), None);
        assert!(ioreg_bsd_names(b"not xml", &dev()).is_err());
    }

    #[test]
    fn bsd_mount_parsing() {
        let text = "/dev/disk3s1s1 on / (apfs, sealed, local, read-only, journaled)\n\
                    devfs on /dev (devfs, local, nobrowse)\n\
                    /dev/disk6s1 on /Volumes/My Disk (on) (msdos, local, nodev, nosuid, noowners)\n";
        let m = parse_bsd_mount(text);
        assert_eq!(m.len(), 3);
        assert_eq!(
            m[2],
            Mount {
                source: "/dev/disk6s1".into(),
                target: "/Volumes/My Disk (on)".into()
            }
        );
    }

    #[test]
    fn mountinfo_and_swaps_parsing() {
        let info = "36 35 8:17 / /media/user/My\\040Disk rw,relatime shared:1 - vfat /dev/sdb1 rw\n\
                    25 1 0:22 / /proc rw - proc proc rw\n";
        let m = parse_mountinfo(info);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].dev, Some((8, 17)));
        assert_eq!(
            m[0].mount,
            Mount {
                source: "/dev/sdb1".into(),
                target: "/media/user/My Disk".into()
            }
        );
        let swaps =
            parse_swaps("Filename\tType\tSize\tUsed\tPriority\n/dev/sdc2 partition 1024 0 -2\n");
        assert_eq!(swaps, ["/dev/sdc2"]);
        assert_eq!(parse_dev_numbers("259:3\n"), Some((259, 3)));
        assert_eq!(parse_dev_numbers("garbage"), None);
    }

    #[test]
    fn dev_t_split() {
        assert_eq!(linux_dev_split(0x0811), (8, 17));
        let (major, minor) = (259u64, 1_048_576u64);
        let dev = ((major & 0xfff) << 8)
            | ((major & !0xfff) << 32)
            | (minor & 0xff)
            | ((minor & !0xff) << 12);
        assert_eq!(linux_dev_split(dev), (259, 1_048_576));
    }

    #[test]
    fn linux_blockers_match_device_numbers() {
        let disks = vec!["sdb".to_owned()];
        let devs = [(8, 16), (8, 17)];
        let open = |dev, path: &str| DevOpen {
            dev,
            file: OpenFile {
                pid: 7,
                command: "wipe".into(),
                path: path.into(),
            },
        };
        // A node made with mknod has another path but the same device number.
        let b = linux_blockers(&disks, &devs, &[], &[open(Some((8, 16)), "/tmp/wipe-node")]);
        assert_eq!(b, ["/tmp/wipe-node is open by pid 7 (wipe)"]);
        assert!(linux_blockers(&disks, &devs, &[], &[open(Some((8, 0)), "/dev/sda")]).is_empty());
        assert_eq!(
            linux_blockers(&disks, &devs, &[], &[open(None, "/dev/sdb1")]).len(),
            1
        );
        let m = DevMount {
            dev: Some((8, 17)),
            mount: Mount {
                source: "/dev/disk/by-label/X".into(),
                target: "/mnt".into(),
            },
        };
        assert_eq!(
            linux_blockers(&disks, &devs, &[m], &[]),
            ["/dev/disk/by-label/X is mounted on /mnt"]
        );
    }

    #[test]
    fn lsof_parsing() {
        let text =
            "p4242\ncnwipe\nf3\nn/dev/rdisk6\np77\ncdd\nf1\nn/dev/disk6s1\nf2\nn/dev/ttys001\n";
        let o = parse_lsof(text);
        assert_eq!(o.len(), 3);
        assert_eq!(
            o[0],
            OpenFile {
                pid: 4242,
                command: "nwipe".into(),
                path: "/dev/rdisk6".into()
            }
        );
        assert_eq!(o[2].pid, 77);
        assert!(parse_lsof("").is_empty());
    }

    #[test]
    fn belongs_rules() {
        let mac = vec!["disk6".to_owned()];
        for p in [
            "/dev/disk6",
            "/dev/rdisk6",
            "/dev/disk6s1",
            "/dev/rdisk6s2",
            "disk6s1s1",
        ] {
            assert!(macos_belongs(p, &mac), "{p}");
        }
        for p in [
            "/dev/disk60",
            "/dev/disk61s1",
            "/dev/disk7",
            "/dev/rdisk",
            "devfs",
        ] {
            assert!(!macos_belongs(p, &mac), "{p}");
        }
        let lin = vec!["sdb".to_owned()];
        assert!(linux_belongs("/dev/sdb", &lin) && linux_belongs("/dev/sdb12", &lin));
        assert!(!linux_belongs("/dev/sdba", &lin) && !linux_belongs("/dev/sda1", &lin));
        assert!(!linux_belongs("/dev/sdbs", &lin));
    }

    #[test]
    fn blockers_and_decisions() {
        let disks = vec!["disk6".to_owned()];
        let mounts =
            parse_bsd_mount("/dev/disk6s1 on /Volumes/X (msdos)\n/dev/disk3s1 on / (apfs)\n");
        let open = parse_lsof("p4242\ncnwipe\nn/dev/rdisk6\np1\nclaunchd\nn/dev/null\n");
        let b = blockers(&disks, &mounts, &open, macos_belongs);
        assert_eq!(
            b,
            [
                "/dev/disk6s1 is mounted on /Volumes/X",
                "/dev/rdisk6 is open by pid 4242 (nwipe)"
            ]
        );

        let e = decide(&b, &[], true).unwrap_err().to_string();
        assert!(
            e.contains("nwipe"),
            "force never overrides a detected blocker"
        );
        assert!(decide(&[], &["lsof not found".into()], false).is_err());
        let notes = decide(&[], &["lsof not found".into()], true).unwrap();
        assert!(notes[0].contains("--force"));
        assert!(decide(&[], &[], false).unwrap().is_empty());
        assert!(blockers(&disks, &[], &[], macos_belongs).is_empty());
    }

    #[test]
    fn lsof_exit_status_rules() {
        let hit = "p4242\ncnwipe\nf3\nn/dev/rdisk6\n";
        let (open, err) = interpret_lsof(Some(0), hit, "");
        assert_eq!((open.len(), err), (1, None));
        assert_eq!(
            interpret_lsof(Some(1), "", ""),
            (Vec::new(), None),
            "clean no-match"
        );

        let (open, err) = interpret_lsof(
            Some(1),
            "",
            "lsof: status error on /dev/rdisk6: Permission denied\n",
        );
        assert!(open.is_empty());
        assert!(err.unwrap().contains("Permission denied"));
        assert!(
            interpret_lsof(Some(0), "", "").1.is_some(),
            "exit 0 without records"
        );
        assert!(interpret_lsof(None, "", "").1.is_some(), "killed by signal");
        assert!(interpret_lsof(Some(2), "", "").1.is_some());

        // A partial failure still reports what it found; decide() refuses on it.
        let (open, err) = interpret_lsof(Some(1), hit, "lsof: WARNING: something\n");
        assert_eq!(open.len(), 1);
        assert!(err.is_some());
        let disks = vec!["disk6".to_owned()];
        let b = blockers(&disks, &[], &open, macos_belongs);
        assert!(decide(&b, &[err.unwrap()], true).is_err());
        let (_, err) = interpret_lsof(Some(1), "", "lsof: no pwd entry\n");
        assert!(
            decide(&[], &[err.unwrap()], false).is_err(),
            "unverified refuses without --force"
        );
    }

    #[test]
    fn io_errors_become_unverified() {
        use std::io::{Error, ErrorKind};
        let mut u = Vec::new();
        assert_eq!(note_io(Ok(5), "x", &mut u), Some(5));
        assert_eq!(
            note_io::<u8>(Err(Error::from(ErrorKind::NotFound)), "gone", &mut u),
            None
        );
        assert!(u.is_empty(), "vanished entries are ignored");
        assert_eq!(
            note_io::<u8>(
                Err(Error::from(ErrorKind::PermissionDenied)),
                "/proc/swaps",
                &mut u
            ),
            None
        );
        assert_eq!(u.len(), 1);
        assert!(u[0].contains("/proc/swaps"));
        assert!(decide(&[], &u, false).is_err(), "refuses by default");
        assert!(decide(&[], &u, true).is_ok());
    }

    #[test]
    fn proc_scan_covers_threads_and_missing_tables() {
        use std::fs;
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("ddreport-proc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mk = |p: &str| fs::create_dir_all(root.join(p)).unwrap();
        let link = |target: &str, p: &str| symlink(target, root.join(p)).unwrap();
        // 100: main thread exited; worker 101 still holds the disk.
        mk("100/fd");
        mk("100/task/100/fd");
        mk("100/task/101/fd");
        link("/dev/sdb", "100/task/101/fd/3");
        fs::write(root.join("100/comm"), "wipe\n").unwrap();
        // 200: unrelated files, listed in both the process and thread tables.
        mk("200/fd");
        mk("200/task/200/fd");
        link("/dev/null", "200/fd/0");
        link("/dev/sdb1", "200/fd/4");
        link("/dev/sdb1", "200/task/200/fd/4");
        // 300: live process whose fd and task tables are unavailable.
        mk("300");
        // 400: skipped (our own pid); non-numeric entries are ignored.
        mk("400/fd");
        link("/dev/sdb", "400/fd/1");
        mk("self");

        let disks = vec!["sdb".to_owned()];
        let (open, failed) = scan_proc_fds(&root, 400, |_, p| linux_belongs(p, &disks));
        fs::remove_dir_all(&root).unwrap();

        let mut found: Vec<(u32, &str, &str)> = open
            .iter()
            .map(|o| (o.file.pid, o.file.path.as_str(), o.file.command.as_str()))
            .collect();
        found.sort();
        assert_eq!(found, [(100, "/dev/sdb", "wipe"), (200, "/dev/sdb1", "")]);
        assert_eq!(failed.len(), 2, "{failed:?}");
        assert!(failed.iter().all(|f| f.contains("/300/")));
        assert!(
            decide(&[], &failed, false).is_err(),
            "unreadable live process refuses"
        );
    }
}
