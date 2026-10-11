//! The window: header, sidebar, the current page, dialogs and toasts. Owns
//! all UI state and turns page actions into background work.

use super::Page;
use super::data::{Backend, Event, PanelAction, Snapshot, Source};
use super::demo::World;
use super::gamepad::Gamepad;
use super::jobs::{self, Job, Kind as JobKind, Outcome, TEST_SECONDS};
use super::pages::{Action, Dialog, View, builtin, diagnostics, external, verify};
use super::panel_test::{self, PanelTest, Phase};
use super::prefs::{self, Prefs};
use super::theme::{self, bold, font, palette, tint};
use super::updates::{self, UpdateAction, Updates};
use super::widgets::{self, Button, Icon, Kind, caption, chip, dim, faint, nav_item, tag};
use crate::gamescope::TrialState;
use crate::update::{self, Channel};
use eframe::egui::{
    self, Align, CentralPanel, Color32, Frame, Id, Key, Layout, Margin, Modifiers, Panel, RichText,
    ScrollArea, Shape, Stroke, Theme, Ui, ViewportCommand, pos2, vec2,
};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the system is rescanned (not while measuring on Verify).
const SCAN_EVERY: Duration = Duration::from_secs(3);
const TOAST_FOR: Duration = Duration::from_secs(6);
const MAX_CONTENT_WIDTH: f32 = 1180.0;
const LOG_LINES: usize = 400;

pub struct Setup {
    pub demo: bool,
    pub page: Option<Page>,
    pub game_mode: bool,
    pub fullscreen: bool,
}

struct Toast {
    ok: bool,
    text: String,
    at: Instant,
}

