//! Made-up hardware for `monitor-oc gui --demo` (or `MONITOR_OC_DEMO=1`): a
//! ROG Ally X running SteamOS with a 1440p 144 Hz monitor on DisplayPort.
//! Everything happens in memory; nothing on the system is read or written.

use super::data::{Monitor, PanelAction, PanelInfo, Settings, Snapshot, Tools, panel_info};
use crate::doctor::{Check, Level, ago, now_secs};
use crate::edid::{Dtd, Edid};
use crate::gamescope::{self, Method, Panel, PanelId, Report, State, StockProfile, TrialState};
use crate::timing;
use anyhow::{Result, bail};
use std::path::PathBuf;

const STOCK_MIN: u32 = 48;
const STOCK_MAX: u32 = 120;
const PROFILE_NAME: &str = "ASUS ROG Ally / ROG Ally X LCD";

pub struct World {
    state: State,
    profile_installed: bool,
    report: Option<Report>,
    /// Settings changed after the pretend Gamescope last loaded them.
    changed: bool,
    monitor_overridden: bool,
    monitor_installed: Option<String>,
}

impl World {
    /// A 135 Hz trial that is running in the current Game Mode session, loaded
    /// a couple of minutes ago.
    pub fn new() -> World {
        let mut world = World {
            state: State {
                max: 0,
                method: Method::Clock,
                trial: Some((135, TrialState::Armed)),
                live: None,
            },
            profile_installed: true,
            report: None,
            changed: false,
            monitor_overridden: false,
            monitor_installed: None,
        };
        world.load_profile(now_secs().saturating_sub(130));
        world
    }

    /// What Gamescope does with monitor-oc's profile when it starts (see
    /// gamescope.lua): advance the trial and record a report.
    fn load_profile(&mut self, at: u64) {
        let mut limit = self.state.max;
        let mut trial = None;
        if let Some((hz, ts)) = self.state.trial {
            let next = match ts {
                TrialState::Armed => {
                    limit = limit.max(hz);
                    TrialState::Running
                }
                TrialState::Running | TrialState::Expired => TrialState::Expired,
            };
            self.state.trial = Some((hz, next));
            trial = Some(if next == TrialState::Running {
                "running"
            } else {
                "expired"
            });
        }
        let extended = limit > STOCK_MAX;
        let top = if extended { limit } else { STOCK_MAX };
        self.report = Some(Report {
            loaded_at: Some(at),
            base: Some("rogally_lcd".into()),
            base_name: Some(PROFILE_NAME.into()),
            stock_min: STOCK_MIN,
            stock_max: STOCK_MAX,
            limit,
            extended,
            trial: trial.map(String::from),
            last_mode: Some(format!("1920x1080@{top}")),
            // The Ally X panel's 2080x1111 total, scaled like the "clock" method.
            last_clock_khz: Some(2080 * 1111 * top as u64 / 1000),
        });
        self.changed = false;
    }

    /// Pretend Game Mode started again (the demo's "Switch to Game Mode").
    pub fn restart_game_mode(&mut self) -> String {
        if !self.profile_installed {
            return "Demo: Game Mode restarted with the stock range.".into();
        }
        let before = self.state.trial;
        self.load_profile(now_secs());
        match (before, self.state.trial) {
            (Some((hz, TrialState::Armed)), _) => {
                format!("Demo: Game Mode restarted; the {hz} Hz trial is now running.")
            }
            (Some((hz, TrialState::Running)), _) => format!(
                "Demo: Game Mode restarted before {hz} Hz was confirmed, so the trial expired."
            ),
            _ => "Demo: Game Mode restarted and loaded your settings.".into(),
        }
    }

