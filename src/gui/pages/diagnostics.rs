//! Diagnostics: `monitor-oc doctor` with icons, plus a report to paste into
//! bug reports.

use super::{Action, View, header, row};
use crate::doctor::Level;
use crate::gui::theme::{bold, palette};
use crate::gui::widgets::{Kind, button, card, chip, level_color, paint_status};
use eframe::egui::{self, RichText, Sense, Ui, vec2};

pub fn show(ui: &mut Ui, view: &View, out: &mut Vec<Action>) {
    header(
        ui,
        "Diagnostics",
        "What monitor-oc sees on this system, and anything that would stop an overclock from working.",
    );
    let p = palette(ui.ctx());
    let checks = &view.snap.checks;
    let count = |level| checks.iter().filter(|c| c.level == level).count();
    row(ui, |ui| {
        if button(ui, "Refresh", Kind::Secondary).clicked() {
            out.push(Action::Refresh);
        }
        if button(ui, "Copy report", Kind::Primary).clicked() {
            out.push(Action::Copy(report(view)));
        }
        ui.add_space(8.0);
        let (fail, warn) = (count(Level::Fail), count(Level::Warn));
        if fail > 0 {
            chip(ui, &plural(fail, "problem"), p.bad);
        }
        if warn > 0 {
            chip(ui, &plural(warn, "warning"), p.warn);
        }
        if fail + warn == 0 {
            chip(ui, "All good", p.good);
        }
    });
    ui.add_space(4.0);
    card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, check) in checks.iter().enumerate() {
            if i > 0 {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(rect, 0, p.border);
            }
            ui.add_space(12.0);
            ui.horizontal_top(|ui| {
                // Icon centered on the first text line.
                let line = ui.text_style_height(&egui::TextStyle::Body);
                let (rect, _) = ui.allocate_exact_size(vec2(28.0, line), Sense::hover());
                paint_status(
                    ui,
                    egui::Rect::from_center_size(rect.center(), vec2(22.0, 22.0)),
                    check.level,
                );
                ui.add_space(6.0);
                ui.allocate_ui_with_layout(
                    vec2(210.0, line),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(210.0);
                        ui.label(RichText::new(&check.title).font(bold(16.0)).color(p.text));
                    },
                );
                let color = match check.level {
                    Level::Ok | Level::Info => p.text_dim,
                    level => level_color(p, level),
                };
                ui.add(egui::Label::new(RichText::new(&check.detail).color(color)).wrap());
            });
            ui.add_space(12.0);
        }
    });
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// Plain text for bug reports.
pub fn report(view: &View) -> String {
    let snap = view.snap;
    let mut out = format!(
        "monitor-oc {}{}\nSession: {}\n",
        env!("CARGO_PKG_VERSION"),
        if view.demo { " (demo data)" } else { "" },
        if view.game_mode {
            "Game Mode"
        } else {
            "Desktop"
        }
    );
    for c in &snap.checks {
        let tag = match c.level {
            Level::Ok => " ok ",
            Level::Info => "info",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        };
        out += &format!("[{tag}] {:<22} {}\n", c.title, c.detail);
    }
    for m in &snap.monitors {
        out += &format!("\n{} ({}):\n", m.connector, m.name());
        for (_, dtd) in m.edid.timings() {
            out += &format!("  {dtd}\n");
        }
    }
    out
}
