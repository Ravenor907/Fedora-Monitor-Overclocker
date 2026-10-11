//! The app's own controls: big, touch-friendly, keyboard-focusable and drawn
//! with the palette so both themes stay consistent.

use super::theme::{CARD_RADIUS, Palette, RADIUS, TOUCH, bold, font, palette, tint};
use crate::doctor::Level;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, EventFilter, Frame, Key, Margin, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Ui, Vec2, Widget, WidgetInfo, WidgetType, pos2,
    text::LayoutJob, vec2,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// A rounded button with a visible focus ring.
pub struct Button {
    text: String,
    kind: Kind,
    min_width: f32,
    height: f32,
    text_size: f32,
}

impl Button {
    pub fn new(text: impl Into<String>, kind: Kind) -> Button {
        Button {
            text: text.into(),
            kind,
            min_width: 0.0,
            height: TOUCH,
            text_size: 16.0,
        }
    }

    pub fn text_size(mut self, size: f32) -> Button {
        self.text_size = size;
        self
    }

    pub fn min_width(mut self, width: f32) -> Button {
        self.min_width = width;
        self
    }

    pub fn height(mut self, height: f32) -> Button {
        self.height = height;
        self
    }
}

impl Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        let p = palette(ui.ctx());
        let galley = ui.painter().layout_no_wrap(
            self.text.clone(),
            bold(self.text_size),
            Color32::PLACEHOLDER,
        );
        let width = (galley.size().x + 40.0)
            .max(self.min_width)
            .max(self.height);
        let (rect, response) = ui.allocate_exact_size(vec2(width, self.height), Sense::click());
        response
            .widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &self.text));
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let hovered = response.hovered() && ui.is_enabled();
        let pressed = response.is_pointer_button_down_on();
        let (fill, stroke, text) = match self.kind {
            Kind::Primary => (
                if pressed || hovered {
                    p.accent_hover
                } else {
                    p.accent
                },
                Stroke::NONE,
                p.on_accent,
            ),
            Kind::Secondary => (
                if hovered { p.raised_hover } else { p.raised },
                Stroke::new(1.0, p.border),
                p.text,
            ),
            Kind::Danger => (
                tint(p.bad, if hovered { 60 } else { 34 }),
                Stroke::new(1.0, tint(p.bad, 90)),
                p.bad,
            ),
            Kind::Ghost => (
                if hovered {
                    p.raised
                } else {
                    Color32::TRANSPARENT
                },
                Stroke::NONE,
                p.accent,
            ),
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(RADIUS),
            fill,
            stroke,
            StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, text);
        focus_ring(ui, &response, rect, RADIUS);
        response
    }
}

/// `[−]  135 Hz  [+]` across the full width, with large buttons.
pub fn stepper(
    ui: &mut Ui,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    unit: &str,
    size: f32,
) -> bool {
    let p = palette(ui.ctx());
    let (lo, hi) = (*range.start(), *range.end());
    let height = (size * 1.2).max(TOUCH);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let side = TOUCH + 6.0;
    let button_rect = |x: f32| Rect::from_center_size(pos2(x, rect.center().y), vec2(side, side));
    let old = *value;
    let minus = Button::new("−", Kind::Secondary)
        .min_width(side)
        .height(side)
        .text_size(22.0);
    if ui
        .put(button_rect(rect.min.x + side / 2.0), |ui: &mut Ui| {
            ui.add_enabled(*value > lo, minus)
        })
        .clicked()
    {
        *value -= 1;
    }
    let plus = Button::new("+", Kind::Secondary)
        .min_width(side)
        .height(side)
        .text_size(22.0);
    if ui
        .put(button_rect(rect.max.x - side / 2.0), |ui: &mut Ui| {
            ui.add_enabled(*value < hi, plus)
        })
        .clicked()
    {
        *value += 1;
    }
    let job = number_job(&value.to_string(), unit, p.text, p.text_dim, size);
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, p.text);
    *value != old
}

fn number_job(
    value: &str,
    unit: &str,
    color: Color32,
    unit_color: Color32,
    size: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        value,
        0.0,
        egui::TextFormat {
            font_id: bold(size),
            color,
            ..Default::default()
        },
    );
    job.append(
        unit,
        size * 0.12,
        egui::TextFormat {
            font_id: font(size * 0.38),
            color: unit_color,
            valign: egui::Align::BOTTOM,
            ..Default::default()
        },
    );
    job
}

