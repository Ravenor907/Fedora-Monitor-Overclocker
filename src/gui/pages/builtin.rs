//! Built-in screen: extend Gamescope's refresh range with a one-session trial.

use super::{Action, Dialog, View, columns, header, res, row};
use crate::doctor::ago;
use crate::gamescope::{Method, TrialState};
use crate::gui::data::{PanelAction, PanelInfo, Settings};
use crate::gui::theme::{bold, font, palette};
use crate::gui::widgets::{
    Button, Kind, banner, button, caption, caption_row, card, chip, dim, facts, faint, segmented,
    slider, stepper, tag,
};
use eframe::egui::{self, RichText, Ui, vec2};

#[derive(Default)]
pub struct State {
    /// Target rate for the next trial; 0 until the panel is known.
    target: u32,
    method: Method,
}

pub fn show(ui: &mut Ui, st: &mut State, view: &View, out: &mut Vec<Action>) {
    header(
        ui,
        "Built-in screen",
        "Let Game Mode offer higher refresh rates on this handheld's screen. Runs as you: no password needed.",
    );
    let info = match &view.snap.panel {
        Ok(info) => info,
        Err(why) => {
            card(ui, |ui| {
                caption(ui, "No built-in screen", palette(ui.ctx()).text_faint);
                dim(ui, why.clone());
            });
            return;
        }
    };
    let p = palette(ui.ctx());
    if !info.gamescope {
        banner(ui, p.accent, |ui| {
            ui.label(RichText::new("Gamescope isn't installed").font(bold(18.0)));
            dim(
                ui,
                "This page changes Game Mode (SteamOS, Bazzite and other Gamescope sessions). \
                 Without Gamescope there is nothing to configure here; use External monitors for \
                 screens that read the kernel's EDID.",
            );
        });
    }
    let settings = match &info.settings {
        Ok(s) => Some(s),
        Err(why) => {
            banner(ui, p.warn, |ui| {
                ui.label(RichText::new("Settings unavailable").font(bold(18.0)));
                dim(ui, why.clone());
            });
            None
        }
    };
    if st.target == 0 {
        let current = settings.map_or(0, |s| s.state.trial.map_or(s.state.max, |(hz, _)| hz));
        st.target = if current > info.stock_max {
            current
        } else {
            info.stock_max + 10
        }
        .clamp(info.stock_max + 1, info.max_allowed);
        st.method = settings.map_or(Method::Clock, |s| s.state.method);
    }
    if let Some(s) = settings {
        trial_banner(ui, info, s, view, out);
    }
    let usable = info.gamescope && settings.is_some();
    columns(ui, 0.57, |ui, col| {
        if col == 0 {
            new_limit(ui, st, info, usable, out);
            return;
        }
        screen_card(ui, info, settings);
        gamescope_card(ui, settings, view);
        if settings.is_some_and(|s| s.installed) {
            manage_card(ui, out);
        }
    });
}

