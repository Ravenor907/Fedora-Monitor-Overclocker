//! External monitors: build a mode with a live preview of the EDID change,
//! then test it live or install it through pkexec.

use super::{Action, Dialog, View, columns, header, res, row};
use crate::edid::Location;
use crate::gui::data::Monitor;
use crate::gui::jobs::{Kind as JobKind, Outcome, TEST_SECONDS};
use crate::gui::theme::{TOUCH, bold, palette};
use crate::gui::widgets::{
    Button, Kind, banner, big_number, button_if, caption, card, chip, dim, facts, faint, picker,
    segmented, slider, tag, toggle,
};
use crate::plan::{self, Plan};
use crate::timing::{Mode, Strategy};
use eframe::egui::{self, RichText, ScrollArea, Ui};

#[derive(Default)]
pub struct State {
    connector: String,
    /// Index into the monitor's resolutions.
    res: usize,
    hz: u32,
    strategy: Option<Strategy>,
    preferred: bool,
    switch_mode: bool,
    pub log_open: bool,
}

impl State {
    /// Fresh settings for `m`. In Game Mode the new mode is made the preferred
    /// one by default, since Gamescope uses that on external screens.
    fn reset_for(&mut self, m: &Monitor, game_mode: bool) {
        let native = m.resolutions().first().map_or(60.0, |r| r.2);
        *self = State {
            connector: m.connector.clone(),
            hz: native.round() as u32 + 11,
            strategy: Some(Strategy::Auto),
            switch_mode: true,
            preferred: game_mode,
            log_open: self.log_open,
            ..State::default()
        };
    }

    /// The mode being edited and the job that would test or apply it.
    fn mode(&self, m: &Monitor) -> Option<Mode> {
        let (width, height, _) = *m.resolutions().get(self.res)?;
        Some(Mode {
            width,
            height,
            hz: self.hz as f64,
        })
    }
}

pub fn show(ui: &mut Ui, st: &mut State, view: &View, out: &mut Vec<Action>) {
    header(
        ui,
        "External monitors",
        "Add a faster mode to a monitor's EDID so every desktop and Game Mode can use it. Asks for your password.",
    );
    let p = palette(ui.ctx());
    let monitors = &view.snap.monitors;
    if monitors.is_empty() {
        card(ui, |ui| {
            ui.label(RichText::new("No external monitor connected").font(bold(19.0)));
            dim(
                ui,
                "Plug a monitor into DisplayPort, HDMI or USB-C (a dock works too). It shows up here \
                 within a few seconds.",
            );
        });
        log_panel(ui, st, view);
        return;
    }

    if !monitors.iter().any(|m| m.connector == st.connector) {
        st.reset_for(&monitors[0], view.game_mode);
    }
    if monitors.len() > 1 {
        let mut chosen = monitors
            .iter()
            .position(|m| m.connector == st.connector)
            .unwrap_or(0);
        let labels: Vec<String> = monitors
            .iter()
            .map(|m| format!("{}  ·  {}", m.connector, m.name()))
            .collect();
        let options: Vec<(usize, &str)> = labels
            .iter()
            .enumerate()
            .map(|(i, l)| (i, l.as_str()))
            .collect();
        if segmented(ui, &mut chosen, &options) {
            st.reset_for(&monitors[chosen], view.game_mode);
        }
    }
    let Some(monitor) = monitors.iter().find(|m| m.connector == st.connector) else {
        return;
    };

    monitor_card(ui, monitor);
    let mode = st.mode(monitor);
    let strategy = st.strategy.unwrap_or(Strategy::Auto);
    let built = mode.map(|m| (m, plan::build(&monitor.edid, &m, strategy, st.preferred)));
    let gate = Gate::new(view);
    columns(ui, 0.5, |ui, col| match (col, &built) {
        (0, _) => editor(ui, st, monitor),
        (_, Some((mode, Ok(plan)))) => card(ui, |ui| {
            preview(ui, mode, plan, monitor);
            ui.add_space(8.0);
            primary_actions(ui, st, &gate, monitor, *mode, plan, out);
        }),
        (_, Some((_, Err(e)))) => card(ui, |ui| {
            caption(ui, "Preview", p.text_faint);
            ui.label(
                RichText::new("This mode can't be built")
                    .font(bold(18.0))
                    .color(p.bad),
            );
            dim(ui, format!("{e:#}"));
        }),
        (_, None) => {}
    });
    if let Some((mode, _)) = &built {
        more_actions(ui, st, &gate, monitor, *mode, out);
    }
    log_panel(ui, st, view);
}

