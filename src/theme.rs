//! Palette "Zen Modern" e stile dell'interfaccia.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const BG: Color32 = Color32::from_rgb(0xF4, 0xF7, 0xF6);
pub const SURFACE: Color32 = Color32::WHITE;
pub const PRIMARY: Color32 = Color32::from_rgb(0x2A, 0x9D, 0x8F);
pub const PRIMARY_LIGHT: Color32 = Color32::from_rgb(0xE8, 0xF5, 0xF3);
pub const SECONDARY: Color32 = Color32::from_rgb(0xE9, 0xC4, 0x6A);
pub const TEXT: Color32 = Color32::from_rgb(0x26, 0x46, 0x53);
pub const MUTED: Color32 = Color32::from_rgb(0x6C, 0x7A, 0x89);
pub const ERROR: Color32 = Color32::from_rgb(0xE7, 0x6F, 0x51);
pub const BORDER: Color32 = Color32::from_rgb(0xE0, 0xE6, 0xED);
pub const LOG_BG: Color32 = Color32::from_rgb(0x1D, 0x2D, 0x35);
pub const LOG_INFO: Color32 = Color32::from_rgb(0x4D, 0x96, 0xFF);
pub const LOG_WARN: Color32 = Color32::from_rgb(0xFF, 0xD9, 0x3D);
pub const LOG_ERR: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
pub const LOG_OK: Color32 = Color32::from_rgb(0x6B, 0xCB, 0x77);
pub const AMBIGUOUS_BG: Color32 = Color32::from_rgb(0xFF, 0xF3, 0xCD);
pub const AMBIGUOUS_FG: Color32 = Color32::from_rgb(0x85, 0x64, 0x04);
pub const DISABLED: Color32 = Color32::from_rgb(0xB0, 0xBE, 0xC5);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = PRIMARY_LIGHT;
    v.selection.stroke = Stroke::new(1.0, PRIMARY);
    v.hyperlink_color = PRIMARY;
    let r = CornerRadius::same(6);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        w.corner_radius = r;
    }
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, PRIMARY);
    v.widgets.active.bg_fill = PRIMARY;
    v.widgets.hovered.weak_bg_fill = PRIMARY_LIGHT;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
}
