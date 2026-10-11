//! The Updates page and the background work behind it (see `crate::update`
//! for how channels, builds and rollbacks work).

use super::pages::{Action, Dialog, header, row};
use super::theme::{bold, palette};
use super::widgets::{
    Kind, banner, button, button_if, caption, card, dim, facts, segmented, toggle,
};
use crate::update::{self, Build, Channel, Paths, Settings};
use eframe::egui::{self, RichText, Ui};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(Build),
    Installing(Build),
    /// Installed; takes effect when the app restarts.
    Installed(String),
    RollingBack,
    Failed(String),
}

impl Status {
    fn busy(&self) -> bool {
        matches!(
            self,
            Status::Checking | Status::Installing(_) | Status::RollingBack
        )
    }
}

pub enum UpdateAction {
    Check,
    Install(Build),
    Rollback,
    Channel(Channel),
    Auto(bool),
    Restart,
}

enum Msg {
    Checked {
        result: Result<update::Check, String>,
        auto: bool,
    },
    Installed(Result<Build, String>),
    RolledBack(Result<String, String>),
}

pub struct Updates {
    /// None in the demo, or when there is no user home to update.
    paths: Option<Paths>,
    pub settings: Settings,
    pub status: Status,
    /// Stable's build, offered when leaving Testing.
    stable: Option<Build>,
    /// The version kept for a rollback.
    rollback: Option<String>,
    /// A build rolled back on its own at startup.
    pub rolled_back_at_start: Option<String>,
    toasts: Vec<(bool, String)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
}

impl Updates {
    pub fn new(ctx: &egui::Context, demo: bool) -> Updates {
        let paths = if demo {
            None
        } else {
            Paths::for_current_user().ok()
        };
        let settings = paths
            .as_ref()
            .map_or_else(|| Settings::default_for(update::version()), Paths::settings);
        let (tx, rx) = mpsc::channel();
        let mut updates = Updates {
            rollback: paths.as_ref().and_then(Paths::rollback_target),
            paths,
            settings,
            status: Status::Idle,
            stable: None,
            rolled_back_at_start: std::env::var("MONITOR_OC_ROLLED_BACK")
                .ok()
                .filter(|v| !v.is_empty()),
            toasts: Vec::new(),
            tx,
            rx,
            ctx: ctx.clone(),
        };
        if let Some(paths) = &updates.paths {
            let _ = update::sync_rescue(paths, updates.settings.channel);
        }
        if let Some(bad) = updates.rolled_back_at_start.clone() {
            updates.toasts.push((
                false,
                format!(
                    "{} didn't start properly, so Monitor OC went back to the previous version.",
                    update::label_of(&bad)
                ),
            ));
        }
        updates.check(updates.settings.auto);
        updates
    }

    pub fn available(&self) -> bool {
        self.paths.is_some()
    }