fn trial_banner(ui: &mut Ui, info: &PanelInfo, s: &Settings, view: &View, out: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    let Some((hz, trial)) = s.state.trial else {
        return;
    };
    let can_switch = view.snap.tools.session_select && !view.game_mode;
    match trial {
        TrialState::Armed => banner(ui, p.accent, |ui| {
            caption(ui, "Trial ready", p.accent);
            ui.label(
                RichText::new(format!("{hz} Hz is ready for the next Game Mode session"))
                    .font(bold(20.0)),
            );
            steps(
                ui,
                &[
                    if view.game_mode {
                        "Restart Game Mode (for example switch to Desktop and back) so it loads the trial."
                            .to_string()
                    } else {
                        "Switch to Game Mode.".to_string()
                    },
                    "Open Monitor OC there and press Test.".to_string(),
                    "Keep it within 10 seconds if the picture is right, and once more after 5 minutes."
                        .to_string(),
                ],
            );
            faint(
                ui,
                "Nothing to keep? It goes back to stock by itself, black screen included.",
            );
            ui.add_space(4.0);
            row(ui, |ui| {
                if can_switch && button(ui, "Switch to Game Mode", Kind::Primary).clicked() {
                    out.push(Action::Ask(Dialog::GameMode));
                }
                if button(ui, "Discard", Kind::Secondary).clicked() {
                    out.push(Action::Panel(PanelAction::Discard));
                }
            });
        }),
        TrialState::Running => banner(ui, p.accent, |ui| {
            let loaded = s
                .report
                .as_ref()
                .is_some_and(|r| r.trial.as_deref() == Some("running"))
                && !s.stale
                && (view.game_mode || view.demo);
            caption(ui, "Trial ready to test", p.accent);
            ui.label(RichText::new(format!("Test {hz} Hz")).font(bold(24.0)));
            dim(
                ui,
                if loaded {
                    format!(
                        "The screen switches to {hz} Hz. Keep it within 10 seconds if the picture is \
                         right, then once more after 5 minutes. No answer, or a black screen, and it \
                         goes back to {} Hz by itself.",
                        info.stock_max.max(s.state.max)
                    )
                } else {
                    "Open Monitor OC in the Game Mode session that loaded the trial to test it. Until \
                     then the screen stays at its stock rate, even if a higher one is picked in Quick \
                     Access."
                        .to_string()
                },
            );
            ui.add_space(4.0);
            row(ui, |ui| {
                if loaded && button(ui, format!("Test {hz} Hz"), Kind::Primary).clicked() {
                    out.push(Action::PanelTest {
                        hz,
                        stock_max: info.stock_max.max(s.state.max),
                    });
                }
                if button(ui, "Discard", Kind::Secondary).clicked() {
                    out.push(Action::Panel(PanelAction::Discard));
                }
            });
        }),
        TrialState::Expired => banner(ui, p.warn, |ui| {
            caption(ui, "Trial ended", p.warn);
            ui.label(RichText::new(format!("{hz} Hz was turned off for safety")).font(bold(20.0)));
            dim(
                ui,
                format!(
                    "Game Mode restarted before the trial was confirmed (after a black screen and a \
                     forced restart, or just a restart), so it went back to {} Hz. Try again and keep \
                     it this time if it works, or pick a lower rate.",
                    info.stock_max.max(s.state.max)
                ),
            );
            ui.add_space(4.0);
            row(ui, |ui| {
                if button(ui, "Try again", Kind::Primary).clicked() {
                    out.push(Action::Panel(PanelAction::Try {
                        hz,
                        method: s.state.method,
                    }));
                }
                if button(ui, "Discard", Kind::Secondary).clicked() {
                    out.push(Action::Panel(PanelAction::Discard));
                }
            });
        }),
    }
    ui.add_space(6.0);
}

fn steps(ui: &mut Ui, lines: &[String]) {
    let p = palette(ui.ctx());
    for (i, line) in lines.iter().enumerate() {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 12.0, p.raised_hover);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                bold(13.0),
                p.text,
            );
            ui.add(egui::Label::new(RichText::new(line).color(p.text)).wrap());
        });
    }
}

fn new_limit(ui: &mut Ui, st: &mut State, info: &PanelInfo, usable: bool, out: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    card(ui, |ui| {
        ui.add_enabled_ui(usable, |ui| {
            let range = info.stock_max + 1..=info.max_allowed;
            caption_row(ui, "New refresh limit", |ui| {
                ui.label(
                    RichText::new(format!("{}–{} Hz", range.start(), range.end()))
                        .font(font(13.0))
                        .color(p.text_faint),
                );
            });
            stepper(ui, &mut st.target, range.clone(), "Hz", 64.0);
            slider(ui, &mut st.target, range);
            ui.add_space(4.0);
            caption(ui, "Method", p.text_faint);
            segmented(
                ui,
                &mut st.method,
                &[
                    (Method::Clock, "Raise pixel clock"),
                    (Method::Porch, "Shrink blanking"),
                ],
            );
            faint(
                ui,
                match st.method {
                    Method::Clock => {
                        "Keeps the panel's timing and runs it faster. Works on most panels (like CRU's default)."
                    }
                    Method::Porch => {
                        "Keeps the pixel clock and trims the blank lines first. Gentler on the link, harder on the panel."
                    }
                },
            );
            ui.add_space(6.0);
            let gain = st.target.saturating_sub(info.stock_max);
            if ui
                .add(
                    Button::new(format!("Start trial at {} Hz", st.target), Kind::Primary)
                        .min_width(ui.available_width())
                        .height(52.0),
                )
                .clicked()
            {
                out.push(Action::Panel(PanelAction::Try {
                    hz: st.target,
                    method: st.method,
                }));
            }
            faint(
                ui,
                format!(
                    "+{gain} Hz over stock, for one Game Mode session. Game Mode then offers every rate \
                     from {} Hz up to it.",
                    info.stock_min
                ),
            );
        });
    });
}

