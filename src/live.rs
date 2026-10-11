//! Testing a built-in screen rate live in Game Mode, with an automatic way back.
//!
//! During a trial session Gamescope's profile holds untested rates back at the
//! stock maximum (see gamescope.lua). A test unlocks one rate for a short
//! deadline in the state file and asks Gamescope to switch to it through the
//! `GAMESCOPE_DYNAMIC_REFRESH` root property, the same one Steam's refresh
//! slider uses:
//!
//! 1. The rate is live for [`CONFIRM_SECS`]; no answer goes back to stock.
//! 2. Kept: it stays live, and after [`RECHECK_SECS`] it has to be kept again
//!    within [`CONFIRM_SECS`], or it goes back to stock. Then it is confirmed.
//!
//! The app does the switching. If the app dies or hangs mid-test, a watchdog
//! (`monitor-oc panel watchdog`, started outside the app's process tree)
//! notices the missed deadline and restarts Game Mode, which drops the
//! unconfirmed trial and brings the screen back at the stock range.

use crate::gamescope::{self, Paths, Report, State, TrialState};
use crate::x11;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// Seconds to say "keep" after a switch (and again at the double check).
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub const CONFIRM_SECS: u64 = 10;
/// Seconds after the first "keep" before the double check.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub const RECHECK_SECS: u64 = 300;
/// Extra time on the deadline for the switch itself and slow frames.
pub const GRACE_SECS: u64 = 4;

const REFRESH_PROP: &str = "GAMESCOPE_DYNAMIC_REFRESH";
const VRR_PROP: &str = "GAMESCOPE_VRR_ENABLED";
/// Gamescope waits 600 ms before acting on a new target rate.
const SWITCH_SETTLE: Duration = Duration::from_millis(1200);

/// What the root window had before a test, to put back afterwards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub refresh: Option<u32>,
    pub vrr: Option<u32>,
}

/// Switch Game Mode to `hz`. Gamescope only switches when the requested rate
/// changes, so go through the stock maximum first (that also regenerates the
/// mode, now unlocked). Variable refresh is turned off for the test, since
/// with it on Gamescope ignores the requested rate.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn switch_to(stock_max: u32, hz: u32) -> Result<Saved> {
    let mut x = x11::Display::open().context("connecting to Gamescope")?;
    let saved = Saved {
        refresh: x.get_root(REFRESH_PROP)?,
        vrr: x.get_root(VRR_PROP)?,
    };
    if saved.vrr.unwrap_or(0) != 0 {
        x.set_root(VRR_PROP, 0)?;
    }
    x.set_root(REFRESH_PROP, stock_max)?;
    thread::sleep(SWITCH_SETTLE);
    x.set_root(REFRESH_PROP, hz)?;
    Ok(saved)
}

/// Back to the stock maximum, then restore what Steam had set (with the test
/// over, Gamescope's profile holds any untested rate back at stock anyway).
pub fn switch_back(stock_max: u32, saved: Saved) -> Result<()> {
    let mut x = x11::Display::open().context("connecting to Gamescope")?;
    x.set_root(REFRESH_PROP, stock_max)?;
    thread::sleep(SWITCH_SETTLE);
    match saved.refresh {
        Some(hz) => x.set_root(REFRESH_PROP, hz)?,
        None => x.delete_root(REFRESH_PROP)?,
    }
    if let Some(vrr) = saved.vrr.filter(|v| *v != 0) {
        x.set_root(VRR_PROP, vrr)?;
    }
    Ok(())
}

/// Put back what Steam had set, keeping the current rate (after a keep).
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn restore(saved: Saved) -> Result<()> {
    let mut x = x11::Display::open().context("connecting to Gamescope")?;
    match saved.refresh {
        Some(hz) => x.set_root(REFRESH_PROP, hz)?,
        None => x.delete_root(REFRESH_PROP)?,
    }
    if let Some(vrr) = saved.vrr.filter(|v| *v != 0) {
        x.set_root(VRR_PROP, vrr)?;
    }
    Ok(())
}

