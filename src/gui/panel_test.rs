//! The app side of a live built-in screen test in Game Mode: switch, 10 s to
//! keep, a double check after 5 minutes, and the way back (see `crate::live`).

use crate::gamescope::{self, Paths};
use crate::live::{self, CONFIRM_SECS, GRACE_SECS, RECHECK_SECS, Saved};
use eframe::egui;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Asking Gamescope for the new rate.
    Switching,
    /// First answer: keep or go back.
    Confirm,
    /// Kept for now; the double check comes at the end.
    Waiting,
    /// Second answer, after five minutes.
    Recheck,
    /// Going back or putting Steam's settings back.
    Finishing,
}

#[derive(Debug)]
pub enum Outcome {
    Kept(u32),
    Reverted { hz: u32, why: String },
}

pub struct PanelTest {
    pub hz: u32,
    pub stock_max: u32,
    pub phase: Phase,
    since: Instant,
    saved: Saved,
    /// None in the demo: nothing on the system changes.
    paths: Option<Paths>,
    switched: Option<Receiver<Result<Saved, String>>>,
    finished: Option<Receiver<Outcome>>,
    ctx: egui::Context,
}

impl PanelTest {
    pub fn start(
        ctx: &egui::Context,
        hz: u32,
        stock_max: u32,
        demo: bool,
    ) -> Result<PanelTest, String> {
        let paths = if demo {
            None
        } else {
            let paths = Paths::for_current_user().map_err(|e| format!("{e:#}"))?;
            gamescope::start_live(&paths, hz, CONFIRM_SECS + GRACE_SECS, std::process::id())
                .map_err(|e| format!("{e:#}"))?;
            if let Err(e) = live::spawn_watchdog() {
                let _ = gamescope::end_live(&paths);
                return Err(format!("could not start the safety watchdog: {e:#}"));
            }
            Some(paths)
        };
        let (tx, rx) = mpsc::channel();
        let ctx2 = ctx.clone();
        let live = paths.is_some();
        thread::spawn(move || {
            let result = if live {
                live::switch_to(stock_max, hz).map_err(|e| format!("{e:#}"))
            } else {
                thread::sleep(Duration::from_millis(1200));
                Ok(Saved::default())
            };
            let _ = tx.send(result);
            ctx2.request_repaint();
        });
        Ok(PanelTest {
            hz,
            stock_max,
            phase: Phase::Switching,
            since: Instant::now(),
            saved: Saved::default(),
            paths,
            switched: Some(rx),
            finished: None,
            ctx: ctx.clone(),
        })
    }

    pub fn is_demo(&self) -> bool {
        self.paths.is_none()
    }

    /// Seconds left in the current phase.
    pub fn seconds_left(&self) -> u64 {
        let total = match self.phase {
            Phase::Confirm | Phase::Recheck => CONFIRM_SECS,
            Phase::Waiting => RECHECK_SECS,
            Phase::Switching | Phase::Finishing => return 0,
        };
        total.saturating_sub(self.since.elapsed().as_secs())
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.since = Instant::now();
    }

    fn extend(&self, secs: u64) {
        if let Some(paths) = &self.paths {
            let _ = gamescope::extend_live(paths, secs);
        }
    }

    /// Advance the clock; returns the outcome once the test is over.
    pub fn tick(&mut self) -> Option<Outcome> {
        if let Some(rx) = &self.switched {
            match rx.try_recv() {
                Ok(Ok(saved)) => {
                    self.saved = saved;
                    self.switched = None;
                    // The deadline starts now that the rate is on screen.
                    self.extend(CONFIRM_SECS + GRACE_SECS);
                    self.enter(Phase::Confirm);
                }
                Ok(Err(e)) => {
                    self.switched = None;
                    self.revert(format!("Gamescope didn't switch ({e})"));
                }
                Err(TryRecvError::Disconnected) => {
                    self.switched = None;
                    self.revert("the switch didn't finish".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let elapsed = self.since.elapsed().as_secs();
        match self.phase {
            Phase::Confirm | Phase::Recheck if elapsed >= CONFIRM_SECS => {
                self.revert("it wasn't kept in time".into());
            }
            Phase::Waiting if elapsed >= RECHECK_SECS => {
                self.extend(CONFIRM_SECS + GRACE_SECS);
                self.enter(Phase::Recheck);
            }
            _ => {}
        }
        self.finished.as_ref().and_then(|rx| rx.try_recv().ok())
    }

    /// The user kept the rate: on to the double check, or done.
    pub fn keep(&mut self) {
        match self.phase {
            Phase::Confirm => {
                self.extend(RECHECK_SECS + CONFIRM_SECS + GRACE_SECS);
                self.enter(Phase::Waiting);
            }
            Phase::Recheck => {
                if let Some(paths) = &self.paths
                    && let Err(e) = gamescope::confirm(paths)
                {
                    self.revert(format!("it couldn't be saved ({e:#})"));
                    return;
                }
                self.finish(Outcome::Kept(self.hz), live::restore);
            }
            _ => {}
        }
    }

    /// Go back to the stock rate now.
    pub fn revert(&mut self, why: String) {
        if self.phase == Phase::Finishing {
            return;
        }
        if let Some(paths) = &self.paths {
            let _ = gamescope::end_live(paths);
        }
        let (hz, stock) = (self.hz, self.stock_max);
        self.finish(Outcome::Reverted { hz, why }, move |saved| {
            live::switch_back(stock, saved)
        });
    }

    fn finish(
        &mut self,
        outcome: Outcome,
        work: impl FnOnce(Saved) -> anyhow::Result<()> + Send + 'static,
    ) {
        self.enter(Phase::Finishing);
        let (tx, rx) = mpsc::channel();
        self.finished = Some(rx);
        let (saved, live, ctx) = (self.saved, self.paths.is_some(), self.ctx.clone());
        thread::spawn(move || {
            if live {
                // Failing here leaves the watchdog and the profile's hold-back
                // in charge; the outcome is what the user chose either way.
                let _ = work(saved);
            }
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
    }

    /// The app is closing mid-test: go back right away, without threads.
    pub fn abort(&mut self) {
        if self.phase == Phase::Finishing {
            return;
        }
        self.phase = Phase::Finishing;
        if let Some(paths) = &self.paths {
            let _ = gamescope::end_live(paths);
            let _ = live::switch_back(self.stock_max, self.saved);
        }
    }
}
