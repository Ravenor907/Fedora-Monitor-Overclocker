//! One module per page. Pages draw from a read-only [`View`] and ask the app
//! to do things by pushing [`Action`]s, so all side effects live in `app.rs`.

pub mod builtin;
pub mod diagnostics;
pub mod external;
pub mod verify;

use super::data::{PanelAction, Snapshot};
use super::jobs::{Job, Kind};
use super::theme::{bold, font, palette};
use eframe::egui::{self, Align, Layout, RichText, Ui, vec2};

pub struct View<'a> {
    pub snap: &'a Snapshot,
    pub demo: bool,
    pub game_mode: bool,
    pub job: Option<&'a Job>,
    pub log: &'a [String],
}

pub enum Action {
    /// Open Verify with this target rate.
    Verify(u32),
    Panel(PanelAction),
    Ask(Dialog),
    Run(Kind),
    Copy(String),
    Refresh,
    Update(crate::gui::updates::UpdateAction),
}

/// Changes that need a second, deliberate "yes".
pub enum Dialog {
    GameMode,
    ResetPanel,
    RemoveProfile,
    Apply {
        kind: Kind,
        summary: Vec<String>,
    },
    Remove(Kind),
    /// Switch the update channel to Testing.
    Testing,
}

pub fn header(ui: &mut Ui, title: &str, subtitle: &str) {
    let p = palette(ui.ctx());
    ui.label(RichText::new(title).font(bold(28.0)).color(p.text));
    ui.add_space(-4.0);
    ui.add(egui::Label::new(RichText::new(subtitle).font(font(16.0)).color(p.text_dim)).wrap());
    ui.add_space(10.0);
}

/// Two columns side by side, or stacked when the window is narrow. `add` is
/// called with column 0, then column 1.
pub fn columns(ui: &mut Ui, left_share: f32, mut add: impl FnMut(&mut Ui, usize)) {
    let width = ui.available_width();
    let gap = 20.0;
    if width < 820.0 {
        add(ui, 0);
        add(ui, 1);
        return;
    }
    let lw = ((width - gap) * left_share).floor();
    let rw = (width - gap - lw).floor() - 1.0;
    ui.horizontal_top(|ui| {
        for (col, w) in [(0, lw), (1, rw)] {
            if col == 1 {
                ui.add_space(gap - ui.spacing().item_spacing.x);
            }
            ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                add(ui, col);
            });
        }
    });
}

/// Lay out widgets left to right, wrapping onto new lines when needed.
pub fn row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
        add(ui);
    });
}

/// "1920x1080" with a proper multiplication sign.
pub fn res(w: impl std::fmt::Display, h: impl std::fmt::Display) -> String {
    format!("{w} × {h}")
}
