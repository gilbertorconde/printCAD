//! The clipping plane's side in the app: what the clip pill's commands
//! make of it, the picks in the view that aim it (a flat face, a datum
//! plane, three points), the formula its offset follows, and the handle
//! standing on it, an arrow along its normal that drags the offset.

use core_document::{ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette};
use glam::{Vec2, Vec3};

use super::gesture::HandleTarget;
use super::scalar_arrow::{ARROW_PX, Held, ScalarArrow, ScalarDrag};
use crate::PrintCadApp;
use crate::app::measure::{DatumPick, MeasurePick};
use crate::camera::CameraController;
use crate::camera::section::{SectionAim, SectionPick, SectionPlane, SectionToggle};

/// The steps a dragged offset goes in, mm.
const STEP_MM: f32 = 0.1;

/// The handle as it stands in the view: an arrow from the plane's centre
/// along the normal into the side it keeps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Handle {
    pub base: Vec2,
    pub tip: Vec2,
    /// Screen pixels the plane moves for each millimetre of offset.
    pub pixels_per_mm: Vec2,
    pub offset: f32,
    /// The scene's span along the normal: the offsets that cut something,
    /// which a drag stops at the ends of.
    pub range: (f32, f32),
}

impl Handle {
    /// The drag a press at `press` starts.
    pub fn held(&self, press: Vec2, formula: Option<String>) -> Held<HandleTarget> {
        Held {
            of: HandleTarget::Section(formula),
            drag: ScalarDrag {
                press,
                pixels_per_unit: self.pixels_per_mm,
                value: self.offset,
                step: STEP_MM,
                stops: self.range,
            },
        }
    }
}

/// Where `plane`'s handle stands, `None` when its centre is out of view or
/// it faces the viewer square on.
pub(crate) fn handle(camera: &CameraController, plane: &SectionPlane) -> Option<Handle> {
    let centre = plane.centre(camera.scene_bounds());
    let to_screen = |p: Vec3| camera.world_to_viewport(p).map(|(x, y)| Vec2::new(x, y));
    let base = to_screen(centre)?;
    let per_mm = to_screen(centre + plane.normal)? - base;
    // Seen end on, the arrow would stand on its foot: it points up the
    // screen instead and drags by the pointer's height, a millimetre for
    // what a millimetre across the plane spans there.
    let (way, pixels_per_mm) = if per_mm.length() < 0.05 {
        let across = plane.normal.any_orthonormal_vector();
        let span = (to_screen(centre + across)? - base).length();
        (Vec2::NEG_Y, Vec2::new(0.0, -span))
    } else {
        (per_mm.normalize(), per_mm)
    };
    let sign = if plane.flipped { -1.0 } else { 1.0 };
    Some(Handle {
        base,
        tip: base + way * sign * ARROW_PX,
        pixels_per_mm,
        offset: plane.offset,
        range: plane.range(camera.scene_bounds()),
    })
}

impl Handle {
    /// Whether the pointer at `at` takes hold of it: near its tip, or on
    /// the arrow's shaft away from its foot.
    pub fn is_under(&self, at: Vec2) -> bool {
        self.arrow().is_under(at)
    }

    /// What draws it: the arrow, a dot at its foot and the offset beside
    /// its tip, brighter while held or under the pointer.
    pub fn drawn(
        &self,
        hot: bool,
        unit: core_document::Unit,
    ) -> (
        Vec<ScreenSpaceOverlay>,
        Vec<ScreenSpaceMark>,
        ScreenSpaceLabel,
    ) {
        let palette = SketchPalette::default();
        let color = if hot {
            palette.preselect
        } else {
            palette.preview
        };
        self.arrow()
            .drawn(core_document::format_length_mm(self.offset, unit, 2), color)
    }

    fn arrow(&self) -> ScalarArrow {
        ScalarArrow {
            base: self.base,
            tip: self.tip,
        }
    }
}

impl PrintCadApp {
    fn eye_and_forward(&self) -> (Vec3, Vec3) {
        let camera = &self.session.camera;
        let eye = Vec3::from_array(camera.position());
        (
            eye,
            (Vec3::from_array(camera.target()) - eye).normalize_or_zero(),
        )
    }

