//! A boot-time oneshot that keeps overrides working across SteamOS updates and
//! early-boot firmware misses.

use crate::system::{self, Os};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const UNIT_NAME: &str = "monitor-oc.service";
const UNIT_PATH: &str = "/etc/systemd/system/monitor-oc.service";

/// Where the binary is copied so the service can run it. SteamOS's /usr is
/// replaced on every update, so use /var there; elsewhere /usr/local/bin is
/// writable (also on Fedora Atomic) and correctly labelled for SELinux.
fn binary_path(os: &Os) -> PathBuf {
    if os.is_steamos() {
        PathBuf::from("/var/lib/monitor-oc/monitor-oc")
    } else {
        PathBuf::from("/usr/local/bin/monitor-oc")
    }
}

fn unit_text(binary: &Path) -> String {
    format!(
        "[Unit]
Description=Re-apply monitor-oc EDID overrides
After=systemd-modules-load.service systemd-udev-settle.service
Before=display-manager.service

[Service]
Type=oneshot
ExecStart={} reapply --boot

[Install]
WantedBy=multi-user.target
",
        binary.display()
    )
}

pub fn install(os: &Os, dry_run: bool) -> Result<()> {
    let target = binary_path(os);
    if dry_run {
        println!("would install {} and enable {UNIT_NAME}", target.display());
        return Ok(());
    }
    let current = std::env::current_exe().context("locating the monitor-oc binary")?;
    if current != target {
        fs::create_dir_all(target.parent().unwrap())?;
        // Copy to a temp name and rename so a running copy is never truncated.
        let tmp = target.with_extension("new");
        fs::copy(&current, &tmp).with_context(|| format!("copying to {}", tmp.display()))?;
        fs::rename(&tmp, &target)?;
        if system::which("restorecon") {
            let _ = system::run(&["restorecon", &target.to_string_lossy()], false);
        }
    }
    fs::write(UNIT_PATH, unit_text(&target)).with_context(|| format!("writing {UNIT_PATH}"))?;
    system::run(&["systemctl", "daemon-reload"], false)?;
    system::run(&["systemctl", "enable", UNIT_NAME], false)
}

pub fn uninstall(os: &Os, dry_run: bool) -> Result<()> {
    if dry_run {
        println!(
            "would disable {UNIT_NAME} and remove {}",
            binary_path(os).display()
        );
        return Ok(());
    }
    if Path::new(UNIT_PATH).exists() {
        system::run(&["systemctl", "disable", UNIT_NAME], false)?;
        fs::remove_file(UNIT_PATH)?;
        system::run(&["systemctl", "daemon-reload"], false)?;
    }
    let _ = fs::remove_file(binary_path(os));
    Ok(())
}

pub fn installed() -> bool {
    Path::new(UNIT_PATH).exists()
}
