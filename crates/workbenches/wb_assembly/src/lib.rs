//! The Assembly workbench: bodies placed against each other with joints.
//!
//! A joint belongs to the body it moves and names the body it holds
//! against. Mate puts two flat faces together (with a gap, or facing the
//! same way); Align puts two round faces on one axis; a hinge, a slider, a
//! fixed joint, parallel, perpendicular, distance, angle and tangent joints
//! hold what their names say (`JointTool` lists them, what each takes and
//! how each starts). An axis comes from a round face or an edge. Joints are solved when
//! one is made or edited and when a body is moved, and the bodies' new
//! placements are ordinary edits, so a joint and the move it causes undo as
//! one step and reach every copy of the document the same way.

// Without the egui feature the crate is the headless subset its dependents'
// tests and the kernel bench use; the panel-only state and helpers go unused there.
#![cfg_attr(not(feature = "egui"), allow(dead_code, unused_imports))]

mod checks;
mod collide;
mod commands;
mod components;
mod coupling;
mod drag;
mod exploded;
mod frames;
mod group;
mod handles;
mod input;
mod interference;
mod joint;
mod mass;
mod menus;
mod motion;
#[cfg(feature = "egui")]
mod panel;
mod parameters;
mod parts;
mod picking;
mod registration;
mod replace;
mod session;
mod shapes;
mod solve;
mod states;
mod sweep_check;
mod tasks;
mod viewport_out;

use core_document::{
    BodyId, BodyPlacement, FeatureId, FeatureInfo, FeatureNode, HostRequest, InputResult,
    Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};

pub use coupling::{COUPLING_KIND, Coupling, Gearing};
pub use exploded::{EXPLODED_KIND, ExplodeStep, ExplodedView};
pub use frames::{sweep_frames, sweep_values};
pub use group::{GROUP_KIND, RigidGroup};
pub use interference::{Clash, Interference, interference};
pub use joint::{
    Anchor, Drive, JOINT_KIND, JointFeature, JointKind, JointTool, ORIGIN, Rigid, Takes, WORLD,
};
pub use mass::{BodyMass, MassReport};
pub use motion::{MOTION_KIND, MotionStudy, TimedDrive};
pub use parts::{Part, parts_csv, parts_list};
pub use solve::{
    HOLDS_MM, Joint, Motion, SolveError, counted_couplings, drag, draggable, freedom, joints,
    redundant, solve, unmet,
};
pub use states::{AssemblyState, STATE_KIND};
pub use sweep_check::MotionClash;

use frames::explode;

/// A joint being made: the kind, and the first face once picked.
#[derive(Debug, Clone)]
struct Picking {
    kind: JointTool,
    /// The first body, its anchor, the radius of a round face, and the
    /// face's name.
    first: Option<(BodyId, Anchor, Option<f32>, kernel_api::TopoName)>,
    /// The joint whose faces are picked again, when it is not a new one.
    repick: Option<FeatureId>,
    /// A width's second face on the moving body, then its first on the
    /// other, once picked.
    tab: Option<Anchor>,
    slot: Option<(BodyId, Anchor)>,
}

/// A motion study open: the study (once kept), its settings as edited,
/// the frames worked out, the frame shown, whether it plays, and where the
/// bodies sat, to go back to.
#[derive(Debug, Clone)]
pub(crate) struct Studying {
    pub(crate) study: Option<FeatureId>,
    pub(crate) draft: MotionStudy,
    pub(crate) frames: Option<Frames>,
    pub(crate) frame: usize,
    pub(crate) playing: bool,
    pub(crate) clock: f32,
    pub(crate) placements: Vec<(BodyId, BodyPlacement)>,
    /// Points followed through the frames: a body and a point of it in its
    /// own frame.
    pub(crate) traces: Vec<(BodyId, [f32; 3])>,
    /// A click on a body adds a point to follow.
    pub(crate) tracing: bool,
}

/// A motion's frames: each time and every body's placement then.
pub(crate) type Frames = Vec<(f32, Vec<(BodyId, BodyPlacement)>)>;

/// An exploded view's steps being shown or made: the view (once it has a
/// step), how far through them it stands, the bodies picked for the next
/// step and its shift, turn and spread, and whether it plays.
#[derive(Debug, Clone, Default)]
pub(crate) struct Stepping {
    view: Option<FeatureId>,
    at: f32,
    picked: Vec<BodyId>,
    shift: [f32; 3],
    /// The world axis the next step turns about, through the middle of
    /// its bodies: X, Y or Z.
    turn_axis: usize,
    /// How far it turns, degrees; none at 0.
    turn_degrees: f32,
    /// How far it pushes each body out from the middle of its bodies,
    /// mm; none at 0.
    radial: f32,
    playing: bool,
}

