//! The view toolbar's clipping plane: a plane that hides the part of the
//! scene on one side of it, in every render pass and in picking alike. It
//! stands square to a document axis, square to the view, on a flat face or
//! a datum plane, or through three points, moved along its normal by an
//! offset. It is view state, kept per tab beside the camera; the document
//! keeps named copies of it (`core_document::SavedSection`), which the
//! view switches to, but the plane on screen is never part of it. A bench may cut the scene at a plane of any direction
//! instead (`Workbench::clip_plane`); both reach the renderer as the same
//! equation, which [`keeps`] reads.

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

/// How a section plane was aimed, which the clip pill names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SectionAim {
    /// Square to a document axis, its offset the coordinate along it.
    Axis(SectionAxis),
    /// Square to the view as it was when aimed.
    View,
    /// On a flat face's plane.
    Face,
    /// On a datum plane.
    Datum,
    /// Through three picked points.
    Points,
}

/// What the clipping plane controls ask for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SectionToggle {
    /// Across the middle of the scene, facing the viewer.
    On,
    /// This plane exactly.
    Set(SectionPlane),
    /// Square to the view, through the scene's middle.
    View,
    /// Picks in the view aim it: a flat face or a datum plane (taken from
    /// the selection when it holds one), or three points.
    Pick(SectionPick),
}

/// A pick in the view the clipping plane waits for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SectionPick {
    /// A flat face or a datum plane.
    Plane,
    /// Three points; the ones picked so far.
    Points(Vec<Vec3>),
}

impl SectionPick {
    /// What the clip pill asks for while it waits.
    pub fn prompt(&self) -> String {
        match self {
            SectionPick::Plane => "Click a flat face or a datum plane".to_string(),
            SectionPick::Points(points) => format!("Click point {} of 3", points.len() + 1),
        }
    }
}

/// A plane square to unit `normal`, standing `base + offset` millimetres
/// along it from the document's origin. It keeps the side the normal
/// points to, or the other when `flipped`. `base` is where it was aimed
/// (zero square to an axis, so the offset is the coordinate there);
/// `offset` moves it from there, set by `formula` when one is given.
/// The bodies in `uncut` it leaves whole (a shaft through a housing).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SectionPlane {
    pub normal: Vec3,
    pub base: f32,
    pub offset: f32,
    pub flipped: bool,
    pub aim: SectionAim,
    pub formula: Option<String>,
    pub uncut: Vec<core_document::BodyId>,
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
        let mut plane = Self::on(SectionAim::Axis(axis), axis.unit(), 0.0);
        plane.flipped = forward.dot(axis.unit()) < 0.0;
        plane.offset = plane.middle(bounds);
        plane
    }

    /// Square to `forward` through the scene's middle, keeping the far half.
    pub fn square_to_view(forward: Vec3, bounds: Option<(Vec3, Vec3)>) -> Option<Self> {
        let normal = forward.try_normalize()?;
        let mut plane = Self::on(SectionAim::View, normal, 0.0);
        plane.base = plane.middle(bounds);
        Some(plane)
    }

    /// The plane through `origin` square to `normal`, keeping the side
    /// away from `eye`, so the cut faces the viewer.
    pub fn through(aim: SectionAim, origin: Vec3, normal: Vec3, eye: Vec3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        let mut plane = Self::on(aim, normal, origin.dot(normal));
        plane.flipped = keeps(plane.equation(), eye);
        Some(plane)
    }

    /// The plane through three points, `None` when they stand in a line.
    pub fn through_points(points: [Vec3; 3], eye: Vec3) -> Option<Self> {
        let [a, b, c] = points;
        let normal = (b - a).cross(c - a);
        let span = (b - a).length().max((c - a).length()).max(1e-6);
        if normal.length() < 1e-4 * span * span {
            return None;
        }
        Self::through(SectionAim::Points, a, normal, eye)
    }

    fn on(aim: SectionAim, normal: Vec3, base: f32) -> Self {
        Self {
            normal,
            base,
            offset: 0.0,
            flipped: false,
            aim,
            formula: None,
            uncut: Vec::new(),
        }
    }

    /// The same plane square to another axis, moved to the scene's middle
    /// there, keeping the side it kept and the bodies it leaves whole.
    pub fn on_axis(&self, axis: SectionAxis, bounds: Option<(Vec3, Vec3)>) -> Self {
        let mut plane = Self::on(SectionAim::Axis(axis), axis.unit(), 0.0);
        plane.flipped = self.flipped;
        plane.uncut = self.uncut.clone();
        plane.offset = plane.middle(bounds);
        plane
    }

    /// The span of the scene's box along the normal, measured from the
    /// base: where the offset cuts something.
    pub fn range(&self, bounds: Option<(Vec3, Vec3)>) -> (f32, f32) {
        match bounds {
            Some((lo, hi)) => {
                let along = (0..8).map(|corner| {
                    Vec3::new(
                        if corner & 1 == 0 { lo.x } else { hi.x },
                        if corner & 2 == 0 { lo.y } else { hi.y },
                        if corner & 4 == 0 { lo.z } else { hi.z },
                    )
                    .dot(self.normal)
                        - self.base
                });
                along.fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| {
                    (a.min(v), b.max(v))
                })
            }
            None => (-100.0, 100.0),
        }
    }

    fn middle(&self, bounds: Option<(Vec3, Vec3)>) -> f32 {
        let (lo, hi) = self.range(bounds);
        (lo + hi) * 0.5
    }

    /// The point of the plane nearest the scene's middle: where its handle
    /// stands.
    pub fn centre(&self, bounds: Option<(Vec3, Vec3)>) -> Vec3 {
        let middle = bounds.map_or(Vec3::ZERO, |(lo, hi)| (lo + hi) * 0.5);
        middle - self.normal * (middle.dot(self.normal) - self.base - self.offset)
    }

    /// The renderer's form, `[a, b, c, d]` keeping `a·x + b·y + c·z + d >= 0`.
    pub fn equation(&self) -> [f32; 4] {
        let sign = if self.flipped { -1.0 } else { 1.0 };
        let n = self.normal * sign;
        [n.x, n.y, n.z, -sign * (self.base + self.offset)]
    }
}

