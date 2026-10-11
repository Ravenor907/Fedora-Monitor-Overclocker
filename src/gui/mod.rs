//! The graphical app: `monitor-oc gui`, or plain `monitor-oc` in a desktop
//! or Game Mode session. Built on egui (eframe with OpenGL, X11 and Wayland).
//!
//! The UI thread only draws: scans, file changes and privileged commands run
//! on worker threads that report back over a channel (see [`data::Backend`]).

mod app;
mod data;
mod demo;
mod gamepad;
mod jobs;
mod pages;
mod panel_test;
mod prefs;
mod theme;
mod updates;
mod widgets;

use anyhow::{Result, anyhow};
use eframe::egui;

/// Window and Wayland app id; the .desktop file has the same name so docks and
/// Steam match the window to it.
pub use crate::install::APP_ID;

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
    /// Updates, error reports and uninstalling.
    Settings,
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
    rollback_if_failing();
    let demo = opts.demo || std::env::var("MONITOR_OC_DEMO").is_ok_and(|v| v == "1");
    let game_mode = crate::gamescope::in_game_mode();
    if game_mode && std::env::var_os("WINIT_X11_SCALE_FACTOR").is_none() {
        // Gamescope's Xwayland has no Xft.dpi, so winit would guess the scale
        // from the panel's physical size: about 3.25 on a 7" 1080p handheld.
        // That made every window size and font ~3x too big (and the window
        // larger than the screen) before our own 150% handheld zoom applied.
        // Gamescope already shows the window 1:1, so use physical pixels.
        //
        // SAFETY: called before eframe or anything else starts a thread.
        unsafe { std::env::set_var("WINIT_X11_SCALE_FACTOR", "1") };
    }
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

/// A freshly updated build that keeps failing to start is rolled back here,
/// before any window opens, and the restored version is started in its place.
fn rollback_if_failing() {
    use std::os::unix::process::CommandExt;
    let Ok(paths) = crate::update::Paths::for_current_user() else {
        return;
    };
    let Some(bad) = crate::update::startup_check(&paths) else {
        return;
    };
    let err = std::process::Command::new(&paths.binary)
        .args(std::env::args_os().skip(1))
        .env("MONITOR_OC_ROLLED_BACK", &bad)
        .exec();
    eprintln!(
        "Rolled back from {bad}, but could not start {}: {err}",
        paths.binary.display()
    );
}

/// Open Steam's on-screen keyboard, the only way to type in Game Mode.
pub fn steam_keyboard() {
    if !crate::gamescope::in_game_mode() {
        return;
    }
    let child = std::process::Command::new("steam")
        .arg("steam://open/keyboard")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || child.wait());
    }
}
