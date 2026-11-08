//! The clipping plane's side in the app: what the clip pill's commands
//! make of it, the picks in the view that aim it (a flat face, a datum
//! plane, three points), the formula its offset follows, and the handle
//! standing on it, an arrow along its normal that drags the offset.

use glam::{Vec2, Vec3};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::{Key, NamedKey};

use core_document::{ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette};

use crate::PrintCadApp;
use crate::app::measure::{DatumPick, MeasurePick};
use crate::camera::CameraController;
use crate::camera::section::{SectionAim, SectionPick, SectionPlane, SectionToggle};

/// How long the arrow is on screen, physical pixels.
const ARROW_PX: f32 = 56.0;
/// How near the arrow a press takes hold of it, physical pixels.
const REACH_PX: f32 = 12.0;
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
}

/// A drag of the handle: where it was taken hold of and the offset then.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OffsetDrag {
    press: Vec2,
    pixels_per_mm: Vec2,
    offset: f32,
}

impl OffsetDrag {
    /// The offset with the pointer at `at`: the press's offset changed by
    /// how far the pointer has gone along the normal on screen, in steps.
    pub fn offset_at(&self, at: Vec2) -> f32 {
        let along = self.pixels_per_mm.length_squared();
        if along < 1e-12 {
            return self.offset;
        }
        let moved = (at - self.press).dot(self.pixels_per_mm) / along;
        ((self.offset + moved) / STEP_MM).round() * STEP_MM
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
    })
}

impl Handle {
    /// Whether the pointer at `at` takes hold of it: near its tip, or on
    /// the arrow's shaft away from its foot.
    pub fn is_under(&self, at: Vec2) -> bool {
        if at.distance(self.tip) <= REACH_PX {
            return true;
        }
        let shaft = self.tip - self.base;
        let t = ((at - self.base).dot(shaft) / shaft.length_squared()).clamp(0.3, 1.0);
        at.distance(self.base + shaft * t) <= REACH_PX * 0.5
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
        let way = (self.tip - self.base).normalize_or_zero();
        let side = Vec2::new(-way.y, way.x);
        let head = |s: f32| self.tip - way * 10.0 + side * s * 5.0;
        let line =
            |a: Vec2, b: Vec2| ScreenSpaceOverlay::new(a.to_array(), b.to_array(), color, 2.0);
        let lines = vec![
            line(self.base, self.tip),
            line(head(1.0), self.tip),
            line(head(-1.0), self.tip),
        ];
        let marks = vec![ScreenSpaceMark::dot(self.base.to_array(), 3.0, color)];
        let text = core_document::format_length_mm(self.offset, unit, 2);
        let at = self.tip + way * 14.0 + side * 10.0;
        let label = ScreenSpaceLabel::new(at.to_array(), text, color, 12.0).pill();
        (lines, marks, label)
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
        self.session.section_drag = None;
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
        let document = &self.session.document;
        let node = document.feature_tree().get_node(id)?;
        if node.workbench_id.as_str() != core_document::DATUM_KIND {
            return None;
        }
        let feature: core_document::DatumFeature =
            serde_json::from_value(document.feature_values(id)?.clone()).ok()?;
        let placement = node
            .body
            .map(|body| document.body_placement(body))
            .unwrap_or_default();
        match DatumPick::of(&feature, placement, Vec3::ZERO, Vec3::Z).0 {
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
        self.session.section_drag.is_some()
            || self
                .cursor_in_viewport
                .is_some_and(|(x, y)| handle.is_under(Vec2::new(x, y)))
    }

    /// A press on the handle takes hold of it, moves drag the offset, the
    /// release lets go and Escape puts it back; Escape also stops a pick
    /// that aims the plane. Whether it took the event.
    pub(crate) fn section_input(&mut self, event: &WindowEvent) -> bool {
        let cursor = self.cursor_in_viewport.map(|(x, y)| Vec2::new(x, y));
        match event {
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => {
                let Some(at) = cursor else {
                    return false;
                };
                let Some(handle) = self.section_handle().filter(|h| h.is_under(at)) else {
                    return false;
                };
                self.session.section_drag = Some(OffsetDrag {
                    press: at,
                    pixels_per_mm: handle.pixels_per_mm,
                    offset: handle.offset,
                });
                true
            }
            WindowEvent::CursorMoved { .. } => {
                let (Some(drag), Some(at)) = (self.session.section_drag, cursor) else {
                    return self.session.section_drag.is_some();
                };
                if let Some(plane) = self.session.section.as_mut() {
                    plane.offset = drag.offset_at(at);
                    plane.formula = None;
                }
                true
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Released,
                ..
            } => self.session.section_drag.take().is_some(),
            WindowEvent::KeyboardInput { event: key, .. }
                if key.state == ElementState::Pressed
                    && key.logical_key == Key::Named(NamedKey::Escape) =>
            {
                if let Some(drag) = self.session.section_drag.take() {
                    if let Some(plane) = self.session.section.as_mut() {
                        plane.offset = drag.offset;
                    }
                    return true;
                }
                self.session.section_pick.take().is_some()
            }
            _ => false,
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
    fn a_drag_along_the_normal_moves_the_offset_in_steps_and_across_it_does_not() {
        let drag = OffsetDrag {
            press: Vec2::new(100.0, 100.0),
            pixels_per_mm: Vec2::new(0.0, -4.0),
            offset: 5.0,
        };
        assert_eq!(drag.offset_at(Vec2::new(100.0, 60.0)), 15.0);
        assert_eq!(drag.offset_at(Vec2::new(180.0, 100.0)), 5.0);
        let fine = drag.offset_at(Vec2::new(100.0, 98.7));
        assert!((fine - 5.3).abs() < 1e-4, "{fine}");
    }
}
