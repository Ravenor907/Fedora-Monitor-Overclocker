//! Colors, fonts and the egui style: near-black surfaces, one electric-cyan
//! accent, large touch targets.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow,
    Stroke, Style, TextStyle, Theme, Visuals,
    style::{HandleShape, Selection, WidgetVisuals, Widgets},
};
use std::sync::Arc;

/// Every color the app paints with, per theme.
pub struct Palette {
    /// Window background.
    pub bg: Color32,
    /// Top bar and sidebar.
    pub chrome: Color32,
    /// Cards.
    pub surface: Color32,
    /// Controls and wells inside cards.
    pub raised: Color32,
    /// Hovered controls.
    pub raised_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub dark: bool,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x09, 0x0B, 0x10),
    chrome: Color32::from_rgb(0x0D, 0x10, 0x16),
    surface: Color32::from_rgb(0x13, 0x17, 0x20),
    raised: Color32::from_rgb(0x1B, 0x21, 0x2D),
    raised_hover: Color32::from_rgb(0x24, 0x2B, 0x3A),
    border: Color32::from_rgb(0x22, 0x29, 0x36),
    text: Color32::from_rgb(0xEA, 0xEE, 0xF5),
    text_dim: Color32::from_rgb(0xA0, 0xA9, 0xBA),
    text_faint: Color32::from_rgb(0x6A, 0x74, 0x87),
    accent: Color32::from_rgb(0x1F, 0xCF, 0xFF),
    accent_hover: Color32::from_rgb(0x66, 0xDF, 0xFF),
    on_accent: Color32::from_rgb(0x02, 0x10, 0x18),
    good: Color32::from_rgb(0x3D, 0xDC, 0x97),
    warn: Color32::from_rgb(0xFF, 0xB5, 0x47),
    bad: Color32::from_rgb(0xFF, 0x5D, 0x6C),
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF2, 0xF4, 0xF8),
    chrome: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    raised: Color32::from_rgb(0xEC, 0xEF, 0xF4),
    raised_hover: Color32::from_rgb(0xE0, 0xE5, 0xEC),
    border: Color32::from_rgb(0xDC, 0xE1, 0xE9),
    text: Color32::from_rgb(0x0E, 0x13, 0x20),
    text_dim: Color32::from_rgb(0x4A, 0x55, 0x68),
    text_faint: Color32::from_rgb(0x7A, 0x85, 0x97),
    accent: Color32::from_rgb(0x00, 0x86, 0xC9),
    accent_hover: Color32::from_rgb(0x00, 0x74, 0xAF),
    on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    good: Color32::from_rgb(0x0E, 0x9F, 0x67),
    warn: Color32::from_rgb(0xB8, 0x6E, 0x00),
    bad: Color32::from_rgb(0xD4, 0x2E, 0x45),
    dark: false,
};

pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if ctx.theme() == Theme::Light {
        &LIGHT
    } else {
        &DARK
    }
}

/// `color` at `alpha` (0-255) opacity, for tinted backgrounds.
pub fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub const RADIUS: u8 = 12;
pub const CARD_RADIUS: u8 = 14;
/// Minimum height of anything you can press: comfortable for thumbs.
pub const TOUCH: f32 = 46.0;

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_style_of(Theme::Dark, style(&DARK));
    ctx.set_style_of(Theme::Light, style(&LIGHT));
}

/// Inter (embedded, Latin subset) with egui's fonts as fallback for other
/// scripts and symbols.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "fonts/Inter-Regular.otf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "fonts/Inter-SemiBold.otf"
        ))),
    );
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut regular = vec!["Inter".to_string()];
    regular.extend(fallback.iter().cloned());
    let mut strong = vec!["Inter-SemiBold".to_string(), "Inter".to_string()];
    strong.extend(fallback);
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(semibold(), strong);
    ctx.set_fonts(fonts);
}

fn widget(fill: Color32, stroke: Stroke, fg: Color32, expansion: f32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(RADIUS),
        fg_stroke: Stroke::new(1.5, fg),
        expansion,
    }
}

fn style(p: &Palette) -> Style {
    let mut visuals = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let none = Stroke::NONE;
    visuals.widgets = Widgets {
        noninteractive: widget(p.surface, Stroke::new(1.0, p.border), p.text_dim, 0.0),
        inactive: widget(p.raised, none, p.text, 0.0),
        hovered: widget(p.raised_hover, none, p.text, 0.0),
        active: widget(p.raised_hover, Stroke::new(2.0, p.accent), p.text, 0.0),
        open: widget(p.raised_hover, Stroke::new(1.0, p.border), p.text, 0.0),
    };
    visuals.selection = Selection {
        bg_fill: tint(p.accent, if p.dark { 90 } else { 60 }),
        stroke: Stroke::new(1.5, p.accent),
    };
    visuals.override_text_color = Some(p.text);
    visuals.weak_text_color = Some(p.text_dim);
    visuals.hyperlink_color = p.accent;
    visuals.faint_bg_color = p.raised;
    visuals.extreme_bg_color = p.bg;
    visuals.code_bg_color = p.raised;
    visuals.warn_fg_color = p.warn;
    visuals.error_fg_color = p.bad;
    visuals.window_fill = p.surface;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(18);
    visuals.window_shadow = Shadow {
        offset: [0, 12],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 160 } else { 60 }),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 140 } else { 40 }),
    };
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.panel_fill = p.bg;
    visuals.handle_shape = HandleShape::Circle;
    visuals.slider_trailing_fill = true;
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = false;

    let mut style = Style {
        visuals,
        ..Style::default()
    };
    style.text_styles = [
        (TextStyle::Small, font(13.0)),
        (TextStyle::Body, font(16.0)),
        (TextStyle::Button, bold(16.0)),
        (TextStyle::Heading, bold(26.0)),
        (
            TextStyle::Monospace,
            FontId::new(13.5, FontFamily::Monospace),
        ),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(12.0, 12.0);
    s.button_padding = egui::vec2(18.0, 11.0);
    s.interact_size = egui::vec2(48.0, TOUCH);
    s.menu_margin = Margin::same(8);
    s.window_margin = Margin::same(24);
    s.combo_height = 320.0;
    s.icon_width = 22.0;
    s.icon_width_inner = 12.0;
    s.icon_spacing = 10.0;
    s.scroll.bar_width = 8.0;
    s.scroll.floating = true;
    style.interaction.selectable_labels = false;
    style.animation_time = 0.12;
    style
}