    /// What the clip pill or the toggle asked for.
    pub(crate) fn set_section(&mut self, toggle: Option<SectionToggle>) {
        let bounds = self.session.camera.scene_bounds();
        let (eye, forward) = self.eye_and_forward();
        self.session.section_pick = None;
        if self
            .session
            .gestures
            .handle_held()
            .is_some_and(|held| matches!(held, HandleTarget::Section(_)))
        {
            self.session.gestures.held = None;
        }
        let Some(toggle) = toggle else {
            self.session.section = None;
            return;
        };
        let plane = match toggle {
            SectionToggle::On => SectionPlane::facing(forward, bounds),
            SectionToggle::Set(plane) => {
                self.session.section = Some(plane);
                return;
            }
            SectionToggle::Wedge(way) => {
                if let Some(plane) = self.session.section.as_mut() {
                    plane.wedge = way.map(|way| plane.wedge_at(way, bounds, Some(eye)));
                }
                return;
            }
            SectionToggle::View => match SectionPlane::square_to_view(forward, bounds) {
                Some(plane) => plane,
                None => return,
            },
            SectionToggle::Pick(pick) => {
                if pick == SectionPick::Plane
                    && let Some(plane) = self.selected_section_plane(eye)
                {
                    plane
                } else {
                    self.session.section_pick = Some(pick);
                    if self.session.section.is_none() {
                        self.session.section = Some(SectionPlane::facing(forward, bounds));
                    }
                    return;
                }
            }
        };
        self.aim_section(plane);
    }

    /// `plane` as the section; aimed afresh, it leaves whole what the one
    /// before it did. A plane the pill sets keeps its own list.
    fn aim_section(&mut self, plane: SectionPlane) {
        let uncut = self
            .session
            .section
            .as_ref()
            .map(|old| old.uncut.clone())
            .unwrap_or_default();
        self.session.section = Some(SectionPlane { uncut, ..plane });
    }

    /// What the scene is cut with, as the renderer has it.
    pub(crate) fn scene_clip(&self) -> Option<crate::camera::section::Clip> {
        let frame = &self.frame_submission;
        frame.clip_plane.map(|plane| crate::camera::section::Clip {
            plane,
            wedge: frame.clip_wedge,
        })
    }

    /// The clipping equation `body` is drawn and picked with: the scene's,
    /// or none when the toolbar's section leaves it whole.
    pub(crate) fn clip_of(
        &self,
        body: core_document::BodyId,
    ) -> Option<crate::camera::section::Clip> {
        crate::camera::section::clip_for(self.scene_clip(), self.session.section.as_ref(), body)
    }

    /// The plane of a flat face or a datum plane the selection holds.
    fn selected_section_plane(&self, eye: Vec3) -> Option<SectionPlane> {
        if let Some((_, face)) = &self.session.last_face_hit
            && let Some(kernel_api::FaceSurface::Plane { origin, normal }) = face.surface
        {
            return SectionPlane::through(
                SectionAim::Face,
                Vec3::from_array(origin),
                Vec3::from_array(normal),
                eye,
            );
        }
        let Some(crate::ui::TreeItemId::Feature(id)) = self.session.tree_selection else {
            return None;
        };
        let (origin, normal) = self.datum_plane(id)?;
        SectionPlane::through(SectionAim::Datum, origin, normal, eye)
    }

    /// The datum plane `id`, placed in the world.
    fn datum_plane(&self, id: core_document::FeatureId) -> Option<(Vec3, Vec3)> {
        let placed = self.session.document.placed_datum(id)?;
        match DatumPick::of(&placed.feature, placed.placement, Vec3::ZERO, Vec3::Z).0 {
            DatumPick::Plane { origin, normal } => {
                Some((Vec3::from_array(origin), Vec3::from_array(normal)))
            }
            _ => None,
        }
    }

