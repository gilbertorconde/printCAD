//! Design tokens: the colors, sizes, radii and spacing of the design system.

use egui::{Color32, Shadow};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

// Surfaces, darkest to lightest.
pub const BG0: Color32 = rgb(0x0F1216);
pub const BG1: Color32 = rgb(0x151A20);
pub const BG2: Color32 = rgb(0x1C222A);
pub const BG3: Color32 = rgb(0x242C36);
pub const BG4: Color32 = rgb(0x2C3541);
pub const BORDER: Color32 = rgb(0x2B333E);
pub const BORDER_STRONG: Color32 = rgb(0x3A4553);

// Text.
pub const TEXT1: Color32 = rgb(0xE6EBF0);
pub const TEXT2: Color32 = rgb(0x9AA5B1);
pub const TEXT3: Color32 = rgb(0x667180);
/// Toolbar icon color at rest.
pub const ICON: Color32 = rgb(0xC9D1D9);

// Accent and semantic colors.
pub const ACCENT: Color32 = rgb(0x4FA3E6);
pub const ACCENT_HOVER: Color32 = rgb(0x6DB5EC);
/// Accent at 18% over a dark surface.
pub const ACCENT_DIM: Color32 = Color32::from_rgba_premultiplied(14, 29, 41, 46);
/// Accent at 8%: note cards and primary card fills.
pub const ACCENT_FAINT: Color32 = Color32::from_rgba_premultiplied(6, 13, 18, 20);
/// Text on an accent-filled surface.
pub const ACCENT_TEXT: Color32 = rgb(0x0B1520);
pub const SUCCESS: Color32 = rgb(0x4FD08F);
pub const WARNING: Color32 = rgb(0xE6A44F);
pub const DANGER: Color32 = rgb(0xE86E6E);
/// A mesh body in the tree: a hue no other state uses, so a mesh never
/// reads as a solid body (accent), the tip (success) or an error (danger).
pub const MESH: Color32 = rgb(0xC77DFF);
pub const INFO: Color32 = ACCENT;

// Sketch entity colors.
pub const SKETCH_GEOMETRY: Color32 = rgb(0xE6EBF0);
pub const SKETCH_CONSTRUCTION: Color32 = rgb(0x4FA3E6);
pub const SKETCH_EXTERNAL: Color32 = rgb(0xC77DFF);
pub const SKETCH_CONSTRAINT: Color32 = rgb(0xE6A44F);
/// A value a formula sets.
pub const SKETCH_FORMULA: Color32 = rgb(0x6FD3C9);
pub const SKETCH_FULLY_CONSTRAINED: Color32 = rgb(0x4FD08F);
pub const SKETCH_SELECTED: Color32 = rgb(0x7CC4F5);
pub const SKETCH_PRESELECT: Color32 = rgb(0xF2D479);

/// Annotations an imported file carries: dimensions, tolerances, datums
/// and notes, drawn over the scene.
pub const ANNOTATION: Color32 = rgb(0x7FD1E8);

// Axes.
pub const AXIS_X: Color32 = rgb(0xE86E6E);
pub const AXIS_Y: Color32 = rgb(0x4FD08F);
pub const AXIS_Z: Color32 = rgb(0x4FA3E6);

// Viewport backdrop.
pub const VIEWPORT_TOP: Color32 = rgb(0x1A2028);
pub const VIEWPORT_BOTTOM: Color32 = rgb(0x0D1014);

/// Floating cards over the viewport: bg1 at 92%.
pub const OVERLAY_CARD: Color32 = Color32::from_rgba_premultiplied(19, 24, 29, 235);

// Type scale.
pub const FONT_XS: f32 = 11.0;
pub const FONT_SM: f32 = 12.0;
pub const FONT_MD: f32 = 13.0;
pub const FONT_LG: f32 = 15.0;
pub const FONT_XL: f32 = 20.0;
pub const FONT_XXL: f32 = 28.0;

// Radii.
pub const RADIUS_SM: f32 = 3.0;
pub const RADIUS_MD: f32 = 6.0;
pub const RADIUS_LG: f32 = 10.0;

// Spacing.
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_5: f32 = 24.0;
pub const SPACE_6: f32 = 32.0;

// Control sizes.
pub const TOOLBAR_ICON: f32 = 20.0;
pub const TOOLBAR_BUTTON: f32 = 30.0;
pub const MENU_BAR: f32 = 28.0;
pub const TOOLBAR: f32 = 38.0;
pub const STATUS_BAR: f32 = 24.0;
pub const TREE_ROW: f32 = 24.0;
pub const INPUT: f32 = 26.0;
pub const TAB_BAR: f32 = 30.0;

/// Padding around a viewport label's text when it draws a pill, in pixels.
/// Shared with the sketcher's hit-testing so a click lands where the pill
/// is painted.
pub const PILL_PAD: [f32; 2] = [5.0, 2.0];

pub const SHADOW_POPOVER: Shadow = Shadow {
    offset: [0, 8],
    blur: 24,
    spread: 0,
    color: Color32::from_black_alpha(128),
};
pub const SHADOW_DIALOG: Shadow = Shadow {
    offset: [0, 16],
    blur: 48,
    spread: 0,
    color: Color32::from_black_alpha(153),
};

/// A token as the `[r, g, b]` floats the workbench overlay contract uses.
pub fn rgb_f32(color: Color32) -> [f32; 3] {
    let [r, g, b, _] = color.to_array();
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// `color` with its alpha scaled to `alpha` (0..=1).
pub fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha)
}