fn screen_card(ui: &mut Ui, info: &PanelInfo, settings: Option<&Settings>) {
    let p = palette(ui.ctx());
    card(ui, |ui| {
        caption(ui, "Screen", p.text_faint);
        let id = &info.panel.id;
        let model = [&id.model, &id.data_string]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "Built-in panel".into());
        ui.label(RichText::new(format!("{} {model}", id.vendor)).font(bold(19.0)));
        row(ui, |ui| {
            tag(ui, &info.panel.connector);
            tag(ui, &res(info.panel.width, info.panel.height));
        });
        ui.add_space(4.0);
        let limit = match settings.map(|s| s.state.max) {
            Some(max) if max > info.stock_max => format!("{max} Hz (kept)"),
            _ => format!("{} Hz (stock)", info.stock_max),
        };
        let mut rows = vec![(
            "Stock range",
            format!("{}–{} Hz", info.stock_min, info.stock_max),
        )];
        if let Some(name) = info.stock.as_ref().and_then(|s| s.pretty_name.clone()) {
            rows.push(("Profile", name));
        }
        rows.push(("Game Mode limit", limit));
        facts(ui, "screen-facts", &rows);
    });
}

fn gamescope_card(ui: &mut Ui, settings: Option<&Settings>, view: &View) {
    let p = palette(ui.ctx());
    let Some(s) = settings else { return };
    card(ui, |ui| {
        caption_row(ui, "Gamescope", |ui| {
            match (&s.report, s.stale) {
                (None, _) => chip(ui, "Not loaded yet", p.text_dim),
                (Some(_), true) => chip(ui, "Restart needed", p.warn),
                (Some(_), false) => chip(ui, "Loaded", p.good),
            };
        });
        let Some(r) = &s.report else {
            dim(
                ui,
                if view.game_mode {
                    "Gamescope hasn't loaded monitor-oc's profile yet. Restart Game Mode to load it."
                } else {
                    "Gamescope hasn't loaded monitor-oc's profile yet. It does when Game Mode starts."
                },
            );
            return;
        };
        let mut rows = vec![(
            "Loaded",
            r.loaded_at.map(ago).unwrap_or_else(|| "unknown".into()),
        )];
        rows.push((
            "Range",
            if r.extended {
                format!("{}–{} Hz (extended)", r.stock_min, r.limit)
            } else {
                format!("{}–{} Hz (stock)", r.stock_min, r.stock_max)
            },
        ));
        if let Some(mode) = &r.last_mode {
            rows.push((
                "Last mode",
                mode.replace('x', " × ").replace('@', " @ ") + " Hz",
            ));
        }
        if let Some(khz) = r.last_clock_khz {
            rows.push(("Pixel clock", format!("{:.1} MHz", khz as f64 / 1000.0)));
        }
        facts(ui, "gamescope-facts", &rows);
        if s.stale {
            ui.add_space(2.0);
            ui.label(
                RichText::new("Settings changed after this load. Restart Game Mode to apply them.")
                    .color(p.warn),
            );
        }
    });
}

fn manage_card(ui: &mut Ui, out: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    card(ui, |ui| {
        caption(ui, "Manage", p.text_faint);
        row(ui, |ui| {
            if button(ui, "Reset to stock", Kind::Secondary).clicked() {
                out.push(Action::Ask(Dialog::ResetPanel));
            }
            if button(ui, "Remove profile", Kind::Danger).clicked() {
                out.push(Action::Ask(Dialog::RemoveProfile));
            }
        });
        faint(
            ui,
            "Reset goes back to the stock range. Remove deletes monitor-oc's Gamescope profile.",
        );
    });
}