/// Whether privileged actions can run right now, and why not.
struct Gate {
    blocked: Option<&'static str>,
    busy: bool,
    kscreen: bool,
    game_mode: bool,
}

impl Gate {
    fn new(view: &View) -> Gate {
        let tools = view.snap.tools;
        Gate {
            blocked: if !tools.sudo && !tools.pkexec && !tools.root {
                Some(
                    "Neither sudo nor pkexec is installed, so the app can't run the change as an \
                     administrator. Use the terminal command below instead.",
                )
            } else {
                None
            },
            busy: view.job.is_some_and(|j| j.running()),
            kscreen: tools.kscreen_doctor && !view.game_mode,
            game_mode: view.game_mode,
        }
    }

    fn ready(&self) -> bool {
        self.blocked.is_none() && !self.busy
    }
}

fn test_kind(st: &State, m: &Monitor, mode: Mode, gate: &Gate) -> JobKind {
    JobKind::Test {
        connector: m.connector.clone(),
        mode,
        strategy: st.strategy.unwrap_or(Strategy::Auto),
        preferred: st.preferred,
        switch_mode: st.switch_mode && gate.kscreen,
    }
}

fn monitor_card(ui: &mut Ui, m: &Monitor) {
    let p = palette(ui.ctx());
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(m.name()).font(bold(20.0)));
            tag(ui, &m.connector);
            if let Some(driver) = &m.driver {
                tag(ui, driver);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(mode) = &m.installed {
                    chip(ui, &format!("Installed: {mode}"), p.good);
                }
                if m.overridden {
                    chip(ui, "Modified EDID active", p.accent);
                }
            });
        });
        let mut modes: Vec<String> = m
            .resolutions()
            .iter()
            .map(|(w, h, hz)| format!("{} @ {hz:.0} Hz", res(w, h)))
            .collect();
        modes.dedup();
        if let Some(r) = m.edid.range_limits() {
            modes.push(format!(
                "range {}–{} Hz, up to {} MHz",
                r.min_v_hz, r.max_v_hz, r.max_clock_mhz
            ));
        }
        faint(ui, modes.join("   ·   "));
    });
}

fn editor(ui: &mut Ui, st: &mut State, m: &Monitor) {
    let p = palette(ui.ctx());
    let resolutions = m.resolutions();
    card(ui, |ui| {
        caption(ui, "New mode", p.text_faint);
        ui.add_space(2.0);
        ui.label(RichText::new("Resolution").color(p.text_dim));
        let labels: Vec<String> = resolutions
            .iter()
            .map(|(w, h, hz)| format!("{}  ·  now up to {hz:.0} Hz", res(w, h)))
            .collect();
        picker(ui, &mut st.res, &labels);
        let native_hz = resolutions[st.res.min(resolutions.len() - 1)].2.round() as u32;
        let (lo, hi) = (native_hz + 1, (native_hz as f64 * 1.6).round() as u32);
        st.hz = st.hz.clamp(lo, hi);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Refresh rate").color(p.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        st.hz < hi,
                        Button::new("+", Kind::Secondary)
                            .min_width(TOUCH)
                            .text_size(22.0),
                    )
                    .clicked()
                {
                    st.hz += 1;
                }
                ui.label(
                    RichText::new(format!("{} Hz", st.hz))
                        .font(bold(30.0))
                        .color(p.text),
                );
                if ui
                    .add_enabled(
                        st.hz > lo,
                        Button::new("−", Kind::Secondary)
                            .min_width(TOUCH)
                            .text_size(22.0),
                    )
                    .clicked()
                {
                    st.hz -= 1;
                }
            });
        });
        slider(ui, &mut st.hz, lo..=hi);
        ui.add_space(6.0);
        ui.label(RichText::new("Timing").color(p.text_dim));
        let mut strategy = st.strategy.unwrap_or(Strategy::Auto);
        segmented(
            ui,
            &mut strategy,
            &[
                (Strategy::Auto, "Auto"),
                (Strategy::Native, "Native"),
                (Strategy::CvtRb, "CVT-RB"),
                (Strategy::CvtRb2, "CVT-RB2"),
            ],
        );
        st.strategy = Some(strategy);
        faint(
            ui,
            match strategy {
                Strategy::Auto => "Native when the monitor has this resolution, otherwise CVT-RB2.",
                Strategy::Native => {
                    "The monitor's own blanking with a faster pixel clock. Most likely to work."
                }
                Strategy::CvtRb => "VESA reduced blanking v1: very widely accepted.",
                Strategy::CvtRb2 => {
                    "VESA reduced blanking v2: lowest bandwidth, for cable or port limits."
                }
            },
        );
        ui.add_space(4.0);
        toggle(
            ui,
            &mut st.preferred,
            "Make it the preferred mode (Game Mode uses it on external screens)",
        );
    });
}

