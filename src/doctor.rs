//! Health checks shared by `monitor-oc doctor` and the GUI's Diagnostics page.

use crate::drm;
use crate::gamescope::{self, Paths, TrialState};
use crate::state;
use crate::system::{self, Boot, Os};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

#[derive(Clone, Debug)]
pub struct Check {
    pub title: String,
    pub level: Level,
    pub detail: String,
}

fn check(title: &str, level: Level, detail: impl Into<String>) -> Check {
    Check {
        title: title.into(),
        level,
        detail: detail.into(),
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(then: u64) -> String {
    let secs = now_secs().saturating_sub(then);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}

fn modified_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

pub fn checks() -> Vec<Check> {
    let mut out = Vec::new();
    let os = Os::detect();
    out.push(check("System", Level::Info, os.pretty_name.clone()));
    if let Some(hw) = drm::hardware_name() {
        out.push(check("Hardware", Level::Info, hw));
    }

    let connectors = drm::connectors().unwrap_or_default();
    let drivers: BTreeSet<String> = connectors.iter().filter_map(|c| c.driver()).collect();
    if drivers.contains("nvidia") || (drivers.is_empty() && drm::nvidia_loaded()) {
        out.push(check(
            "GPU driver",
            Level::Warn,
            "NVIDIA's proprietary driver ignores kernel EDID overrides; use nvidia-settings for external monitors",
        ));
    } else if drivers.is_empty() {
        out.push(check(
            "GPU driver",
            Level::Fail,
            "no display connectors found in /sys/class/drm",
        ));
    } else {
        let list: Vec<_> = drivers.into_iter().collect();
        out.push(check("GPU driver", Level::Ok, list.join(", ")));
    }

    let boot = Boot::detect(&os);
    out.push(check(
        "Boot setup",
        if boot == Boot::Manual {
            Level::Warn
        } else {
            Level::Ok
        },
        if boot == Boot::Manual {
            "unknown bootloader: external-monitor overrides print kernel arguments to add by hand"
                .to_string()
        } else {
            boot.describe()
        },
    ));

    match fs::read_to_string("/sys/kernel/security/lockdown") {
        Ok(text) if text.contains("[none]") => {
            out.push(check("Kernel lockdown", Level::Ok, "off; live testing of external monitors works"))
        }
        Ok(text) => out.push(check(
            "Kernel lockdown",
            Level::Warn,
            format!(
                "{} (Secure Boot): live testing of external monitors is blocked; Apply + reboot still works",
                text.trim()
            ),
        )),
        Err(_) => {}
    }

    panel_checks(&mut out);
    external_checks(&mut out, &connectors);
    out
}

fn panel_checks(out: &mut Vec<Check>) {
    let has_gamescope = gamescope::gamescope_installed();
    out.push(check(
        "Gamescope",
        Level::Info,
        match (has_gamescope, gamescope::in_game_mode()) {
            (true, true) => "installed; this is a Game Mode session",
            (true, false) => "installed; this is a desktop session",
            (false, _) => "not installed (no Game Mode)",
        },
    ));

    let panel = match gamescope::Panel::detect() {
        Ok(p) => p,
        Err(_) => return,
    };
    out.push(check(
        "Built-in screen",
        Level::Info,
        format!(
            "{}: {} {}x{}",
            panel.connector,
            panel.id.label(),
            panel.width,
            panel.height
        ),
    ));
    if !has_gamescope {
        return;
    }
    let stock = gamescope::stock_profile(&panel.id);
    out.push(check(
        "Stock Game Mode range",
        Level::Info,
        match &stock {
            Some(s) => format!(
                "{}-{} Hz ({})",
                s.min_hz,
                s.max_hz,
                s.pretty_name
                    .clone()
                    .unwrap_or_else(|| s.file.display().to_string())
            ),
            None => format!(
                "no Gamescope profile for this panel; Game Mode uses the EDID rates ({})",
                panel
                    .edid_rates
                    .iter()
                    .map(|r| format!("{r} Hz"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    ));

    let Ok(paths) = Paths::for_current_user() else {
        out.push(check(
            "Built-in overclock",
            Level::Info,
            "run the GUI or `monitor-oc doctor` as your normal user to see Game Mode settings",
        ));
        return;
    };
    if !gamescope::installed(&paths) {
        out.push(check("Built-in overclock", Level::Info, "not set up"));
        return;
    }
    let st = gamescope::State::load(&paths);
    let mut summary = if st.max > 0 {
        format!("confirmed up to {} Hz", st.max)
    } else {
        "stock range".into()
    };
    let mut level = Level::Ok;
    if let Some((hz, trial)) = st.trial {
        summary += &match trial {
            TrialState::Armed => format!("; {hz} Hz trial waiting for the next Game Mode start"),
            TrialState::Running => format!("; {hz} Hz trial active, confirm it if it worked"),
            TrialState::Expired => {
                level = Level::Warn;
                format!(
                    "; {hz} Hz trial ended without confirmation (Game Mode restarted), so it was turned off"
                )
            }
        };
    }
    out.push(check("Built-in overclock", level, summary));

    match gamescope::Report::load(&paths) {
        None => out.push(check(
            "Gamescope profile",
            Level::Warn,
            "Gamescope has not loaded monitor-oc's profile yet; switch to Game Mode",
        )),
        Some(r) => {
            let stale = match (
                r.loaded_at,
                modified_secs(&paths.state).max(modified_secs(&paths.script)),
            ) {
                (Some(loaded), Some(changed)) => changed > loaded,
                _ => false,
            };
            let when = r
                .loaded_at
                .map(ago)
                .unwrap_or_else(|| "at an unknown time".into());
            let mut detail = if r.extended {
                format!("loaded {when}; extended to {} Hz", r.limit)
            } else {
                format!(
                    "loaded {when}; stock range ({}-{} Hz)",
                    r.stock_min, r.stock_max
                )
            };
            if let Some(mode) = &r.last_mode {
                detail += &format!("; last mode {mode}");
                if let Some(khz) = r.last_clock_khz {
                    detail += &format!(" at {:.1} MHz", khz as f64 / 1000.0);
                }
            }
            if stale {
                detail += "; settings changed since, restart Game Mode to load them";
            }
            out.push(check(
                "Gamescope profile",
                if stale { Level::Warn } else { Level::Ok },
                detail,
            ));
        }
    }
}

fn external_checks(out: &mut Vec<Check>, connectors: &[drm::Connector]) {
    let entries = state::load();
    if entries.is_empty() {
        return;
    }
    let cmdline = system::cmdline();
    let active = state::kernel_args(&entries)
        .iter()
        .all(|a| cmdline.split_whitespace().any(|c| c == a));
    out.push(check(
        "Kernel arguments",
        if active { Level::Ok } else { Level::Warn },
        if active {
            "active this boot"
        } else {
            "not active this boot; reboot after Apply"
        },
    ));
    for entry in &entries {
        let wanted = fs::read(state::firmware_path(&entry.connector)).ok();
        let conn = connectors.iter().find(|c| c.name == entry.connector);
        let live = conn.and_then(|c| c.edid());
        let (level, detail) = match (&wanted, &live) {
            (None, _) => (
                Level::Fail,
                "override file missing; Apply again".to_string(),
            ),
            (Some(w), Some(l)) if w == l => (Level::Ok, format!("{} active", entry.mode)),
            (Some(_), Some(_)) => (
                Level::Warn,
                format!("{} installed but not active", entry.mode),
            ),
            (Some(_), None) => (
                Level::Info,
                format!("{} installed; monitor not connected", entry.mode),
            ),
        };
        let level = if conn.is_some_and(|c| c.is_internal()) && gamescope::gamescope_installed() {
            out.push(check(
                &entry.connector,
                Level::Warn,
                "kernel EDID overrides do not change Game Mode's range on built-in screens; use the Built-in screen page",
            ));
            Level::Info
        } else {
            level
        };
        out.push(check(&entry.connector, level, detail));
    }
}