pub struct App {
    backend: Backend,
    events: Receiver<Event>,
    snap: Option<Snapshot>,
    setup: Setup,
    page: Page,
    /// The page was picked (by `--page` or the user), so don't pick one.
    page_chosen: bool,
    prefs: Prefs,
    /// The zoom in use, once the screen size is known.
    scale: Option<f32>,
    fit_tries: u8,
    last_scan: Instant,
    toasts: Vec<Toast>,
    dialog: Option<Dialog>,
    /// Move keyboard focus into the dialog that just opened.
    focus_dialog: bool,
    job: Option<Job>,
    next_job: u64,
    log: Vec<String>,
    builtin: builtin::State,
    external: external::State,
    verify: verify::State,
    updates: Updates,
    /// Controller input, in Game Mode (or with MONITOR_OC_GAMEPAD=1).
    pad: Option<Gamepad>,
    started: Instant,
    /// A fresh build was told it works (see `update::mark_healthy`).
    healthy: bool,
    /// A live test of a built-in screen rate in Game Mode.
    panel_test: Option<PanelTest>,
    /// "Report a problem" being written: the text and the page it came from.
    report_draft: Option<(String, String)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, setup: Setup) -> App {
        let ctx = &cc.egui_ctx;
        theme::install(ctx);
        let prefs = Prefs::load();
        ctx.set_theme(if prefs.dark {
            Theme::Dark
        } else {
            Theme::Light
        });
        // Ctrl +/- change the saved UI scale instead of egui's own zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let source = if setup.demo {
            Source::Demo(Arc::new(Mutex::new(World::new())))
        } else {
            Source::Live
        };
        let (backend, events) = Backend::new(source, ctx.clone());
        backend.refresh();
        let pad = (setup.game_mode || std::env::var("MONITOR_OC_GAMEPAD").is_ok_and(|v| v == "1"))
            .then(|| Gamepad::start(ctx));
        App {
            updates: Updates::new(ctx, setup.demo),
            pad,
            started: Instant::now(),
            healthy: setup.demo,
            panel_test: None,
            report_draft: None,
            backend,
            events,
            snap: None,
            page: setup.page.unwrap_or(Page::Builtin),
            page_chosen: setup.page.is_some(),
            setup,
            prefs,
            scale: None,
            fit_tries: 0,
            last_scan: Instant::now(),
            toasts: Vec::new(),
            dialog: None,
            focus_dialog: false,
            job: None,
            next_job: 1,
            log: Vec::new(),
            builtin: builtin::State::default(),
            external: external::State::default(),
            verify: verify::State::default(),
        }
    }

    /// Pages to offer: the built-in screen only when there is one.
    fn pages(&self) -> Vec<(Page, Icon, &'static str)> {
        let mut pages = Vec::new();
        if self.snap.as_ref().is_some_and(|s| s.panel.is_ok()) || self.page == Page::Builtin {
            pages.push((Page::Builtin, Icon::Handheld, "Built-in screen"));
        }
        pages.extend([
            (Page::External, Icon::Monitor, "External monitors"),
            (Page::Verify, Icon::Stopwatch, "Verify"),
            (Page::Diagnostics, Icon::Pulse, "Diagnostics"),
            (Page::Settings, Icon::Update, "Settings"),
        ]);
        pages
    }

    fn toast(&mut self, ok: bool, text: impl Into<String>) {
        let text = text.into();
        crate::report::log(if ok { "app" } else { "app-error" }, &text);
        if self.toasts.len() >= 3 {
            self.toasts.remove(0);
        }
        self.toasts.push(Toast {
            ok,
            text,
            at: Instant::now(),
        });
    }

    fn pump(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Snapshot(snap) => {
                    if !self.page_chosen {
                        self.page = if snap.panel.is_ok() {
                            Page::Builtin
                        } else {
                            Page::External
                        };
                        self.page_chosen = true;
                    }
                    self.snap = Some(*snap);
                }
                Event::Toast { ok, text } => self.toast(ok, text),
                Event::Job { id, event } => self.job_event(id, event),
            }
        }
    }

    fn job_event(&mut self, id: u64, event: jobs::Event) {
        let Some(job) = self.job.as_mut().filter(|j| j.id == id) else {
            return;
        };
        match event {
            jobs::Event::Line(line) => {
                self.log.extend(line.lines().map(String::from));
                let excess = self.log.len().saturating_sub(LOG_LINES);
                self.log.drain(..excess);
            }
            jobs::Event::RecheckWait => job.enter(jobs::Phase::Waiting),
            jobs::Event::Recheck => {
                job.enter(jobs::Phase::Recheck);
                self.focus_dialog = true;
            }
            jobs::Event::Live => {
                job.live_since = Some(Instant::now());
                if let JobKind::Test {
                    connector,
                    mode,
                    switch_mode: true,
                    ..
                } = &job.kind
                {
                    jobs::switch_mode_kde(&self.backend, id, connector, mode);
                }
            }
            jobs::Event::Exit(outcome) => {
                job.outcome = Some(outcome.clone());
                if let Outcome::Failed(why) = &outcome {
                    let tail = self.log[self.log.len().saturating_sub(80)..].join("\n");
                    crate::report::auto(
                        crate::report::Kind::CommandFailure,
                        &format!("{} failed: {why}", job.kind.title()),
                        &format!(
                            "`{}` failed: {why}\n\nOutput:\n```\n{tail}\n```",
                            job.kind.command_line()
                        ),
                    );
                }
                let (ok, text) = outcome_message(job, &outcome);
                self.toast(ok, text);
                self.backend.refresh();
            }
        }
    }

    fn start_job(&mut self, kind: JobKind) {
        if self.job.as_ref().is_some_and(Job::running) {
            return;
        }
        if let JobKind::Test { mode, .. } = &kind {
            self.verify.retarget(mode.hz.round() as u32);
        }
        self.log.clear();
        self.external.log_open = true;
        let id = self.next_job;
        self.next_job += 1;
        self.job = Some(jobs::start(&self.backend, id, kind));
        self.focus_dialog = true;
    }

    fn act(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Verify(hz) => {
                self.verify.retarget(hz);
                self.go(Page::Verify);
            }
            Action::Panel(action) => self.backend.panel(action),
            Action::Ask(dialog) => {
                self.dialog = Some(dialog);
                self.focus_dialog = true;
            }
            Action::Run(kind) => self.start_job(kind),
            Action::Copy(text) => {
                ctx.copy_text(text);
                self.toast(true, "Copied to the clipboard.");
            }
            Action::Update(action) => self.updates.act(action),
            Action::ReportProblem(page) => {
                self.report_draft = Some((String::new(), page));
                self.focus_dialog = true;
            }
            Action::PanelTest { hz, stock_max } => {
                if self.panel_test.is_none() {
                    match PanelTest::start(ctx, hz, stock_max, self.setup.demo) {
                        Ok(test) => {
                            self.panel_test = Some(test);
                            self.focus_dialog = true;
                        }
                        Err(e) => self.toast(false, format!("Couldn't start the test: {e}")),
                    }
                }
            }
            Action::Refresh => {
                self.backend.refresh();
                self.last_scan = Instant::now();
            }
        }
    }

    fn set_scale(&mut self, ctx: &egui::Context, scale: f32) {
        let scale = (scale * 20.0).round() / 20.0;
        let scale = scale.clamp(prefs::MIN_SCALE, prefs::MAX_SCALE);
        self.scale = Some(scale);
        self.apply_zoom(ctx, scale);
        self.prefs.scale = Some(scale);
        self.save_prefs();
    }

    fn save_prefs(&mut self) {
        // The demo changes nothing on the system, preferences included.
        if self.setup.demo {
            return;
        }
        if let Err(e) = self.prefs.save() {
            self.toast(false, format!("Could not save preferences: {e:#}"));
        }
    }

    /// Once the screen is known: in full screen make sure the window really
    /// covers it (some window managers ignore the request), then pick the UI
    /// scale. Both happen before any zoom so points and pixels still agree.
    fn screen_setup(&mut self, ctx: &egui::Context) {
        if self.scale.is_some() {
            return;
        }
        let (monitor, inner) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().inner_rect));
        if monitor.is_none() && ctx.cumulative_frame_nr() < 5 {
            ctx.request_repaint();
            return;
        }
        if self.setup.fullscreen
            && self.fit_tries < 2
            && let (Some(monitor), Some(inner)) = (monitor, inner)
        {
            self.fit_tries += 1;
            if inner.width() + 2.0 < monitor.x || inner.height() + 2.0 < monitor.y {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(0.0, 0.0)));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(monitor));
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
        }
        let ppp = ctx.pixels_per_point();
        let scale = self.prefs.scale.unwrap_or_else(|| {
            prefs::default_scale(self.setup.game_mode, monitor.map(|m| m.y * ppp))
        });
        self.scale = Some(scale);
        self.apply_zoom(ctx, scale);
        ctx.request_repaint();
    }

    /// `scale` is in physical pixels per point. In Game Mode Gamescope shows
    /// the window 1:1, so any display scale the toolkit reports there (X11
    /// is pinned to 1 in `gui::run`; this covers Wayland) is cancelled out.
    fn apply_zoom(&self, ctx: &egui::Context, scale: f32) {
        let native = if self.setup.game_mode {
            ctx.native_pixels_per_point().unwrap_or(1.0).max(0.25)
        } else {
            1.0
        };
        ctx.set_zoom_factor(scale / native);
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        self.page_chosen = true;
    }

    /// Keys for keyboards and Steam Input layouts: F1-F4 pick a page,
    /// Ctrl+Tab / PageDown and Ctrl+Shift+Tab / PageUp cycle, F5 rescans,
    /// Ctrl +/-/0 change the UI size.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let pages: Vec<Page> = self.pages().into_iter().map(|(p, _, _)| p).collect();
        let current = pages.iter().position(|p| *p == self.page).unwrap_or(0);
        let ctrl = Modifiers::COMMAND;
        let (prev, next, refresh, bigger, smaller, reset, direct) = ctx.input_mut(|i| {
            let fkeys = [Key::F1, Key::F2, Key::F3, Key::F4];
            // Most specific first: consume_key ignores an extra Shift.
            let prev = i.consume_key(ctrl | Modifiers::SHIFT, Key::Tab)
                || i.consume_key(Modifiers::NONE, Key::PageUp);
            let next =
                i.consume_key(ctrl, Key::Tab) || i.consume_key(Modifiers::NONE, Key::PageDown);
            (
                prev,
                next,
                i.consume_key(Modifiers::NONE, Key::F5),
                i.consume_key(ctrl, Key::Plus) || i.consume_key(ctrl, Key::Equals),
                i.consume_key(ctrl, Key::Minus),
                i.consume_key(ctrl, Key::Num0),
                fkeys
                    .iter()
                    .position(|k| i.consume_key(Modifiers::NONE, *k)),
            )
        });
        if self.dialog.is_none() {
            if next {
                self.go(pages[(current + 1) % pages.len()]);
            }
            if prev {
                self.go(pages[(current + pages.len() - 1) % pages.len()]);
            }
            if let Some(page) = direct.and_then(|i| pages.get(i)) {
                self.go(*page);
            }
        }
        if refresh {
            self.act(ctx, Action::Refresh);
        }
        let scale = self.scale.unwrap_or(1.0);
        if bigger {
            self.set_scale(ctx, scale + 0.1);
        }
        if smaller {
            self.set_scale(ctx, scale - 0.1);
        }
        if reset {
            self.set_scale(ctx, 1.0);
        }
    }

    fn header(&self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        ui.horizontal_centered(|ui| {
            widgets::logo(ui, 34.0);
            ui.add_space(2.0);
            ui.label(RichText::new("monitor-oc").font(bold(20.0)).color(p.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(badge) = self.updates.badge() {
                    chip(ui, badge, p.good);
                }
                if update::is_test_build(update::version()) {
                    chip(ui, "Test build", p.warn);
                }
                if self.setup.game_mode {
                    chip(ui, "Game Mode", p.accent);
                } else {
                    chip(ui, "Desktop", p.text_dim);
                }
                if let Some(snap) = &self.snap {
                    tag(ui, &snap.os);
                    if let Some(hw) = &snap.hardware {
                        tag(ui, &short_hardware(hw));
                    }
                }
                if self.setup.demo {
                    chip(ui, "DEMO", p.warn)
                        .on_hover_text("Made-up hardware. Nothing on this system is changed.");
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        for (page, icon, label) in self.pages() {
            if nav_item(ui, icon, label, self.page == page).clicked() {
                self.go(page);
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("v{}", update::label()))
                    .font(font(12.0))
                    .color(p.text_faint),
            );
            let ctx = ui.ctx().clone();
            let scale = self.scale.unwrap_or(1.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let small = |text| {
                    Button::new(text, Kind::Secondary)
                        .min_width(46.0)
                        .height(40.0)
                        .text_size(20.0)
                };
                if ui
                    .add_enabled(scale > prefs::MIN_SCALE, small("−"))
                    .clicked()
                {
                    self.set_scale(&ctx, scale - 0.1);
                }
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width() - 52.0, 40.0),
                    Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        ui.label(
                            RichText::new(format!("{:.0}%", scale * 100.0))
                                .font(bold(15.0))
                                .color(p.text),
                        );
                    },
                );
                if ui
                    .add_enabled(scale < prefs::MAX_SCALE, small("+"))
                    .clicked()
                {
                    self.set_scale(&ctx, scale + 0.1);
                }
            });
            caption(ui, "Interface size", p.text_faint);
            ui.add_space(6.0);
            let mut dark = self.prefs.dark;
            if widgets::segmented(ui, &mut dark, &[(true, "Dark"), (false, "Light")]) {
                self.prefs.dark = dark;
                ctx.set_theme(if dark { Theme::Dark } else { Theme::Light });
                self.save_prefs();
            }
            caption(ui, "Theme", p.text_faint);
            if self.pad.as_ref().is_some_and(Gamepad::connected) {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("A select  ·  B back  ·  LB/RB pages")
                        .font(font(13.0))
                        .color(p.text_faint),
                );
            }
        });
    }

    fn content(&mut self, ui: &mut Ui, out: &mut Vec<Action>) {
        let width = ui.available_width();
        let content_w = (width - 2.0 * 32.0).min(MAX_CONTENT_WIDTH);
        let pad = ((width - content_w) / 2.0).floor();
        ui.add_space(26.0);
        ui.horizontal_top(|ui| {
            ui.add_space(pad);
            ui.allocate_ui_with_layout(vec2(content_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(content_w);
                self.panel_test_banner(ui);
                self.job_wait_banner(ui);
                if self.page == Page::Settings {
                    updates::show(ui, &mut self.updates, out);
                    return;
                }
                let Some(snap) = &self.snap else {
                    ui.add_space(120.0);
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Spinner::new().size(36.0));
                        dim(ui, "Looking at your displays…");
                    });
                    return;
                };
                let view = View {
                    snap,
                    demo: self.setup.demo,
                    game_mode: self.setup.game_mode,
                    job: self.job.as_ref(),
                    log: &self.log,
                };
                match self.page {
                    Page::Builtin => builtin::show(ui, &mut self.builtin, &view, out),
                    Page::External => external::show(ui, &mut self.external, &view, out),
                    Page::Verify => verify::show(ui, &mut self.verify, &view),
                    Page::Diagnostics => diagnostics::show(ui, &view, out),
                    Page::Settings => {}
                }
            });
        });
        ui.add_space(36.0);
    }

    fn modal_frame(ctx: &egui::Context) -> Frame {
        let p = palette(ctx);
        Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(18)
            .inner_margin(Margin::same(28))
            .shadow(ctx.global_style().visuals.window_shadow)
    }

    fn dialogs(&mut self, ctx: &egui::Context, out: &mut Vec<Action>) {
        if self
            .panel_test
            .as_ref()
            .is_some_and(|t| t.phase != Phase::Waiting)
        {
            self.panel_test_dialog(ctx);
            return;
        }
        if self.report_draft.is_some() {
            self.report_dialog(ctx, out);
            return;
        }
        if self.job.as_ref().is_some_and(|j| {
            j.running() && matches!(j.kind, JobKind::Test { .. }) && j.phase != jobs::Phase::Waiting
        }) {
            self.test_dialog(ctx);
            return;
        }
        let Some(dialog) = &self.dialog else {
            return;
        };
        let p = palette(ctx);
        let focus = std::mem::take(&mut self.focus_dialog);
        let trial_hz = self.armed_trial();
        let mut decision = None;
        let response = egui::Modal::new(Id::new("confirm"))
            .frame(Self::modal_frame(ctx))
            .backdrop_color(Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(520.0);
                let (title, body, confirm, kind): (String, Vec<String>, String, Kind) = match dialog {
                    Dialog::GameMode => (
                        "Switch to Game Mode?".into(),
                        vec![
                            "This ends the desktop session, so save your work in other apps first."
                                .into(),
                            match trial_hz {
                                Some(hz) => format!("The {hz} Hz trial starts with Game Mode."),
                                None => "Game Mode loads your settings when it starts.".into(),
                            },
                        ],
                        "Switch to Game Mode".into(),
                        Kind::Primary,
                    ),
                    Dialog::ResetPanel => (
                        "Go back to the stock range?".into(),
                        vec!["Game Mode uses the stock refresh rates from its next start. The kept limit and any trial are cleared.".into()],
                        "Reset to stock".into(),
                        Kind::Primary,
                    ),
                    Dialog::RemoveProfile => (
                        "Remove the Gamescope profile?".into(),
                        vec!["Deletes monitor-oc's Gamescope profile and its settings. Restart Game Mode to finish.".into()],
                        "Remove".into(),
                        Kind::Danger,
                    ),
                    Dialog::Apply { kind, summary } => (
                        match kind {
                            JobKind::Apply { mode, connector, .. } => {
                                format!("Install {mode} on {connector}?")
                            }
                            _ => "Install this mode?".into(),
                        },
                        summary.clone(),
                        "Apply…".into(),
                        Kind::Primary,
                    ),
                    Dialog::Remove(JobKind::Reset { connector }) => (
                        format!("Drop the live override on {connector}?"),
                        vec![format!("{connector} goes back to its own EDID right away. Nothing was installed, so no reboot is needed.")],
                        "Drop it…".into(),
                        Kind::Danger,
                    ),
                    Dialog::Uninstall => (
                        "Uninstall Monitor OC?".into(),
                        vec![
                            "Removes the app, its menu entries, updates and settings. The built-in \
                             screen goes back to stock from the next Game Mode start."
                                .into(),
                            "Overrides for external monitors are removed too (that needs your \
                             password)."
                                .into(),
                        ],
                        "Uninstall".into(),
                        Kind::Danger,
                    ),
                    Dialog::Testing => (
                        "Switch to test builds?".into(),
                        update::TESTING_DISCLAIMER
                            .iter()
                            .map(|l| l.to_string())
                            .collect(),
                        "Use test builds".into(),
                        Kind::Primary,
                    ),
                    Dialog::Remove(kind) => (
                        match kind {
                            JobKind::Remove { connector } => {
                                format!("Remove the override on {connector}?")
                            }
                            _ => "Remove the override?".into(),
                        },
                        vec!["Removes the installed EDID and its kernel arguments, and reverts a live test. Reboot to finish.".into()],
                        "Remove…".into(),
                        Kind::Danger,
                    ),
                };
                ui.label(RichText::new(title).font(bold(22.0)).color(p.text));
                ui.add_space(4.0);
                for line in &body {
                    dim(ui, line.clone());
                }
                if matches!(dialog, Dialog::Apply { .. } | Dialog::Remove(_)) {
                    faint(ui, "You'll be asked for your password.");
                }
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(Button::new(confirm, kind)).clicked() {
                        decision = Some(true);
                    }
                    let cancel = ui.add(Button::new("Cancel", Kind::Secondary));
                    if focus {
                        cancel.request_focus();
                    }
                    if cancel.clicked() {
                        decision = Some(false);
                    }
                });
            });
        if response.should_close() {
            decision = decision.or(Some(false));
        }
        if let Some(yes) = decision {
            let dialog = self.dialog.take();
            if yes {
                match dialog {
                    Some(Dialog::GameMode) => self.backend.switch_to_game_mode(),
                    Some(Dialog::ResetPanel) => out.push(Action::Panel(PanelAction::Reset)),
                    Some(Dialog::RemoveProfile) => out.push(Action::Panel(PanelAction::Remove)),
                    Some(Dialog::Apply { kind, .. }) | Some(Dialog::Remove(kind)) => {
                        out.push(Action::Run(kind))
                    }
                    Some(Dialog::Uninstall) => out.push(Action::Update(UpdateAction::Uninstall)),
                    Some(Dialog::Testing) => {
                        out.push(Action::Update(UpdateAction::Channel(Channel::Testing)))
                    }
                    None => {}
                }
            }
        }
    }

    fn armed_trial(&self) -> Option<u32> {
        let info = self.snap.as_ref()?.panel.as_ref().ok()?;
        match info.settings.as_ref().ok()?.state.trial {
            Some((hz, TrialState::Armed)) => Some(hz),
            _ => None,
        }
    }

    /// "Keep this mode?" with the countdown of the root process.
    fn test_dialog(&mut self, ctx: &egui::Context) {
        let p = palette(ctx);
        let focus = std::mem::take(&mut self.focus_dialog);
        let Some(job) = self.job.as_mut() else {
            return;
        };
        let JobKind::Test {
            connector, mode, ..
        } = job.kind.clone()
        else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(200));
        let mut answer = None;
        egui::Modal::new(Id::new("test"))
            .frame(Self::modal_frame(ctx))
            .backdrop_color(Color32::from_black_alpha(170))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                caption(ui, &format!("Live test  ·  {connector}"), p.accent);
                let Some(left) = job.seconds_left() else {
                    ui.label(RichText::new("Waiting for your password…").font(bold(22.0)));
                    dim(
                        ui,
                        format!(
                            "Approve the prompt to switch {connector} to {mode}. Nothing changes until then."
                        ),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(26.0));
                        faint(ui, "The password prompt may open behind this window.");
                    });
                    return;
                };
                if job.answered == Some(true) {
                    ui.label(RichText::new("Keeping it…").font(bold(22.0)));
                    return;
                }
                if job.answered == Some(false) {
                    ui.label(RichText::new("Reverting…").font(bold(22.0)));
                    return;
                }
                ui.label(
                    RichText::new(if job.phase == jobs::Phase::Recheck {
                        "Still good after 5 minutes?"
                    } else {
                        "Keep this mode?"
                    })
                    .font(bold(24.0)),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    countdown(ui, left, TEST_SECONDS);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("Reverting in {left} s"))
                                .font(bold(18.0))
                                .color(p.text),
                        );
                        dim(
                            ui,
                            format!(
                                "Pick {:.0} Hz in your display settings if it hasn't switched. If the \
                                 screen stays black, just wait.",
                                mode.hz
                            ),
                        );
                    });
                });
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(Button::new(format!("Keep {:.0} Hz", mode.hz), Kind::Primary))
                        .clicked()
                    {
                        answer = Some(true);
                    }
                    let revert = ui.add(Button::new("Revert now", Kind::Secondary));
                    if focus || ui.memory(|m| m.focused().is_none()) {
                        revert.request_focus();
                    }
                    if revert.clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(keep) = answer {
            job.answer(keep);
        }
    }

    fn tick_panel_test(&mut self, ctx: &egui::Context) {
        let Some(test) = self.panel_test.as_mut() else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(250));
        let Some(outcome) = test.tick() else {
            return;
        };
        let demo = test.is_demo();
        self.panel_test = None;
        match outcome {
            panel_test::Outcome::Kept(hz) => {
                if demo {
                    self.backend.panel(PanelAction::Confirm);
                }
                self.toast(
                    true,
                    format!(
                        "{hz} Hz passed both checks and is kept. Game Mode offers it from now on."
                    ),
                );
            }
            panel_test::Outcome::Reverted { hz, why, failed } => {
                if failed {
                    crate::report::auto(
                        crate::report::Kind::TestFailure,
                        &format!("Built-in screen test at {hz} Hz failed: {why}"),
                        &format!("The live test of {hz} Hz went back to stock because {why}."),
                    );
                }
                self.toast(
                    false,
                    format!("Back to stock: {hz} Hz was undone because {why}."),
                );
            }
        }
        self.backend.refresh();
    }

    /// "Report a problem": what went wrong, in the user's words.
    fn report_dialog(&mut self, ctx: &egui::Context, out: &mut Vec<Action>) {
        let p = palette(ctx);
        let focus = std::mem::take(&mut self.focus_dialog);
        let Some((text, page)) = self.report_draft.as_mut() else {
            return;
        };
        let mut decision = None;
        let response = egui::Modal::new(Id::new("report"))
            .frame(Self::modal_frame(ctx))
            .backdrop_color(Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(
                    RichText::new("Report a problem")
                        .font(bold(22.0))
                        .color(p.text),
                );
                ui.add_space(4.0);
                dim(
                    ui,
                    "What happened, and what did you expect? The diagnostics and recent activity \
                     are added for you (user and host names taken out).",
                );
                if self.setup.game_mode {
                    faint(ui, "In Game Mode, press STEAM + X for the keyboard.");
                }
                ui.add_space(6.0);
                let edit = ui.add(
                    egui::TextEdit::multiline(text)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY)
                        .hint_text(
                            "For example: the screen flickers at 130 Hz after a few minutes",
                        ),
                );
                if focus {
                    edit.request_focus();
                }
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !text.trim().is_empty(),
                            Button::new("Send report", Kind::Primary),
                        )
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    if ui.add(Button::new("Cancel", Kind::Secondary)).clicked() {
                        decision = Some(false);
                    }
                });
            });
        if response.should_close() {
            decision = decision.or(Some(false));
        }
        if let Some(send) = decision {
            let (text, page) = (std::mem::take(text), page.clone());
            self.report_draft = None;
            if send {
                out.push(Action::Update(UpdateAction::Report { text, page }));
            }
        }
    }

    /// The live test's "keep it?" questions, with the countdown.
    fn panel_test_dialog(&mut self, ctx: &egui::Context) {
        let p = palette(ctx);
        let focus = std::mem::take(&mut self.focus_dialog);
        let Some(test) = self.panel_test.as_mut() else {
            return;
        };
        let (hz, stock, left) = (test.hz, test.stock_max, test.seconds_left());
        let mut answer = None;
        egui::Modal::new(Id::new("panel-test"))
            .frame(Self::modal_frame(ctx))
            .backdrop_color(Color32::from_black_alpha(170))
            .show(ctx, |ui| {
                ui.set_width(500.0);
                match test.phase {
                    Phase::Switching | Phase::Finishing => {
                        caption(ui, &format!("Live test  ·  {hz} Hz"), p.accent);
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(26.0));
                            ui.label(
                                RichText::new(if test.phase == Phase::Switching {
                                    format!("Switching to {hz} Hz…")
                                } else {
                                    "Finishing…".into()
                                })
                                .font(bold(22.0)),
                            );
                        });
                        return;
                    }
                    Phase::Waiting => return,
                    Phase::Confirm | Phase::Recheck => {}
                }
                let recheck = test.phase == Phase::Recheck;
                caption(
                    ui,
                    if recheck {
                        "Double check  ·  5 minutes in"
                    } else {
                        "Live test  ·  check 1 of 2"
                    },
                    p.accent,
                );
                ui.label(
                    RichText::new(if recheck {
                        format!("Still good at {hz} Hz?")
                    } else {
                        format!("Keep {hz} Hz?")
                    })
                    .font(bold(24.0)),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    countdown(ui, left, crate::live::CONFIRM_SECS);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("Back to {stock} Hz in {left} s"))
                                .font(bold(18.0))
                                .color(p.text),
                        );
                        dim(
                            ui,
                            if recheck {
                                "Keep it to make it permanent. If anything flickered or froze, let it go back."
                            } else {
                                "Check for flicker, lines or a black screen. If you can't see anything, just wait."
                            },
                        );
                    });
                });
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(Button::new(format!("Keep {hz} Hz"), Kind::Primary))
                        .clicked()
                    {
                        answer = Some(true);
                    }
                    let back = ui.add(Button::new("Go back now", Kind::Secondary));
                    if focus || ui.memory(|m| m.focused().is_none()) {
                        back.request_focus();
                    }
                    if back.clicked() {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => test.keep(),
            Some(false) => test.revert("you chose to go back".into()),
            None => {}
        }
    }

    /// An external monitor test between its two checks.
    fn job_wait_banner(&mut self, ui: &mut Ui) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        if !job.running() || job.phase != jobs::Phase::Waiting {
            return;
        }
        let JobKind::Test {
            connector, mode, ..
        } = job.kind.clone()
        else {
            return;
        };
        ui.ctx().request_repaint_after(Duration::from_millis(500));
        let p = palette(ui.ctx());
        let left = job.seconds_left().unwrap_or(0);
        let mut back = false;
        widgets::banner(ui, p.accent, |ui| {
            caption(ui, &format!("Live test  ·  {connector}"), p.accent);
            ui.label(
                RichText::new(format!(
                    "Running {mode}  ·  double check in {}:{:02}",
                    left / 60,
                    left % 60
                ))
                .font(bold(20.0)),
            );
            dim(
                ui,
                "Use it normally. Be back here for the double check: if nobody keeps it then, \
                 the monitor goes back to its own EDID.",
            );
            ui.add_space(4.0);
            back = ui.add(Button::new("Revert now", Kind::Secondary)).clicked();
        });
        ui.add_space(12.0);
        if back {
            job.answer(false);
        }
    }

    /// Between the two checks: the countdown to the double check.
    fn panel_test_banner(&mut self, ui: &mut Ui) {
        let Some(test) = self.panel_test.as_mut() else {
            return;
        };
        if test.phase != Phase::Waiting {
            return;
        }
        let p = palette(ui.ctx());
        let left = test.seconds_left();
        let mut back = false;
        let mut verify = false;
        widgets::banner(ui, p.accent, |ui| {
            caption(ui, "Live test  ·  check 2 of 2 coming up", p.accent);
            ui.label(
                RichText::new(format!(
                    "Running at {} Hz  ·  double check in {}:{:02}",
                    test.hz,
                    left / 60,
                    left % 60
                ))
                .font(bold(20.0)),
            );
            dim(
                ui,
                "Use it normally or measure it on Verify. Be back in Monitor OC for the double check: \
                 if nobody keeps it then, the screen goes back to stock.",
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                verify = ui.add(Button::new("Verify  →", Kind::Ghost)).clicked();
                back = ui
                    .add(Button::new("Go back now", Kind::Secondary))
                    .clicked();
            });
        });
        ui.add_space(12.0);
        if back {
            test.revert("you chose to go back".into());
        }
        if verify {
            let hz = test.hz;
            self.verify.retarget(hz);
            self.go(Page::Verify);
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.at.elapsed() < TOAST_FOR);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        let p = palette(ctx);
        egui::Area::new(Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    let color = if toast.ok { p.good } else { p.bad };
                    Frame::new()
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, tint(color, 120)))
                        .corner_radius(14)
                        .inner_margin(Margin::symmetric(18, 14))
                        .shadow(ctx.global_style().visuals.popup_shadow)
                        .show(ui, |ui| {
                            ui.set_width(380.0);
                            ui.horizontal(|ui| {
                                widgets::status_icon(
                                    ui,
                                    if toast.ok {
                                        crate::doctor::Level::Ok
                                    } else {
                                        crate::doctor::Level::Fail
                                    },
                                    22.0,
                                );
                                ui.add(
                                    egui::Label::new(RichText::new(&toast.text).color(p.text))
                                        .wrap(),
                                );
                            });
                        });
                }
            });
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(test) = self.panel_test.as_mut() {
            test.abort();
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(pad) = &mut self.pad {
            pad.feed(ctx, raw_input);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump();
        self.updates.pump();
        self.tick_panel_test(&ctx);
        for (ok, text) in self.updates.take_toasts() {
            self.toast(ok, text);
        }
        if let Some(at) = self.updates.close_at {
            if Instant::now() >= at {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        if !self.healthy && self.started.elapsed().as_secs() >= update::HEALTHY_AFTER_SECS {
            self.healthy = true;
            if let Ok(paths) = update::Paths::for_current_user() {
                update::mark_healthy(&paths);
            }
        } else if !self.healthy {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        self.screen_setup(&ctx);
        self.shortcuts(&ctx);
        if self.page != Page::Verify && self.last_scan.elapsed() >= SCAN_EVERY {
            self.backend.refresh();
            self.last_scan = Instant::now();
        }
        ctx.request_repaint_after(SCAN_EVERY);

        let p = palette(&ctx);
        let mut actions = Vec::new();
        Panel::top("header")
            .exact_size(68.0)
            .frame(
                Frame::new()
                    .fill(p.chrome)
                    .inner_margin(Margin::symmetric(22, 0)),
            )
            .show(ui, |ui| self.header(ui));
        Panel::left("nav")
            .exact_size(240.0)
            .resizable(false)
            .frame(Frame::new().fill(p.chrome).inner_margin(Margin::same(14)))
            .show(ui, |ui| self.sidebar(ui));
        CentralPanel::default()
            .frame(Frame::new().fill(p.bg))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.content(ui, &mut actions));
            });
        self.dialogs(&ctx, &mut actions);
        self.toasts(&ctx);
        for action in actions {
            self.act(&ctx, action);
        }
    }
}