pub fn button(ui: &mut Ui, text: impl Into<String>, kind: Kind) -> Response {
    ui.add(Button::new(text, kind))
}

pub fn button_if(ui: &mut Ui, enabled: bool, text: impl Into<String>, kind: Kind) -> Response {
    ui.add_enabled(enabled, Button::new(text, kind))
}

/// A 2 px accent outline around keyboard-focused controls (gamepad users
/// navigate with the D-pad mapped to arrow keys).
pub fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: u8) {
    if response.gained_focus() {
        // Keyboard and gamepad users move focus off-screen; follow it.
        response.scroll_to_me(None);
    }
    if response.has_focus() {
        let p = palette(ui.ctx());
        ui.painter().rect_stroke(
            rect.expand(3.0),
            CornerRadius::same(radius.saturating_add(3)),
            Stroke::new(2.0, p.accent),
            StrokeKind::Outside,
        );
    }
}

/// A raised card.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A tinted callout with a colored edge, for states that need attention.
pub fn banner<R>(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    let shown = Frame::new()
        .fill(tint(color, if p.dark { 24 } else { 22 }))
        .stroke(Stroke::new(1.0, tint(color, if p.dark { 80 } else { 110 })))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin {
            left: 26,
            right: 20,
            top: 18,
            bottom: 18,
        })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        });
    let rect = shown.response.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.min.x + 5.0, rect.max.y)),
        CornerRadius {
            nw: CARD_RADIUS,
            sw: CARD_RADIUS,
            ne: 0,
            se: 0,
        },
        color,
    );
    shown.inner
}

/// Small letter-spaced uppercase caption, e.g. "MEASURED".
pub fn caption(ui: &mut Ui, text: &str, color: Color32) {
    let mut job = LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: bold(12.0),
            color,
            extra_letter_spacing: 1.2,
            ..Default::default()
        },
    );
    ui.label(job);
}

/// A caption with something right-aligned on the same line (a chip, a
/// value), without the touch-height row `ui.horizontal` would reserve.
pub fn caption_row(ui: &mut Ui, text: &str, right: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal(|ui| {
            caption(ui, text, p.text_faint);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
        });
    });
}

/// Dim body text that wraps.
pub fn dim(ui: &mut Ui, text: impl Into<String>) -> Response {
    let p = palette(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(p.text_dim)).wrap())
}

/// Faint small text that wraps.
pub fn faint(ui: &mut Ui, text: impl Into<String>) -> Response {
    let p = palette(ui.ctx());
    ui.add(
        egui::Label::new(
            egui::RichText::new(text.into())
                .font(font(14.0))
                .color(p.text_faint),
        )
        .wrap(),
    )
}

/// A pill-shaped status label.
pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let p = palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), bold(13.0), color);
    let size = vec2(galley.size().x + 24.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(14),
            tint(color, if p.dark { 34 } else { 26 }),
            Stroke::new(1.0, tint(color, if p.dark { 70 } else { 90 })),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    response
}

/// A neutral pill for plain facts (connector names, drivers).
pub fn tag(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font(13.5), p.text_dim);
    let size = vec2(galley.size().x + 22.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(14),
            p.raised,
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, p.text_dim);
    }
    response
}

pub fn level_color(p: &Palette, level: Level) -> Color32 {
    match level {
        Level::Ok => p.good,
        Level::Info => p.accent,
        Level::Warn => p.warn,
        Level::Fail => p.bad,
    }
}

/// A round status badge: check, info, exclamation or cross.
pub fn status_icon(ui: &mut Ui, level: Level, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_status(ui, rect, level);
    response
}

pub fn paint_status(ui: &Ui, rect: Rect, level: Level) {
    let p = palette(ui.ctx());
    let color = level_color(p, level);
    let c = rect.center();
    let r = rect.width() / 2.0;
    let painter = ui.painter();
    painter.circle(c, r, tint(color, 40), Stroke::new(1.0, tint(color, 120)));
    let stroke = Stroke::new((r * 0.16).max(1.6), color);
    let at = |x: f32, y: f32| c + vec2(x, y) * r;
    match level {
        Level::Ok => {
            painter.add(Shape::line(
                vec![at(-0.38, 0.02), at(-0.1, 0.3), at(0.4, -0.26)],
                stroke,
            ));
        }
        Level::Info => {
            painter.circle_filled(at(0.0, -0.36), stroke.width * 0.7, color);
            painter.line_segment([at(0.0, -0.08), at(0.0, 0.42)], stroke);
        }
        Level::Warn => {
            painter.line_segment([at(0.0, -0.42), at(0.0, 0.12)], stroke);
            painter.circle_filled(at(0.0, 0.38), stroke.width * 0.7, color);
        }
        Level::Fail => {
            painter.line_segment([at(-0.3, -0.3), at(0.3, 0.3)], stroke);
            painter.line_segment([at(0.3, -0.3), at(-0.3, 0.3)], stroke);
        }
    }
}

