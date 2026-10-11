//! External-monitor changes need root, so the app runs its own CLI through
//! pkexec (`pkexec /path/to/monitor-oc test ...`), streams the output and
//! answers `test`'s keep/revert prompt over stdin. The root process reverts on
//! its own when the time runs out, so a black screen recovers even if the app
//! is gone; closing the app closes stdin, which also reverts.

use super::data::{Backend, Event as DataEvent, Source};
use super::demo::World;
use crate::plan;
use crate::system;
use crate::timing::{Mode, Strategy};
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long `test` waits for each "keep" when run from the app.
pub const TEST_SECONDS: u64 = 10;
/// How long after the first "keep" `test` asks again.
pub const RECHECK_SECONDS: u64 = 300;

/// The line `monitor-oc test` prints once the new EDID is live.
const LIVE_MARKER: &str = "Live override active on";

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Test {
        connector: String,
        mode: Mode,
        strategy: Strategy,
        preferred: bool,
        /// Ask KDE to switch to the new mode once it is live.
        switch_mode: bool,
    },
    Apply {
        connector: String,
        mode: Mode,
        strategy: Strategy,
        preferred: bool,
    },
    /// Uninstall what `apply` installed (and any live override).
    Remove { connector: String },
    /// Drop a live override kept from a test (nothing installed).
    Reset { connector: String },
}

impl Kind {
    /// Arguments for the CLI.
    pub fn args(&self) -> Vec<String> {
        let target = |connector: &str, mode: &Mode, strategy: &Strategy, preferred: bool| {
            let mut args = vec![
                connector.to_string(),
                mode.to_string(),
                "--timing".into(),
                strategy.to_string(),
            ];
            if preferred {
                args.push("--preferred".into());
            }
            args
        };
        match self {
            Kind::Test {
                connector,
                mode,
                strategy,
                preferred,
                ..
            } => {
                let mut args = vec!["test".to_string()];
                args.extend(target(connector, mode, strategy, *preferred));
                args.extend([
                    "--seconds".into(),
                    TEST_SECONDS.to_string(),
                    "--recheck".into(),
                    RECHECK_SECONDS.to_string(),
                ]);
                args
            }
            Kind::Apply {
                connector,
                mode,
                strategy,
                preferred,
            } => {
                let mut args = vec!["apply".to_string()];
                args.extend(target(connector, mode, strategy, *preferred));
                args
            }
            Kind::Remove { connector } => vec!["remove".into(), connector.clone()],
            Kind::Reset { connector } => vec!["reset".into(), connector.clone()],
        }
    }

    /// The same change as a terminal command, for copying.
    pub fn command_line(&self) -> String {
        format!("sudo monitor-oc {}", self.args().join(" "))
    }

