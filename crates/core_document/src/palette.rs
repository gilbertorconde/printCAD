//! The colors sketch geometry, constraints and dimensions draw in. The
//! defaults are the design system's sketch tokens; the runtime context
//! carries the active palette so a workbench never hard-codes a color.

/// RGB in 0..=1, the overlay contract's color form.
pub type Rgb = [f32; 3];

const fn hex(v: u32) -> Rgb {
    [
        ((v >> 16) & 0xFF) as f32 / 255.0,
        ((v >> 8) & 0xFF) as f32 / 255.0,
        (v & 0xFF) as f32 / 255.0,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchPalette {
    pub geometry: Rgb,
    pub construction: Rgb,
    pub external: Rgb,
    pub fully_constrained: Rgb,
    pub selected: Rgb,
    pub preselect: Rgb,
    /// Driving constraints and dimensions.
    pub constraint: Rgb,
    /// Reference (non-driving) dimensions.
    pub reference: Rgb,
    /// Inactive constraints.
    pub inactive: Rgb,
    pub axis_x: Rgb,
    pub axis_y: Rgb,
    /// Tool previews and ghosts.
    pub preview: Rgb,
    /// The span a trim would remove.
    pub trim: Rgb,
    /// Label pill background.
    pub pill_fill: Rgb,
}

impl Default for SketchPalette {
    fn default() -> Self {
        Self {
            geometry: hex(0xE6EBF0),
            construction: hex(0x4FA3E6),
            external: hex(0xC77DFF),
            fully_constrained: hex(0x4FD08F),
            selected: hex(0x7CC4F5),
            preselect: hex(0xF2D479),
            constraint: hex(0xE6A44F),
            reference: hex(0x4FA3E6),
            inactive: hex(0x667180),
            axis_x: hex(0xE86E6E),
            axis_y: hex(0x4FD08F),
            preview: hex(0x7CC4F5),
            trim: hex(0xE86E6E),
            pill_fill: hex(0x0F1216),
        }
    }
}