    /// A click while the clipping plane waits for a pick: a flat face or
    /// a datum plane aims it there, a point joins the three it waits for.
    /// Whether it took the click.
    pub(crate) fn section_pick_click(&mut self) -> bool {
        let Some(pick) = self.session.section_pick.clone() else {
            return false;
        };
        let (eye, _) = self.eye_and_forward();
        let datum = self.datum_under_cursor();
        let edge = self.session.hovered_edge.map(|edge| MeasurePick::Edge {
            point: edge.point,
            direction: edge.direction,
            length: edge.length_mm,
            circle: edge.circle,
            body: None,
        });
        let mesh = self.session.hovered_body.and_then(|body| {
            self.session
                .document
                .imported_geometry(core_document::BodyId(body))
        });
        let surface = || {
            crate::app::measure::pick_at(
                None,
                mesh.map(|geometry| &*geometry.mesh),
                self.session.hovered_world_pos,
            )
        };
        match pick {
            SectionPick::Plane => {
                let plane = match (datum, surface()) {
                    (
                        Some(MeasurePick::Datum {
                            datum: DatumPick::Plane { origin, normal },
                            ..
                        }),
                        _,
                    ) => SectionPlane::through(
                        SectionAim::Datum,
                        Vec3::from_array(origin),
                        Vec3::from_array(normal),
                        eye,
                    ),
                    (
                        _,
                        Some(MeasurePick::Face {
                            surface: Some(kernel_api::FaceSurface::Plane { origin, normal }),
                            ..
                        }),
                    ) => SectionPlane::through(
                        SectionAim::Face,
                        Vec3::from_array(origin),
                        Vec3::from_array(normal),
                        eye,
                    ),
                    _ => {
                        crate::app_log::info(
                            "The clipping plane takes a flat face or a datum plane",
                        );
                        return true;
                    }
                };
                if let Some(plane) = plane {
                    self.aim_section(plane);
                    self.session.section_pick = None;
                }
            }
            SectionPick::Points(mut points) => {
                let Some(point) = datum
                    .or(edge)
                    .or_else(surface)
                    .map(|pick| Vec3::from_array(pick.point()))
                else {
                    return true;
                };
                points.push(point);
                if let [a, b, c] = points[..] {
                    match SectionPlane::through_points([a, b, c], eye) {
                        Some(plane) => {
                            self.aim_section(plane);
                            self.session.section_pick = None;
                        }
                        None => {
                            crate::app_log::info(
                                "Those three points stand in a line: pick the third again",
                            );
                            points.pop();
                            self.session.section_pick = Some(SectionPick::Points(points));
                        }
                    }
                } else {
                    self.session.section_pick = Some(SectionPick::Points(points));
                }
            }
        }
        true
    }

    /// The offset as its formula reads it, so the plane follows the
    /// variables it names; a formula that fails leaves the offset be.
    pub(crate) fn follow_section_formula(&mut self) {
        let Some(plane) = self.session.section.as_mut() else {
            return;
        };
        let Some(formula) = &plane.formula else {
            return;
        };
        if let Ok(q) = self
            .session
            .document
            .evaluate_formula(formula, Some(core_document::expr::Dim::LENGTH))
        {
            let value = q.value as f32;
            if value != plane.offset {
                plane.offset = value;
                self.redraw_needed = true;
            }
        }
    }

    /// The clipping plane's handle, where it stands now; none while a pick
    /// aims the plane or a bench cuts the scene its own way.
    pub(crate) fn section_handle(&self) -> Option<Handle> {
        let plane = self.session.section.as_ref()?;
        if self.session.section_pick.is_some()
            || self.frame_submission.clip_plane != Some(plane.equation())
        {
            return None;
        }
        handle(&self.session.camera, plane)
    }

    pub(crate) fn section_handle_hot(&self, handle: &Handle) -> bool {
        matches!(
            self.session.gestures.handle_held(),
            Some(HandleTarget::Section(_))
        ) || self
            .input
            .cursor_in_viewport
            .is_some_and(|(x, y)| handle.is_under(Vec2::new(x, y)))
    }