    pub fn title(&self) -> String {
        match self {
            Kind::Test {
                connector, mode, ..
            } => format!("Testing {mode} on {connector}"),
            Kind::Apply {
                connector, mode, ..
            } => format!("Installing {mode} on {connector}"),
            Kind::Remove { connector } => format!("Removing the override on {connector}"),
            Kind::Reset { connector } => format!("Dropping the live override on {connector}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Success,
    /// The password prompt was dismissed (pkexec exit 126).
    Cancelled,
    /// No polkit agent, not authorized, or the program was not found (127).
    NotAuthorized,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Line(String),
    /// The test mode is live; the countdown started.
    Live,
    /// Kept once; waiting for the double check.
    RecheckWait,
    /// The double check: keep again or it reverts.
    Recheck,
    Exit(Outcome),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Confirm,
    /// Kept once, until the double check.
    Waiting,
    Recheck,
}

pub struct Job {
    pub id: u64,
    pub kind: Kind,
    pub live_since: Option<Instant>,
    /// Which question the test is at, and since when.
    pub phase: Phase,
    /// The user already answered keep/revert.
    pub answered: Option<bool>,
    pub outcome: Option<Outcome>,
    input: Sender<String>,
}

impl Job {
    pub fn running(&self) -> bool {
        self.outcome.is_none()
    }

    /// Seconds before the root process reverts (or asks again) on its own.
    pub fn seconds_left(&self) -> Option<u64> {
        let since = self.live_since?;
        let total = match self.phase {
            Phase::Confirm | Phase::Recheck => TEST_SECONDS,
            Phase::Waiting => RECHECK_SECONDS,
        };
        Some(total.saturating_sub(since.elapsed().as_secs()))
    }

    /// Move to the next question; its countdown starts now.
    pub fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.live_since = Some(Instant::now());
        self.answered = None;
    }

    /// Answer `test`'s current prompt: keep, or revert now.
    pub fn answer(&mut self, keep: bool) {
        if self.answered.is_none() {
            self.answered = Some(keep);
            let _ = self.input.send(if keep { "keep\n" } else { "\n" }.into());
        }
    }
}

/// Start `kind` in the background; events arrive through the backend.
pub fn start(backend: &Backend, id: u64, kind: Kind) -> Job {
    let emit = {
        let backend = backend.clone();
        move |event: Event| backend.send(DataEvent::Job { id, event })
    };
    let (input, rx) = mpsc::channel::<String>();
    match &backend.source {
        Source::Demo(world) => {
            let (world, kind) = (world.clone(), kind.clone());
            thread::spawn(move || simulate(&kind, &world, rx, emit));
        }
        Source::Live => {
            if let Err(e) = spawn(&kind, rx, emit.clone()) {
                emit(Event::Exit(Outcome::Failed(format!("{e:#}"))));
            }
        }
    }
    Job {
        id,
        kind,
        live_since: None,
        phase: Phase::Confirm,
        answered: None,
        outcome: None,
        input,
    }
}

fn spawn(
    kind: &Kind,
    input: mpsc::Receiver<String>,
    emit: impl Fn(Event) + Clone + Send + 'static,
) -> Result<()> {
    let exe = std::env::current_exe().context("locating the monitor-oc binary")?;
    let mut cmd = if system::is_root() {
        Command::new(&exe)
    } else {
        let mut cmd = Command::new("pkexec");
        cmd.arg(&exe);
        cmd
    };
    cmd.args(kind.args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let shown =
        kind.command_line()
            .replacen("sudo ", if system::is_root() { "" } else { "pkexec " }, 1);
    emit(Event::Line(format!("$ {shown}")));
    let mut child = cmd.spawn().context("starting pkexec")?;

    let mut stdin = child.stdin.take().context("no stdin")?;
    thread::spawn(move || {
        for line in input {
            if stdin
                .write_all(line.as_bytes())
                .and_then(|_| stdin.flush())
                .is_err()
            {
                break;
            }
        }
    });
    let readers = [
        forward(child.stdout.take(), emit.clone()),
        forward(child.stderr.take(), emit.clone()),
    ];
    thread::spawn(move || {
        let status = child.wait();
        for reader in readers {
            let _ = reader.join();
        }
        emit(Event::Exit(match status {
            Ok(s) if s.success() => Outcome::Success,
            Ok(s) if s.code() == Some(126) => Outcome::Cancelled,
            Ok(s) if s.code() == Some(127) => Outcome::NotAuthorized,
            Ok(s) => Outcome::Failed(format!("exited with {s}")),
            Err(e) => Outcome::Failed(e.to_string()),
        }));
    });
    Ok(())
}

fn forward(
    pipe: Option<impl Read + Send + 'static>,
    emit: impl Fn(Event) + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Some(pipe) = pipe else { return };
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            let event = if line.starts_with(LIVE_MARKER) {
                Some(Event::Live)
            } else if line.starts_with(crate::RECHECK_WAIT_MARKER) {
                Some(Event::RecheckWait)
            } else if line.starts_with(crate::RECHECK_MARKER) {
                Some(Event::Recheck)
            } else {
                None
            };
            emit(Event::Line(line));
            if let Some(event) = event {
                emit(event);
            }
        }
    })
}

