//! The egui theme: bundled fonts, text styles, spacing and widget colors
//! derived from the tokens.

use std::sync::Arc;

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Style,
    TextStyle, Visuals,
};

use crate::tokens::*;

const SANS_MEDIUM: &str = "sans-medium";
const SANS_SEMIBOLD: &str = "sans-semibold";
const MONO_MEDIUM: &str = "mono-medium";

/// Proportional UI text at `size` px.
pub fn sans(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// Proportional UI text, weight 500.
pub fn sans_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SANS_MEDIUM.into()))
}

/// Proportional UI text, weight 600: titles and card headers.
pub fn sans_semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SANS_SEMIBOLD.into()))
}

/// Monospace text: every number, unit, path and identifier.
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Monospace text, weight 500.
pub fn mono_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MONO_MEDIUM.into()))
}

/// Named text styles beyond egui's five, keyed by the token scale.
pub fn text_style(name: &str) -> TextStyle {
    TextStyle::Name(Arc::from(name))
}

/// The bundled font set: egui's defaults stay as fallbacks for symbols the
/// Plex families lack.
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        "plex-sans",
        include_bytes!("../fonts/IBMPlexSans-Regular.ttf"),
    );
    add(
        "plex-sans-medium",
        include_bytes!("../fonts/IBMPlexSans-Medium.ttf"),
    );
    add(
        "plex-sans-semibold",
        include_bytes!("../fonts/IBMPlexSans-SemiBold.ttf"),
    );
    add(
        "plex-mono",
        include_bytes!("../fonts/IBMPlexMono-Regular.ttf"),
    );
    add(
        "plex-mono-medium",
        include_bytes!("../fonts/IBMPlexMono-Medium.ttf"),
    );

    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with = |first: &[&str]| -> Vec<String> {
        first
            .iter()
            .map(|s| s.to_string())
            .chain(fallbacks.iter().cloned())
            .collect()
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with(&["plex-sans"]));
    fonts
        .families
        .insert(FontFamily::Monospace, with(&["plex-mono", "plex-sans"]));
    fonts.families.insert(
        FontFamily::Name(SANS_MEDIUM.into()),
        with(&["plex-sans-medium", "plex-sans"]),
    );
    fonts.families.insert(
        FontFamily::Name(SANS_SEMIBOLD.into()),
        with(&["plex-sans-semibold", "plex-sans"]),
    );
    fonts.families.insert(
        FontFamily::Name(MONO_MEDIUM.into()),
        with(&["plex-mono-medium", "plex-mono", "plex-sans"]),
    );
    fonts
}

fn widget(
    bg: Color32,
    weak_bg: Color32,
    border: Color32,
    fg: Color32,
) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: weak_bg,
        bg_stroke: Stroke::new(1.0, border),
        corner_radius: CornerRadius::same(RADIUS_MD as u8),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    }
}

/// The visuals block: dark surfaces, one accent, borders everywhere.
pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT1);
    v.weak_text_color = Some(TEXT3);
    v.panel_fill = BG1;
    v.window_fill = BG1;
    v.extreme_bg_color = BG2;
    v.faint_bg_color = BG2;
    v.code_bg_color = BG0;
    v.text_edit_bg_color = Some(BG2);
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.window_corner_radius = CornerRadius::same(RADIUS_LG as u8);
    v.window_shadow = SHADOW_DIALOG;
    v.popup_shadow = SHADOW_POPOVER;
    v.menu_corner_radius = CornerRadius::same(RADIUS_MD as u8);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARNING;
    v.error_fg_color = DANGER;
    v.widgets.noninteractive = widget(BG2, BG1, BORDER, TEXT2);
    v.widgets.inactive = widget(BG3, BG2, BORDER, TEXT2);
    v.widgets.hovered = widget(BG3, BG3, BORDER_STRONG, TEXT1);
    v.widgets.active = widget(ACCENT_DIM, ACCENT_DIM, ACCENT, ACCENT);
    v.widgets.open = widget(BG3, BG3, BORDER_STRONG, TEXT1);
    v.striped = false;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.disabled_alpha = 0.5;
    v
}

/// Install fonts, text styles, spacing and visuals on `ctx`. Called once
/// when the UI layer is created.
pub fn apply_theme(ctx: &Context) {
    ctx.set_fonts(font_definitions());
    ctx.all_styles_mut(|style: &mut Style| {
        style.text_styles = [
            (TextStyle::Small, sans(FONT_XS)),
            (TextStyle::Body, sans(FONT_MD)),
            (TextStyle::Button, sans(FONT_SM)),
            (TextStyle::Heading, sans_semibold(FONT_LG)),
            (TextStyle::Monospace, mono(FONT_SM)),
            (text_style("xs"), sans(FONT_XS)),
            (text_style("sm"), sans(FONT_SM)),
            (text_style("md"), sans(FONT_MD)),
            (text_style("lg"), sans_semibold(FONT_LG)),
            (text_style("xl"), sans_semibold(FONT_XL)),
            (text_style("xxl"), sans_semibold(FONT_XXL)),
            (text_style("mono-xs"), mono(FONT_XS)),
        ]
        .into_iter()
        .collect();
        style.spacing.item_spacing = egui::vec2(SPACE_2, SPACE_1);
        style.spacing.button_padding = egui::vec2(SPACE_2, 3.0);
        style.spacing.interact_size = egui::vec2(INPUT, INPUT);
        style.spacing.indent = SPACE_4;
        style.spacing.menu_margin = egui::Margin::same(SPACE_1 as i8);
        style.spacing.window_margin = egui::Margin::same(SPACE_3 as i8);
        style.spacing.icon_width = 14.0;
        style.spacing.combo_width = 160.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = false;
        style.visuals = visuals();
    });
}
