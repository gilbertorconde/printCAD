//! Runtime context and hooks for workbenches.
//!
//! This module provides the runtime API that workbenches use to interact with
//! the application shell: logging, document access, camera/picking info, and
//! overlay drawing.

use crate::{Document, FeatureId};

/// Log levels for workbench messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// A pending log entry from a workbench.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
}

/// Runtime context passed to workbench hooks.
///
/// This is the primary interface workbenches use to interact with the host
/// application. It provides:
/// - Logging (routed to the in-app log panel)
/// - Read/write access to the active document
/// - Camera and viewport information (read-only)
/// - Picking/selection state
/// - Overlay drawing registration (for tool visualizations)
pub struct WorkbenchRuntimeContext<'a> {
    /// The active document (mutable access for edits).
    pub document: &'a mut Document,

    /// Pending log entries to be flushed by the host after the hook returns.
    pending_logs: Vec<LogEntry>,

    /// Current camera position in world space.
    pub camera_position: [f32; 3],

    /// Current camera target (orbit center) in world space.
    pub camera_target: [f32; 3],

    /// Viewport dimensions (x, y, width, height) in pixels.
    pub viewport: (u32, u32, u32, u32),

    /// View-projection matrix for transforming 3D world coordinates to clip space.
    /// Used for projecting 3D points to screen coordinates.
    pub view_proj: Option<[[f32; 4]; 4]>,

    /// World position under the cursor (if any geometry is hovered).
    pub hovered_world_pos: Option<[f32; 3]>,

    /// ID of the body currently under the cursor (if any).
    pub hovered_body_id: Option<uuid::Uuid>,

    /// ID of the currently selected body (if any).
    pub selected_body_id: Option<uuid::Uuid>,

    /// Active document object (selected feature in tree - separate from editing mode).
    pub active_document_object: Option<FeatureId>,

    /// Current cursor position in viewport-local coordinates (if inside viewport).
    pub cursor_viewport_pos: Option<(f32, f32)>,

    /// What the bench asked of the host during this hook, in the order it
    /// asked. The host takes them when the hook returns.
    requests: Vec<HostRequest>,
    /// What the user did through the UI in this hook, as the commands that
    /// do the same (see [`Self::record`]).
    recorded: Vec<crate::command::Recorded>,

    /// Host → workbench: a "start on this body" request another bench
    /// made ([`HostRequest::StartOn`]), carried by the host until the
    /// bench it was for TAKES it, typically to open its plane picker.
    pub attach_request: Option<SketchAttachRequest>,

    /// Host → workbench: whether Ctrl is held (multi-select modifier).
    pub ctrl_down: bool,

    /// Host → workbench: the face under the last body selection, when the
    /// GPU pick landed on solid geometry (surface point + outward normal in
    /// world space). Lets "New Sketch" attach to the clicked face.
    pub selected_face: Option<FaceRef>,

    /// Host → workbench: the colors sketch overlays draw in.
    pub sketch_palette: crate::palette::SketchPalette,

    /// Host → workbench: the edges picked in the viewport, each as a point
    /// on it and its direction there, in world space (millimetres). A
    /// fillet or chamfer takes them by the point.
    pub selected_edges: Vec<EdgeRef>,

    /// Host → workbench: the kernel's answers to geometry questions asked
    /// while a hook runs, such as an edge projected onto a sketch plane.
    /// `None` where no kernel is at hand (tests, a context built for a
    /// panel only).
    pub kernel: Option<&'static dyn kernel_api::KernelQueries>,
}

/// A picked edge on a solid body: a point on the edge, the edge's
/// direction there and its length, all in world space (millimetres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRef {
    pub point: [f32; 3],
    pub direction: [f32; 3],
    pub length_mm: f32,
    /// The body the edge belongs to.
    pub body: uuid::Uuid,
    /// The circle the edge runs round, when it is a circle or an arc of
    /// one: a hole's rim brings the hole's axis.
    pub circle: Option<EdgeCircle>,
}

/// A circle an edge lies on, in the same frame as the edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeCircle {
    pub center: [f32; 3],
    pub normal: [f32; 3],
    pub radius: f32,
}