/// Rows of "label   value" with dim labels.
pub fn facts(ui: &mut Ui, id: &str, rows: &[(&str, String)]) {
    let p = palette(ui.ctx());
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(vec2(24.0, 10.0))
        .min_row_height(22.0)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(egui::RichText::new(*label).color(p.text_dim));
                ui.add(egui::Label::new(egui::RichText::new(value).color(p.text)).wrap());
                ui.end_row();
            }
        });
}

/// Pick one of many with ‹ and › (D-pad friendly, unlike a drop-down).
/// Returns true when the choice changed.
pub fn picker(ui: &mut Ui, index: &mut usize, labels: &[String]) -> bool {
    let p = palette(ui.ctx());
    if labels.is_empty() {
        return false;
    }
    let old = *index;
    *index = (*index).min(labels.len() - 1);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), TOUCH), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), p.raised);
    let side = TOUCH;
    let left = Rect::from_min_size(rect.min, vec2(side, side));
    let right = Rect::from_min_size(pos2(rect.max.x - side, rect.min.y), vec2(side, side));
    let arrow = |text: &str| {
        Button::new(text, Kind::Ghost)
            .min_width(side)
            .height(side)
            .text_size(24.0)
    };
    if ui
        .put(left, |ui: &mut Ui| ui.add_enabled(*index > 0, arrow("‹")))
        .clicked()
    {
        *index -= 1;
    }
    if ui
        .put(right, |ui: &mut Ui| {
            ui.add_enabled(*index + 1 < labels.len(), arrow("›"))
        })
        .clicked()
    {
        *index += 1;
    }
    let text = format!("{}   ({}/{})", labels[*index], *index + 1, labels.len());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        font(16.0),
        p.text,
    );
    *index != old
}

/// One option per segment; returns true when the selection changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = palette(ui.ctx());
    let width = ui.available_width();
    let (track, _) = ui.allocate_exact_size(vec2(width, TOUCH), Sense::hover());
    ui.painter()
        .rect_filled(track, CornerRadius::same(RADIUS), p.raised);
    let inner = track.shrink(4.0);
    let seg_w = inner.width() / options.len().max(1) as f32;
    let mut changed = false;
    for (i, (option, label)) in options.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(inner.min.x + seg_w * i as f32, inner.min.y),
            vec2(seg_w, inner.height()),
        );
        let id = ui.id().with(("segment", i, *label));
        let response = ui.interact(rect, id, Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::RadioButton,
                ui.is_enabled(),
                *value == *option,
                label,
            )
        });
        if response.clicked() && *value != *option {
            *value = *option;
            changed = true;
        }
        let selected = *value == *option;
        let (fill, text) = if selected {
            (tint(p.accent, if p.dark { 44 } else { 34 }), p.accent)
        } else if response.hovered() && ui.is_enabled() {
            (p.raised_hover, p.text)
        } else {
            (Color32::TRANSPARENT, p.text_dim)
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(RADIUS - 3), fill);
        if selected {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(RADIUS - 3),
                Stroke::new(1.0, tint(p.accent, 140)),
                StrokeKind::Inside,
            );
        }
        let font = if selected { bold(15.0) } else { font(15.0) };
        ui.painter()
            .text(rect.center(), Align2::CENTER_CENTER, label, font, text);
        focus_ring(ui, &response, rect, RADIUS - 3);
    }
    changed
}