impl SectionPlane {
    /// The plane kept by `name` in the document.
    pub fn saved(&self, name: &str) -> core_document::SavedSection {
        use core_document::SectionAim as Saved;
        core_document::SavedSection {
            name: name.to_string(),
            normal: self.normal.to_array(),
            base: self.base,
            offset: self.offset,
            flipped: self.flipped,
            aim: match self.aim {
                SectionAim::Axis(SectionAxis::X) => Saved::X,
                SectionAim::Axis(SectionAxis::Y) => Saved::Y,
                SectionAim::Axis(SectionAxis::Z) => Saved::Z,
                SectionAim::View => Saved::View,
                SectionAim::Face => Saved::Face,
                SectionAim::Datum => Saved::Datum,
                SectionAim::Points => Saved::Points,
            },
            formula: self.formula.clone(),
            uncut: self.uncut.clone(),
        }
    }

    /// A saved plane as the view shows it; `None` for one with no direction.
    pub fn from_saved(saved: &core_document::SavedSection) -> Option<Self> {
        use core_document::SectionAim as Saved;
        // A unit normal is kept to the bit, so the plane saves back equal.
        let normal = Vec3::from_array(saved.normal);
        let normal = if (normal.length() - 1.0).abs() < 1e-5 {
            normal
        } else {
            normal.try_normalize()?
        };
        Some(Self {
            normal,
            base: saved.base,
            offset: saved.offset,
            flipped: saved.flipped,
            aim: match saved.aim {
                Saved::X => SectionAim::Axis(SectionAxis::X),
                Saved::Y => SectionAim::Axis(SectionAxis::Y),
                Saved::Z => SectionAim::Axis(SectionAxis::Z),
                Saved::View => SectionAim::View,
                Saved::Face => SectionAim::Face,
                Saved::Datum => SectionAim::Datum,
                Saved::Points => SectionAim::Points,
            },
            formula: saved.formula.clone(),
            uncut: saved.uncut.clone(),
        })
    }
}

/// The clipping equation for one body: none when the toolbar's section
/// `plane` cuts the scene at `clip` and leaves `body` whole.
pub(crate) fn clip_for(
    clip: Option<[f32; 4]>,
    plane: Option<&SectionPlane>,
    body: core_document::BodyId,
) -> Option<[f32; 4]> {
    let whole = plane.is_some_and(|p| clip == Some(p.equation()) && p.uncut.contains(&body));
    clip.filter(|_| !whole)
}