impl EdgeCircle {
    /// The circle through every one of `points`, when they lie on one (to
    /// a thousandth of its radius): an edge's outline, drawn with its
    /// vertices on the curve. A straight edge has none.
    pub fn fit(points: &[[f32; 3]]) -> Option<EdgeCircle> {
        use glam::DVec3;
        let points: Vec<DVec3> = points
            .iter()
            .map(|p| DVec3::from_array(p.map(f64::from)))
            .collect();
        let a = *points.first()?;
        // Three points far apart: the first, the one farthest from it, and
        // the one farthest from the line through those two.
        let b = *points
            .iter()
            .max_by(|p, q| p.distance_squared(a).total_cmp(&q.distance_squared(a)))?;
        let chord = (b - a).normalize_or_zero();
        let off = |p: &DVec3| (*p - a).reject_from_normalized(chord).length_squared();
        let c = *points.iter().max_by(|p, q| off(p).total_cmp(&off(q)))?;
        let (ab, ac) = (b - a, c - a);
        let n = ab.cross(ac);
        if n.length_squared() <= (1e-6 * ab.length_squared()).powi(2) {
            return None;
        }
        let center = a
            + (ac.length_squared() * n.cross(ab) + ab.length_squared() * ac.cross(n))
                / (2.0 * n.length_squared());
        let normal = n.normalize();
        let radius = center.distance(a);
        let tolerance = 1e-3 * radius + 1e-5;
        points
            .iter()
            .all(|p| {
                (p.distance(center) - radius).abs() <= tolerance
                    && (*p - center).dot(normal).abs() <= tolerance
            })
            .then(|| EdgeCircle {
                center: center.as_vec3().to_array(),
                normal: normal.as_vec3().to_array(),
                radius: radius as f32,
            })
    }
}

/// A picked face on a solid body: a point on the surface and its outward
/// normal, both in world space (millimetres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceRef {
    pub point: [f32; 3],
    pub normal: [f32; 3],
    /// The face's exact surface, in world space, when the mesh records
    /// it: a picked bore brings its axis, a flat face its plane.
    pub surface: Option<kernel_api::FaceSurface>,
}

impl EdgeRef {
    /// The same edge seen from a frame `placement` moves points into.
    pub fn moved(&self, placement: &crate::BodyPlacement) -> Self {
        Self {
            point: placement.point(self.point),
            direction: placement.direction(self.direction),
            length_mm: self.length_mm,
            body: self.body,
            circle: self.circle.map(|c| EdgeCircle {
                center: placement.point(c.center),
                normal: placement.direction(c.normal),
                radius: c.radius,
            }),
        }
    }
}

impl FaceRef {
    /// The same face seen from a frame `placement` moves points into.
    pub fn moved(&self, placement: &crate::BodyPlacement) -> Self {
        Self {
            point: placement.point(self.point),
            normal: placement.direction(self.normal),
            surface: self
                .surface
                .map(|s| s.moved(|p| placement.point(p), |d| placement.direction(d))),
        }
    }
}

/// Request to create a sketch attached to a body, optionally referenced on
/// one of its faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchAttachRequest {
    pub body: uuid::Uuid,
    pub face: Option<FaceRef>,
}

/// Request to orient camera to a specific plane.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraOrientRequest {
    pub plane_origin: [f32; 3],
    pub plane_normal: [f32; 3],
    pub plane_up: [f32; 3],
}

/// Something a bench asks the host to do once the hook returns. The host
/// applies a hook's requests in this order, whatever order they were made
/// in: tool, selection and journal first, then a bench switch, then the
/// camera, then the end of an edit session. A lifecycle hook (activate,
/// deactivate) cannot switch benches: its switch and start requests are
/// dropped, since it is running inside a switch already.
#[derive(Debug, Clone, PartialEq)]
pub enum HostRequest {
    /// Make this tool the active one (a right click on empty space drops
    /// the sketcher back to Select).
    ActivateTool(String),
    /// Make `body` the tree selection and the active body.
    SelectBody(crate::BodyId),
    /// Label the undo entry this frame closes.
    JournalLabel(String),
    /// Switch to `workbench` and hand it `attach` on its next hook, as its
    /// `attach_request`.
    StartOn {
        workbench: crate::WorkbenchId,
        attach: SketchAttachRequest,
    },
    /// Switch the active workbench (Part Design's "Edit sketch" jumps to
    /// the sketcher, which picks the active object up as its session).
    SwitchWorkbench(crate::WorkbenchId),
    /// Look square onto this plane.
    OrientCamera(CameraOrientRequest),
    /// End the active bench's edit session; the host also returns to the
    /// bench the session was started from.
    FinishEditing,
    /// Ask where to save `contents` (a file the bench made: a parts list)
    /// and write it there. `name` is the suggested file name, `kind` the
    /// dialog's filter label and `extension` its extension.
    SaveFile {
        name: String,
        kind: String,
        extension: String,
        contents: Vec<u8>,
    },
    /// Ask where to save an animation and write it there: the scene drawn
    /// as the camera sees it, once per frame with the bodies at that
    /// frame's placements (bodies left out stay where they are),
    /// `frame_ms` apart.
    RecordAnimation {
        name: String,
        frames: Vec<Vec<(crate::BodyId, crate::BodyPlacement)>>,
        frame_ms: u32,
    },
}

