//! A dragged number in whole steps that stops on its limits: a handle in
//! the view (or a pointer drag anywhere) never carries a number past what
//! it can be, while a typed one may go past and be refused.

/// The number a drag at `raw` sets, in whole `step`s within
/// `least..=most`: the nearest step, and at or past a limit the limit
/// itself, whether or not it is a whole step. With a step of 1 and a most
/// of 5.3 a drag up goes 4, 5, 5.3, and back down 5.3, 5, 4. A step of
/// zero or less leaves `raw` unstepped.
pub fn nearest(raw: f64, step: f64, least: f64, most: f64) -> f64 {
    if raw >= most {
        return most;
    }
    if raw <= least {
        return least;
    }
    let stepped = if step > 0.0 {
        (raw / step).round() * step
    } else {
        raw
    };
    stepped.clamp(least, most)
}

#[cfg(test)]
mod tests {
    use super::nearest;

    #[test]
    fn a_drag_steps_and_stops_on_its_limits() {
        // (raw, step, least, most) -> set
        let cases = [
            // Up to a most of 5.3 in whole steps: 4, 5, then 5.3.
            (4.2, 1.0, 0.01, 5.3, 4.0),
            (5.1, 1.0, 0.01, 5.3, 5.0),
            (5.31, 1.0, 0.01, 5.3, 5.3),
            (9.0, 1.0, 0.01, 5.3, 5.3),
            // Back down: the nearest whole step below, then the next.
            (5.29, 1.0, 0.01, 5.3, 5.0),
            (4.4, 1.0, 0.01, 5.3, 4.0),
            // Fine and coarse steps.
            (5.27, 0.1, 0.01, 5.33, 5.3),
            (5.34, 0.1, 0.01, 5.33, 5.33),
            (13.0, 5.0, 0.01, 12.5, 12.5),
            (11.0, 5.0, 0.01, 12.5, 10.0),
            // The least holds the same way.
            (-3.0, 1.0, 0.01, 5.3, 0.01),
            (0.3, 1.0, 0.01, 5.3, 0.01),
            // No limit, no step.
            (7.4, 1.0, 0.01, f64::INFINITY, 7.0),
            (7.4, 0.0, 0.01, 5.3, 5.3),
        ];
        for (raw, step, least, most, want) in cases {
            let got = nearest(raw, step, least, most);
            assert!(
                (got - want).abs() < 1e-9,
                "{raw} in steps of {step} within {least}..={most}: {got}, not {want}"
            );
        }
    }
}