    pub fn panel_action(&mut self, action: PanelAction) -> Result<String> {
        let text = match action {
            PanelAction::Try { hz, method } => {
                if hz <= STOCK_MAX {
                    bail!(
                        "{hz} Hz is within the stock range (up to {STOCK_MAX} Hz); pick something higher"
                    );
                }
                if hz > gamescope::max_allowed(STOCK_MAX) {
                    bail!(
                        "monitor-oc stops at {} Hz",
                        gamescope::max_allowed(STOCK_MAX)
                    );
                }
                self.state.method = method;
                self.state.trial = Some((hz, TrialState::Armed));
                self.profile_installed = true;
                format!("Trial armed: up to {hz} Hz for the next Game Mode session only.")
            }
            PanelAction::Confirm => match self.state.trial {
                None => bail!("there is no trial to confirm"),
                Some((hz, TrialState::Armed)) => {
                    bail!("the {hz} Hz trial has not run yet; start Game Mode and test it first")
                }
                Some((hz, _)) => {
                    self.state.max = hz;
                    self.state.trial = None;
                    format!("Kept: Game Mode now offers up to {hz} Hz on the built-in screen.")
                }
            },
            PanelAction::Discard => {
                self.state.trial = None;
                "Trial discarded.".into()
            }
            PanelAction::Reset => {
                self.state = State {
                    method: self.state.method,
                    ..State::default()
                };
                "Back to the stock range from the next Game Mode start.".into()
            }
            PanelAction::Remove => {
                self.state = State::default();
                self.profile_installed = false;
                self.report = None;
                "Removed monitor-oc's Gamescope profile; restart Game Mode to finish.".into()
            }
        };
        self.changed = true;
        Ok(text)
    }

    /// Same rule as [`Report::is_stale`], with "changed" tracked in memory.
    fn stale(&self) -> bool {
        self.changed && self.report.as_ref().is_some_and(|r| !r.serves(&self.state))
    }

    pub fn set_override(&mut self, active: bool) {
        self.monitor_overridden = active;
    }

    pub fn set_installed(&mut self, mode: Option<String>) {
        self.monitor_installed = mode;
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut info = panel_info(ally_panel(), Some(stock_profile()), true);
        info.settings = Ok(Settings {
            state: self.state.clone(),
            report: self.report.clone(),
            stale: self.stale(),
            installed: self.profile_installed,
        });
        let checks = self.checks(&info);
        Snapshot {
            hardware: Some("ASUSTeK COMPUTER INC. ROG Ally X RC72LA".into()),
            os: "SteamOS 3.7.13".into(),
            panel: Ok(info),
            monitors: vec![Monitor {
                connector: "DP-1".into(),
                driver: Some("amdgpu".into()),
                edid: monitor_edid(),
                overridden: self.monitor_overridden,
                installed: self.monitor_installed.clone(),
            }],
            checks,
            tools: Tools {
                pkexec: true,
                sudo: true,
                root: false,
                kscreen_doctor: false,
                session_select: true,
            },
        }
    }

