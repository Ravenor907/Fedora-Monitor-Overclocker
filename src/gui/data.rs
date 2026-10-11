//! What the app shows, gathered on worker threads, and the built-in screen
//! actions that change it. The live source reads this system; the demo source
//! is made up and never writes anything.

use super::demo::World;
use super::jobs;
use crate::doctor::{self, Check};
use crate::drm;
use crate::edid::Edid;
use crate::gamescope::{self, Method, Panel, Paths, Report, State, StockProfile};
use crate::state;
use crate::system::{self, Os};
use anyhow::{Result, bail};
use eframe::egui;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

pub struct Snapshot {
    pub hardware: Option<String>,
    pub os: String,
    /// The built-in screen, or why there is none.
    pub panel: Result<PanelInfo, String>,
    /// Connected external monitors.
    pub monitors: Vec<Monitor>,
    pub checks: Vec<Check>,
    pub tools: Tools,
}

#[derive(Clone)]
pub struct PanelInfo {
    pub panel: Panel,
    pub stock: Option<StockProfile>,
    pub stock_min: u32,
    pub stock_max: u32,
    pub max_allowed: u32,
    pub gamescope: bool,
    /// The user's Game Mode settings, or why they cannot be read.
    pub settings: Result<Settings, String>,
}

#[derive(Clone)]
pub struct Settings {
    pub state: State,
    pub report: Option<Report>,
    /// Changed after Gamescope last loaded the profile.
    pub stale: bool,
    pub installed: bool,
}

#[derive(Clone)]
pub struct Monitor {
    pub connector: String,
    pub driver: Option<String>,
    /// The monitor's own EDID (the saved original when monitor-oc changed it).
    pub edid: Edid,
    /// The kernel is using a modified EDID right now.
    pub overridden: bool,
    /// Mode installed with `apply`, if any.
    pub installed: Option<String>,
}

impl Monitor {
    pub fn name(&self) -> String {
        self.edid
            .name()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "{} {:04X}",
                    self.edid.manufacturer(),
                    self.edid.product_code()
                )
            })
    }

    /// Distinct progressive resolutions with their fastest rate, in EDID order
    /// (the native one first).
    pub fn resolutions(&self) -> Vec<(u16, u16, f64)> {
        let mut out: Vec<(u16, u16, f64)> = Vec::new();
        for (_, d) in self.edid.timings() {
            if d.interlaced() {
                continue;
            }
            match out
                .iter_mut()
                .find(|(w, h, _)| (*w, *h) == (d.h_active, d.v_active))
            {
                Some(entry) => entry.2 = entry.2.max(d.refresh_hz()),
                None => out.push((d.h_active, d.v_active, d.refresh_hz())),
            }
        }
        out
    }
}

/// Helper programs the app can use.
#[derive(Clone, Copy, Default)]
pub struct Tools {
    pub pkexec: bool,
    pub root: bool,
    pub kscreen_doctor: bool,
    pub session_select: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelAction {
    Try { hz: u32, method: Method },
    Confirm,
    Discard,
    Reset,
    Remove,
}

#[derive(Clone)]
pub enum Source {
    Live,
    Demo(Arc<Mutex<World>>),
}

impl Source {
    fn snapshot(&self) -> Snapshot {
        match self {
            Source::Live => live_snapshot(),
            Source::Demo(world) => world.lock().unwrap().snapshot(),
        }
    }

    fn panel_action(&self, action: PanelAction) -> Result<String> {
        match self {
            Source::Live => live_panel_action(action),
            Source::Demo(world) => world.lock().unwrap().panel_action(action),
        }
    }
}

pub enum Event {
    Snapshot(Box<Snapshot>),
    Toast { ok: bool, text: String },
    Job { id: u64, event: jobs::Event },
}

/// Runs slow or privileged work off the UI thread and reports back through a
/// channel, waking the UI each time.
#[derive(Clone)]
pub struct Backend {
    pub source: Source,
    tx: Sender<Event>,
    ctx: egui::Context,
    scanning: Arc<AtomicBool>,
}

impl Backend {
    pub fn new(source: Source, ctx: egui::Context) -> (Backend, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let backend = Backend {
            source,
            tx,
            ctx,
            scanning: Arc::new(AtomicBool::new(false)),
        };
        (backend, rx)
    }

    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }

    /// Rescan in the background unless a scan is already running.
    pub fn refresh(&self) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        std::thread::spawn(move || {
            let snapshot = this.source.snapshot();
            this.scanning.store(false, Ordering::SeqCst);
            this.send(Event::Snapshot(Box::new(snapshot)));
        });
    }

    pub fn panel(&self, action: PanelAction) {
        let this = self.clone();
        std::thread::spawn(move || {
            let event = match this.source.panel_action(action) {
                Ok(text) => Event::Toast { ok: true, text },
                Err(e) => Event::Toast {
                    ok: false,
                    text: format!("{e:#}"),
                },
            };
            this.send(event);
            this.refresh();
        });
    }

    /// Leave the desktop for Game Mode (SteamOS, Bazzite). In the demo this
    /// only pretends that Game Mode started again.
    pub fn switch_to_game_mode(&self) {
        let this = self.clone();
        std::thread::spawn(move || {
            let event = match &this.source {
                Source::Demo(world) => Event::Toast {
                    ok: true,
                    text: world.lock().unwrap().restart_game_mode(),
                },
                Source::Live => match std::process::Command::new("steamos-session-select")
                    .arg("gamescope")
                    .spawn()
                {
                    Ok(_) => Event::Toast {
                        ok: true,
                        text: "Switching to Game Mode…".into(),
                    },
                    Err(e) => Event::Toast {
                        ok: false,
                        text: format!("could not run steamos-session-select: {e}"),
                    },
                },
            };
            this.send(event);
            this.refresh();
        });
    }
}

fn live_snapshot() -> Snapshot {
    let os = Os::detect();
    Snapshot {
        hardware: drm::hardware_name(),
        os: if os.pretty_name.is_empty() {
            "Linux".into()
        } else {
            os.pretty_name
        },
        panel: live_panel(),
        monitors: live_monitors(),
        checks: doctor::checks(),
        tools: Tools {
            pkexec: system::which("pkexec"),
            root: system::is_root(),
            kscreen_doctor: system::which("kscreen-doctor"),
            session_select: system::which("steamos-session-select"),
        },
    }
}

pub fn panel_info(panel: Panel, stock: Option<StockProfile>, gamescope: bool) -> PanelInfo {
    let stock_max = gamescope::stock_max(&panel, stock.as_ref());
    let stock_min = stock
        .as_ref()
        .map(|s| s.min_hz)
        .or_else(|| panel.edid_rates.first().copied())
        .unwrap_or(stock_max);
    PanelInfo {
        stock_min,
        stock_max,
        max_allowed: gamescope::max_allowed(stock_max),
        gamescope,
        settings: Err(String::new()),
        panel,
        stock,
    }
}

fn live_panel() -> Result<PanelInfo, String> {
    let panel = Panel::detect().map_err(|e| format!("{e:#}"))?;
    let stock = gamescope::stock_profile(&panel.id);
    let mut info = panel_info(panel, stock, gamescope::gamescope_installed());
    info.settings = Paths::for_current_user()
        .map(|paths| {
            let report = Report::load(&paths);
            Settings {
                state: State::load(&paths),
                stale: report.as_ref().is_some_and(|r| r.is_stale(&paths)),
                report,
                installed: gamescope::installed(&paths),
            }
        })
        .map_err(|e| format!("{e:#}"));
    Ok(info)
}

fn live_monitors() -> Vec<Monitor> {
    let installed = state::load();
    drm::connectors()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !c.is_internal() && c.connected())
        .filter_map(|c| {
            let live = c.edid()?;
            let saved = fs::read(state::original_path(&c.name)).ok();
            let edid = Edid::parse(saved.as_deref().unwrap_or(&live)).ok()?;
            Some(Monitor {
                overridden: saved.is_some_and(|s| s != live),
                installed: installed
                    .iter()
                    .find(|e| e.connector == c.name)
                    .map(|e| e.mode.clone()),
                driver: c.driver(),
                connector: c.name,
                edid,
            })
        })
        .collect()
}

/// Same steps and wording as `monitor-oc panel ...`.
fn live_panel_action(action: PanelAction) -> Result<String> {
    let paths = Paths::for_current_user()?;
    Ok(match action {
        PanelAction::Try { hz, method } => {
            let panel = Panel::detect()?;
            if !gamescope::gamescope_installed() {
                bail!("Gamescope is not installed; this only applies to Game Mode");
            }
            gamescope::try_rate(&panel, &paths, hz, method)?;
            format!("Trial armed: up to {hz} Hz for the next Game Mode session only.")
        }
        PanelAction::Confirm => {
            let hz = gamescope::confirm(&paths)?;
            format!("Kept: Game Mode now offers up to {hz} Hz on the built-in screen.")
        }
        PanelAction::Discard => {
            gamescope::discard(&paths)?;
            "Trial discarded.".into()
        }
        PanelAction::Reset => {
            gamescope::reset(&paths)?;
            "Back to the stock range from the next Game Mode start.".into()
        }
        PanelAction::Remove => {
            gamescope::remove(&paths)?;
            "Removed monitor-oc's Gamescope profile; restart Game Mode to finish.".into()
        }
    })
}
