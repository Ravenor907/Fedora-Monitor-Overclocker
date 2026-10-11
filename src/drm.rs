//! Kernel DRM connectors: discovery through sysfs and runtime EDID overrides
//! through debugfs.

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

const SYSFS: &str = "/sys/class/drm";
const DEBUGFS: &str = "/sys/kernel/debug/dri";

#[derive(Clone, Debug)]
pub struct Connector {
    pub card: u32,
    /// Kernel connector name, e.g. `DP-1` or `HDMI-A-1`.
    pub name: String,
    pub sys: PathBuf,
}

impl Connector {
    pub fn status(&self) -> String {
        read_trimmed(&self.sys.join("status")).unwrap_or_else(|| "unknown".into())
    }

    pub fn connected(&self) -> bool {
        self.status() == "connected"
    }

    pub fn edid(&self) -> Option<Vec<u8>> {
        fs::read(self.sys.join("edid"))
            .ok()
            .filter(|b| !b.is_empty())
    }

    /// Kernel driver bound to the GPU, e.g. `amdgpu`, `i915`, `nvidia`.
    pub fn driver(&self) -> Option<String> {
        let link = fs::read_link(Path::new(SYSFS).join(format!("card{}/device/driver", self.card)))
            .ok()?;
        Some(link.file_name()?.to_string_lossy().into_owned())
    }

    /// Built-in panels (laptop and handheld screens).
    pub fn is_internal(&self) -> bool {
        ["eDP", "LVDS", "DSI"]
            .iter()
            .any(|p| self.name.starts_with(p))
    }

    fn debugfs_dir(&self) -> Result<PathBuf> {
        let direct = Path::new(DEBUGFS)
            .join(self.card.to_string())
            .join(&self.name);
        if direct.join("edid_override").exists() {
            return Ok(direct);
        }
        if !Path::new(DEBUGFS).exists() {
            bail!(
                "{DEBUGFS} is not available; debugfs must be mounted \
                 (and not blocked by kernel lockdown / Secure Boot) for live testing"
            );
        }
        bail!(
            "no edid_override entry for {} under {DEBUGFS}; this driver may not support live EDID overrides",
            self.name
        )
    }

    /// Make the kernel use `edid` for this connector until reset or reboot.
    pub fn set_override(&self, edid: &[u8]) -> Result<()> {
        let path = self.debugfs_dir()?.join("edid_override");
        fs::write(&path, edid).with_context(|| format!("writing {}", path.display()))?;
        self.notify()
    }

    pub fn reset_override(&self) -> Result<()> {
        let path = self.debugfs_dir()?.join("edid_override");
        fs::write(&path, "reset").with_context(|| format!("writing {}", path.display()))?;
        self.notify()
    }

    /// Ask the kernel to re-read the EDID (picking up firmware overrides that
    /// were not loadable when the driver first probed, e.g. from the initramfs).
    pub fn reprobe(&self) -> Result<()> {
        let path = self.sys.join("status");
        fs::write(&path, "detect").with_context(|| format!("writing {}", path.display()))
    }

    /// Re-probe the connector and send a hotplug event so compositors
    /// (Gamescope, KWin, Mutter, ...) pick up the new mode list.
    fn notify(&self) -> Result<()> {
        if let Ok(dir) = self.debugfs_dir() {
            // amdgpu exposes a real simulated hotplug.
            if fs::write(dir.join("trigger_hotplug"), "1").is_ok() {
                return Ok(());
            }
        }
        // Generic fallback: toggling the forced state emits a hotplug uevent each time.
        let status = self.sys.join("status");
        fs::write(&status, "on").with_context(|| format!("writing {}", status.display()))?;
        fs::write(&status, "detect").with_context(|| format!("writing {}", status.display()))
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn connectors() -> Result<Vec<Connector>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(SYSFS).with_context(|| format!("reading {SYSFS}"))? {
        let entry = entry?;
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some((card, name)) = file.strip_prefix("card").and_then(|r| r.split_once('-')) else {
            continue;
        };
        let Ok(card) = card.parse() else { continue };
        if name.starts_with("Writeback") || !entry.path().join("status").exists() {
            continue;
        }
        out.push(Connector {
            card,
            name: name.to_string(),
            sys: entry.path(),
        });
    }
    out.sort_by(|a, b| (a.card, &a.name).cmp(&(b.card, &b.name)));
    Ok(out)
}

/// Look up a connector by `DP-1` or `card1-DP-1`. With several GPUs exposing
/// the same name, the connected one wins.
pub fn find(name: &str) -> Result<Connector> {
    let all = connectors()?;
    let mut matches: Vec<_> = all
        .iter()
        .filter(|c| c.name == name || format!("card{}-{}", c.card, c.name) == name)
        .cloned()
        .collect();
    matches.sort_by_key(|c| !c.connected());
    matches.into_iter().next().with_context(|| {
        let names: Vec<_> = all.iter().map(|c| c.name.as_str()).collect();
        format!(
            "no connector named '{name}' (available: {})",
            names.join(", ")
        )
    })
}

pub fn nvidia_loaded() -> bool {
    Path::new("/sys/module/nvidia_drm").exists()
}

/// True on Valve hardware (Steam Deck LCD "Jupiter" / OLED "Galileo").
pub fn is_steam_deck() -> bool {
    read_trimmed(Path::new("/sys/class/dmi/id/board_vendor")).as_deref() == Some("Valve")
}

/// Vendor and model from DMI, e.g. "ASUSTeK COMPUTER INC. ROG Ally X RC72LA".
pub fn hardware_name() -> Option<String> {
    let dmi =
        |f: &str| read_trimmed(&Path::new("/sys/class/dmi/id").join(f)).filter(|s| !s.is_empty());
    match (dmi("sys_vendor"), dmi("product_name")) {
        (Some(v), Some(p)) => Some(format!("{v} {p}")),
        (v, p) => v.or(p),
    }
}