fn preview(ui: &mut Ui, mode: &Mode, plan: &Plan, m: &Monitor) {
    let p = palette(ui.ctx());
    caption(ui, "Preview", p.text_faint);
    let d = &plan.dtd;
    big_number(
        ui,
        &format!("{:.1}", d.clock_mhz()),
        "MHz pixel clock",
        p.text,
        40.0,
    );
    ui.add_space(2.0);
    let placement = match plan.location {
        Location::DisplayId { .. } => format!("{} (over 655 MHz)", plan.location),
        _ => plan.location.to_string(),
    };
    facts(
        ui,
        "plan-facts",
        &[
            ("Refresh", format!("{:.3} Hz", d.refresh_hz())),
            ("Totals", format!("{} × {}", d.h_total(), d.v_total())),
            ("Line rate", format!("{:.2} kHz", d.h_freq_khz())),
            ("Timing", plan.strategy.to_string()),
            ("Stored in", placement),
        ],
    );
    if !plan.limit_changes.is_empty() {
        ui.add_space(2.0);
        ui.label(RichText::new("Range limits raised").color(p.text_dim));
        for change in &plan.limit_changes {
            faint(ui, format!("•  {change}"));
        }
    }
    for note in plan.notes(mode, Some(&m.connector)) {
        let color = if note.is_warning() {
            p.warn
        } else {
            p.text_dim
        };
        ui.add(egui::Label::new(RichText::new(note.to_string()).color(color)).wrap());
    }
}

/// Test and Apply, right under the preview they act on.
fn primary_actions(
    ui: &mut Ui,
    st: &mut State,
    gate: &Gate,
    m: &Monitor,
    mode: Mode,
    plan: &Plan,
    out: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    row(ui, |ui| {
        let test = format!("Test live  ·  {TEST_SECONDS} s");
        if button_if(ui, gate.ready(), test, Kind::Primary).clicked() {
            st.log_open = true;
            out.push(Action::Run(test_kind(st, m, mode, gate)));
        }
        if button_if(ui, gate.ready(), "Apply permanently…", Kind::Secondary).clicked() {
            out.push(Action::Ask(Dialog::Apply {
                kind: JobKind::Apply {
                    connector: m.connector.clone(),
                    mode,
                    strategy: st.strategy.unwrap_or(Strategy::Auto),
                    preferred: st.preferred,
                },
                summary: apply_summary(&m.connector, &mode, plan),
            }));
        }
        if gate.busy {
            ui.add(egui::Spinner::new().size(22.0));
        }
    });
    match gate.blocked {
        Some(why) => ui.add(egui::Label::new(RichText::new(why).color(p.warn)).wrap()),
        None => faint(
            ui,
            format!(
                "Test live reverts by itself after {TEST_SECONDS} s unless you keep it, so a black \
                 screen fixes itself. Apply installs the mode for every boot."
            ),
        ),
    };
}