/// An axis copies are turned about: a point on it, its direction, and
/// the angle, degrees, they spread over.
type Around = ([f32; 3], [f32; 3], f32);

/// What the task panel holds open.
#[derive(Debug, Clone)]
enum Task {
    /// A joint's settings. `before` is its data when the task opened, or
    /// `None` for a joint the tool just made.
    Joint {
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: Vec<(BodyId, BodyPlacement)>,
    },
    /// A coupling's settings, as a joint's.
    Coupling {
        id: FeatureId,
        before: Option<serde_json::Value>,
        placements: Vec<(BodyId, BodyPlacement)>,
    },
    /// A body moved by hand.
    Move {
        body: BodyId,
        placements: Vec<(BodyId, BodyPlacement)>,
        /// Every coupling's whole turns when the task opened, put back on
        /// Cancel with the placements.
        turns: Vec<(FeatureId, i64)>,
    },
    /// Where bodies clash, as found at edit `seq` of the document; `None`
    /// while the check runs. `around` is the one body checked against the
    /// others, when the check is not of every pair; `clearance` the gap,
    /// mm, when it looks for pairs nearer than that rather than clashes.
    Interference {
        found: Option<Interference>,
        seq: u64,
        around: Option<BodyId>,
        clearance: Option<f32>,
    },
    /// The bodies spread apart to show how they go together; they go back
    /// to `placements` when it closes.
    Explode {
        placements: Vec<(BodyId, BodyPlacement)>,
        spread: f32,
        steps: Box<Stepping>,
    },
    /// Every part and how many of it.
    Parts,
    /// A motion over time being set up or played.
    Motion(Box<Studying>),
    /// Linked copies of `body` to insert: how many, and how far apart, or
    /// turned about an axis (a point on it, its direction, the angle they
    /// spread over).
    Copies {
        body: BodyId,
        count: u32,
        step: [f32; 3],
        around: Option<Around>,
        /// A mirror image instead: the plane, a point on it and its
        /// normal, in the world.
        mirror: Option<([f32; 3], [f32; 3])>,
    },
    /// A body to replace, and the one picked to take its place.
    Replace { old: BodyId, new: Option<BodyId> },
    /// Bodies picked for a rigid group, one click each (a second click
    /// takes one out); `editing` the group changed, `None` for a new one.
    Group {
        editing: Option<FeatureId>,
        members: Vec<BodyId>,
    },
    /// The assembly's mass and centre of mass at `density` g/cm³; `None`
    /// while it is measured.
    Mass {
        found: Option<MassReport>,
        density: f32,
    },
}

#[derive(Default)]
pub struct AssemblyWorkbench {
    /// What the bench holds for the document on screen, moved with its
    /// tab.
    tab: session::Tab,
    /// The parts' volumes for the parts list, measured once per geometry.
    #[cfg(feature = "egui")]
    volumes: parts::Volumes,
}

/// A body taken by the mouse: the point taken, in the body's own frame,
/// and the plane facing the view it is dragged across.
#[derive(Debug, Clone)]
struct Grab {
    body: BodyId,
    point: [f32; 3],
    plane: ([f32; 3], [f32; 3]),
    press: (f32, f32),
    dragging: bool,
    placements: Vec<(BodyId, BodyPlacement)>,
    /// Every coupling's whole turns when the drag began, put back with
    /// the placements.
    turns: Vec<(FeatureId, i64)>,
    /// Where the pull last went without a collision.
    reached: Option<[f32; 3]>,
    /// What bodies shared when the drag began.
    baseline: collide::Baseline,
    /// A body no joint places, moved straight across the view plane.
    free: bool,
    /// With Shift held on a body whose island nothing grounds, every body
    /// of the island, moved straight across the view plane together.
    island: Vec<BodyId>,
}

/// Steps a joint's range is checked at for collisions.
const SWEEP_STEPS: usize = 24;

/// A joint's drive being swept: the value it held before, to go back to,
/// and how far round the sweep is (radians of a cosine).
#[cfg(feature = "egui")]
#[derive(Debug, Clone, Copy)]
struct Play {
    joint: FeatureId,
    start: f32,
    phase: f64,
}