    /// Something for the header: a build waiting to install or to restart.
    pub fn badge(&self) -> Option<&'static str> {
        match self.status {
            Status::Available(_) => Some("Update available"),
            Status::Installed(_) => Some("Restart to update"),
            _ => None,
        }
    }

    pub fn take_toasts(&mut self) -> Vec<(bool, String)> {
        std::mem::take(&mut self.toasts)
    }

    fn check(&mut self, auto: bool) {
        let Some(paths) = self.paths.clone() else {
            return;
        };
        if self.status.busy() {
            return;
        }
        self.status = Status::Checking;
        let channel = self.settings.channel;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let _ = thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let result = update::check(&paths, channel).map_err(|e| format!("{e:#}"));
                let _ = tx.send(Msg::Checked { result, auto });
                ctx.request_repaint();
            });
    }

    fn install(&mut self, build: Build) {
        let Some(paths) = self.paths.clone() else {
            return;
        };
        self.status = Status::Installing(build);
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let _ = thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let result = update::install(&paths, build).map_err(|e| format!("{e:#}"));
                let _ = tx.send(Msg::Installed(result));
                ctx.request_repaint();
            });
    }

    fn rollback(&mut self) {
        let Some(paths) = self.paths.clone() else {
            return;
        };
        self.status = Status::RollingBack;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let _ = thread::Builder::new()
            .name("update-rollback".into())
            .spawn(move || {
                let result = update::rollback(&paths).map_err(|e| format!("{e:#}"));
                let _ = tx.send(Msg::RolledBack(result));
                ctx.request_repaint();
            });
    }

    /// Take in finished background work.
    pub fn pump(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Checked { result, auto } => match result {
                    Ok(found) => {
                        self.stable = found.stable;
                        match found.newer {
                            Some(build) if auto => self.install(build),
                            Some(build) => self.status = Status::Available(build),
                            None => self.status = Status::UpToDate,
                        }
                    }
                    // Offline at startup is normal; only the page says so.
                    Err(e) => self.status = Status::Failed(e),
                },
                Msg::Installed(Ok(build)) => {
                    self.status = Status::Installed(build.to_string());
                    self.refresh_rollback();
                    self.toasts.push((
                        true,
                        format!(
                            "Updated to {}. Restart Monitor OC to use it.",
                            build.label()
                        ),
                    ));
                }
                Msg::Installed(Err(e)) => {
                    self.toasts.push((false, format!("Update failed: {e}")));
                    self.status = Status::Failed(e);
                }
                Msg::RolledBack(Ok(version)) => {
                    self.status = Status::Installed(version.clone());
                    self.refresh_rollback();
                    self.toasts.push((
                        true,
                        format!(
                            "Rolled back to {}. Restart Monitor OC to use it.",
                            update::label_of(&version)
                        ),
                    ));
                }
                Msg::RolledBack(Err(e)) => {
                    self.toasts.push((false, format!("Rollback failed: {e}")));
                    self.status = Status::Failed(e);
                }
            }
        }
    }

    fn refresh_rollback(&mut self) {
        self.rollback = self.paths.as_ref().and_then(Paths::rollback_target);
    }

    pub fn act(&mut self, action: UpdateAction) {
        match action {
            UpdateAction::Check => self.check(false),
            UpdateAction::Install(build) => {
                if !self.status.busy() {
                    self.install(build);
                }
            }
            UpdateAction::Rollback => {
                if !self.status.busy() {
                    self.rollback();
                }
            }
            UpdateAction::Channel(channel) => {
                let Some(paths) = &self.paths else {
                    return;
                };
                match update::set_channel(paths, channel) {
                    Ok(settings) => {
                        self.settings = settings;
                        self.stable = None;
                        self.status = Status::Idle;
                        self.check(false);
                    }
                    Err(e) => self
                        .toasts
                        .push((false, format!("Could not change the channel: {e:#}"))),
                }
            }
            UpdateAction::Auto(on) => {
                let Some(paths) = &self.paths else {
                    return;
                };
                let mut settings = self.settings;
                settings.auto = on;
                match paths.save_settings(&settings) {
                    Ok(()) => self.settings = settings,
                    Err(e) => self
                        .toasts
                        .push((false, format!("Could not save the setting: {e:#}"))),
                }
            }
            UpdateAction::Restart => self.restart(),
        }
    }

    /// Replace this process with the installed build, same arguments.
    fn restart(&mut self) {
        use std::os::unix::process::CommandExt;
        let Some(paths) = &self.paths else {
            return;
        };
        let err = std::process::Command::new(&paths.binary)
            .args(std::env::args_os().skip(1))
            .env_remove("MONITOR_OC_ROLLED_BACK")
            .exec();
        self.toasts.push((
            false,
            format!("Could not restart ({err}). Close Monitor OC and open it again."),
        ));
    }
}

