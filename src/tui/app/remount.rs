//! Remount-and-read for USB drives behind a bridge: unmount, read through the
//! native backend, and let the OS mount the volumes again.

use crate::tui::device::DeviceSource;
use crate::tui::worker::Job;

use super::{App, Modal};

impl App {
    pub(super) fn remount_read(&mut self) {
        let Some(d) = self.selected_device() else {
            return self.error("No drive selected");
        };
        let DeviceSource::NativeUsb { bridge, .. } = &d.source else {
            return self.error("Remount and read applies to USB drives (native backend)");
        };
        if d.loading() {
            return self.error("A read is already in flight");
        }
        self.modal = Some(Modal::Remount {
            key: d.source.key(),
            label: d.source.label(),
            bridge: bridge.clone(),
            mounts: d.source.mounts().to_vec(),
        });
    }

    pub(super) fn remount_confirmed(&mut self, key: &str) -> Option<Job> {
        let Some(idx) = self.devices.iter().position(|d| d.source.key() == key) else {
            self.error("Cancelled: the drive is gone");
            return None;
        };
        self.info("Unmounting volumes and reading; macOS mounts them again afterwards");
        self.load_with(idx, true)
    }
}