/// What to tell the user when a privileged command ends.
fn outcome_message(job: &Job, outcome: &Outcome) -> (bool, String) {
    match outcome {
        Outcome::Success => (
            true,
            match &job.kind {
                JobKind::Test { mode, connector, .. } => {
                    if job.answered == Some(true) {
                        format!("Kept {mode} on {connector} until reboot. Apply it to make it permanent.")
                    } else {
                        format!("{connector} is back on its own EDID.")
                    }
                }
                JobKind::Apply { mode, .. } => {
                    format!("Installed {mode}. Reboot to load it at boot.")
                }
                JobKind::Remove { .. } => "Override removed. Reboot to finish.".into(),
                JobKind::Reset { connector } => format!("{connector} is back on its own EDID."),
            },
        ),
        Outcome::Cancelled => (false, "Cancelled: the password prompt was dismissed.".into()),
        Outcome::NotAuthorized => (
            false,
            "Not authorized. This needs Desktop Mode (for the password prompt) and an administrator \
             password; on SteamOS set one with `passwd`."
                .into(),
        ),
        Outcome::Failed(why) => (
            false,
            format!("{} failed ({why}). See Output for details.", job.kind.title()),
        ),
    }
}

/// "ASUSTeK COMPUTER INC. ROG Ally X RC72LA" -> "ASUS ROG Ally X RC72LA".
fn short_hardware(name: &str) -> String {
    match name {
        "Valve Jupiter" => "Steam Deck".into(),
        "Valve Galileo" => "Steam Deck OLED".into(),
        _ => name
            .replace("ASUSTeK COMPUTER INC.", "ASUS")
            .replace("LENOVO", "Lenovo")
            .replace("Micro-Star International Co., Ltd.", "MSI"),
    }
}

/// A ring that empties as the countdown runs out.
fn countdown(ui: &mut Ui, left: u64, total: u64) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(84.0, 84.0), egui::Sense::hover());
    let c = rect.center();
    let r = 36.0;
    let painter = ui.painter();
    painter.circle_stroke(c, r, Stroke::new(7.0, p.raised_hover));
    let frac = left as f32 / total as f32;
    let color = if left <= 5 { p.warn } else { p.accent };
    let points: Vec<_> = (0..=48)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / 48.0;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    if frac > 0.0 {
        painter.add(Shape::line(points, Stroke::new(7.0, color)));
    }
    painter.text(
        c,
        egui::Align2::CENTER_CENTER,
        left.to_string(),
        bold(28.0),
        p.text,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_names_are_shortened() {
        assert_eq!(
            short_hardware("ASUSTeK COMPUTER INC. ROG Ally X RC72LA"),
            "ASUS ROG Ally X RC72LA"
        );
        assert_eq!(short_hardware("Valve Jupiter"), "Steam Deck");
    }
}
