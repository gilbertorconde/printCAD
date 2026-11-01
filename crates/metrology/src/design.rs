//! The value a measured size was most likely designed at: what a designer
//! would have typed, to offer beside the measurement.

/// A length's design value in millimetres: to the half under 10 mm, to the
/// whole millimetre above.
pub fn length_mm(measured: f64) -> f64 {
    if measured.abs() < 10.0 {
        (measured * 2.0).round() / 2.0
    } else {
        measured.round()
    }
}

/// A radius's design value in millimetres: to the half millimetre.
pub fn radius_mm(measured: f64) -> f64 {
    (measured * 2.0).round() / 2.0
}

/// An angle's design value in degrees: to five degrees.
pub fn angle_deg(measured: f64) -> f64 {
    (measured / 5.0).round() * 5.0
}

/// A length's design value in inches, given in millimetres and answered
/// in millimetres: to the sixteenth under an inch, the eighth under four
/// inches, the quarter above.
pub fn length_inch(measured_mm: f64) -> f64 {
    let inches = measured_mm / 25.4;
    let step = match inches.abs() {
        a if a < 1.0 => 1.0 / 16.0,
        a if a < 4.0 => 1.0 / 8.0,
        _ => 0.25,
    };
    (inches / step).round() * step * 25.4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_sizes_round_to_what_was_typed() {
        assert_eq!(length_mm(5.27), 5.5);
        assert_eq!(length_mm(24.38), 24.0);
        assert_eq!(radius_mm(2.79), 3.0);
        assert_eq!(angle_deg(44.48), 45.0);
        assert!((length_inch(12.6) - 12.7).abs() < 1e-9);
        assert!((length_inch(77.0) - 76.2).abs() < 1e-9);
    }
}