/// Removing an override and the same change as a terminal command.
fn more_actions(
    ui: &mut Ui,
    st: &mut State,
    gate: &Gate,
    m: &Monitor,
    mode: Mode,
    out: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    if m.installed.is_some() || m.overridden {
        card(ui, |ui| {
            caption(ui, "Active override", p.text_faint);
            // `apply` installs files and kernel arguments; a kept test only
            // lives in the kernel until reboot.
            let connector = m.connector.clone();
            let (label, kind, note) = match &m.installed {
                Some(mode) => (
                    "Remove override…",
                    JobKind::Remove { connector },
                    format!("{mode} is installed. Removing it needs a reboot to finish."),
                ),
                None => (
                    "Drop live override…",
                    JobKind::Reset { connector },
                    "A kept test is active until reboot; nothing is installed.".to_string(),
                ),
            };
            row(ui, |ui| {
                if button_if(ui, gate.ready(), label, Kind::Danger).clicked() {
                    out.push(Action::Ask(Dialog::Remove(kind)));
                }
                faint(ui, note);
            });
        });
    }
    // A terminal command is no use in Game Mode.
    if gate.game_mode {
        return;
    }
    card(ui, |ui| {
        caption(ui, "Prefer a terminal?", p.text_faint);
        if gate.kscreen {
            toggle(
                ui,
                &mut st.switch_mode,
                "Switch to the new mode automatically while testing (KDE)",
            );
        }
        let command = test_kind(st, m, mode, gate).command_line();
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(p.raised)
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 110.0);
                    ui.add(
                        egui::Label::new(RichText::new(&command).monospace().color(p.text_dim))
                            .wrap(),
                    );
                });
            if ui
                .add(Button::new("Copy", Kind::Ghost).min_width(90.0))
                .on_hover_text("Copy the same test as a terminal command")
                .clicked()
            {
                out.push(Action::Copy(command.clone()));
            }
        });
    });
}

fn apply_summary(connector: &str, mode: &Mode, plan: &Plan) -> Vec<String> {
    let mut lines = vec![
        format!(
            "Adds {} ({:.1} MHz) to {connector}'s EDID, {}.",
            mode,
            plan.dtd.clock_mhz(),
            plan.location
        ),
        format!(
            "Saves it as /etc/firmware/edid/monitor-oc-{connector}.bin and adds kernel arguments to load it at boot."
        ),
        "Installs monitor-oc.service, which re-applies it after OS updates.".into(),
    ];
    lines.extend(
        plan.limit_changes
            .iter()
            .map(|c| format!("Raises the {c}.")),
    );
    lines.push(
        "Takes effect after a reboot. If the monitor shows no picture, use another input or port and Remove it."
            .into(),
    );
    lines
}

fn log_panel(ui: &mut Ui, st: &mut State, view: &View) {
    if view.log.is_empty() {
        return;
    }
    let p = palette(ui.ctx());
    card(ui, |ui| {
        let title = match view.job {
            Some(job) if job.running() => format!("Output  ·  {}", job.kind.title()),
            Some(job) => format!(
                "Output  ·  {}",
                match &job.outcome {
                    Some(Outcome::Success) => "finished".to_string(),
                    Some(Outcome::Cancelled) => "password prompt dismissed".to_string(),
                    Some(Outcome::NotAuthorized) => "not authorized".to_string(),
                    Some(Outcome::Failed(why)) => format!("failed: {why}"),
                    None => String::new(),
                }
            ),
            None => "Output".into(),
        };
        let response = ui.add(
            Button::new(
                format!("{}  {title}", if st.log_open { "▾" } else { "▸" }),
                Kind::Ghost,
            )
            .height(40.0),
        );
        if response.clicked() {
            st.log_open = !st.log_open;
        }
        if st.log_open {
            egui::Frame::new()
                .fill(p.bg)
                .corner_radius(10)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ScrollArea::vertical()
                        .max_height(240.0)
                        .stick_to_bottom(true)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for line in view.log {
                                ui.label(RichText::new(line).monospace().color(p.text_dim));
                            }
                        });
                });
        }
    });
}