/// Start the watchdog outside the app's process tree (Steam ends a game's
/// whole tree when it quits). `systemd-run --user` when there is one.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn spawn_watchdog() -> Result<()> {
    let exe = std::env::current_exe().context("locating monitor-oc")?;
    let mut env = Vec::new();
    for key in ["DISPLAY", "XAUTHORITY", "XDG_CONFIG_HOME", "HOME"] {
        if let Ok(v) = std::env::var(key) {
            env.push(format!("--setenv={key}={v}"));
        }
    }
    let systemd = Command::new("systemd-run")
        .args(["--user", "--collect", "--quiet"])
        .arg(format!("--unit=monitor-oc-watchdog-{}", std::process::id()))
        .args(&env)
        .arg(&exe)
        .args(["panel", "watchdog"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if systemd.is_ok_and(|s| s.success()) {
        return Ok(());
    }
    use std::os::unix::process::CommandExt;
    Command::new(&exe)
        .args(["panel", "watchdog"])
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the watchdog")?;
    Ok(())
}

fn alive(pid: u32) -> bool {
    pid == 0 || Path::new(&format!("/proc/{pid}")).exists()
}

/// What the watchdog should do about `state` at time `now`.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing is being tested any more: stop watching.
    Done,
    Wait,
    /// The test outlived its deadline or its app: take the rate back.
    Revert,
}

pub fn verdict(state: &State, now: u64, app_alive: bool) -> Verdict {
    let Some(live) = state.live else {
        return Verdict::Done;
    };
    if !matches!(state.trial, Some((_, TrialState::Running))) {
        return Verdict::Done;
    }
    if !app_alive || now > live.until + GRACE_SECS {
        Verdict::Revert
    } else {
        Verdict::Wait
    }
}

/// `monitor-oc panel watchdog`.
pub fn watchdog() -> Result<()> {
    let paths = Paths::for_current_user()?;
    loop {
        let state = State::load(&paths);
        let pid = state.live.map_or(0, |l| l.pid);
        match verdict(&state, gamescope::now_secs(), alive(pid)) {
            Verdict::Done => return Ok(()),
            Verdict::Wait => thread::sleep(Duration::from_secs(1)),
            Verdict::Revert => {
                gamescope::end_live(&paths)?;
                eprintln!("monitor-oc: the refresh test was not confirmed; going back to stock");
                // If the switch is possible (an app window has focus), it is
                // instant. A restart is the sure way: it drops the trial.
                let stock = Report::load(&paths).map_or(0, |r| r.stock_max);
                if stock > 0 {
                    let _ = switch_back(stock, Saved::default());
                }
                restart_game_mode();
                return Ok(());
            }
        }
    }
}

/// End the Game Mode session so it starts again (SteamOS logs straight back
/// into Game Mode). Steam first, the compositor if Steam doesn't go.
pub fn restart_game_mode() {
    let _ = Command::new("steam")
        .arg("-shutdown")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    for _ in 0..15 {
        thread::sleep(Duration::from_secs(1));
        if gamescope_pids().is_empty() {
            return;
        }
    }
    for pid in gamescope_pids() {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/// Gamescope processes of this user.
fn gamescope_pids() -> Vec<u32> {
    use std::os::unix::fs::MetadataExt;
    let Ok(me) = std::fs::metadata("/proc/self").map(|m| m.uid()) else {
        return Vec::new();
    };
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            if e.metadata().ok()?.uid() != me {
                return None;
            }
            let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
            matches!(comm.trim(), "gamescope" | "gamescope-wl").then_some(pid)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamescope::Live;

    fn running(live: Option<Live>) -> State {
        State {
            trial: Some((135, TrialState::Running)),
            live,
            ..State::default()
        }
    }

    #[test]
    fn watchdog_verdicts() {
        let live = Some(Live {
            hz: 135,
            until: 1000,
            pid: 1,
        });
        assert_eq!(verdict(&running(None), 0, true), Verdict::Done);
        assert_eq!(verdict(&running(live), 1000, true), Verdict::Wait);
        assert_eq!(
            verdict(&running(live), 1000 + GRACE_SECS, true),
            Verdict::Wait
        );
        assert_eq!(
            verdict(&running(live), 1001 + GRACE_SECS, true),
            Verdict::Revert
        );
        assert_eq!(verdict(&running(live), 900, false), Verdict::Revert);
        // Confirmed (trial gone) or dropped: nothing left to guard.
        let mut kept = running(live);
        kept.trial = None;
        assert_eq!(verdict(&kept, 5000, false), Verdict::Done);
    }
}
