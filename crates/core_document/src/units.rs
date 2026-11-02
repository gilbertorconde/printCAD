//! Per-document display units.
//!
//! All printCAD geometry is stored internally in **millimetres** (matching
//! the kernel's STEP import output). This module only describes how a length
//! is shown to the user and read from what they type; stored values never
//! change with it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A length unit that can be picked per document for display purposes.
///
/// Internal storage stays in millimetres; this enum controls how values are
/// rendered in the UI (status bar, dimension labels, etc.) and the unit a
/// bare number in a length formula is taken in.
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

impl Unit {
    /// The decimals a measured length shows with in this unit: three in
    /// millimetres, one more for each tenfold larger unit (four in inches).
    pub fn measure_decimals(self) -> usize {
        match self {
            Unit::Mm => 3,
            Unit::Cm => 4,
            Unit::In => 4,
            Unit::Ft => 5,
            Unit::M => 6,
        }
    }
}

/// A measured length given in millimetres, as `unit` shows it: converted,
/// at that unit's measuring precision, with its suffix (`0.0049 in`).
pub fn show_mm(value_mm: f64, unit: Unit) -> String {
    let converted = value_mm / f64::from(unit.mm_per_unit());
    format!(
        "{:.*} {}",
        unit.measure_decimals(),
        converted,
        unit.short_label()
    )
}

/// As [`show_mm`], with its sign always shown (a deviation).
pub fn show_mm_signed(value_mm: f64, unit: Unit) -> String {
    let converted = value_mm / f64::from(unit.mm_per_unit());
    format!(
        "{:+.*} {}",
        unit.measure_decimals(),
        converted,
        unit.short_label()
    )
}

/// Format an area given in square millimetres in `unit`², with its suffix.
pub fn format_area_mm2(value_mm2: f64, unit: Unit, decimals: usize) -> String {
    let per = f64::from(unit.mm_per_unit());
    format!(
        "{:.*} {}²",
        decimals,
        value_mm2 / (per * per),
        unit.short_label()
    )
}

/// Format a volume given in cubic millimetres in `unit`³, with its suffix.
pub fn format_volume_mm3(value_mm3: f64, unit: Unit, decimals: usize) -> String {
    let per = f64::from(unit.mm_per_unit());
    format!(
        "{:.*} {}³",
        decimals,
        value_mm3 / (per * per * per),
        unit.short_label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_and_volumes_scale_by_the_unit_squared_and_cubed() {
        assert_eq!(format_area_mm2(100.0, Unit::Cm, 2), "1.00 cm²");
        assert_eq!(format_volume_mm3(1_000.0, Unit::Cm, 3), "1.000 cm³");
        assert_eq!(format_volume_mm3(4_000.0, Unit::Mm, 1), "4000.0 mm³");
    }

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
    fn a_measured_length_shows_one_more_decimal_in_inches() {
        assert_eq!(show_mm(1.0, Unit::Mm), "1.000 mm");
        assert_eq!(show_mm(25.4, Unit::In), "1.0000 in");
        assert_eq!(show_mm_signed(-0.127, Unit::In), "-0.0050 in");
    }

    #[test]
    fn format_length_mm_includes_suffix() {
        assert_eq!(format_length_mm(25.4, Unit::In, 3), "1.000 in");
        assert_eq!(format_length_mm(1_000.0, Unit::M, 2), "1.00 m");
        assert_eq!(format_length_mm(12.5, Unit::Mm, 1), "12.5 mm");
    }
}