    /// Set the offset a handle drag reached; a cancel puts back the
    /// formula the press found, any other value stands alone. The plane is
    /// the view's, so none of it is an undo step.
    pub(super) fn set_section_offset(
        &mut self,
        offset: f32,
        formula: Option<String>,
        cancel: bool,
    ) {
        if let Some(plane) = self.session.section.as_mut() {
            plane.offset = offset;
            plane.formula = if cancel { formula } else { None };
        }
    }
}

impl PrintCadApp {
    /// The `PRINTCAD_BENCH_SECTION` hook's plane (frame.rs names its form).
    pub(crate) fn bench_section(&mut self, spec: &str) {
        let (spec, whole) = match spec.split_once('~') {
            Some((spec, name)) => (spec, Some(name)),
            None => (spec, None),
        };
        let (spec, wedge) = match spec.split_once('^') {
            Some((spec, way)) => (spec, way.parse::<usize>().ok()),
            None => (spec, None),
        };
        let flip = spec.ends_with('!');
        let spec = spec.trim_end_matches('!');
        let (aim, offset) = match spec.split_once('@') {
            Some((aim, offset)) => (aim, offset.parse::<f32>().ok()),
            None => (spec, None),
        };
        let toggle = match aim {
            "view" => SectionToggle::View,
            saved if saved.starts_with("saved:") => {
                self.saved_section_action(crate::ui::SavedSectionAction::Show(
                    saved["saved:".len()..].to_string(),
                ));
                return;
            }
            "face" => SectionToggle::Pick(SectionPick::Plane),
            "x" | "y" | "z" => {
                let axis = match aim {
                    "x" => crate::camera::section::SectionAxis::X,
                    "y" => crate::camera::section::SectionAxis::Y,
                    _ => crate::camera::section::SectionAxis::Z,
                };
                let (_, forward) = self.eye_and_forward();
                let bounds = self.session.camera.scene_bounds();
                SectionToggle::Set(SectionPlane::facing(forward, bounds).on_axis(axis, bounds))
            }
            _ => {
                let points: Vec<Vec3> = aim
                    .strip_prefix("points:")
                    .unwrap_or_default()
                    .split(';')
                    .filter_map(|p| {
                        let v: Vec<f32> = p.split(',').filter_map(|c| c.parse().ok()).collect();
                        (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
                    })
                    .collect();
                let (eye, _) = self.eye_and_forward();
                match points[..] {
                    [a, b, c] => match SectionPlane::through_points([a, b, c], eye) {
                        Some(plane) => SectionToggle::Set(plane),
                        None => return,
                    },
                    _ => {
                        tracing::warn!(target: "printcad.frame", "bench section `{spec}` not understood");
                        return;
                    }
                }
            }
        };
        self.set_section(Some(toggle));
        if let Some(plane) = self.session.section.as_mut() {
            if let Some(offset) = offset {
                plane.offset = offset;
            }
            plane.flipped ^= flip;
            if let Some(name) = whole
                && let Some(body) = self
                    .session
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.name == name)
            {
                plane.uncut.push(body.id);
            }
        }
        if wedge.is_some() {
            self.set_section(Some(SectionToggle::Wedge(wedge)));
        }
        self.redraw_needed = true;
        tracing::info!(target: "printcad.frame", "bench section {:?}", self.session.section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_end_on_handle_points_up_and_flipping_keeps_its_drag_direction() {
        let camera = CameraController::new(&settings::CameraSettings::default(), (800, 600));
        let forward =
            (Vec3::from_array(camera.target()) - Vec3::from_array(camera.position())).normalize();
        let mut plane = SectionPlane::square_to_view(forward, camera.scene_bounds()).unwrap();
        let normal = handle(&camera, &plane).unwrap();
        assert!((normal.tip - normal.base - Vec2::NEG_Y * ARROW_PX).length() < 1e-3);
        assert!(normal.pixels_per_mm.y < 0.0);
        plane.flipped = true;
        let flipped = handle(&camera, &plane).unwrap();
        assert!((flipped.tip - flipped.base + Vec2::NEG_Y * ARROW_PX).length() < 1e-3);
        assert_eq!(normal.pixels_per_mm, flipped.pixels_per_mm);
    }
}