/// Ask KDE (Plasma Desktop, SteamOS Desktop Mode, Bazzite KDE) to use the
/// new mode once the kernel lists it. Elsewhere the user picks it.
pub fn switch_mode_kde(backend: &Backend, id: u64, connector: &str, mode: &Mode) {
    let backend = backend.clone();
    let arg = format!(
        "output.{connector}.mode.{}x{}@{}",
        mode.width,
        mode.height,
        mode.hz.round()
    );
    thread::spawn(move || {
        // Give KWin a moment to process the hotplug.
        thread::sleep(Duration::from_millis(1500));
        let line = match Command::new("kscreen-doctor").arg(&arg).output() {
            Ok(out) if out.status.success() => format!("kscreen-doctor {arg}: switched"),
            Ok(out) => format!(
                "kscreen-doctor {arg} failed ({}); pick the mode in System Settings > Display Configuration",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => format!("could not run kscreen-doctor: {e}"),
        };
        backend.send(DataEvent::Job {
            id,
            event: Event::Line(line),
        });
    });
}

/// The demo's stand-in for the root process: same output, nothing written.
fn simulate(
    kind: &Kind,
    world: &Arc<Mutex<World>>,
    input: mpsc::Receiver<String>,
    emit: impl Fn(Event),
) {
    let pause = |ms| thread::sleep(Duration::from_millis(ms));
    let say = |text: &str| emit(Event::Line(text.to_string()));
    say(&format!(
        "$ {}",
        kind.command_line().replacen("sudo ", "pkexec ", 1)
    ));
    pause(900);
    match kind {
        Kind::Test {
            connector,
            mode,
            strategy,
            preferred,
            ..
        } => {
            plan_lines(connector, mode, *strategy, *preferred, &say);
            say(&format!(
                "\n{LIVE_MARKER} {connector}. Now pick {:.0} Hz in your display settings.",
                mode.hz
            ));
            emit(Event::Live);
            world.lock().unwrap().set_override(true);
            let kept = |input: &mpsc::Receiver<String>| {
                matches!(
                    input.recv_timeout(Duration::from_secs(TEST_SECONDS)),
                    Ok(line) if line.trim() == "keep"
                )
            };
            let mut keep = kept(&input);
            if keep {
                say(&format!(
                    "{} {RECHECK_SECONDS} s: use it normally.",
                    crate::RECHECK_WAIT_MARKER
                ));
                emit(Event::RecheckWait);
                keep = match input.recv_timeout(Duration::from_secs(RECHECK_SECONDS)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        say(&format!(
                            "{} still good? Type 'keep' within {TEST_SECONDS} s.",
                            crate::RECHECK_MARKER
                        ));
                        emit(Event::Recheck);
                        kept(&input)
                    }
                    _ => false,
                };
            }
            if keep {
                say(&format!(
                    "Kept until reboot. Make it permanent with:\n  sudo monitor-oc apply {connector} {mode}"
                ));
            } else {
                world.lock().unwrap().set_override(false);
                say("Reverted to the monitor's own EDID.");
            }
        }
        Kind::Apply {
            connector,
            mode,
            strategy,
            preferred,
        } => {
            plan_lines(connector, mode, *strategy, *preferred, &say);
            for line in [
                "\nBoot setup: SteamOS GRUB (/etc/default/grub + update-grub)".to_string(),
                format!("wrote /etc/firmware/edid/monitor-oc-{connector}.bin"),
                "updated /etc/default/grub: GRUB_CMDLINE_LINUX_DEFAULT=\"... firmware_class.path=/etc/firmware drm.edid_firmware=...\"".into(),
                "+ steamos-readonly disable".into(),
                "+ update-grub".into(),
                "+ steamos-readonly enable".into(),
                "+ systemctl daemon-reload".into(),
                "+ systemctl enable monitor-oc.service".into(),
                "\nDone. Reboot to load the new EDID at boot.".into(),
            ] {
                pause(250);
                say(&line);
            }
            world.lock().unwrap().set_installed(Some(mode.to_string()));
        }
        Kind::Remove { connector } => {
            say(&format!("removed override for {connector}"));
            pause(300);
            say("+ systemctl disable monitor-oc.service");
            say("Reboot to finish.");
            let mut world = world.lock().unwrap();
            world.set_installed(None);
            world.set_override(false);
        }
        Kind::Reset { connector } => {
            say(&format!("Live override on {connector} removed."));
            world.lock().unwrap().set_override(false);
        }
    }
    emit(Event::Exit(Outcome::Success));
}

/// The first lines `test` and `apply` print, from the same plan.
fn plan_lines(
    connector: &str,
    mode: &Mode,
    strategy: Strategy,
    preferred: bool,
    say: &impl Fn(&str),
) {
    let edid = super::demo::monitor_edid();
    match plan::build(&edid, mode, strategy, preferred) {
        Ok(p) => {
            say(&format!("Requested:  {mode}"));
            say(&format!("Timing:     {} ({})", p.dtd, p.strategy));
            say(&format!("Placed in:  {}", p.location));
            for change in &p.limit_changes {
                say(&format!("Range:      {change}"));
            }
            for note in p.notes(mode, Some(connector)) {
                say(&format!("Note:       {note}"));
            }
        }
        Err(e) => say(&format!("error: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_arguments() {
        let kind = Kind::Test {
            connector: "DP-1".into(),
            mode: "2560x1440@165".parse().unwrap(),
            strategy: Strategy::CvtRb2,
            preferred: true,
            switch_mode: false,
        };
        assert_eq!(
            kind.args().join(" "),
            "test DP-1 2560x1440@165 --timing cvt-rb2 --preferred --seconds 10 --recheck 300"
        );
        assert_eq!(
            Kind::Remove {
                connector: "HDMI-A-1".into()
            }
            .command_line(),
            "sudo monitor-oc remove HDMI-A-1"
        );
    }
}