/// Whether the clipping equation `[a, b, c, d]` keeps the point: the
/// renderer draws what has `a·x + b·y + c·z + d >= 0`.
pub(crate) fn keeps([a, b, c, d]: [f32; 4], p: Vec3) -> bool {
    a * p.x + b * p.y + c * p.z + d >= 0.0
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
        assert_eq!(plane.aim, SectionAim::Axis(SectionAxis::Z));
        assert!((plane.offset - 7.0).abs() < 1e-5);
        assert!(keeps(plane.equation(), Vec3::new(0.0, 0.0, 3.0)));
        assert!(!keeps(plane.equation(), Vec3::new(0.0, 0.0, 11.0)));
        let plane = SectionPlane::facing(Vec3::new(1.0, 0.1, 0.0), SCENE);
        assert_eq!(plane.aim, SectionAim::Axis(SectionAxis::X));
        assert!(keeps(plane.equation(), Vec3::new(30.0, 0.0, 0.0)));
        assert!(!keeps(plane.equation(), Vec3::new(10.0, 0.0, 0.0)));
        // Flipped, it keeps the other side.
        let mut plane = plane;
        plane.flipped = !plane.flipped;
        assert!(keeps(plane.equation(), Vec3::new(10.0, 0.0, 0.0)));
        assert!(!keeps(plane.equation(), Vec3::new(30.0, 0.0, 0.0)));
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

    /// A plane aimed at a face or three points cuts through them, keeps
    /// the side away from the eye, and its offset moves it along its
    /// normal from there.
    #[test]
    fn an_aimed_plane_cuts_through_what_aimed_it_and_its_offset_moves_it_along() {
        let eye = Vec3::new(0.0, 0.0, 100.0);
        let slant = Vec3::new(1.0, 0.0, 1.0).normalize();
        let face =
            SectionPlane::through(SectionAim::Face, Vec3::new(20.0, 0.0, 7.0), slant, eye).unwrap();
        let on = Vec3::new(20.0, 5.0, 7.0) + Vec3::new(1.0, 0.0, -1.0) * 3.0;
        let [a, b, c, d] = face.equation();
        assert!((a * on.x + b * on.y + c * on.z + d).abs() < 1e-4);
        assert!(!keeps(face.equation(), eye), "the eye's side is cut away");
        assert!(keeps(face.equation(), Vec3::new(0.0, 0.0, -100.0)));
        let mut moved = face.clone();
        moved.offset = 2.0;
        let shifted = on + face.normal * 2.0;
        let [a, b, c, d] = moved.equation();
        assert!((a * shifted.x + b * shifted.y + c * shifted.z + d).abs() < 1e-4);
        let centre = moved.centre(SCENE);
        assert!((centre.dot(face.normal) - face.base - 2.0).abs() < 1e-4);

        let points = [
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(10.0, 0.0, 5.0),
            Vec3::new(0.0, 10.0, 5.0),
        ];
        let plane = SectionPlane::through_points(points, eye).unwrap();
        assert!((plane.normal.z.abs() - 1.0).abs() < 1e-6);
        assert!(keeps(plane.equation(), Vec3::new(3.0, 3.0, 4.0)));
        assert!(!keeps(plane.equation(), Vec3::new(3.0, 3.0, 6.0)));
        let in_line = [points[0], points[1], Vec3::new(20.0, 0.0, 5.0)];
        assert!(SectionPlane::through_points(in_line, eye).is_none());
    }

    /// A body the section leaves whole is drawn and picked with no clip,
    /// but only while the section is what cuts the scene: a bench's own
    /// plane cuts it as any other.
    #[test]
    fn a_body_left_whole_escapes_the_section_and_no_other_plane() {
        let shaft = core_document::BodyId::new();
        let housing = core_document::BodyId::new();
        let mut plane = SectionPlane::facing(Vec3::NEG_Z, SCENE);
        plane.uncut.push(shaft);
        let clip = Some(plane.equation());
        assert_eq!(clip_for(clip, Some(&plane), shaft), None);
        assert_eq!(clip_for(clip, Some(&plane), housing), clip);
        let bench = Some([0.0, 1.0, 0.0, 0.0]);
        assert_eq!(clip_for(bench, Some(&plane), shaft), bench);
        // Aimed at another axis, it still leaves the shaft whole.
        assert_eq!(plane.on_axis(SectionAxis::X, SCENE).uncut, [shaft]);
        // Kept in the document and read back, it is the same plane.
        let mut aimed = SectionPlane::square_to_view(Vec3::new(1.0, 2.0, -3.0), SCENE).unwrap();
        aimed.formula = Some("Plate.t / 2".into());
        aimed.uncut.push(shaft);
        assert_eq!(SectionPlane::from_saved(&aimed.saved("Mid")), Some(aimed));
    }
}