/// A switch with a label; the whole row is the hit target.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let p = palette(ui.ctx());
    let galley = ui.painter().layout(
        label.to_string(),
        font(16.0),
        p.text,
        ui.available_width() - 76.0,
    );
    let height = (galley.size().y + 12.0).max(TOUCH);
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, label));
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let track = Rect::from_min_size(pos2(rect.min.x, rect.center().y - 15.0), vec2(54.0, 30.0));
    let off_fill = if p.dark { p.raised_hover } else { p.border };
    let fill = lerp_color(off_fill, p.accent, t);
    ui.painter()
        .rect_filled(track, CornerRadius::same(15), fill);
    let knob_x = egui::lerp(track.min.x + 15.0..=track.max.x - 15.0, t);
    ui.painter()
        .circle_filled(pos2(knob_x, track.center().y), 11.0, Color32::WHITE);
    ui.painter().galley(
        pos2(track.max.x + 16.0, rect.center().y - galley.size().y / 2.0),
        galley,
        p.text,
    );
    focus_ring(ui, &response, track, 15);
    response
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A wide slider for whole numbers with a large knob. Left/Right (D-pad)
/// step by one when it has focus.
pub fn slider(ui: &mut Ui, value: &mut u32, range: std::ops::RangeInclusive<u32>) -> Response {
    let p = palette(ui.ctx());
    let (lo, hi) = (*range.start(), (*range.end()).max(*range.start() + 1));
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(ui.available_width(), TOUCH), Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), *value as f64, ""));
    let knob_r = 13.0;
    let rail = Rect::from_min_max(
        pos2(rect.min.x + knob_r, rect.center().y - 4.0),
        pos2(rect.max.x - knob_r, rect.center().y + 4.0),
    );
    let old = *value;
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    if let Some(pos) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked())
    {
        let t = ((pos.x - rail.min.x) / rail.width()).clamp(0.0, 1.0);
        *value = lo + ((hi - lo) as f32 * t).round() as u32;
    }
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            )
        });
        let (down, up) = ui.input(|i| {
            (
                i.num_presses(Key::ArrowLeft) as u32,
                i.num_presses(Key::ArrowRight) as u32,
            )
        });
        *value = value.saturating_add(up).saturating_sub(down);
    }
    *value = (*value).clamp(lo, hi);
    if *value != old {
        response.mark_changed();
    }
    let t = (*value - lo) as f32 / (hi - lo) as f32;
    let x = egui::lerp(rail.x_range(), t);
    let painter = ui.painter();
    painter.rect_filled(rail, CornerRadius::same(4), p.raised_hover);
    painter.rect_filled(
        Rect::from_min_max(rail.min, pos2(x, rail.max.y)),
        CornerRadius::same(4),
        p.accent,
    );
    let center = pos2(x, rail.center().y);
    if response.has_focus() || response.dragged() {
        painter.circle_filled(center, knob_r + 7.0, tint(p.accent, 50));
    }
    painter.circle(center, knob_r, Color32::WHITE, Stroke::new(3.0, p.accent));
    response
}

/// Navigation icons, drawn as vectors so they stay crisp at any UI scale.
#[derive(Clone, Copy)]
pub enum Icon {
    Handheld,
    Monitor,
    Stopwatch,
    Pulse,
    Update,
}