impl HostRequest {
    /// The position in the host's application order.
    pub fn rank(&self) -> u8 {
        match self {
            HostRequest::ActivateTool(_) => 0,
            HostRequest::SelectBody(_) | HostRequest::JournalLabel(_) => 1,
            HostRequest::StartOn { .. } | HostRequest::SwitchWorkbench(_) => 2,
            HostRequest::OrientCamera(_) => 3,
            HostRequest::FinishEditing => 4,
            HostRequest::SaveFile { .. } | HostRequest::RecordAnimation { .. } => 5,
        }
    }
}

/// Everything a hook may have left for the host, taken from the context
/// before its borrow ends.
#[derive(Debug, Clone, PartialEq)]
pub struct HookOutcome {
    /// The active document object as the hook left it.
    pub active_document_object: Option<FeatureId>,
    /// The attach inbox as the hook left it: `None` once the bench it was
    /// for took it.
    pub attach_request: Option<SketchAttachRequest>,
    /// The hook's requests, in the host's application order.
    pub requests: Vec<HostRequest>,
    /// What the hook recorded, in order.
    pub recorded: Vec<crate::command::Recorded>,
}

impl HookOutcome {
    pub fn take(ctx: &mut WorkbenchRuntimeContext<'_>) -> Self {
        let mut requests = ctx.take_requests();
        requests.sort_by_key(HostRequest::rank);
        Self {
            active_document_object: ctx.active_document_object,
            attach_request: ctx.attach_request,
            requests,
            recorded: std::mem::take(&mut ctx.recorded),
        }
    }
}

impl<'a> WorkbenchRuntimeContext<'a> {
    /// Create a new runtime context.
    pub fn new(
        document: &'a mut Document,
        camera_position: [f32; 3],
        camera_target: [f32; 3],
        viewport: (u32, u32, u32, u32),
    ) -> Self {
        Self {
            document,
            pending_logs: Vec::new(),
            camera_position,
            camera_target,
            viewport,
            hovered_world_pos: None,
            hovered_body_id: None,
            selected_body_id: None,
            cursor_viewport_pos: None,
            active_document_object: None,
            view_proj: None,
            requests: Vec::new(),
            recorded: Vec::new(),
            attach_request: None,
            selected_face: None,
            selected_edges: Vec::new(),
            kernel: None,
            ctrl_down: false,
            sketch_palette: crate::palette::SketchPalette::default(),
        }
    }

    /// The picked face in `body`'s own frame, where its features keep their
    /// references.
    pub fn selected_face_in(&self, body: crate::BodyId) -> Option<FaceRef> {
        let into_body = self.document.body_placement(body).inverse();
        self.selected_face.map(|face| face.moved(&into_body))
    }

    /// The picked edges in `body`'s own frame.
    pub fn selected_edges_in(&self, body: crate::BodyId) -> Vec<EdgeRef> {
        let into_body = self.document.body_placement(body).inverse();
        self.selected_edges
            .iter()
            .map(|edge| edge.moved(&into_body))
            .collect()
    }

    /// Log an info message to the application log panel.
    pub fn log_info(&mut self, message: impl Into<String>) {
        self.pending_logs.push(LogEntry {
            level: LogLevel::Info,
            message: message.into(),
        });
    }

    /// Log a warning message to the application log panel.
    pub fn log_warn(&mut self, message: impl Into<String>) {
        self.pending_logs.push(LogEntry {
            level: LogLevel::Warn,
            message: message.into(),
        });
    }

    /// Log an error message to the application log panel.
    pub fn log_error(&mut self, message: impl Into<String>) {
        self.pending_logs.push(LogEntry {
            level: LogLevel::Error,
            message: message.into(),
        });
    }