    fn checks(&self, info: &PanelInfo) -> Vec<Check> {
        let check = |title: &str, level, detail: String| Check {
            title: title.into(),
            level,
            detail,
        };
        let mut out = vec![
            check("System", Level::Info, "SteamOS 3.7.13".into()),
            check(
                "Hardware",
                Level::Info,
                "ASUSTeK COMPUTER INC. ROG Ally X RC72LA".into(),
            ),
            check("GPU driver", Level::Ok, "amdgpu".into()),
            check(
                "Boot setup",
                Level::Ok,
                "SteamOS GRUB (/etc/default/grub + update-grub)".into(),
            ),
            check(
                "Kernel lockdown",
                Level::Ok,
                "off; live testing of external monitors works".into(),
            ),
            check(
                "Gamescope",
                Level::Info,
                if gamescope::in_game_mode() {
                    "installed; this is a Game Mode session"
                } else {
                    "installed; this is a desktop session"
                }
                .into(),
            ),
            check(
                "Built-in screen",
                Level::Info,
                format!(
                    "{}: {} {}x{}",
                    info.panel.connector,
                    info.panel.id.label(),
                    info.panel.width,
                    info.panel.height
                ),
            ),
            check(
                "Stock Game Mode range",
                Level::Info,
                format!("{STOCK_MIN}-{STOCK_MAX} Hz ({PROFILE_NAME})"),
            ),
        ];
        if !self.profile_installed {
            out.push(check(
                "Built-in overclock",
                Level::Info,
                "not set up".into(),
            ));
            return out;
        }
        let mut summary = if self.state.max > 0 {
            format!("confirmed up to {} Hz", self.state.max)
        } else {
            "stock range".into()
        };
        let mut level = Level::Ok;
        if let Some((hz, trial)) = self.state.trial {
            summary += &match trial {
                TrialState::Armed => {
                    format!("; {hz} Hz trial waiting for the next Game Mode start")
                }
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
        if let Some(r) = &self.report {
            let mut detail = format!("loaded {}; ", r.loaded_at.map(ago).unwrap_or_default());
            detail += &if r.extended {
                format!("extended to {} Hz", r.limit)
            } else {
                format!("stock range ({}-{} Hz)", r.stock_min, r.stock_max)
            };
            if let (Some(mode), Some(khz)) = (&r.last_mode, r.last_clock_khz) {
                detail += &format!("; last mode {mode} at {:.1} MHz", khz as f64 / 1000.0);
            }
            if self.stale() {
                detail += "; settings changed since, restart Game Mode to load them";
            }
            out.push(check(
                "Gamescope profile",
                if self.stale() { Level::Warn } else { Level::Ok },
                detail,
            ));
        }
        out
    }
}

pub fn ally_panel() -> Panel {
    Panel {
        connector: "eDP-1".into(),
        id: PanelId {
            vendor: "TMX".into(),
            product: 2,
            model: "TL070FVXS01-0".into(),
            data_string: String::new(),
        },
        width: 1920,
        height: 1080,
        edid_rates: vec![STOCK_MAX],
    }
}

fn stock_profile() -> StockProfile {
    StockProfile {
        file: PathBuf::from(
            "/usr/share/gamescope/scripts/00-gamescope/displays/asus.rogally.lcd.lua",
        ),
        pretty_name: Some(PROFILE_NAME.into()),
        min_hz: STOCK_MIN,
        max_hz: STOCK_MAX,
    }
}

/// A 27" 2560x1440 144 Hz monitor with a 1080p 120 Hz extra mode, built
/// the same way as the EDID tests' sample.
pub fn monitor_edid() -> Edid {
    let mut b = vec![0u8; 256];
    b[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    b[8..10].copy_from_slice(&0x1E6Du16.to_be_bytes()); // "GSM"
    b[10..12].copy_from_slice(&0x5BEEu16.to_le_bytes());
    b[18] = 1;
    b[19] = 4;
    let native = Dtd {
        clock: 59_615,
        h_active: 2560,
        h_blank: 160,
        h_front: 48,
        h_sync: 32,
        v_active: 1440,
        v_blank: 82,
        v_front: 3,
        v_sync: 5,
        h_size_mm: 597,
        v_size_mm: 336,
        h_border: 0,
        v_border: 0,
        flags: 0x1A,
    };
    b[54..72].copy_from_slice(&native.encode().expect("valid timing"));
    // Range limits: 48-144 Hz, 30-230 kHz, 600 MHz.
    b[72..90].copy_from_slice(&[
        0, 0, 0, 0xFD, 0, 48, 144, 30, 230, 60, 1, 0x0A, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    ]);
    let mut name = [0x20u8; 18];
    name[..5].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
    name[5..18].copy_from_slice(b"LG ULTRAGEAR\n");
    b[90..108].copy_from_slice(&name);
    b[108..126].copy_from_slice(&[0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b[126] = 1;
    b[128] = 0x02;
    b[129] = 3;
    b[130] = 4;
    let fhd = timing::cvt_rb(&"1920x1080@120".parse().expect("valid mode"), false)
        .and_then(|d| d.encode())
        .expect("valid timing");
    b[132..150].copy_from_slice(&fhd);
    let mut edid = Edid { bytes: b };
    edid.fix_checksums();
    edid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_edid_is_valid() {
        let edid = Edid::parse(&monitor_edid().bytes).unwrap();
        assert_eq!(edid.name().as_deref(), Some("LG ULTRAGEAR"));
        let timings = edid.timings();
        assert_eq!(timings.len(), 2);
        assert!((timings[0].1.refresh_hz() - 144.0).abs() < 0.1);
    }

    #[test]
    fn demo_trial_follows_gamescope_rules() {
        let mut w = World::new();
        assert_eq!(w.state.trial, Some((135, TrialState::Running)));
        w.panel_action(PanelAction::Confirm).unwrap();
        assert_eq!((w.state.max, w.state.trial), (135, None));
        w.panel_action(PanelAction::Try {
            hz: 150,
            method: Method::Porch,
        })
        .unwrap();
        assert!(w.panel_action(PanelAction::Confirm).is_err());
        w.restart_game_mode();
        assert_eq!(w.state.trial, Some((150, TrialState::Running)));
        w.restart_game_mode();
        assert_eq!(w.state.trial, Some((150, TrialState::Expired)));
        assert!(
            w.panel_action(PanelAction::Try {
                hz: 120,
                method: Method::Clock
            })
            .is_err()
        );
    }
}