pub fn paint_icon(ui: &Ui, rect: Rect, icon: Icon, color: Color32) {
    let painter = ui.painter();
    let c = rect.center();
    let s = rect.height();
    let stroke = Stroke::new(1.8, color);
    let at = |x: f32, y: f32| c + vec2(x, y) * s;
    match icon {
        Icon::Handheld => {
            let body = Rect::from_center_size(c, vec2(s * 1.0, s * 0.58));
            painter.rect_stroke(
                body,
                CornerRadius::same((s * 0.22) as u8),
                stroke,
                StrokeKind::Middle,
            );
            let screen = Rect::from_center_size(c, vec2(s * 0.40, s * 0.32));
            painter.rect_stroke(screen, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.circle_filled(at(-0.33, -0.04), s * 0.065, color);
            painter.circle_filled(at(0.33, 0.06), s * 0.065, color);
        }
        Icon::Monitor => {
            let screen = Rect::from_center_size(at(0.0, -0.08), vec2(s * 0.95, s * 0.62));
            painter.rect_stroke(screen, CornerRadius::same(3), stroke, StrokeKind::Middle);
            painter.line_segment([pos2(c.x, screen.max.y), at(0.0, 0.38)], stroke);
            painter.line_segment([at(-0.22, 0.38), at(0.22, 0.38)], stroke);
        }
        Icon::Stopwatch => {
            let r = s * 0.4;
            let center = at(0.0, 0.08);
            let points: Vec<Pos2> = (0..=32)
                .map(|i| {
                    let a = (i as f32 / 32.0) * std::f32::consts::TAU;
                    center + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            painter.add(Shape::line(points, stroke));
            painter.line_segment([at(-0.12, -0.46), at(0.12, -0.46)], stroke);
            painter.line_segment([at(0.0, -0.46), at(0.0, -0.32)], stroke);
            painter.line_segment([center, center + vec2(0.16, -0.16) * s], stroke);
        }
        Icon::Update => {
            // A circular arrow, like a refresh symbol.
            let r = s * 0.36;
            let points: Vec<Pos2> = (0..=28)
                .map(|i| {
                    let a = -1.2 + (i as f32 / 28.0) * 5.0;
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            let tip = points[points.len() - 1];
            painter.add(Shape::line(points, stroke));
            let back: f32 = -1.2 + 5.0;
            let along = vec2(-back.sin(), back.cos());
            let out = vec2(back.cos(), back.sin());
            let len = s * 0.2;
            painter.line_segment([tip, tip - along * len + out * len * 0.6], stroke);
            painter.line_segment([tip, tip - along * len - out * len * 0.6], stroke);
        }
        Icon::Pulse => {
            let points = [
                (-0.5, 0.05),
                (-0.22, 0.05),
                (-0.12, -0.3),
                (0.04, 0.38),
                (0.16, -0.12),
                (0.24, 0.05),
                (0.5, 0.05),
            ];
            painter.add(Shape::line(
                points.iter().map(|&(x, y)| at(x, y)).collect(),
                stroke,
            ));
        }
    }
}

/// A sidebar entry: icon and label, accent bar when selected.
pub fn nav_item(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let p = palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label));
    let fill = if selected {
        tint(p.accent, if p.dark { 30 } else { 26 })
    } else if response.hovered() {
        p.raised
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS), fill);
    if selected {
        painter.rect_filled(
            Rect::from_center_size(pos2(rect.min.x + 2.0, rect.center().y), vec2(4.0, 24.0)),
            CornerRadius::same(2),
            p.accent,
        );
    }
    let color = if selected { p.accent } else { p.text_dim };
    paint_icon(
        ui,
        Rect::from_center_size(pos2(rect.min.x + 30.0, rect.center().y), Vec2::splat(22.0)),
        icon,
        color,
    );
    painter.text(
        pos2(rect.min.x + 56.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        if selected { bold(16.0) } else { font(16.0) },
        if selected { p.text } else { p.text_dim },
    );
    focus_ring(ui, &response, rect, RADIUS);
    response
}

/// The app mark: a speed gauge sweeping from cyan to hot pink, like the
/// app icon.
pub fn logo(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same((size * 0.28) as u8),
        Color32::from_rgb(0x16, 0x1B, 0x2A),
    );
    let center = rect.center() + vec2(0.0, size * 0.06);
    let r = size * 0.32;
    let stops = [
        Color32::from_rgb(0x22, 0xD3, 0xEE),
        Color32::from_rgb(0x8B, 0x5C, 0xF6),
        Color32::from_rgb(0xF4, 0x3F, 0x5E),
    ];
    let segments = 24;
    for i in 0..segments {
        let t0 = i as f32 / segments as f32;
        let t1 = (i + 1) as f32 / segments as f32;
        let angle = |t: f32| (135.0 + 270.0 * t).to_radians();
        let point = |t: f32| center + vec2(angle(t).cos(), angle(t).sin()) * r;
        let color = if t0 < 0.5 {
            lerp_color(stops[0], stops[1], t0 * 2.0)
        } else {
            lerp_color(stops[1], stops[2], (t0 - 0.5) * 2.0)
        };
        painter.line_segment([point(t0), point(t1)], Stroke::new(size * 0.1, color));
    }
    let needle = 300f32.to_radians();
    painter.line_segment(
        [center, center + vec2(needle.cos(), needle.sin()) * r * 0.75],
        Stroke::new(size * 0.07, Color32::WHITE),
    );
    painter.circle_filled(center, size * 0.07, Color32::WHITE);
}

/// A large bold number with a small unit, for values that matter.
pub fn big_number(ui: &mut Ui, value: &str, unit: &str, color: Color32, size: f32) -> Response {
    let p = palette(ui.ctx());
    ui.label(number_job(value, unit, color, p.text_dim, size))
}
