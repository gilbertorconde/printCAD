//! Framing the view: fit to the scene, the selection or an edit session,
//! fly to a standard view, look square at a face. Every fit frames what
//! the view draws, so a hidden body is left out.

use core_document::Document;
use glam::Vec3;

use crate::PrintCadApp;
use crate::log_panel as app_log;
use crate::orientation_cube::CameraSnapView;

/// The box `(min, max)` of every body the view draws, in world space:
/// hidden bodies are left out. `None` when nothing is drawn.
pub(crate) fn document_imported_aabb(document: &Document) -> Option<(Vec3, Vec3)> {
    drawn_body_bounds(document)
        .map(|(lo, hi)| (Vec3::from_array(lo), Vec3::from_array(hi)))
        .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
}

/// The bounds of each body the view draws.
pub(crate) fn drawn_body_bounds(
    document: &Document,
) -> impl Iterator<Item = ([f32; 3], [f32; 3])> + '_ {
    document
        .imported_geometries()
        .filter(|(body, _)| document.imported_body_effective_visible(**body))
        .filter_map(|(_, geometry)| geometry.bounds())
}

/// The sphere a fit frames for a box: its middle, and half its diagonal
/// (at least 1 mm).
pub(crate) fn aabb_fit_center_radius(aabb_min: Vec3, aabb_max: Vec3) -> (Vec3, f32) {
    let center = (aabb_min + aabb_max) * 0.5;
    let extents = aabb_max - aabb_min;
    let radius = extents.length() * 0.5;
    (center, radius.max(1.0))
}

impl PrintCadApp {
    /// Turn the view square to a face of a body, looking at it from
    /// outside, centred on it, keeping the view's up as near as it goes.
    pub(crate) fn look_at_face(&mut self, body: core_document::BodyId, face: u32) {
        let Some(frame) = self
            .session
            .document
            .imported_geometry(body)
            .and_then(|g| crate::app::input::face_frame(&g.mesh, face))
        else {
            return;
        };
        let (centre, normal) = frame;
        let (forward, up) = self.session.camera.view_basis();
        // Looking straight along the view's up, the view's forward is up.
        let up = if up.dot(normal).abs() > 0.99 {
            forward
        } else {
            up
        };
        self.session.camera.orient_to_plane(
            centre,
            normal,
            up,
            Some(centre),
            &self.user_settings.camera,
        );
    }

    /// Frame the selected body (else the active one); the whole scene when
    /// neither has geometry.
    pub(crate) fn fit_view_to_selection(&mut self) {
        if self.fit_edit_session() {
            return;
        }
        let body = self
            .session
            .selected_body
            .map(core_document::BodyId)
            .or(self.session.active_body_id);
        let aabb = body.and_then(|body| {
            let geometry = self.session.document.imported_geometry(body)?;
            geometry.bounds()
        });
        let Some((mn, mx)) = aabb else {
            self.fit_view_to_scene();
            return;
        };
        let (mn, mx) = (Vec3::from_array(mn), Vec3::from_array(mx));
        let (center, radius) = aabb_fit_center_radius(mn, mx);
        self.session
            .camera
            .reset_to_fit(center, radius, None, &self.user_settings.camera);
    }

    /// Frame the camera around the imported geometry (or the default box).
    pub(crate) fn fit_view_to_scene(&mut self) {
        app_log::info("Fit View requested");
        if self.fit_edit_session() {
            return;
        }
        self.frame_scene();
    }

    /// While an edit session holds the view square to its plane, frame
    /// what it edits without turning the view: the edited feature's
    /// geometry, else the bodies, else 50 mm about where the view is.
    /// False when no such session is open.
    fn fit_edit_session(&mut self) -> bool {
        if !self.sketch_editing_active() {
            return false;
        }
        let edited = self
            .registry
            .workbench(&self.session.active_workbench.0)
            .ok()
            .and_then(|wb| wb.editing_feature())
            .and_then(|id| self.registry.feature_geometry(&self.session.document, id))
            .and_then(|geometry| geometry.mesh.bounds())
            .map(|(mn, mx)| (Vec3::from_array(mn), Vec3::from_array(mx)));
        let (center, radius) =
            match edited.or_else(|| document_imported_aabb(&self.session.document)) {
                Some((mn, mx)) => aabb_fit_center_radius(mn, mx),
                None => (self.session.camera.focal_point_world(), 50.0),
            };
        self.session
            .camera
            .fit_keeping_orientation(center, radius, &self.user_settings.camera);
        true
    }

    /// [`Self::fit_view_to_scene`] without the log line.
    pub(crate) fn frame_scene(&mut self) {
        let settings = &self.user_settings.camera;
        let aabb = self.drawn_bodies_aabb();
        // The split view's side pane, with a camera of its own, is framed
        // alike.
        let session = &mut self.session;
        for camera in std::iter::once(&mut session.camera).chain(session.side_camera.as_mut()) {
            if let Some(aabb) = aabb {
                let (center, radius) = aabb_fit_center_radius(aabb.0, aabb.1);
                camera.reset_to_fit(center, radius, Some(aabb), settings);
            } else {
                camera.reset_to_fit(Vec3::ZERO, 50.0, None, settings);
            }
        }
    }

    /// The box around the bodies as the view draws them: the document's
    /// geometry and, while a feature is edited, its preview's tool, which
    /// stands in for a body that has no built shape yet.
    fn drawn_bodies_aabb(&self) -> Option<(Vec3, Vec3)> {
        let previews = self
            .session
            .previews
            .iter()
            .filter(|(body, _)| {
                self.session
                    .document
                    .imported_body_effective_visible(**body)
            })
            .filter_map(|(_, preview)| preview.tool.bounds())
            .map(|(lo, hi)| (Vec3::from_array(lo), Vec3::from_array(hi)));
        document_imported_aabb(&self.session.document)
            .into_iter()
            .chain(previews)
            .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
    }

    /// Turn to `view` (the default corner view when `None`) framing what
    /// [`Self::frame_scene`] frames, as one transition.
    pub(crate) fn fly_to_scene(&mut self, view: Option<CameraSnapView>) {
        let (center, radius) = self
            .drawn_bodies_aabb()
            .map_or((Vec3::ZERO, 50.0), |(mn, mx)| {
                aabb_fit_center_radius(mn, mx)
            });
        self.session
            .camera
            .fly_to_fit(view, center, radius, &self.user_settings.camera);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fit frames what is drawn: a hidden body's box is left out.
    #[test]
    fn the_drawn_bounds_leave_hidden_bodies_out() {
        let mut document = Document::new("Fit");
        let cube = |at: f32| crate::test_support::block([1.0; 3], [at; 3]);
        let near = document.create_body(Some("Near".into()));
        let far = document.create_body(Some("Far".into()));
        document.set_imported_geometry(near, crate::test_support::geometry(cube(0.0)));
        document.set_imported_geometry(far, crate::test_support::geometry(cube(100.0)));
        assert_eq!(
            document_imported_aabb(&document),
            Some((Vec3::ZERO, Vec3::splat(101.0)))
        );
        document.set_body_visible(far, false);
        assert_eq!(
            document_imported_aabb(&document),
            Some((Vec3::ZERO, Vec3::ONE))
        );
        document.set_body_visible(near, false);
        assert_eq!(document_imported_aabb(&document), None);
    }
}
