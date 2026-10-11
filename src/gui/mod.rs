//! The graphical app: `monitor-oc gui`, or plain `monitor-oc` in a desktop
//! or Game Mode session. Built on egui (eframe with OpenGL, X11 and Wayland).
//!
//! The UI thread only draws: scans, file changes and privileged commands run
//! on worker threads that report back over a channel (see [`data::Backend`]).

mod app;
mod data;
mod demo;
mod jobs;
mod pages;
mod prefs;
mod theme;
mod widgets;

use anyhow::{Result, anyhow};
use eframe::egui;

/// Window and Wayland app id; the .desktop file has the same name so docks and
/// Steam match the window to it.
pub const APP_ID: &str = "io.github.ravenor907.MonitorOC";

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Page {
    /// The handheld's own screen in Game Mode.
    Builtin,
    /// Monitors on DisplayPort / HDMI.
    External,
    /// Measure the real refresh rate and look for skipped frames.
    Verify,
    /// `monitor-oc doctor`.
    Diagnostics,
}

#[derive(Default)]
pub struct Options {
    /// Made-up hardware; changes nothing (also `MONITOR_OC_DEMO=1`).
    pub demo: bool,
    /// Page to open first instead of picking one.
    pub page: Option<Page>,
    /// Fill the screen (always on in Game Mode).
    pub fullscreen: bool,
}

/// True when there is a graphical session to open a window on.
pub fn display_available() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
}

pub fn run(opts: Options) -> Result<()> {
    let demo = opts.demo || std::env::var("MONITOR_OC_DEMO").is_ok_and(|v| v == "1");
    let game_mode = crate::gamescope::in_game_mode();
    let fullscreen = opts.fullscreen || game_mode;
    let viewport = egui::ViewportBuilder::default()
        .with_app_id(APP_ID)
        .with_title("monitor-oc")
        .with_inner_size([1200.0, 780.0])
        .with_min_inner_size([760.0, 520.0])
        .with_fullscreen(fullscreen);
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let setup = app::Setup {
        demo,
        page: opts.page,
        game_mode,
        fullscreen,
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, setup)))),
    )
    .map_err(|e| anyhow!("could not open the window: {e}"))
}
