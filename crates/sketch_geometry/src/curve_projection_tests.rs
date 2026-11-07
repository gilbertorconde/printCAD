use crate::curves::{closest_on_ellipse_arc, closest_on_hyperbola_arc, closest_on_parabola_arc};
use crate::types::{Angle, HyperbolaBranch, HyperbolaParam, ParabolaParam};
use crate::world::{WorldLength, WorldPoint};

#[test]
fn projecting_a_point_on_a_conic_is_a_fixed_point_between_samples() {
    let center = WorldPoint::new(20., 30.);
    for reverse in [false, true] {
        let ends = if reverse { [1., -1.] } else { [-1., 1.] };
        for t in [-0.73_f64, 0.37, 0.83] {
            let ellipse = WorldPoint::new(20. + 5. * t.cos(), 30. + 4. * t.sin());
            let projected = closest_on_ellipse_arc(
                ellipse,
                center,
                WorldPoint::new(23., 30.),
                WorldLength::new(4.),
                Angle(ends[0]),
                Angle(ends[1]),
                reverse,
            );
            assert!(
                projected.distance(ellipse).0 < 1e-8,
                "ellipse {t}: {projected:?}"
            );
            for branch in [HyperbolaBranch::Positive, HyperbolaBranch::Negative] {
                let hyperbola =
                    WorldPoint::new(20. + branch.sign() * 3. * t.cosh(), 30. + 4. * t.sinh());
                let projected = closest_on_hyperbola_arc(
                    hyperbola,
                    center,
                    WorldPoint::new(25., 30.),
                    WorldLength::new(4.),
                    HyperbolaParam(ends[0]),
                    HyperbolaParam(ends[1]),
                    branch,
                );
                assert!(
                    projected.distance(hyperbola).0 < 1e-8,
                    "hyperbola {t}: {projected:?}"
                );
            }
            let parabola = WorldPoint::new(20. + t * t / 8., 30. + t);
            let projected = closest_on_parabola_arc(
                parabola,
                center,
                WorldPoint::new(22., 30.),
                ParabolaParam(ends[0]),
                ParabolaParam(ends[1]),
            );
            assert!(
                projected.distance(parabola).0 < 1e-8,
                "parabola {t}: {projected:?}"
            );
        }
    }
}