pub fn show(ui: &mut Ui, updates: &Updates, out: &mut Vec<Action>) {
    header(
        ui,
        "Updates",
        "Monitor OC keeps itself up to date. Test builds get new fixes first.",
    );
    let p = palette(ui.ctx());
    if !updates.available() {
        banner(ui, p.text_faint, |ui| {
            dim(ui, "Updates are off in the demo and when running as root.");
        });
        return;
    }
    card(ui, |ui| {
        facts(
            ui,
            "update-facts",
            &[
                ("Installed", update::label()),
                ("Channel", updates.settings.channel.to_string()),
                (
                    "Kept for rollback",
                    updates
                        .rollback
                        .as_deref()
                        .map_or_else(|| "Nothing yet".into(), update::label_of),
                ),
            ],
        );
        ui.add_space(14.0);
        caption(ui, "Update channel", p.text_faint);
        let mut channel = updates.settings.channel;
        if segmented(
            ui,
            &mut channel,
            &[(Channel::Stable, "Stable"), (Channel::Testing, "Testing")],
        ) {
            match channel {
                Channel::Testing => out.push(Action::Ask(Dialog::Testing)),
                Channel::Stable => out.push(Action::Update(UpdateAction::Channel(Channel::Stable))),
            }
        }
        ui.add_space(8.0);
        let mut auto = updates.settings.auto;
        if toggle(ui, &mut auto, "Install updates when Monitor OC starts").changed() {
            out.push(Action::Update(UpdateAction::Auto(auto)));
        }
    });
    ui.add_space(12.0);

    let busy = updates.status.busy();
    card(ui, |ui| {
        let (text, color) = match &updates.status {
            Status::Idle => ("Not checked yet.".to_string(), p.text_dim),
            Status::Checking => ("Checking for updates…".into(), p.text_dim),
            Status::UpToDate => ("You're up to date.".into(), p.good),
            Status::Available(b) => (format!("{} is available.", b.label()), p.accent),
            Status::Installing(b) => (format!("Installing {}…", b.label()), p.text_dim),
            Status::Installed(v) => (
                format!(
                    "{} is installed. Restart Monitor OC to use it.",
                    update::label_of(v)
                ),
                p.good,
            ),
            Status::RollingBack => ("Rolling back…".into(), p.text_dim),
            Status::Failed(e) => (e.clone(), p.bad),
        };
        ui.horizontal_wrapped(|ui| {
            if busy {
                ui.add(egui::Spinner::new().size(20.0));
            }
            ui.add(egui::Label::new(RichText::new(text).font(bold(17.0)).color(color)).wrap());
        });
        ui.add_space(10.0);
        row(ui, |ui| {
            match &updates.status {
                Status::Available(b) => {
                    if button_if(ui, !busy, format!("Install {b}"), Kind::Primary).clicked() {
                        out.push(Action::Update(UpdateAction::Install(*b)));
                    }
                }
                Status::Installed(_) => {
                    if button(ui, "Restart now", Kind::Primary).clicked() {
                        out.push(Action::Update(UpdateAction::Restart));
                    }
                }
                _ => {}
            }
            if button_if(ui, !busy, "Check now", Kind::Secondary).clicked() {
                out.push(Action::Update(UpdateAction::Check));
            }
            if let Some(stable) = updates.stable
                && updates.settings.channel == Channel::Stable
                && button_if(
                    ui,
                    !busy,
                    format!("Go back to Stable {stable}"),
                    Kind::Secondary,
                )
                .clicked()
            {
                out.push(Action::Update(UpdateAction::Install(stable)));
            }
            if let Some(prev) = &updates.rollback
                && button_if(
                    ui,
                    !busy,
                    format!("Roll back to {}", update::label_of(prev)),
                    Kind::Danger,
                )
                .clicked()
            {
                out.push(Action::Update(UpdateAction::Rollback));
            }
        });
    });

    if updates.settings.channel == Channel::Testing {
        ui.add_space(12.0);
        banner(ui, p.warn, |ui| {
            ui.label(
                RichText::new("You're on test builds")
                    .font(bold(17.0))
                    .color(p.text),
            );
            for line in update::TESTING_DISCLAIMER {
                dim(ui, *line);
            }
        });
    }
}