    /// Drain pending log entries (called by host after hook returns).
    pub fn drain_logs(&mut self) -> Vec<LogEntry> {
        std::mem::take(&mut self.pending_logs)
    }

    /// Ask the host for something once this hook returns.
    pub fn request(&mut self, request: HostRequest) {
        self.requests.push(request);
    }

    /// Say that the user just did, through the UI, what command `id` with
    /// `args` does, and that it answered `result`. A bench calls this where
    /// a click, a key or a panel ends in the same code the command runs, so
    /// a recording of the session replays it as a script. Commands run by a
    /// script never record.
    pub fn record(
        &mut self,
        id: impl Into<String>,
        args: crate::CommandArgs,
        result: serde_json::Value,
    ) {
        self.recorded.push(crate::command::Recorded {
            id: id.into(),
            args,
            result,
        });
    }

    /// The requests made so far, in the order they were made (the host
    /// takes them through [`HookOutcome::take`]).
    pub fn take_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Convert a world position to viewport-local pixel coordinates (the
    /// same space as [`Self::cursor_viewport_pos`], i.e. without the
    /// viewport's screen offset).
    ///
    /// NDC is Y-down: the host camera bakes the Vulkan Y flip into
    /// `view_proj`, so no flip is applied here (this mirrors the app shell's
    /// `CameraController::world_to_screen`).
    ///
    /// Returns `None` when no `view_proj` was provided, the viewport is
    /// degenerate, or the point is behind the camera.
    pub fn world_to_viewport(&self, world_pos: [f32; 3]) -> Option<(f32, f32)> {
        world_to_viewport(self.view_proj?, self.viewport, world_pos)
    }

    /// Convert viewport-local pixel coordinates to a world-space ray.
    /// Returns `(origin, direction)` with `direction` normalized, or `None`
    /// when no `view_proj` was provided or the unprojection is degenerate.
    ///
    /// Depth follows the Vulkan convention (near plane at NDC z = 0, far at
    /// z = 1); the ray origin sits on the near plane.
    pub fn viewport_to_ray(&self, viewport_pos: (f32, f32)) -> Option<([f32; 3], [f32; 3])> {
        viewport_to_ray(self.view_proj?, self.viewport, viewport_pos)
    }

    /// Convert viewport-local pixel coordinates to the intersection of the
    /// cursor ray with a plane. Returns `None` when the ray is (nearly)
    /// parallel to the plane or the intersection lies behind the near plane.
    pub fn viewport_to_plane(
        &self,
        viewport_pos: (f32, f32),
        plane_origin: [f32; 3],
        plane_normal: [f32; 3],
    ) -> Option<[f32; 3]> {
        viewport_to_plane(
            self.view_proj?,
            self.viewport,
            viewport_pos,
            plane_origin,
            plane_normal,
        )
    }
}

/// Free-function variants of the context's transform helpers, for hosts
/// that need the math outside a workbench hook (all take viewport-local
/// pixel coordinates; NDC is Y-down with the Vulkan flip baked into
/// `view_proj`).
pub fn world_to_viewport(
    view_proj: [[f32; 4]; 4],
    viewport: (u32, u32, u32, u32),
    world_pos: [f32; 3],
) -> Option<(f32, f32)> {
    let view_proj = glam::Mat4::from_cols_array_2d(&view_proj);
    let (_, _, w, h) = viewport;
    if w == 0 || h == 0 {
        return None;
    }
    let clip = view_proj * glam::Vec3::from_array(world_pos).extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some((
        (ndc.x + 1.0) * 0.5 * w as f32,
        (ndc.y + 1.0) * 0.5 * h as f32,
    ))
}

/// See [`WorkbenchRuntimeContext::viewport_to_ray`].
pub fn viewport_to_ray(
    view_proj: [[f32; 4]; 4],
    viewport: (u32, u32, u32, u32),
    viewport_pos: (f32, f32),
) -> Option<([f32; 3], [f32; 3])> {
    let view_proj = glam::Mat4::from_cols_array_2d(&view_proj);
    let (_, _, w, h) = viewport;
    if w == 0 || h == 0 {
        return None;
    }
    let ndc_x = (viewport_pos.0 / w as f32) * 2.0 - 1.0;
    let ndc_y = (viewport_pos.1 / h as f32) * 2.0 - 1.0;
    let inv = view_proj.inverse();
    let near_clip = inv * glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
    let far_clip = inv * glam::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
    if near_clip.w.abs() < 1e-12 || far_clip.w.abs() < 1e-12 {
        return None;
    }
    let near_world = near_clip.truncate() / near_clip.w;
    let far_world = far_clip.truncate() / far_clip.w;
    let dir = far_world - near_world;
    if dir.length_squared() < 1e-24 {
        return None;
    }
    Some((near_world.to_array(), dir.normalize().to_array()))
}

/// See [`WorkbenchRuntimeContext::viewport_to_plane`].
pub fn viewport_to_plane(
    view_proj: [[f32; 4]; 4],
    viewport: (u32, u32, u32, u32),
    viewport_pos: (f32, f32),
    plane_origin: [f32; 3],
    plane_normal: [f32; 3],
) -> Option<[f32; 3]> {
    let (origin, dir) = viewport_to_ray(view_proj, viewport, viewport_pos)?;
    let origin = glam::Vec3::from_array(origin);
    let dir = glam::Vec3::from_array(dir);
    let normal = glam::Vec3::from_array(plane_normal).normalize();
    let denom = dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (glam::Vec3::from_array(plane_origin) - origin).dot(normal) / denom;
    if t < 0.0 {
        return None;
    }
    Some((origin + dir * t).to_array())
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    use glam::{Mat4, Vec3, Vec4};

    #[test]
    fn a_circle_is_found_through_an_outline_and_a_line_has_none() {
        let points: Vec<[f32; 3]> = (0..24)
            .map(|i| {
                let t = i as f32 * std::f32::consts::TAU / 24.0;
                [5.0 + 3.0 * t.cos(), -2.0, 1.0 + 3.0 * t.sin()]
            })
            .collect();
        let c = EdgeCircle::fit(&points).expect("a circle");
        assert!((c.radius - 3.0).abs() < 1e-4);
        assert!(Vec3::from_array(c.center).distance(Vec3::new(5.0, -2.0, 1.0)) < 1e-4);
        assert!((c.normal[1].abs() - 1.0).abs() < 1e-5);
        // An arc of it too.
        assert!(EdgeCircle::fit(&points[..5]).is_some());
        let line: Vec<[f32; 3]> = (0..5).map(|i| [i as f32, 0.0, 0.0]).collect();
        assert!(EdgeCircle::fit(&line).is_none());
        let mut bent = points.clone();
        bent[7][1] += 0.5;
        assert!(EdgeCircle::fit(&bent).is_none(), "off the plane");
    }

    /// Build a Vulkan-convention view-projection like the app camera does:
    /// perspective (0..1 depth) with the Y flip baked in, looking at the
    /// origin from +Z.
    fn test_ctx_matrix() -> [[f32; 4]; 4] {
        let proj =
            glam::camera::rh::proj::directx::perspective(60f32.to_radians(), 4.0 / 3.0, 0.1, 100.0);
        let flip_y = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));
        let view =
            glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y);
        (flip_y * proj * view).to_cols_array_2d()
    }

    fn ctx_with_matrix(document: &mut Document) -> WorkbenchRuntimeContext<'_> {
        let mut ctx = WorkbenchRuntimeContext::new(
            document,
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 0.0],
            (0, 0, 800, 600),
        );
        ctx.view_proj = Some(test_ctx_matrix());
        ctx
    }

    #[test]
    fn center_of_viewport_projects_to_camera_target() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        let (x, y) = ctx.world_to_viewport([0.0, 0.0, 0.0]).unwrap();
        assert!((x - 400.0).abs() < 0.5, "x = {x}");
        assert!((y - 300.0).abs() < 0.5, "y = {y}");
    }

    #[test]
    fn behind_camera_returns_none() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        assert!(ctx.world_to_viewport([0.0, 0.0, 20.0]).is_none());
    }

    #[test]
    fn no_view_proj_returns_none() {
        let mut doc = Document::new("t");
        let ctx = WorkbenchRuntimeContext::new(
            &mut doc,
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 0.0],
            (0, 0, 800, 600),
        );
        assert!(ctx.world_to_viewport([0.0, 0.0, 0.0]).is_none());
        assert!(ctx.viewport_to_ray((400.0, 300.0)).is_none());
    }

    #[test]
    fn center_ray_passes_through_target() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        let (origin, dir) = ctx.viewport_to_ray((400.0, 300.0)).unwrap();
        let origin = Vec3::from_array(origin);
        let dir = Vec3::from_array(dir);
        // Ray from the camera through the viewport center should pass
        // (very close to) the world origin.
        let t = (Vec3::ZERO - origin).dot(dir);
        let closest = origin + dir * t;
        assert!(closest.length() < 1e-3, "closest = {closest}");
        // And it points away from the camera (-Z).
        assert!(dir.z < 0.0);
    }

    #[test]
    fn plane_hit_roundtrips_through_projection() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        // Pick an arbitrary viewport point, intersect the Z=0 plane, and
        // project the hit back: it must land on the original pixel.
        let px = (513.0, 222.0);
        let hit = ctx
            .viewport_to_plane(px, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0])
            .unwrap();
        assert!(hit[2].abs() < 1e-4);
        let (x, y) = ctx.world_to_viewport(hit).unwrap();
        assert!((x - px.0).abs() < 0.05, "x = {x}");
        assert!((y - px.1).abs() < 0.05, "y = {y}");
    }

    #[test]
    fn parallel_plane_returns_none() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        // The center ray travels along -Z; a plane containing that axis
        // (normal +Y at the ray height) is parallel to it.
        assert!(
            ctx.viewport_to_plane((400.0, 300.0), [0.0, 5.0, 0.0], [0.0, 1.0, 0.0])
                .is_none()
        );
    }

    #[test]
    fn y_axis_is_screen_down() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        // World +Y should appear ABOVE the center on screen, i.e. smaller
        // viewport y (Y-down pixels, flip baked into the matrix).
        let (_, y_up) = ctx.world_to_viewport([0.0, 2.0, 0.0]).unwrap();
        let (_, y_center) = ctx.world_to_viewport([0.0, 0.0, 0.0]).unwrap();
        assert!(y_up < y_center, "y_up = {y_up}, y_center = {y_center}");
    }

    #[test]
    fn used_vec4_sanity() {
        // Guard against accidental column/row-major confusion in
        // from_cols_array_2d usage: transform a known point both ways.
        let m = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let arr = m.to_cols_array_2d();
        let back = Mat4::from_cols_array_2d(&arr);
        assert_eq!(
            back * Vec4::new(0.0, 0.0, 0.0, 1.0),
            Vec4::new(1.0, 2.0, 3.0, 1.0)
        );
    }
}

