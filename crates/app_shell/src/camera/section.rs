//! The view toolbar's clipping plane: a plane square to one document axis
//! that hides the part of the scene on one side of it, in every render pass
//! and in picking alike. It is view state, kept per tab beside the camera,
//! never part of the document.

use glam::Vec3;

/// The document axis a section plane stands square to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SectionAxis {
    X,
    Y,
    Z,
}

impl SectionAxis {
    pub const ALL: [SectionAxis; 3] = [SectionAxis::X, SectionAxis::Y, SectionAxis::Z];

    pub fn label(self) -> &'static str {
        match self {
            SectionAxis::X => "X",
            SectionAxis::Y => "Y",
            SectionAxis::Z => "Z",
        }
    }

    pub fn unit(self) -> Vec3 {
        match self {
            SectionAxis::X => Vec3::X,
            SectionAxis::Y => Vec3::Y,
            SectionAxis::Z => Vec3::Z,
        }
    }
}

/// What the clipping plane controls ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SectionToggle {
    /// Across the middle of the scene, facing the viewer.
    On,
    /// This plane exactly.
    Set(SectionPlane),
}

/// A plane at `offset` millimetres along `axis`. It keeps the side where
/// the axis coordinate is at least `offset`, or at most it when `flipped`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SectionPlane {
    pub axis: SectionAxis,
    pub offset: f32,
    pub flipped: bool,
}

impl SectionPlane {
    /// The plane across the scene's middle on the axis the view looks down
    /// most directly, keeping the far half, so the cut faces the viewer.
    pub fn facing(forward: Vec3, bounds: Option<(Vec3, Vec3)>) -> Self {
        let axis = SectionAxis::ALL
            .into_iter()
            .max_by(|a, b| {
                forward
                    .dot(a.unit())
                    .abs()
                    .total_cmp(&forward.dot(b.unit()).abs())
            })
            .unwrap_or(SectionAxis::Z);
        let mut plane = Self {
            axis,
            offset: 0.0,
            flipped: forward.dot(axis.unit()) < 0.0,
        };
        plane.offset = plane.middle(bounds);
        plane
    }

    /// The same plane on another axis, moved to the scene's middle there.
    pub fn on_axis(self, axis: SectionAxis, bounds: Option<(Vec3, Vec3)>) -> Self {
        let mut plane = Self { axis, ..self };
        plane.offset = plane.middle(bounds);
        plane
    }

    /// The span of the scene along the plane's axis, where the offset
    /// can go.
    pub fn range(&self, bounds: Option<(Vec3, Vec3)>) -> (f32, f32) {
        match bounds {
            Some((lo, hi)) => {
                let a = self.axis.unit();
                let (lo, hi) = (lo.dot(a), hi.dot(a));
                (lo.min(hi), lo.max(hi))
            }
            None => (-100.0, 100.0),
        }
    }

    fn middle(&self, bounds: Option<(Vec3, Vec3)>) -> f32 {
        let (lo, hi) = self.range(bounds);
        (lo + hi) * 0.5
    }

    /// The renderer's form, `[a, b, c, d]` keeping `a·x + b·y + c·z + d >= 0`.
    pub fn equation(&self) -> [f32; 4] {
        let sign = if self.flipped { -1.0 } else { 1.0 };
        let n = self.axis.unit() * sign;
        [n.x, n.y, n.z, -sign * self.offset]
    }

    /// Whether the point is on the side the plane keeps.
    pub fn keeps(&self, p: Vec3) -> bool {
        let [a, b, c, d] = self.equation();
        a * p.x + b * p.y + c * p.z + d >= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: Option<(Vec3, Vec3)> =
        Some((Vec3::new(0.0, -10.0, 2.0), Vec3::new(40.0, 10.0, 12.0)));

    #[test]
    fn looking_down_an_axis_cuts_across_it_and_keeps_the_far_half() {
        // Looking down -Z from above: the cut is on Z, and the lower half,
        // the far one, stays.
        let plane = SectionPlane::facing(Vec3::new(0.1, 0.2, -1.0), SCENE);
        assert_eq!(plane.axis, SectionAxis::Z);
        assert!((plane.offset - 7.0).abs() < 1e-5);
        assert!(plane.keeps(Vec3::new(0.0, 0.0, 3.0)));
        assert!(!plane.keeps(Vec3::new(0.0, 0.0, 11.0)));
        let plane = SectionPlane::facing(Vec3::new(1.0, 0.1, 0.0), SCENE);
        assert_eq!(plane.axis, SectionAxis::X);
        assert!(plane.keeps(Vec3::new(30.0, 0.0, 0.0)));
        assert!(!plane.keeps(Vec3::new(10.0, 0.0, 0.0)));
    }

    #[test]
    fn flipping_keeps_the_other_side() {
        let mut plane = SectionPlane::facing(Vec3::NEG_Z, SCENE);
        plane.flipped = !plane.flipped;
        assert!(plane.keeps(Vec3::new(0.0, 0.0, 11.0)));
        assert!(!plane.keeps(Vec3::new(0.0, 0.0, 3.0)));
    }

    #[test]
    fn another_axis_starts_across_the_middle_of_the_scene_there() {
        let plane = SectionPlane::facing(Vec3::NEG_Z, SCENE).on_axis(SectionAxis::Y, SCENE);
        assert_eq!(plane.range(SCENE), (-10.0, 10.0));
        assert!(plane.offset.abs() < 1e-5);
        let [a, b, c, d] = plane.equation();
        assert_eq!((a, c), (0.0, 0.0));
        assert!((b.abs() - 1.0).abs() < 1e-6);
        assert!(d.abs() < 1e-6);
    }
}
