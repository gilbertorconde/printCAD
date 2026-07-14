//! Per-document display units.
//!
//! All printCAD geometry is stored internally in **millimetres** (matching
//! OCCT's STEP import output). This module only describes how to *show* a
//! length to the user — conversion is purely a formatting concern.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A length unit that can be picked per document for display purposes.
///
/// Internal storage stays in millimetres; this enum only controls how values
/// are rendered in the UI (status bar, dimension labels, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Unit {
    /// Millimetres (1 unit = 1 mm). Default and matches storage.
    #[default]
    Mm,
    /// Centimetres (1 unit = 10 mm).
    Cm,
    /// Metres (1 unit = 1000 mm).
    M,
    /// Inches (1 unit = 25.4 mm).
    In,
    /// Feet (1 unit = 304.8 mm).
    Ft,
}

impl Unit {
    /// All variants in dropdown-ready order (small → large, then imperial).
    pub const ALL: [Unit; 5] = [Unit::Mm, Unit::Cm, Unit::M, Unit::In, Unit::Ft];

    /// How many millimetres are in one of `self`. Used to convert from the
    /// internal mm representation into the display unit.
    pub const fn mm_per_unit(self) -> f32 {
        match self {
            Unit::Mm => 1.0,
            Unit::Cm => 10.0,
            Unit::M => 1_000.0,
            Unit::In => 25.4,
            Unit::Ft => 304.8,
        }
    }

    /// Short label suitable for status-bar suffixes.
    pub const fn short_label(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Cm => "cm",
            Unit::M => "m",
            Unit::In => "in",
            Unit::Ft => "ft",
        }
    }

    /// Human-readable label for menus/dropdowns.
    pub const fn long_label(self) -> &'static str {
        match self {
            Unit::Mm => "Millimetres (mm)",
            Unit::Cm => "Centimetres (cm)",
            Unit::M => "Metres (m)",
            Unit::In => "Inches (in)",
            Unit::Ft => "Feet (ft)",
        }
    }

    /// Convert a value expressed in millimetres into this unit.
    pub fn from_mm(self, value_mm: f32) -> f32 {
        value_mm / self.mm_per_unit()
    }

    /// Convert a value expressed in this unit back into millimetres.
    pub fn to_mm(self, value: f32) -> f32 {
        value * self.mm_per_unit()
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.short_label())
    }
}

/// Format a length stored in millimetres using `unit` and `decimals` precision,
/// appending the unit's short suffix (e.g. `"12.345 mm"`).
pub fn format_length_mm(value_mm: f32, unit: Unit, decimals: usize) -> String {
    let converted = unit.from_mm(value_mm);
    format!("{:.*} {}", decimals, converted, unit.short_label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_per_unit_matches_known_constants() {
        assert_eq!(Unit::Mm.mm_per_unit(), 1.0);
        assert_eq!(Unit::Cm.mm_per_unit(), 10.0);
        assert_eq!(Unit::M.mm_per_unit(), 1_000.0);
        assert!((Unit::In.mm_per_unit() - 25.4).abs() < f32::EPSILON);
        assert!((Unit::Ft.mm_per_unit() - 304.8).abs() < 1e-3);
    }

    #[test]
    fn round_trip_conversion_is_identity() {
        for unit in Unit::ALL {
            let v = 123.456_f32;
            let back = unit.from_mm(unit.to_mm(v));
            assert!((back - v).abs() < 1e-3, "unit={unit:?} got {back}");
        }
    }

    #[test]
    fn format_length_mm_includes_suffix() {
        assert_eq!(format_length_mm(25.4, Unit::In, 3), "1.000 in");
        assert_eq!(format_length_mm(1_000.0, Unit::M, 2), "1.00 m");
        assert_eq!(format_length_mm(12.5, Unit::Mm, 1), "12.5 mm");
    }
}