/// Each jointed body and the motions its joints leave it.
type Freedom = Vec<(BodyId, Vec<Motion>)>;

/// Joints by id and name.
type Named = Vec<(FeatureId, String)>;

impl AssemblyWorkbench {
    /// The redundant joints, worked out again only after an edit.
    pub(crate) fn redundant_now(&self, ctx: &WorkbenchRuntimeContext) -> Vec<(FeatureId, String)> {
        let seq = ctx.document.mutation_seq();
        let mut cache = self.tab.redundant.lock().unwrap();
        match &*cache {
            Some((at, found)) if *at == seq => found.clone(),
            _ => {
                let found = redundant(ctx.document);
                *cache = Some((seq, found.clone()));
                found
            }
        }
    }

    /// The joints and couplings left apart, worked out again only after
    /// an edit.
    pub(crate) fn unmet_now(&self, ctx: &WorkbenchRuntimeContext) -> Vec<(FeatureId, String)> {
        let seq = ctx.document.mutation_seq();
        let mut cache = self.tab.unmet.lock().unwrap();
        match &*cache {
            Some((at, found)) if *at == seq => found.clone(),
            _ => {
                let found = unmet(ctx.document);
                *cache = Some((seq, found.clone()));
                found
            }
        }
    }

    /// Each jointed body's free motions, worked out again only after an
    /// edit.
    fn freedom_now(&self, ctx: &WorkbenchRuntimeContext) -> Freedom {
        let seq = ctx.document.mutation_seq();
        let mut cache = self.tab.freedom.lock().unwrap();
        match &*cache {
            Some((at, found)) if *at == seq => found.clone(),
            _ => {
                let found = freedom(ctx.document);
                *cache = Some((seq, found.clone()));
                found
            }
        }
    }
}

impl AssemblyWorkbench {
    /// A body clicked while a task picks bodies: a point of it to trace in
    /// a motion study, the body in or out of an exploded view's next step
    /// or a group, or the body to take a replaced one's place.
    fn take_group_pick(&mut self, ctx: &WorkbenchRuntimeContext) {
        if let Some(Task::Motion(studying)) = &mut self.tab.task {
            let clicked = ctx.selected_face.map(|f| f.point);
            let key = ctx.selected_body_id;
            if studying.tracing
                && key != self.tab.group_seen
                && let (Some(body), Some(point)) = (key.map(BodyId), clicked)
            {
                let local = ctx.document.body_placement(body).inverse().point(point);
                studying.traces.push((body, local));
                studying.tracing = false;
            }
            self.tab.group_seen = key;
            return;
        }
        if let Some(Task::Explode { steps, .. }) = &mut self.tab.task {
            let clicked = ctx.selected_body_id;
            if clicked != self.tab.group_seen {
                self.tab.group_seen = clicked;
                if let Some(body) = clicked.map(BodyId) {
                    match steps.picked.iter().position(|b| *b == body) {
                        Some(i) => {
                            steps.picked.remove(i);
                        }
                        None => steps.picked.push(body),
                    }
                }
            }
            return;
        }
        if let Some(Task::Replace { old, new }) = &mut self.tab.task {
            let clicked = ctx.selected_body_id;
            if clicked != self.tab.group_seen {
                self.tab.group_seen = clicked;
                if let Some(body) = clicked.map(BodyId).filter(|b| b != old) {
                    *new = Some(body);
                }
            }
            return;
        }
        let Some(Task::Group { members, .. }) = &mut self.tab.task else {
            return;
        };
        let clicked = ctx.selected_body_id;
        if clicked == self.tab.group_seen {
            return;
        }
        self.tab.group_seen = clicked;
        let Some(body) = clicked.map(BodyId) else {
            return;
        };
        match members.iter().position(|b| *b == body) {
            Some(i) => {
                members.remove(i);
            }
            None => members.push(body),
        }
    }

    /// Put the assembly back as saved state `id` has it, solve, and record
    /// it.
    pub(crate) fn restore_state(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        let Some(state) = ctx
            .document
            .get_feature_data(id)
            .and_then(|d| AssemblyState::from_json(d).ok())
        else {
            return;
        };
        states::restore(ctx.document, &state);
        self.solve_and_apply(ctx);
        ctx.record(
            "asm.restore_state",
            commands::object(serde_json::json!({"state": id.0.to_string()})),
            serde_json::Value::Null,
        );
        ctx.request(HostRequest::JournalLabel("Restore assembly state".into()));
    }