/// Input event passed to workbench on_input hook.
#[derive(Debug, Clone)]
pub enum WorkbenchInputEvent {
    /// Mouse button pressed.
    MousePress {
        button: MouseButton,
        viewport_pos: (f32, f32),
    },
    /// Mouse button released.
    MouseRelease {
        button: MouseButton,
        viewport_pos: (f32, f32),
    },
    /// Mouse moved.
    MouseMove { viewport_pos: (f32, f32) },
    /// Key pressed.
    KeyPress { key: KeyCode },
    /// Key released.
    KeyRelease { key: KeyCode },
    /// A tool was activated from the toolbar, a menu or the palette. Action
    /// tools run on this, in the frame the click happened.
    ToolActivated,
    /// A keyboard action the workbench registered
    /// (`WorkbenchContext::register_action`) was triggered by its shortcut.
    Action { id: String },
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other(u16),
}

/// Simplified key code (extend as needed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Escape,
    Enter,
    Space,
    Delete,
    Backspace,
    Tab,
    // Letters
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    // Punctuation used by numeric entry
    Period,
    Comma,
    Minus,
    // Numbers
    Key0,
    Key1,
    Key2,
    Key3,
    Key4,
    Key5,
    Key6,
    Key7,
    Key8,
    Key9,
    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    // Navigation and editing
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Equals,
    Slash,
    // Modifiers (for reference; actual modifier state tracked separately)
    Shift,
    Control,
    Alt,
    // Other
    Unknown,
}

/// Result of a workbench input handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputResult {
    /// If true, the event was consumed and should not propagate further.
    pub consumed: bool,
    /// If true, the viewport should be redrawn.
    pub redraw: bool,
}

impl InputResult {
    pub fn consumed() -> Self {
        Self {
            consumed: true,
            redraw: true,
        }
    }

    pub fn ignored() -> Self {
        Self::default()
    }

    pub fn redraw_only() -> Self {
        Self {
            consumed: false,
            redraw: true,
        }
    }
}
