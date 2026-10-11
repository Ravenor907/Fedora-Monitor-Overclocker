//! What monitor-oc has installed, and where.
//!
//! Everything lives under /etc and /var because those are the directories
//! that survive SteamOS system updates and are writable on Fedora Atomic.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const STATE_DIR: &str = "/etc/monitor-oc";
const ENTRIES: &str = "/etc/monitor-oc/entries";
/// Extra firmware search path passed to the kernel via `firmware_class.path`.
pub const FIRMWARE_DIR: &str = "/etc/firmware";

pub const KEY_FIRMWARE_PATH: &str = "firmware_class.path";
pub const KEY_EDID_FIRMWARE: &str = "drm.edid_firmware";
pub const MANAGED_KEYS: [&str; 2] = [KEY_FIRMWARE_PATH, KEY_EDID_FIRMWARE];

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub connector: String,
    /// What the user asked for, e.g. `2560x1440@170`; informational.
    pub mode: String,
}

/// Firmware name relative to the firmware search path, as the kernel wants it.
pub fn firmware_name(connector: &str) -> String {
    format!("edid/monitor-oc-{connector}.bin")
}

pub fn firmware_path(connector: &str) -> PathBuf {
    Path::new(FIRMWARE_DIR).join(firmware_name(connector))
}

/// The monitor's untouched EDID, saved before the first override so later
/// runs never build on top of an already-modified one.
pub fn original_path(connector: &str) -> PathBuf {
    Path::new(STATE_DIR).join(format!("{connector}.original.bin"))
}

pub fn load() -> Vec<Entry> {
    fs::read_to_string(ENTRIES)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (connector, mode) = l.split_once('\t')?;
            Some(Entry {
                connector: connector.into(),
                mode: mode.into(),
            })
        })
        .collect()
}

pub fn save(entries: &[Entry]) -> Result<()> {
    fs::create_dir_all(STATE_DIR)?;
    let text: String = entries
        .iter()
        .map(|e| format!("{}\t{}\n", e.connector, e.mode))
        .collect();
    fs::write(ENTRIES, text).with_context(|| format!("writing {ENTRIES}"))
}

/// Kernel arguments that load every saved override at boot.
pub fn kernel_args(entries: &[Entry]) -> Vec<String> {
    if entries.is_empty() {
        return Vec::new();
    }
    let list: Vec<String> = entries
        .iter()
        .map(|e| format!("{}:{}", e.connector, firmware_name(&e.connector)))
        .collect();
    vec![
        format!("{KEY_FIRMWARE_PATH}={FIRMWARE_DIR}"),
        format!("{KEY_EDID_FIRMWARE}={}", list.join(",")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_for_two_monitors() {
        let e = |c: &str| Entry {
            connector: c.into(),
            mode: "x".into(),
        };
        assert_eq!(
            kernel_args(&[e("DP-1"), e("HDMI-A-1")]),
            vec![
                "firmware_class.path=/etc/firmware".to_string(),
                "drm.edid_firmware=DP-1:edid/monitor-oc-DP-1.bin,HDMI-A-1:edid/monitor-oc-HDMI-A-1.bin".into(),
            ]
        );
        assert!(kernel_args(&[]).is_empty());
    }
}