    /// The group being picked made, or the one edited changed to the
    /// bodies picked, where they sit now.
    pub(crate) fn make_group(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        editing: Option<FeatureId>,
        members: &[BodyId],
    ) -> Option<FeatureId> {
        if members.len() < 2 {
            ctx.log_warn("A group takes two bodies or more");
            return None;
        }
        let group = RigidGroup::of(ctx.document, members);
        let id = match editing {
            Some(id) => {
                ctx.document.update_feature_data(id, group.to_json()).ok()?;
                id
            }
            None => {
                let name = commands::next_name(ctx.document, "Group");
                ctx.document
                    .add_feature_in_body(group, name, Some(members[0]))
                    .ok()?
            }
        };
        ctx.document.clear_feature_dirty(id);
        self.solve_and_apply(ctx);
        Some(id)
    }
}

/// Put bodies where `moves` say, the couplings whose drivers they turn
/// past a whole turn counting it.
pub(crate) fn place_bodies(
    document: &mut core_document::Document,
    moves: &[(BodyId, BodyPlacement)],
) {
    for (id, coupling) in counted_couplings(document, moves) {
        if document.update_feature_data(id, coupling.to_json()).is_ok() {
            document.clear_feature_dirty(id);
        }
    }
    for (body, placement) in moves {
        document.set_body_placement(*body, *placement);
    }
}

/// Every body's placement, to put back when a task is cancelled.
pub(crate) fn all_placements(ctx: &WorkbenchRuntimeContext) -> Vec<(BodyId, BodyPlacement)> {
    ctx.document
        .bodies()
        .iter()
        .map(|b| (b.id, b.placement))
        .collect()
}

pub(crate) fn restore_placements(
    ctx: &mut WorkbenchRuntimeContext,
    placements: &[(BodyId, BodyPlacement)],
) {
    for (body, placement) in placements {
        ctx.document.set_body_placement(*body, *placement);
    }
}

/// What a joint's task recorded mid-way stays: one undo step, which its
/// Cancel no longer takes back, as the recording keeps the calls.
pub(crate) fn keep_recorded(ctx: &mut WorkbenchRuntimeContext, created: bool) {
    let label = if created { "Add joint" } else { "Edit joint" };
    ctx.request(core_document::HostRequest::KeepTaskEdits(label.into()));
}

/// A feature's name in the tree, empty when it is gone.
fn feature_name(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> String {
    ctx.document
        .get_feature_meta(id)
        .map(|node| node.name.clone())
        .unwrap_or_default()
}

fn body_name(ctx: &WorkbenchRuntimeContext, body: BodyId) -> String {
    if body == WORLD {
        return "the origin".to_string();
    }
    ctx.document
        .body(body)
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "a removed body".to_string())
}

/// Place every jointed body, as edits: how many moved, or why they could
/// not all be placed.
/// What a solve that moved `count` bodies says.
pub(crate) fn moved_words(count: usize) -> String {
    match count {
        0 => "Every joint holds".to_string(),
        1 => "Moved 1 body; every joint holds".to_string(),
        n => format!("Moved {n} bodies; every joint holds"),
    }
}

pub(crate) fn apply_solve(ctx: &mut WorkbenchRuntimeContext) -> Result<String, String> {
    match solve(ctx.document) {
        Ok(moves) => {
            let count = moves.len();
            place_bodies(ctx.document, &moves);
            Ok(moved_words(count))
        }
        Err(SolveError::Conflict {
            body,
            joints,
            moves,
        }) => {
            // What the conflict does not hold up is placed all the same.
            place_bodies(ctx.document, &moves);
            Err(format!(
                "{} cannot hold all its joints at once: {}",
                body_name(ctx, body),
                joints.join(", ")
            ))
        }
    }
}

impl AssemblyWorkbench {
    /// Place every jointed body, as edits, and say how it went.
    fn solve_and_apply(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.tab.verdict = Some(apply_solve(ctx));
        if let Some(Err(message)) = &self.tab.verdict {
            ctx.log_warn(message.clone());
        }
    }
}

