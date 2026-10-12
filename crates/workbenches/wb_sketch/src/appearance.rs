//! How the sketch draws while it is edited, as the Preferences page sets
//! it: the dimension labels' size and face, the dimension lines' and the
//! axes' width and dash, and how strongly the grid and the closed regions
//! show.

/// The dash a dashed dimension line or axis takes: drawn and gap, in px.
const DASH: (f32, f32) = (4.0, 3.0);

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// The dimension labels' size in pixels.
    pub label_size: f32,
    /// The labels are set in the monospace face; off, in the sans.
    pub label_mono: bool,
    /// The dimension, extension and leader lines' width in pixels.
    pub dimension_width: f32,
    pub dimension_dashed: bool,
    /// The two axes' width in pixels while not picked.
    pub axis_width: f32,
    pub axis_dashed: bool,
    /// How much the grid shows, 0..=1.
    pub grid_opacity: f32,
    /// How much of the geometry colour the closed regions take, 0..=1.
    pub region_opacity: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            label_size: 12.0,
            label_mono: true,
            dimension_width: 1.0,
            dimension_dashed: false,
            axis_width: 1.0,
            axis_dashed: false,
            grid_opacity: 0.35,
            region_opacity: 0.12,
        }
    }
}

impl Appearance {
    /// The dash of a line drawn dashed when `dashed`.
    pub fn dash(dashed: bool) -> Option<(f32, f32)> {
        dashed.then_some(DASH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::{Glyph, GlyphVisual, dimension_overlays};

    /// Settings saved before these existed load as the sketch drew then.
    #[test]
    fn settings_without_an_appearance_draw_as_before() {
        let old: crate::SketchOptions =
            serde_json::from_value(serde_json::json!({"grid_on": true})).unwrap();
        assert_eq!(old.appearance, Appearance::default());
        let partial: Appearance =
            serde_json::from_value(serde_json::json!({"label_size": 16.0})).unwrap();
        assert_eq!(partial.label_size, 16.0);
        assert_eq!(partial.grid_opacity, Appearance::default().grid_opacity);
    }

    /// A dimension's label and lines take the size, face, width and dash
    /// set.
    #[test]
    fn dimensions_draw_as_set() {
        let glyph = Glyph {
            constraint: uuid::Uuid::nil(),
            dimensional: true,
            pos: [10.0, 10.0],
            anchor: [0.0, 0.0],
            visual: GlyphVisual::Pill { text: "12".into() },
            color: [1.0; 3],
            lines: vec![([0.0, 0.0], [10.0, 0.0])],
        };
        let look = Appearance {
            label_size: 16.0,
            label_mono: false,
            dimension_width: 2.5,
            dimension_dashed: true,
            ..Appearance::default()
        };
        let label = glyph.label(&look).unwrap();
        assert_eq!(label.size, 16.0);
        assert!(!label.mono);
        let lines = dimension_overlays(std::slice::from_ref(&glyph), &look);
        assert_eq!(lines[0].thickness, 2.5);
        assert!(lines[0].dash.is_some());
        let plain = dimension_overlays(&[glyph], &Appearance::default());
        assert_eq!(plain[0].thickness, 1.0);
        assert!(plain[0].dash.is_none());
    }
}
