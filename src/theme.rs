//! Palette "Zen Modern" e stile dell'interfaccia.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const BG: Color32 = Color32::from_rgb(0xCD, 0xEF, 0xE6);
pub const SURFACE: Color32 = Color32::from_rgb(0xD8, 0xF5, 0xE8);
pub const PRIMARY: Color32 = Color32::from_rgb(0x2A, 0x9D, 0x8F);
pub const PRIMARY_LIGHT: Color32 = Color32::from_rgb(0xB5, 0xE6, 0xD8);
pub const SECONDARY: Color32 = Color32::from_rgb(0xE9, 0xC4, 0x6A);
pub const TEXT: Color32 = Color32::from_rgb(0x26, 0x46, 0x53);
pub const MUTED: Color32 = Color32::from_rgb(0x3F, 0x7F, 0x8F);
pub const ERROR: Color32 = Color32::from_rgb(0xE7, 0x6F, 0x51);
pub const BORDER: Color32 = Color32::from_rgb(0x8F, 0xCF, 0xBD);
/// Testo sui pulsanti colorati: giallo chiaro (mai bianco).
pub const ON_COLOR: Color32 = Color32::from_rgb(0xFF, 0xF4, 0xB8);
pub const LOG_BG: Color32 = Color32::from_rgb(0x1D, 0x2D, 0x35);
pub const LOG_INFO: Color32 = Color32::from_rgb(0x4D, 0x96, 0xFF);
pub const LOG_WARN: Color32 = Color32::from_rgb(0xFF, 0xD9, 0x3D);
pub const LOG_ERR: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
pub const LOG_OK: Color32 = Color32::from_rgb(0x6B, 0xCB, 0x77);
pub const AMBIGUOUS_BG: Color32 = Color32::from_rgb(0xFF, 0xF3, 0xCD);
pub const AMBIGUOUS_FG: Color32 = Color32::from_rgb(0x85, 0x64, 0x04);
pub const DISABLED: Color32 = Color32::from_rgb(0x9C, 0xC4, 0xCF);

pub fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE;
    v.faint_bg_color = Color32::from_rgb(0xC4, 0xEC, 0xDE);
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
    colorize(
        &mut v,
        Color32::from_rgb(0xB0, 0xE6, 0xD6),
        Color32::from_rgb(0x26, 0x46, 0x53),
    );
    v
}

pub fn apply(ctx: &egui::Context) {
    let v = visuals();
    // Sempre tema chiaro, qualunque sia il tema di Windows: il tema scuro di egui è grigio.
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.set_visuals_of(egui::Theme::Light, v.clone());
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
}

/// Bianco e grigio non devono esistere: ogni colore neutro (anche quelli che egui mette di
/// default, come il testo dei pulsanti, le ombre o i bordi) prende una tinta.
/// I chiari vanno verso `light`, gli scuri verso `dark`.
pub fn colorize(v: &mut egui::Visuals, light: Color32, dark: Color32) {
    fn fix(c: &mut Color32, light: Color32, dark: Color32) {
        if !is_neutral(*c) {
            return;
        }
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let lum = (u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)) / 10;
        let (tone, t) = if lum >= 128 {
            (light, 0.85)
        } else {
            (dark, 0.9)
        };
        let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
        let [tr, tg, tb, _] = tone.to_srgba_unmultiplied();
        *c = Color32::from_rgba_unmultiplied(mix(r, tr), mix(g, tg), mix(b, tb), a);
    }
    for c in [
        &mut v.panel_fill,
        &mut v.window_fill,
        &mut v.extreme_bg_color,
        &mut v.faint_bg_color,
        &mut v.code_bg_color,
        &mut v.window_stroke.color,
        &mut v.window_shadow.color,
        &mut v.popup_shadow.color,
        &mut v.selection.bg_fill,
        &mut v.selection.stroke.color,
        &mut v.hyperlink_color,
        &mut v.text_cursor.stroke.color,
        &mut v.warn_fg_color,
        &mut v.error_fg_color,
    ] {
        fix(c, light, dark);
    }
    for c in [
        &mut v.override_text_color,
        &mut v.weak_text_color,
        &mut v.text_edit_bg_color,
    ]
    .into_iter()
    .flatten()
    {
        fix(c, light, dark);
    }
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        fix(&mut w.bg_fill, light, dark);
        fix(&mut w.weak_bg_fill, light, dark);
        fix(&mut w.bg_stroke.color, light, dark);
        fix(&mut w.fg_stroke.color, light, dark);
    }
}

/// Neutro = bianco, grigio o quasi (i canali RGB differiscono di meno di 24).
pub fn is_neutral(c: Color32) -> bool {
    let [r, g, b, _] = c.to_srgba_unmultiplied().map(i32::from);
    r.max(g).max(b) - r.min(g).min(b) < 24
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_white_or_gray_anywhere_in_the_visuals() {
        let v = visuals();
        let mut named: Vec<(&str, Color32)> = vec![
            ("panel", v.panel_fill),
            ("window", v.window_fill),
            ("faint", v.faint_bg_color),
            ("extreme", v.extreme_bg_color),
            ("code", v.code_bg_color),
            ("window_stroke", v.window_stroke.color),
            ("window_shadow", v.window_shadow.color),
            ("popup_shadow", v.popup_shadow.color),
            ("selection", v.selection.bg_fill),
            ("selection_stroke", v.selection.stroke.color),
            ("hyperlink", v.hyperlink_color),
            ("cursor", v.text_cursor.stroke.color),
            ("warn", v.warn_fg_color),
            ("error", v.error_fg_color),
        ];
        for (n, c) in [
            ("text", v.override_text_color),
            ("weak_text", v.weak_text_color),
            ("text_edit", v.text_edit_bg_color),
        ] {
            if let Some(c) = c {
                named.push((n, c));
            }
        }
        for (n, w) in [
            ("noninteractive", v.widgets.noninteractive),
            ("inactive", v.widgets.inactive),
            ("hovered", v.widgets.hovered),
            ("active", v.widgets.active),
            ("open", v.widgets.open),
        ] {
            named.push((n, w.bg_fill));
            named.push((n, w.weak_bg_fill));
            named.push((n, w.bg_stroke.color));
            named.push((n, w.fg_stroke.color));
        }
        for (n, c) in named {
            assert!(!is_neutral(c), "{n}: {c:?} è bianco o grigio");
        }
    }

    #[test]
    fn palette_constants_are_never_white_or_gray() {
        for (n, c) in [
            ("BG", BG),
            ("SURFACE", SURFACE),
            ("PRIMARY", PRIMARY),
            ("PRIMARY_LIGHT", PRIMARY_LIGHT),
            ("TEXT", TEXT),
            ("MUTED", MUTED),
            ("BORDER", BORDER),
            ("ON_COLOR", ON_COLOR),
        ] {
            assert!(!is_neutral(c), "{n}: {c:?}");
        }
    }
}