impl Workbench for AssemblyWorkbench {
    fn parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        self.parameters_of(node)
    }

    /// A joint's ends picked on named faces follow those faces: a body
    /// rebuilt with a face moved is searched for the face by name, and the
    /// end moves onto it, so the joint holds to the face wherever the rebuild put it.
    fn derive_on_geometry(
        &self,
        node: &FeatureNode,
        values: &mut serde_json::Value,
        document: &core_document::Document,
        _kernel: Option<&dyn kernel_api::KernelQueries>,
    ) -> bool {
        if node.workbench_id.as_str() != JOINT_KIND {
            return false;
        }
        let Ok(mut joint) = serde_json::from_value::<JointFeature>(values.clone()) else {
            return false;
        };
        let mut changed = false;
        for (end, body) in [(0, node.body), (1, Some(joint.other_body))] {
            let name = joint.names[end];
            let Some(body) = body.filter(|_| name != 0) else {
                continue;
            };
            let Some((mesh, _)) = document.local_geometry(body) else {
                continue;
            };
            let Some(surface) = mesh
                .face_names
                .iter()
                .position(|n| *n == name)
                .and_then(|i| mesh.face_surfaces.get(i))
            else {
                continue;
            };
            let candidate = match *surface {
                kernel_api::FaceSurface::Plane { origin, normal } => Anchor::Plane {
                    point: origin,
                    normal,
                },
                kernel_api::FaceSurface::Sphere { center, .. } => Anchor::Point { point: center },
                other => match other.axis() {
                    Some((point, direction)) => Anchor::Axis { point, direction },
                    None => continue,
                },
            };
            let anchor = if end == 0 {
                &mut joint.moving
            } else {
                &mut joint.fixed
            };
            if let Some(found) = replace::nearest_like(anchor, &[candidate]) {
                let ((p, d), (q, e)) = (anchor.parts(), found.parts());
                if p.distance(q) > 1e-5 || d.distance(e) > 1e-6 {
                    *anchor = found;
                    changed = true;
                }
            }
        }
        if changed {
            *values = joint.to_json();
        }
        changed
    }

    /// A joint or coupling whose numbers a formula moved: the bodies
    /// follow.
    fn values_moved(&mut self, ctx: &mut WorkbenchRuntimeContext, moved: &[FeatureId]) {
        let joint_moved = moved.iter().any(|id| {
            ctx.document
                .get_feature_meta(*id)
                .is_some_and(|n| matches!(n.workbench_id.as_str(), JOINT_KIND | COUPLING_KIND))
        });
        if joint_moved {
            self.solve_and_apply(ctx);
            self.note_left(ctx.document);
        }
    }

    fn viewport(&self, ctx: &WorkbenchRuntimeContext) -> core_document::ViewportOutput {
        core_document::ViewportOutput {
            meshes: self.overlay_meshes(ctx, ctx.active_document_object),
            overlays: self.screen_overlays(ctx, ctx.active_document_object),
            polygons: self.screen_polygons(ctx, ctx.active_document_object),
            marks: self.screen_marks(ctx, ctx.active_document_object),
            labels: self.screen_labels(ctx, ctx.active_document_object),
            hud: self.viewport_hud(ctx),
            status: self.status_items(ctx),
            faded: self.faded_bodies(ctx),
            ..Default::default()
        }
    }

    /// Bought parts are not made: exports of the model and the slicer
    /// leave them out.
    fn not_printed(
        &self,
        document: &core_document::Document,
        bought_kinds: &[core_document::WorkbenchId],
    ) -> Vec<BodyId> {
        parts::bought_bodies(document, bought_kinds)
    }

    fn print_parts(
        &self,
        document: &core_document::Document,
        bought_kinds: &[core_document::WorkbenchId],
    ) -> Option<Vec<core_document::PrintPart>> {
        Some(
            parts::parts_list(document, bought_kinds)
                .into_iter()
                .map(|part| core_document::PrintPart {
                    name: part.name,
                    bodies: part.bodies,
                    count: part.print,
                })
                .collect(),
        )
    }

    fn menu_items(
        &self,
        scope: &core_document::MenuScope,
        document: &core_document::Document,
    ) -> Vec<core_document::MenuItem> {
        self.menu_entries(scope, document)
    }

    fn on_command(
        &mut self,
        id: &str,
        scope: &core_document::MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        self.menu_command(id, scope, ctx)
    }

    /// The joints of other bodies that hold them to `body`.
    fn linked_features(&self, document: &core_document::Document, body: BodyId) -> Vec<FeatureId> {
        joints(document)
            .into_iter()
            .filter(|j| {
                j.feature.kind != JointKind::Ground
                    && j.feature.other_body == body
                    && j.body != body
            })
            .map(|j| j.id)
            .collect()
    }

    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            JOINT_KIND,
            "Assembly",
            "Place bodies against each other with joints",
        )
        .icon("workbench-assembly")
        .feature_kinds([
            JOINT_KIND,
            COUPLING_KIND,
            parts::PARTS_KIND,
            GROUP_KIND,
            STATE_KIND,
            EXPLODED_KIND,
            MOTION_KIND,
        ])
        .radial([
            "asm.mate",
            "asm.align",
            "asm.hinge",
            "asm.slider",
            "asm.fix",
            "asm.distance",
            "asm.move",
            "asm.interference",
        ])
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        self.register_tools(context);
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        commands::run(id, args, ctx)
    }

    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        if node.workbench_id.as_str() == MOTION_KIND {
            return FeatureInfo {
                icon: "polar-pattern",
                kind_label: "Motion".to_string(),
                family_label: "Assembly motion over time".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == EXPLODED_KIND {
            return FeatureInfo {
                icon: "scale-geometry",
                kind_label: "Exploded view".to_string(),
                family_label: "Assembly exploded view".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == STATE_KIND {
            return FeatureInfo {
                icon: "save",
                kind_label: "Assembly state".to_string(),
                family_label: "Saved assembly state".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == GROUP_KIND {
            return FeatureInfo {
                icon: "tree-group",
                kind_label: "Rigid group".to_string(),
                family_label: "Assembly rigid group".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == parts::PARTS_KIND {
            return FeatureInfo {
                icon: "file-document",
                kind_label: "Parts list".to_string(),
                family_label: "Assembly parts list".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == COUPLING_KIND {
            let gearing = Coupling::from_json(&node.data).map(|c| c.gearing).ok();
            return FeatureInfo {
                icon: "involute-gear",
                kind_label: gearing.map_or("Coupling", Gearing::label).to_string(),
                family_label: "Assembly coupling".to_string(),
                builds_solid: false,
            };
        }
        let kind = JointFeature::from_json(&node.data).map(|j| j.kind).ok();
        FeatureInfo {
            icon: kind.map_or("joint-mate", |k| k.icon()),
            kind_label: kind.map_or("Joint", |k| k.label()).to_string(),
            family_label: "Assembly joint".to_string(),
            builds_solid: false,
        }
    }

    /// A check or a measuring running away from the window, its answer
    /// shown once it comes, or a motion study, an exploded view or a
    /// joint's drive playing.
    fn busy(&self) -> bool {
        self.tab.checking.is_some()
            || self.tab.measuring.is_some()
            || self.tab.sweeping.is_some()
            || matches!(&self.tab.task, Some(Task::Explode { steps, .. }) if steps.playing)
            || matches!(&self.tab.task, Some(Task::Motion(studying)) if studying.playing)
            || self.panel_running()
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        tool_id == "asm.collisions" && !self.tab.collisions_off
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        self.tool_enabled(tool_id, ctx)
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        let result = self.input_turn(event, tool, ctx);
        self.note_left(ctx.document);
        result
    }

    fn cancel_pointer_gesture(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(body) = self.handled_body(ctx) {
            self.tab.handles.cancel(ctx, body);
        }
        self.tab.handles.reset();
        self.drop_grab(ctx);
        self.note_left(ctx.document);
    }

    fn on_frame(&mut self, dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        self.frame_turn(dt, ctx);
        self.note_left(ctx.document);
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        self.open_task(ctx, id);
    }

    /// The task's title and the name of what it edits under it: the
    /// panel itself carries no title.
    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        self.task_of(ctx)
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.cancel_pointer_gesture(ctx);
        self.put_back_motion(ctx);
        self.tab.picking = None;
        self.tab.checking = None;
        self.tab.measuring = None;
        self.put_back_explosion(ctx);
        self.tab.task = None;
    }

    fn tab_state(&mut self) -> Option<&mut dyn core_document::TabSlot> {
        Some(&mut self.tab)
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.cancel_pointer_gesture(ctx);
        self.put_back_motion(ctx);
        self.tab.picking = None;
        self.tab.checking = None;
        self.tab.measuring = None;
        self.put_back_explosion(ctx);
        self.tab.task = None;
    }
}

#[cfg(feature = "egui")]
impl bench_panel::BenchUi for AssemblyWorkbench {
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        self.draw_task_panel(ui, ctx, request)
    }
}

/// What the crate's unit tests share: a context, joints, geometry and
/// the bench's tools.
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "egui"))]
mod icon_coverage_tests;
