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

mod collide;
mod commands;
mod components;
mod coupling;
mod exploded;
mod group;
mod handles;
mod interference;
mod joint;
mod mass;
mod motion;
#[cfg(feature = "egui")]
mod panel;
mod parts;
mod replace;
mod shapes;
mod solve;
mod states;
mod sweep_check;

use core_document::{
    BodyId, BodyPlacement, FeatureId, FeatureInfo, FeatureNode, HostRequest, InputResult,
    ToolDescriptor, ToolHint, ViewportHud, Workbench, WorkbenchContext, WorkbenchDescriptor,
    WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
};

pub use coupling::{COUPLING_KIND, Coupling, Gearing};
pub use exploded::{EXPLODED_KIND, ExplodeStep, ExplodedView};
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
    redundant, solve,
};
pub use states::{AssemblyState, STATE_KIND};
pub use sweep_check::MotionClash;

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

impl Picking {
    fn new(kind: JointTool, repick: Option<FeatureId>) -> Self {
        Self {
            kind,
            first: None,
            repick,
            tab: None,
            slot: None,
        }
    }

    /// What to click next.
    fn prompt(&self) -> &'static str {
        if self.kind.picks_each() == 2 {
            return match (&self.first, &self.tab, &self.slot) {
                (None, ..) => "Click one face of the tab, on the body to move",
                (Some(_), None, _) => "Click the tab's other face",
                (_, Some(_), None) => "Click one wall of the slot, on another body",
                _ => "Click the slot's other wall",
            };
        }
        self.kind.prompt(self.first.is_some())
    }
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
/// step and its shift, and whether it plays.
#[derive(Debug, Clone, Default)]
pub(crate) struct Stepping {
    view: Option<FeatureId>,
    at: f32,
    picked: Vec<BodyId>,
    shift: [f32; 3],
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
    picking: Option<Picking>,
    /// The selection a pick was last taken from, so a pick is a change.
    seen: Option<(uuid::Uuid, [u32; 3])>,
    task: Option<Task>,
    /// What the last solve said, for the task panel.
    verdict: Option<Result<String, String>>,
    /// What each jointed body may still do, and at which edit of the
    /// document that was worked out: the status bar asks every frame.
    freedom: std::sync::Mutex<Option<(u64, Freedom)>>,
    /// The joints that hold nothing the others do not, at an edit of the
    /// document, for the status bar and the joint panel.
    redundant: std::sync::Mutex<Option<(u64, Named)>>,
    /// A body held by the mouse, dragged with its joints holding.
    grab: Option<Grab>,
    /// Drags go through other bodies rather than stopping at them.
    collisions_off: bool,
    /// An interference check running on its own thread.
    checking: Option<Checking>,
    /// The bodies being measured for their mass, on their own thread.
    measuring: Option<Measuring>,
    /// A joint's motion being checked for collisions, on its own thread.
    sweeping: Option<Sweeping>,
    /// What the last check of a joint's motion found, for that joint.
    motion_clashes: Option<(FeatureId, Result<Vec<MotionClash>, String>)>,
    /// The clearance the interference panel checks for, mm, as last typed.
    clearance_mm: Option<f32>,
    /// How far a joint's Turn turns its body, degrees, as last typed.
    turn_by: Option<f32>,
    /// The body last clicked while a group's bodies are picked, so a click
    /// counts once.
    group_seen: Option<uuid::Uuid>,
    /// The Move task's handles: whether they move or turn, and the one held.
    handles: handles::Handles,
    /// A driven hinge or slider swept through its range to show it move.
    #[cfg(feature = "egui")]
    playing: Option<Play>,
    /// The name of a column to add to the parts list, as typed.
    #[cfg(feature = "egui")]
    parts_column: String,
    /// The parts list as CSV, to put on the clipboard once the panel is
    /// drawn.
    #[cfg(feature = "egui")]
    copied: Option<String>,
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
    /// Where the pull last went without a collision.
    reached: Option<[f32; 3]>,
    /// What bodies shared when the drag began.
    baseline: collide::Baseline,
    /// A body no joint places, moved straight across the view plane.
    free: bool,
}

/// An interference check under way: its answer to come, the pairs asked
/// about so far out of `total`, and the flag that stops it.
struct Checking {
    answer: std::sync::mpsc::Receiver<Result<Interference, String>>,
    done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Checking {
    /// A check nobody waits for stops at its next pair.
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Bodies being measured: the report to come, how many are done of
/// `total`, and the flag that stops it.
struct Measuring {
    answer: std::sync::mpsc::Receiver<Result<MassReport, String>>,
    done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Measuring {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A joint's motion checked for collisions: the answer to come, pairs
/// done of `total`, and the flag that stops it.
struct Sweeping {
    joint: FeatureId,
    answer: std::sync::mpsc::Receiver<Result<Vec<MotionClash>, String>>,
    done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Sweeping {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Steps a joint's range is checked at for collisions.
const SWEEP_STEPS: usize = 24;

/// How far, in pixels, the mouse goes before a press is a drag.
const DRAG_PX: f32 = 4.0;

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
        let mut cache = self.redundant.lock().unwrap();
        match &*cache {
            Some((at, found)) if *at == seq => found.clone(),
            _ => {
                let found = redundant(ctx.document);
                *cache = Some((seq, found.clone()));
                found
            }
        }
    }

    /// Each jointed body's free motions, worked out again only after an
    /// edit.
    fn freedom_now(&self, ctx: &WorkbenchRuntimeContext) -> Freedom {
        let seq = ctx.document.mutation_seq();
        let mut cache = self.freedom.lock().unwrap();
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

/// A drive's numbers, those it has: what it holds the motion at and its
/// limits.
fn drive_parameters(
    variant: &str,
    drive: &Drive,
    dim: core_document::expr::Dim,
) -> Vec<core_document::Parameter> {
    named_drive_parameters(&format!("/kind/{variant}/drive"), ("", ""), drive, dim)
}

/// The same for a drive stored at `base`, its keys and labels starting
/// with `key` and `label` (the bare words, capitalized, when `key` is
/// empty).
fn named_drive_parameters(
    base: &str,
    (key, label): (&str, &str),
    drive: &Drive,
    dim: core_document::expr::Dim,
) -> Vec<core_document::Parameter> {
    use core_document::Parameter;
    let mut out = Vec::new();
    let named = |name: &str| {
        if key.is_empty() {
            (name.to_string(), capitalized(name))
        } else {
            (format!("{key}_{name}"), format!("{label} {name}"))
        }
    };
    if drive.to.is_some() {
        let (name, text) = named("drive");
        out.push(Parameter::new(&name, &text, dim, format!("{base}/to")));
    }
    if drive.limits.is_some() {
        for (end, word) in [(0, "lowest"), (1, "highest")] {
            let (name, text) = named(word);
            out.push(Parameter::new(
                &name,
                &text,
                dim,
                format!("{base}/limits/{end}"),
            ));
        }
    }
    out
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

impl AssemblyWorkbench {
    /// Look for clashes among the visible solid bodies, or between
    /// `around` and the others, and show them.
    pub(crate) fn check_interference(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        around: Option<BodyId>,
    ) {
        self.check_bodies(ctx, around, None);
    }

    /// Look for pairs nearer than `clearance` mm, as a check of shared
    /// material does for clashes.
    pub(crate) fn check_clearance(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        around: Option<BodyId>,
        clearance: f32,
    ) {
        self.check_bodies(ctx, around, Some(clearance));
    }

    fn check_bodies(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        around: Option<BodyId>,
        clearance: Option<f32>,
    ) {
        let Some(kernel) = ctx.kernel else {
            ctx.log_warn("No kernel to check interference with");
            return;
        };
        let mut check = match clearance {
            Some(gap) => interference::plan_clearance(ctx.document, None, f64::from(gap)),
            None => interference::plan(ctx.document, None),
        };
        if let Some(body) = around {
            check = check.around(body);
        }
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (send, answer) = std::sync::mpsc::channel();
        let total = check.pairs();
        {
            let (done, stop) = (done.clone(), stop.clone());
            let spawned = std::thread::Builder::new()
                .name("printcad-interference".into())
                .spawn(move || {
                    let _ = send.send(check.run(kernel, &done, &stop));
                });
            if let Err(why) = spawned {
                ctx.log_warn(format!("The interference check could not start: {why}"));
                return;
            }
        }
        self.checking = Some(Checking {
            answer,
            done,
            total,
            stop,
        });
        self.task = Some(Task::Interference {
            found: None,
            seq: ctx.document.mutation_seq(),
            around,
            clearance,
        });
    }

    /// A finished check's answer, when it has come: shown, and said in the
    /// log.
    pub(crate) fn collect_interference(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if !matches!(self.task, Some(Task::Interference { .. })) {
            // Closed while it ran: nobody waits for it.
            self.checking = None;
            return;
        }
        let Some(checking) = &self.checking else {
            return;
        };
        let answer = match checking.answer.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("the check ended without an answer".to_string())
            }
        };
        self.checking = None;
        match answer {
            Ok(found) => {
                let stopped = if found.stopped {
                    " (stopped early)"
                } else {
                    ""
                };
                ctx.log_info(match (found.clearance, found.clashes.len()) {
                    (Some(gap), _) => format!(
                        "{} pair{} nearer than {gap} mm among {} bodies{stopped}",
                        found.near.len(),
                        if found.near.len() == 1 { "" } else { "s" },
                        found.checked
                    ),
                    (None, 0) => format!("No interference among {} bodies{stopped}", found.checked),
                    (None, n) => format!(
                        "{n} clash{} among {} bodies{stopped}",
                        if n == 1 { "" } else { "es" },
                        found.checked
                    ),
                });
                for u in &found.unchecked {
                    ctx.log_warn(format!(
                        "{} and {} could not be checked: {}",
                        body_name(ctx, u.a),
                        body_name(ctx, u.b),
                        u.why
                    ));
                }
                if let Some(Task::Interference { found: slot, .. }) = &mut self.task {
                    *slot = Some(found);
                }
            }
            Err(why) => {
                ctx.log_warn(format!("The interference check failed: {why}"));
                if matches!(self.task, Some(Task::Interference { .. })) {
                    self.task = None;
                }
            }
        }
    }

    /// How far a running check has got: pairs asked about, of how many.
    pub(crate) fn interference_progress(&self) -> Option<(usize, usize)> {
        let checking = self.checking.as_ref()?;
        Some((
            checking.done.load(std::sync::atomic::Ordering::Relaxed),
            checking.total,
        ))
    }

    /// Stop a running check; what it found so far still comes.
    pub(crate) fn stop_interference(&self) {
        if let Some(checking) = &self.checking {
            checking
                .stop
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The pairs nearer than the clearance shown: the line between their
    /// nearest points on screen, and how near.
    fn near_on_screen(&self, ctx: &WorkbenchRuntimeContext) -> Vec<([f32; 2], [f32; 2], f64)> {
        let near = match &self.task {
            Some(Task::Interference {
                found: Some(found), ..
            }) => found.near.as_slice(),
            _ => &[],
        };
        near.iter()
            .filter_map(|n| {
                let (ax, ay) = ctx.world_to_viewport(n.on_a)?;
                let (bx, by) = ctx.world_to_viewport(n.on_b)?;
                Some(([ax, ay], [bx, by], n.distance_mm))
            })
            .collect()
    }

    /// The joint selected or open in its task, drawn: at each end a dot
    /// where it takes hold, its direction (a flat face's normal, an axis
    /// both ways), a square in a flat face's plane, and a dashed link
    /// between the ends.
    fn joint_drawing(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(
        Vec<core_document::ScreenSpaceOverlay>,
        Vec<[f32; 2]>,
        String,
    )> {
        let id = match &self.task {
            Some(Task::Joint { id, .. }) => *id,
            _ => Self::selected_joint(ctx)?,
        };
        let joint = joints(ctx.document).into_iter().find(|j| j.id == id)?;
        if joint.feature.kind == JointKind::Ground {
            return None;
        }
        let color = ctx.sketch_palette.selected;
        let mut lines = Vec::new();
        let mut dots = Vec::new();
        let ends = [
            (joint.feature.moving, joint.body),
            (joint.feature.fixed, joint.feature.other_body),
        ];
        let screen = |p: glam::DVec3| ctx.world_to_viewport(p.as_vec3().to_array());
        for (anchor, body) in ends {
            let at: Rigid = ctx.document.body_placement(body).into();
            let (p, d) = anchor.placed(&at);
            let Some(centre) = screen(p) else {
                continue;
            };
            dots.push([centre.0, centre.1]);
            // About 40 pixels long wherever the camera stands.
            let Some(unit) = screen(p + d) else {
                continue;
            };
            let px = (unit.0 - centre.0).hypot(unit.1 - centre.1).max(1e-3);
            let reach = f64::from(40.0 / px);
            let line = |a: glam::DVec3, b: glam::DVec3| {
                Some(core_document::ScreenSpaceOverlay::new(
                    screen(a).map(|(x, y)| [x, y])?,
                    screen(b).map(|(x, y)| [x, y])?,
                    color,
                    1.5,
                ))
            };
            match anchor {
                Anchor::Plane { .. } => {
                    lines.extend(line(p, p + d * reach));
                    let side = d.any_orthonormal_pair();
                    let (u, v) = (side.0 * reach * 0.5, side.1 * reach * 0.5);
                    let corners = [p + u + v, p - u + v, p - u - v, p + u - v];
                    for k in 0..4 {
                        lines.extend(line(corners[k], corners[(k + 1) % 4]));
                    }
                }
                Anchor::Axis { .. } => {
                    lines.extend(
                        line(p - d * reach * 1.5, p + d * reach * 1.5).map(|l| l.dashed(6.0, 4.0)),
                    );
                }
                // A point is its dot.
                Anchor::Point { .. } => {}
            }
        }
        if let [a, b] = dots.as_slice() {
            lines.push(
                core_document::ScreenSpaceOverlay::new(*a, *b, color, 1.0)
                    .dashed(3.0, 3.0)
                    .with_alpha(0.8),
            );
        }
        Some((lines, dots, joint.name))
    }

    /// The clashes shown, each where it sits on screen.
    fn clashes_on_screen<'a>(
        &'a self,
        ctx: &'a WorkbenchRuntimeContext,
    ) -> impl Iterator<Item = ([f32; 2], &'a Clash)> + 'a {
        let clashes = match &self.task {
            Some(Task::Interference {
                found: Some(found), ..
            }) => found.clashes.as_slice(),
            _ => &[],
        };
        clashes.iter().filter_map(|clash| {
            let (x, y) = ctx.world_to_viewport(clash.centre)?;
            Some(([x, y], clash))
        })
    }
}

impl AssemblyWorkbench {
    /// A body clicked while a task picks bodies: a point of it to trace in
    /// a motion study, the body in or out of an exploded view's next step
    /// or a group, or the body to take a replaced one's place.
    fn take_group_pick(&mut self, ctx: &WorkbenchRuntimeContext) {
        if let Some(Task::Motion(studying)) = &mut self.task {
            let clicked = ctx.selected_face.map(|f| f.point);
            let key = ctx.selected_body_id;
            if studying.tracing
                && key != self.group_seen
                && let (Some(body), Some(point)) = (key.map(BodyId), clicked)
            {
                let local = ctx.document.body_placement(body).inverse().point(point);
                studying.traces.push((body, local));
                studying.tracing = false;
            }
            self.group_seen = key;
            return;
        }
        if let Some(Task::Explode { steps, .. }) = &mut self.task {
            let clicked = ctx.selected_body_id;
            if clicked != self.group_seen {
                self.group_seen = clicked;
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
        if let Some(Task::Replace { old, new }) = &mut self.task {
            let clicked = ctx.selected_body_id;
            if clicked != self.group_seen {
                self.group_seen = clicked;
                if let Some(body) = clicked.map(BodyId).filter(|b| b != old) {
                    *new = Some(body);
                }
            }
            return;
        }
        let Some(Task::Group { members, .. }) = &mut self.task else {
            return;
        };
        let clicked = ctx.selected_body_id;
        if clicked == self.group_seen {
            return;
        }
        self.group_seen = clicked;
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

    /// Measure the visible solid bodies away from the window and show
    /// their mass and centre of mass.
    pub(crate) fn measure_mass(&mut self, ctx: &mut WorkbenchRuntimeContext, density: f32) {
        let Some(kernel) = ctx.kernel else {
            ctx.log_warn("No kernel to measure with");
            return;
        };
        let weighing = mass::plan(ctx.document, None);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (send, answer) = std::sync::mpsc::channel();
        let total = weighing.len();
        {
            let (done, stop) = (done.clone(), stop.clone());
            let spawned = std::thread::Builder::new()
                .name("printcad-mass".into())
                .spawn(move || {
                    let _ = send.send(weighing.run(kernel, &done, &stop));
                });
            if let Err(why) = spawned {
                ctx.log_warn(format!("The measuring could not start: {why}"));
                return;
            }
        }
        self.measuring = Some(Measuring {
            answer,
            done,
            total,
            stop,
        });
        self.task = Some(Task::Mass {
            found: None,
            density,
        });
    }

    /// Whether the parts list's volumes are being measured.
    fn measuring_volumes(&self) -> bool {
        #[cfg(feature = "egui")]
        return self.volumes.running();
        #[cfg(not(feature = "egui"))]
        false
    }

    /// A finished measuring's report, when it has come.
    pub(crate) fn collect_mass(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if !matches!(self.task, Some(Task::Mass { .. })) {
            self.measuring = None;
            return;
        }
        let Some(measuring) = &self.measuring else {
            return;
        };
        let answer = match measuring.answer.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("the measuring ended without an answer".to_string())
            }
        };
        self.measuring = None;
        match answer {
            Ok(report) => {
                if let Some(Task::Mass { found, .. }) = &mut self.task {
                    *found = Some(report);
                }
            }
            Err(why) => {
                ctx.log_warn(format!("The bodies could not be measured: {why}"));
                self.task = None;
            }
        }
    }

    /// Check `joint`'s motion from `low` to `high` for collisions, away
    /// from the window.
    pub(crate) fn check_sweep(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        joint: FeatureId,
        (low, high): (f32, f32),
    ) {
        let Some(kernel) = ctx.kernel else {
            ctx.log_warn("No kernel to check collisions with");
            return;
        };
        let Some(check) = sweep_check::plan(ctx.document, joint, low, high, SWEEP_STEPS) else {
            self.motion_clashes = Some((joint, Ok(Vec::new())));
            ctx.log_info("Nothing moves through this joint's range");
            return;
        };
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (send, answer) = std::sync::mpsc::channel();
        let total = check.pairs();
        {
            let (done, stop) = (done.clone(), stop.clone());
            let spawned = std::thread::Builder::new()
                .name("printcad-sweep-check".into())
                .spawn(move || {
                    let _ = send.send(check.run(kernel, &done, &stop));
                });
            if let Err(why) = spawned {
                ctx.log_warn(format!("The check could not start: {why}"));
                return;
            }
        }
        self.motion_clashes = None;
        self.sweeping = Some(Sweeping {
            joint,
            answer,
            done,
            total,
            stop,
        });
    }

    /// A finished motion check's answer, when it has come.
    pub(crate) fn collect_sweep(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(sweeping) = &self.sweeping else {
            return;
        };
        let answer = match sweeping.answer.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("the check ended without an answer".to_string())
            }
        };
        let joint = sweeping.joint;
        self.sweeping = None;
        match &answer {
            Ok(found) if found.is_empty() => ctx.log_info("No collisions through the motion"),
            Ok(found) => ctx.log_warn(format!(
                "{} collision{} through the motion",
                found.len(),
                if found.len() == 1 { "" } else { "s" }
            )),
            Err(why) => ctx.log_warn(format!("The motion could not be checked: {why}")),
        }
        self.motion_clashes = Some((joint, answer));
    }

    /// How far a motion check has got, and for which joint.
    pub(crate) fn sweep_progress(&self) -> Option<(FeatureId, usize, usize)> {
        let s = self.sweeping.as_ref()?;
        Some((
            s.joint,
            s.done.load(std::sync::atomic::Ordering::Relaxed),
            s.total,
        ))
    }

    /// How far a measuring has got: bodies done, of how many.
    pub(crate) fn mass_progress(&self) -> Option<(usize, usize)> {
        let m = self.measuring.as_ref()?;
        Some((m.done.load(std::sync::atomic::Ordering::Relaxed), m.total))
    }

    /// The centre of mass shown, on screen.
    fn centre_of_mass_on_screen(&self, ctx: &WorkbenchRuntimeContext) -> Option<[f32; 2]> {
        let Some(Task::Mass {
            found: Some(report),
            density,
        }) = &self.task
        else {
            return None;
        };
        let c = report.centre(f64::from(*density))?;
        let (x, y) = ctx.world_to_viewport(c.map(|v| v as f32))?;
        Some([x, y])
    }

    /// The open exploded view's bodies where its steps have them at the
    /// progress set.
    pub(crate) fn show_steps(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(Task::Explode {
            placements, steps, ..
        }) = &self.task
        else {
            return;
        };
        let Some(view) = steps
            .view
            .and_then(|id| exploded::view_of(ctx.document, id))
        else {
            return;
        };
        for (body, placement) in view.placed_at(placements, steps.at) {
            ctx.document.set_body_placement(body, placement);
        }
    }

    /// The explode lines of the open view: each moved body's middle from
    /// where it sits to where the view has it.
    fn explode_lines(&self, ctx: &WorkbenchRuntimeContext) -> Vec<([f32; 3], [f32; 3])> {
        let Some(Task::Explode {
            placements, steps, ..
        }) = &self.task
        else {
            return Vec::new();
        };
        let Some(view) = steps
            .view
            .and_then(|id| exploded::view_of(ctx.document, id))
        else {
            return Vec::new();
        };
        view.placed_at(placements, steps.at)
            .into_iter()
            .zip(placements)
            .filter(|((_, now), (_, was))| !now.after(&was.inverse()).is_identity())
            .filter_map(|((body, now), (_, was))| {
                Some((
                    exploded::centre(ctx.document, body, was)?,
                    exploded::centre(ctx.document, body, &now)?,
                ))
            })
            .collect()
    }

    /// The body the Move task has open, when no joint places it: its move
    /// handles show and drag.
    fn handled_body(&self, ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
        match &self.task {
            Some(Task::Move { body, .. }) if !Self::held_by_joints(ctx, *body) => Some(*body),
            _ => None,
        }
    }

    /// A press on a move handle takes it; moves slide or turn the body,
    /// Shift in steps; the release lets go, Escape puts the body back. The
    /// task's OK records where it ends.
    fn handle_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        let Some(body) = self.handled_body(ctx) else {
            self.handles.reset();
            return None;
        };
        let taken = match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: (x, y),
            } => self.handles.press(ctx, body, emath::pos2(*x, *y)),
            WorkbenchInputEvent::MouseMove {
                viewport_pos: (x, y),
            } => self.handles.drag(ctx, body, emath::pos2(*x, *y)),
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                ..
            } => self.handles.release(ctx, body),
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } => self.handles.cancel(ctx, body),
            WorkbenchInputEvent::Action { id } if id == handles::MODE_ACTION => {
                self.handles.switch(ctx, body);
                true
            }
            _ => false,
        };
        taken.then(InputResult::consumed)
    }

    /// A motion study open: the bodies back where they were.
    pub(crate) fn put_back_motion(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(Task::Motion(studying)) = &self.task {
            restore_placements(ctx, &studying.placements);
            self.task = None;
        }
    }

    /// The motion study's bodies where frame `frame` has them.
    pub(crate) fn show_frame(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(Task::Motion(studying)) = &self.task else {
            return;
        };
        let Some(frames) = &studying.frames else {
            return;
        };
        if let Some((_, placements)) = frames.get(studying.frame) {
            restore_placements(ctx, placements);
        }
    }

    /// An exploded view open: the bodies back where they were.
    pub(crate) fn put_back_explosion(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(Task::Explode { placements, .. }) = &self.task {
            restore_placements(ctx, placements);
            self.task = None;
        }
    }

    /// A press on a body its joints move, or on one no joint places and
    /// nothing grounds, takes hold of it; the press still goes on to the
    /// host, which selects on a click.
    fn take_hold(&mut self, ctx: &WorkbenchRuntimeContext, at: (f32, f32)) {
        self.grab = None;
        if self.picking.is_some()
            || matches!(self.task, Some(Task::Move { .. } | Task::Explode { .. }))
        {
            return;
        }
        let (Some(body), Some(point)) = (ctx.hovered_body_id, ctx.hovered_world_pos) else {
            return;
        };
        let body = BodyId(body);
        let Some((_, ray)) = ctx.viewport_to_ray(at) else {
            return;
        };
        // The plane square to the view, through the point taken.
        let forward =
            glam::Vec3::from_array(ctx.camera_target) - glam::Vec3::from_array(ctx.camera_position);
        let facing = if forward.length_squared() > 1e-12 {
            forward.normalize().to_array()
        } else {
            ray
        };
        let jointed = draggable(ctx.document, body);
        let unit = ctx.document.rigid_unit_of(body);
        let grounded = joints(ctx.document)
            .iter()
            .any(|j| unit.contains(&j.body) && j.feature.kind == JointKind::Ground);
        if !jointed && grounded {
            return;
        }
        self.grab = Some(Grab {
            body,
            point: ctx.document.body_placement(body).inverse().point(point),
            plane: (point, facing),
            press: at,
            dragging: false,
            baseline: collide::Baseline::new(&all_placements(ctx)),
            placements: all_placements(ctx),
            reached: None,
            free: !jointed,
        });
    }

    /// The held body follows the mouse across the plane it was taken in,
    /// as far as its joints let it. The moves still reach the host, so a
    /// drag is never taken for a click.
    fn drag_to(&mut self, ctx: &mut WorkbenchRuntimeContext, at: (f32, f32)) -> InputResult {
        let Some(grab) = self.grab.as_mut() else {
            return InputResult::ignored();
        };
        let moved = (at.0 - grab.press.0).hypot(at.1 - grab.press.1);
        if !grab.dragging && moved < DRAG_PX {
            return InputResult::ignored();
        }
        grab.dragging = true;
        let (origin, normal) = grab.plane;
        let Some(target) = ctx.viewport_to_plane(at, origin, normal) else {
            return InputResult::ignored();
        };
        if grab.free {
            // Straight across the plane; what is joined to it follows.
            let start = grab
                .placements
                .iter()
                .find(|(b, _)| *b == grab.body)
                .map(|(_, p)| *p)
                .unwrap_or_default();
            let by = glam::Vec3::from_array(target) - glam::Vec3::from_array(origin);
            components::move_with_unit(
                ctx.document,
                grab.body,
                BodyPlacement::new(start.quat(), start.offset() + by),
            );
            if let Ok(moves) = solve(ctx.document) {
                place_bodies(ctx.document, &moves);
            }
            return InputResult::redraw_only();
        }
        let kernel = ctx.kernel.filter(|_| !self.collisions_off);
        let proposed = drag(ctx.document, grab.body, grab.point, target);
        let moves = match kernel {
            None => Some(proposed),
            Some(kernel) => {
                let mut clear = |moves: &[(BodyId, BodyPlacement)]| {
                    collide::collides(ctx.document, kernel, moves, &mut grab.baseline)
                        .map(|hit| !hit)
                        .unwrap_or(true)
                };
                let from = glam::Vec3::from_array(grab.reached.unwrap_or(origin));
                let to = glam::Vec3::from_array(target);
                // The way there in steps short enough that nothing passes
                // through another body between two of them.
                let mut good = 0.0f32;
                let mut blocked = None;
                for t in collide::checkpoints(ctx.document, &proposed) {
                    let tried = if t >= 1.0 {
                        proposed.clone()
                    } else {
                        drag(
                            ctx.document,
                            grab.body,
                            grab.point,
                            from.lerp(to, t).to_array(),
                        )
                    };
                    if clear(&tried) {
                        good = t;
                    } else {
                        blocked = Some(t);
                        break;
                    }
                }
                if let Some(bad) = blocked {
                    // Between the last clear step and the first blocked one,
                    // halving, to the last pose that collides with nothing:
                    // the body stops at contact.
                    let (mut good, mut bad, mut best) = (good, bad, None);
                    for _ in 0..5 {
                        let mid = (good + bad) * 0.5;
                        let pull = from.lerp(to, mid).to_array();
                        let tried = drag(ctx.document, grab.body, grab.point, pull);
                        if clear(&tried) {
                            good = mid;
                            best = Some((tried, pull));
                        } else {
                            bad = mid;
                        }
                    }
                    if best.is_none() && good > 0.0 {
                        let pull = from.lerp(to, good).to_array();
                        best = Some((drag(ctx.document, grab.body, grab.point, pull), pull));
                    }
                    best.map(|(moves, pull)| {
                        grab.reached = Some(pull);
                        moves
                    })
                } else {
                    grab.reached = Some(target);
                    Some(proposed)
                }
            }
        };
        if let Some(moves) = moves {
            place_bodies(ctx.document, &moves);
        }
        InputResult::redraw_only()
    }

    /// The drag ends: the bodies stay where it left them, recorded as
    /// placements.
    fn let_go(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(grab) = self.grab.take() else {
            return InputResult::ignored();
        };
        if !grab.dragging {
            return InputResult::ignored();
        }
        for (body, before) in grab.placements {
            let now = ctx.document.body_placement(body);
            if now.after(&before.inverse()).is_identity() {
                continue;
            }
            ctx.record(
                "asm.place",
                commands::object(serde_json::json!({
                    "body": body.0.to_string(),
                    "translation": now.translation,
                    "rotation": now.rotation,
                })),
                serde_json::Value::Null,
            );
        }
        ctx.request(HostRequest::JournalLabel("Drag body".into()));
        InputResult::redraw_only()
    }
}

/// Every visible body moved out from the middle of the assembly by
/// `spread` times its own distance from it, from where `placements` put
/// them.
pub(crate) fn explode(
    ctx: &mut WorkbenchRuntimeContext,
    placements: &[(BodyId, BodyPlacement)],
    spread: f32,
) {
    let centres: Vec<(BodyId, BodyPlacement, glam::Vec3)> = placements
        .iter()
        .filter(|(body, _)| ctx.document.imported_body_effective_visible(*body))
        .filter_map(|(body, placement)| {
            let (mesh, bounds) = ctx.document.local_geometry(*body)?;
            let (lo, hi) = bounds.or_else(|| mesh.bounds())?;
            let middle = (glam::Vec3::from_array(lo) + glam::Vec3::from_array(hi)) * 0.5;
            let centre = glam::Vec3::from_array(placement.point(middle.to_array()));
            Some((*body, *placement, centre))
        })
        .collect();
    if centres.is_empty() {
        return;
    }
    let middle = centres.iter().map(|(_, _, c)| *c).sum::<glam::Vec3>() / centres.len() as f32;
    for (body, placement, centre) in centres {
        let out = (centre - middle) * spread;
        ctx.document.set_body_placement(
            body,
            BodyPlacement::new(placement.quat(), placement.offset() + out),
        );
    }
}

/// A hinge's or a slider's drive swept from `low` to `high` and back in
/// `count` frames, on a copy of the document: each frame, every body the
/// joints moved and where. Nothing changes in `document`.
pub fn sweep_frames(
    document: &core_document::Document,
    joint: FeatureId,
    low: f32,
    high: f32,
    count: usize,
) -> Vec<Vec<(BodyId, BodyPlacement)>> {
    let values: Vec<f32> = (0..count)
        .map(|i| {
            let t = i as f64 / count as f64;
            (f64::from(low)
                + f64::from(high - low) * (0.5 - 0.5 * (std::f64::consts::TAU * t).cos()))
                as f32
        })
        .collect();
    let frames = sweep_values(document, joint, &values);
    let start: Vec<(BodyId, BodyPlacement)> = document
        .bodies()
        .iter()
        .map(|b| (b.id, b.placement))
        .collect();
    let moved = frames.iter().any(|f| *f != start);
    if moved { frames } else { Vec::new() }
}

/// A hinge's or a slider's drive held at each of `values` in turn, on a
/// copy of the document: every body's placement at each. Empty for a
/// joint that is not a hinge or a slider.
pub fn sweep_values(
    document: &core_document::Document,
    joint: FeatureId,
    values: &[f32],
) -> Vec<Vec<(BodyId, BodyPlacement)>> {
    let Some(mut feature) = document
        .get_feature_data(joint)
        .and_then(|d| JointFeature::from_json(d).ok())
    else {
        return Vec::new();
    };
    let mut copy = document.clone();
    let mut frames = Vec::with_capacity(values.len());
    for value in values {
        match &mut feature.kind {
            JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } => {
                drive.to = Some(*value);
            }
            _ => return Vec::new(),
        }
        if copy.update_feature_data(joint, feature.to_json()).is_err() {
            return Vec::new();
        }
        if let Ok(moves) = solve(&copy) {
            place_bodies(&mut copy, &moves);
        }
        frames.push(copy.bodies().iter().map(|b| (b.id, b.placement)).collect());
    }
    frames
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

fn body_name(ctx: &WorkbenchRuntimeContext, body: BodyId) -> String {
    if body == WORLD {
        return "the origin".to_string();
    }
    ctx.document
        .bodies()
        .iter()
        .find(|b| b.id == body)
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
        self.verdict = Some(apply_solve(ctx));
        if let Some(Err(message)) = &self.verdict {
            ctx.log_warn(message.clone());
        }
    }

    /// Take a new face pick for the joint being made: a face or an edge
    /// picked in the view, or a datum plane or line selected in the tree.
    fn take_pick(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(picking) = self.picking.clone() else {
            return;
        };
        if let Some((body, anchor, signature)) = Self::datum_pick(ctx) {
            if self.seen == Some(signature) {
                return;
            }
            self.seen = Some(signature);
            if !picking
                .kind
                .takes_anchor(&anchor, picking.first.map(|f| f.1))
            {
                ctx.log_warn(picking.kind.refusal());
                return;
            }
            self.picked(ctx, body, anchor, None, 0);
            return;
        }
        // A face, or else the last edge picked, where the joint takes one.
        let edge = ctx.selected_edges.last().copied();
        let (body, point, face, edge) = match (ctx.selected_body_id, ctx.selected_face, edge) {
            (Some(body), Some(face), _) => (body, face.point, Some(face), None),
            (_, None, Some(edge)) => (edge.body, edge.point, None, Some(edge)),
            _ => return,
        };
        let signature = (body, point.map(f32::to_bits));
        if self.seen == Some(signature) {
            return;
        }
        self.seen = Some(signature);
        let body = BodyId(body);
        let to_local = ctx.document.body_placement(body).inverse();
        let face = face.map(|f| f.moved(&to_local));
        let edge = edge.map(|e| e.moved(&to_local));
        let flat = || face.as_ref().and_then(Anchor::plane_of);
        let round = || {
            face.as_ref()
                .and_then(|f| Some((Anchor::axis_of(f)?, Anchor::radius_of(f))))
                .or_else(|| edge.map(|e| (Anchor::axis_of_edge(&e), e.circle.map(|c| c.radius))))
        };
        let first_flat = picking
            .first
            .map(|(_, anchor, ..)| matches!(anchor, Anchor::Plane { .. }));
        let pointed = || {
            face.as_ref()
                .map(|f| (Anchor::point_of(f), Anchor::radius_of(f)))
                .or_else(|| edge.map(|e| (Anchor::point_of_edge(&e), e.circle.map(|c| c.radius))))
        };
        let sphere = face
            .as_ref()
            .is_some_and(|f| matches!(f.surface, Some(kernel_api::FaceSurface::Sphere { .. })));
        let name = face.as_ref().map_or(0, |f| f.name);
        let picked = match (picking.kind.takes(), first_flat) {
            (Takes::Flat, _) => flat().map(|a| (a, None)),
            (Takes::Round, _) => round(),
            (Takes::Any, _) => flat().map(|a| (a, None)).or_else(round),
            (Takes::FlatAndRound, None) => flat().map(|a| (a, None)).or_else(round),
            (Takes::FlatAndRound, Some(true)) => round(),
            (Takes::FlatAndRound, Some(false)) => flat().map(|a| (a, None)),
            (Takes::Point, _) => pointed(),
            (Takes::Directed, _) => flat().map(|a| (a, None)).or_else(round),
            // A ball is its centre; any other face its plane or axis.
            (Takes::Anything, _) if sphere => pointed(),
            (Takes::Anything, _) => flat().map(|a| (a, None)).or_else(round).or_else(pointed),
            (Takes::PointAndLine, None) => pointed(),
            (Takes::PointAndLine, Some(_)) => round(),
            (Takes::PointAndEdge, None) => pointed(),
            // Where on the edge it was picked: the edge itself is kept.
            (Takes::PointAndEdge, Some(_)) => edge
                .map(|e| (Anchor::Point { point: e.point }, None))
                .or_else(|| {
                    face.as_ref()
                        .map(|f| (Anchor::Point { point: f.point }, None))
                }),
            // A roller is the point of its axis beside the pick, and its
            // radius.
            (Takes::PointAndFace, None) => match face.as_ref().and_then(|f| {
                let (p, d) = f.surface?.axis()?;
                Some((f, p, d, Anchor::radius_of(f)?))
            }) {
                Some((f, p, d, radius)) => {
                    let (p, d, at) = (
                        glam::Vec3::from_array(p),
                        glam::Vec3::from_array(d).normalize_or_zero(),
                        glam::Vec3::from_array(f.point),
                    );
                    let centre = p + d * (at - p).dot(d);
                    Some((
                        Anchor::Point {
                            point: centre.to_array(),
                        },
                        Some(radius),
                    ))
                }
                None => pointed(),
            },
            (Takes::PointAndFace, Some(_)) => face
                .as_ref()
                .map(|f| (Anchor::Point { point: f.point }, None)),
        };
        let Some((anchor, radius)) = picked else {
            ctx.log_warn(picking.kind.refusal());
            return;
        };
        self.picked(ctx, body, anchor, radius, name);
    }

    /// A datum plane or line selected in the tree, as an anchor on its
    /// body (or the world's, for one of no body), with the signature a
    /// repeat of it is known by.
    fn datum_pick(
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(BodyId, Anchor, (uuid::Uuid, [u32; 3]))> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        if node.workbench_id.as_str() != "core.datum" {
            return None;
        }
        let values = ctx.document.feature_values(id).unwrap_or(&node.data);
        let datum = core_document::DatumFeature::from_json(values).ok()?;
        let frame = datum.frame();
        let anchor = match datum.shape {
            core_document::DatumShape::Plane { .. } => Anchor::Plane {
                point: frame.origin,
                normal: frame.normal,
            },
            core_document::DatumShape::Line { .. } => Anchor::Axis {
                point: frame.origin,
                direction: frame.x_axis,
            },
            _ => return None,
        };
        Some((node.body.unwrap_or(WORLD), anchor, (id.0, [0; 3])))
    }

    /// An anchor picked on `body` (the world for the origin's planes and
    /// axes): the first end, or the second, which makes the joint.
    pub(crate) fn picked(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: BodyId,
        anchor: Anchor,
        radius: Option<f32>,
        name: kernel_api::TopoName,
    ) {
        let Some(picking) = self.picking.clone() else {
            return;
        };
        if picking.kind.picks_each() == 2
            && let Some((first_body, first_anchor, _, first_name)) = picking.first
        {
            match (picking.tab, picking.slot) {
                (None, _) if body != first_body => {
                    ctx.log_warn("Click the tab's other face, on the same body");
                }
                (None, _) => {
                    self.picking = Some(Picking {
                        tab: Some(anchor),
                        ..picking
                    });
                }
                (Some(_), None) if body == first_body || body == WORLD => {
                    ctx.log_warn("Click a wall of the slot, on another body");
                }
                (Some(_), None) => {
                    self.picking = Some(Picking {
                        slot: Some((body, anchor)),
                        ..picking
                    });
                }
                (Some(_), Some((slot_body, _))) if body != slot_body => {
                    ctx.log_warn("Click the slot's other wall, on the same body");
                }
                (Some(tab), Some((slot_body, wall))) => {
                    self.picking = None;
                    let kind = picking.kind.joint(
                        &first_anchor,
                        &ctx.document.body_placement(first_body).into(),
                        &wall,
                        &ctx.document.body_placement(slot_body).into(),
                        0.0,
                    );
                    self.make_joint(
                        ctx,
                        first_body,
                        JointFeature {
                            names: [first_name, 0],
                            kind,
                            moving: first_anchor,
                            other_body: slot_body,
                            fixed: wall,
                            ends: [0.0; 2],
                            shape: Vec::new(),
                            second: Some([tab, anchor]),
                        },
                    );
                }
            }
            return;
        }
        match picking.first {
            None if body == WORLD => {
                ctx.log_warn("Pick the body to move first; the origin stays where it is");
            }
            None => {
                self.picking = Some(Picking {
                    first: Some((body, anchor, radius, name)),
                    ..picking
                });
            }
            Some((first_body, ..)) if first_body == body => {
                ctx.log_warn("Pick the second face on another body");
            }
            Some((first_body, first_anchor, first_radius, first_name)) => {
                self.picking = None;
                let radius = first_radius.or(radius);
                if picking.kind == JointTool::Tangent && radius.is_none() {
                    ctx.log_warn("A tangent takes a round face with a radius: a cylinder");
                    return;
                }
                if let Some(joint) = picking.repick {
                    self.rejoin(
                        ctx,
                        joint,
                        picking.kind,
                        (first_body, first_anchor),
                        (body, anchor),
                        (radius, [first_name, name]),
                    );
                    return;
                }
                let kind = picking.kind.joint(
                    &first_anchor,
                    &ctx.document.body_placement(first_body).into(),
                    &anchor,
                    &ctx.document.body_placement(body).into(),
                    radius.unwrap_or_default(),
                );
                self.make_joint(
                    ctx,
                    first_body,
                    JointFeature {
                        second: None,
                        shape: Vec::new(),
                        ends: [0.0; 2],
                        names: [first_name, name],
                        kind,
                        moving: first_anchor,
                        other_body: body,
                        fixed: anchor,
                    },
                );
            }
        }
    }

    /// Give `joint` the faces just picked, as `asm.set` would, and open its
    /// settings again.
    fn rejoin(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        joint: FeatureId,
        tool: JointTool,
        moving: (BodyId, Anchor),
        fixed: (BodyId, Anchor),
        (radius, names): (Option<f32>, [kernel_api::TopoName; 2]),
    ) {
        let kept_radius = ctx
            .document
            .get_feature_data(joint)
            .and_then(|d| JointFeature::from_json(d).ok())
            .and_then(|j| match j.kind {
                JointKind::Tangent { radius } => Some(radius),
                _ => None,
            });
        let radius = radius.or(kept_radius).unwrap_or(0.0);
        match commands::rejoined(ctx, tool, moving, fixed, radius) {
            Ok(mut feature) => {
                feature.names = names;
                let face = |anchor: Anchor, body: BodyId, name: kernel_api::TopoName| {
                    let mut face = match anchor.moved(&ctx.document.body_placement(body)) {
                        Anchor::Plane { point, normal } => {
                            serde_json::json!({"point": point, "normal": normal})
                        }
                        Anchor::Axis { point, direction } => serde_json::json!({
                            "axis": {"point": point, "direction": direction},
                            "radius": radius,
                        }),
                        Anchor::Point { point } => serde_json::json!({"point": point}),
                    };
                    if name != 0 {
                        face["name"] = serde_json::json!(name);
                    }
                    face
                };
                let args = commands::object(serde_json::json!({
                    "joint": joint.0.to_string(),
                    "kind": tool.word(),
                    "face": face(moving.1, moving.0, names[0]),
                    "other": fixed.0.0.to_string(),
                    "other_face": face(fixed.1, fixed.0, names[1]),
                }));
                if ctx
                    .document
                    .update_feature_data(joint, feature.to_json())
                    .is_ok()
                {
                    ctx.document.clear_feature_dirty(joint);
                    self.solve_and_apply(ctx);
                    ctx.record("asm.set", args, serde_json::Value::Null);
                    ctx.log_info("Picked the joint's faces again");
                }
            }
            Err(why) => ctx.log_warn(why.to_string()),
        }
        ctx.active_document_object = Some(joint);
        self.task = Some(Task::Joint {
            id: joint,
            before: ctx.document.get_feature_data(joint).cloned(),
            placements: all_placements(ctx),
        });
    }

    /// Start picking `joint`'s faces again, for the kind `tool`.
    pub(crate) fn start_repick(&mut self, joint: FeatureId, tool: JointTool) {
        self.task = None;
        self.seen = None;
        self.picking = Some(Picking::new(tool, Some(joint)));
    }

    /// Add a joint, solve, and open its settings.
    fn make_joint(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        body: BodyId,
        mut joint: JointFeature,
    ) {
        shapes::take_shape(ctx.document, &mut joint);
        let placements = all_placements(ctx);
        let label = joint.kind.label();
        let number = joints(ctx.document)
            .iter()
            .filter(|j| j.feature.kind.label() == label)
            .count()
            + 1;
        let name = format!("{label} {number}");
        commands::ground_first(ctx, joint.other_body);
        match ctx
            .document
            .add_feature_in_body(joint, name.clone(), Some(body))
        {
            Ok(id) => {
                // A joint has no solid to rebuild.
                ctx.document.clear_feature_dirty(id);
                ctx.active_document_object = Some(id);
                self.task = Some(Task::Joint {
                    id,
                    before: None,
                    placements,
                });
                self.solve_and_apply(ctx);
                ctx.log_info(format!("Added {name}"));
            }
            Err(err) => ctx.log_error(format!("Could not add the joint: {err}")),
        }
    }

    /// The joint the tree has selected, if a joint is selected.
    fn selected_joint(ctx: &WorkbenchRuntimeContext) -> Option<FeatureId> {
        Self::selected_of(ctx, JOINT_KIND)
    }

    /// The feature of `kind` the tree has selected.
    fn selected_of(ctx: &WorkbenchRuntimeContext, kind: &str) -> Option<FeatureId> {
        let id = ctx.active_document_object?;
        let node = ctx.document.get_feature_meta(id)?;
        (node.workbench_id.as_str() == kind).then_some(id)
    }

    /// Couple two joints: the selected one leading, when it is a hinge or
    /// a slider, else the first pair that can be tied; then open its
    /// settings.
    fn make_coupling(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let all = joints(ctx.document);
        let movable: Vec<&Joint> = all
            .iter()
            .filter(|j| {
                matches!(
                    j.feature.kind,
                    JointKind::Hinge { .. } | JointKind::Slider { .. }
                )
            })
            .collect();
        let selected = Self::selected_joint(ctx);
        let mut pairs: Vec<(&Joint, &Joint)> = movable
            .iter()
            .flat_map(|a| movable.iter().map(move |b| (*a, *b)))
            .filter(|(a, b)| {
                a.id != b.id && Gearing::suiting(&a.feature.kind, &b.feature.kind).is_some()
            })
            .collect();
        pairs.sort_by_key(|(a, _)| Some(a.id) != selected);
        let Some((driver, driven)) = pairs.first().copied() else {
            ctx.log_warn("Couple joints needs two hinges, or a hinge and a slider");
            return;
        };
        let placements = solve::rigid_placements(ctx.document);
        let gearing =
            Gearing::suiting(&driver.feature.kind, &driven.feature.kind).unwrap_or(Gearing::Gears);
        let Some(coupling) = Coupling::new(
            gearing,
            driver,
            driven,
            gearing.default_ratio(),
            false,
            &placements,
        ) else {
            ctx.log_warn("These joints cannot be coupled where they stand");
            return;
        };
        let body = driven.body;
        let before = all_placements(ctx);
        let name = commands::next_name(ctx.document, gearing.label());
        match ctx
            .document
            .add_feature_in_body(coupling, name.clone(), Some(body))
        {
            Ok(id) => {
                ctx.document.clear_feature_dirty(id);
                ctx.active_document_object = Some(id);
                self.picking = None;
                self.task = Some(Task::Coupling {
                    id,
                    before: None,
                    placements: before,
                });
                self.solve_and_apply(ctx);
                ctx.log_info(format!("Added {name}"));
            }
            Err(err) => ctx.log_error(format!("Could not add the coupling: {err}")),
        }
    }

    /// The body the Move tool moves: the selected one, else the one the
    /// tree has active.
    fn body_to_move(ctx: &WorkbenchRuntimeContext) -> Option<BodyId> {
        ctx.selected_body_id.map(BodyId).or_else(|| {
            ctx.active_document_object
                .and_then(|id| ctx.document.get_feature_meta(id))
                .and_then(|node| node.body)
        })
    }

    /// Whether a body's own joints place it: then moving it by hand is
    /// undone by the next solve.
    fn held_by_joints(ctx: &WorkbenchRuntimeContext, body: BodyId) -> bool {
        joints(ctx.document).iter().any(|j| j.body == body)
    }
}

impl AssemblyWorkbench {
    /// The numbers a joint's kind has, for formulas.
    fn kind_parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        use core_document::Parameter;
        use core_document::expr::Dim;
        match JointFeature::from_json(&node.data).map(|j| j.kind) {
            Ok(JointKind::Mate { .. }) => {
                vec![Parameter::new(
                    "offset",
                    "Offset",
                    Dim::LENGTH,
                    "/kind/Mate/offset",
                )]
            }
            Ok(JointKind::Angle { .. }) => {
                vec![Parameter::new(
                    "angle",
                    "Angle",
                    Dim::ANGLE,
                    "/kind/Angle/degrees",
                )]
            }
            Ok(JointKind::Hinge { drive, .. }) => {
                let mut out = vec![Parameter::new(
                    "offset",
                    "Height",
                    Dim::LENGTH,
                    "/kind/Hinge/offset",
                )];
                out.extend(drive_parameters("Hinge", &drive, Dim::ANGLE));
                out
            }
            Ok(JointKind::Slider { drive, .. }) => drive_parameters("Slider", &drive, Dim::LENGTH),
            Ok(JointKind::Fixed { .. }) => ["x", "y", "z"]
                .into_iter()
                .enumerate()
                .map(|(k, axis)| {
                    Parameter::new(
                        &format!("shift_{axis}"),
                        &format!("Shift {axis}"),
                        Dim::LENGTH,
                        format!("/kind/Fixed/shift/{k}"),
                    )
                })
                .collect(),
            Ok(JointKind::Align { turn, slide, .. }) => {
                let mut out =
                    named_drive_parameters("/kind/Align/turn", ("turn", "Turn"), &turn, Dim::ANGLE);
                out.extend(named_drive_parameters(
                    "/kind/Align/slide",
                    ("slide", "Slide"),
                    &slide,
                    Dim::LENGTH,
                ));
                out
            }
            Ok(JointKind::Distance { .. }) => {
                vec![Parameter::new(
                    "offset",
                    "Distance",
                    Dim::LENGTH,
                    "/kind/Distance/offset",
                )]
            }
            Ok(JointKind::Tangent { .. }) => {
                vec![Parameter::new(
                    "radius",
                    "Radius",
                    Dim::LENGTH,
                    "/kind/Tangent/radius",
                )]
            }
            Ok(JointKind::Cam { .. }) => {
                vec![Parameter::new(
                    "radius",
                    "Radius",
                    Dim::LENGTH,
                    "/kind/Cam/radius",
                )]
            }
            _ => Vec::new(),
        }
    }
}

impl Workbench for AssemblyWorkbench {
    fn parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        use core_document::Parameter;
        use core_document::expr::Dim;
        if node.workbench_id.as_str() == COUPLING_KIND {
            return match Coupling::from_json(&node.data) {
                Ok(c) => {
                    let (label, length) = c.gearing.ratio_label();
                    let dim = if length { Dim::LENGTH } else { Dim::NUMBER };
                    vec![Parameter::new("ratio", label, dim, "/ratio")]
                }
                Err(_) => Vec::new(),
            };
        }
        let ends = [
            ("moving_end", "Moving end offset", "/ends/0"),
            ("fixed_end", "Fixed end offset", "/ends/1"),
        ]
        .map(|(name, label, pointer)| Parameter::new(name, label, Dim::LENGTH, pointer));
        let mut out = self.kind_parameters(node);
        if JointFeature::from_json(&node.data).is_ok_and(|j| j.kind != JointKind::Ground) {
            out.extend(ends);
        }
        out
    }

    /// A joint's ends picked on named faces follow those faces: a body
    /// rebuilt with a face moved is searched for the face by name, and the
    /// end moves onto it, so the joint holds to the face wherever the rebuild put it.
    fn derive_on_geometry(
        &self,
        node: &FeatureNode,
        values: &mut serde_json::Value,
        document: &core_document::Document,
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
        }
    }

    /// While a joint's faces are picked, every body but the one under the
    /// cursor and the one already picked fades, so faces behind show.
    fn faded_bodies(&self, ctx: &WorkbenchRuntimeContext) -> Vec<BodyId> {
        let Some(picking) = &self.picking else {
            return Vec::new();
        };
        let keep = [
            ctx.hovered_body_id.map(BodyId),
            picking.first.map(|(body, ..)| body),
        ];
        ctx.document
            .bodies()
            .iter()
            .map(|b| b.id)
            .filter(|b| !keep.contains(&Some(*b)))
            .collect()
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
        use core_document::MenuItem;
        let id = match scope {
            core_document::MenuScope::TreeFeature(id) => id,
            core_document::MenuScope::TreeBody(body) => {
                let own = document.component_of(*body);
                let mut items = vec![
                    MenuItem::new("asm.menu.component_new", "New component")
                        .icon("tree-group")
                        .hint("Put this body in a component of its own, inside the one it is in")
                        .separator_before(),
                ];
                for component in document.components() {
                    if Some(component.id) != own {
                        items.push(
                            MenuItem::new(
                                format!("asm.menu.component_to.{}", component.id.0),
                                format!("Move to {}", component.name),
                            )
                            .hint("Put this body in that component"),
                        );
                    }
                }
                if let Some(own) = own.and_then(|c| document.component(c)) {
                    items.push(
                        MenuItem::new(
                            "asm.menu.component_out",
                            format!("Take out of {}", own.name),
                        )
                        .hint("Put this body one level up"),
                    );
                }
                return items;
            }
            core_document::MenuScope::TreeComponent(component) => {
                let Some(component) = document.component(*component) else {
                    return Vec::new();
                };
                return vec![
                    if component.flexible {
                        MenuItem::new("asm.menu.component_rigid", "Make rigid")
                            .hint("Move it as one; the joints inside it rest")
                    } else {
                        MenuItem::new("asm.menu.component_flexible", "Make flexible")
                            .hint("Keep the joints inside it live in the assembly")
                    },
                    MenuItem::new("asm.menu.component_sub", "New component inside")
                        .icon("tree-group")
                        .hint("An empty component in this one, to move bodies into"),
                ];
            }
            _ => return Vec::new(),
        };
        let is_state = document
            .get_feature_meta(*id)
            .is_some_and(|n| n.workbench_id.as_str() == STATE_KIND);
        if !is_state {
            return Vec::new();
        }
        vec![
            core_document::MenuItem::new("asm.restore_state", "Restore this state")
                .icon("save")
                .hint("Put every body back where it was, drives and visibility too")
                .separator_before(),
            core_document::MenuItem::new("asm.update_state", "Save the assembly into it")
                .hint("Keep the assembly as it stands now under this state's name"),
        ]
    }

    fn on_command(
        &mut self,
        id: &str,
        scope: &core_document::MenuScope,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        let component_call = match scope {
            core_document::MenuScope::TreeBody(body) => {
                let bodies = serde_json::json!([body.0.to_string()]);
                let own = ctx.document.component_of(*body);
                match id {
                    "asm.menu.component_new" => Some((
                        "asm.component",
                        serde_json::json!({"bodies": bodies, "parent": own.map(|c| c.0.to_string())}),
                        "New component",
                    )),
                    "asm.menu.component_out" => Some((
                        "asm.component_add",
                        serde_json::json!({
                            "bodies": bodies,
                            "component": own
                                .and_then(|c| ctx.document.component(c))
                                .and_then(|c| c.parent)
                                .map(|c| c.0.to_string()),
                        }),
                        "Take out of component",
                    )),
                    _ => id.strip_prefix("asm.menu.component_to.").map(|to| {
                        (
                            "asm.component_add",
                            serde_json::json!({"bodies": bodies, "component": to}),
                            "Move to component",
                        )
                    }),
                }
            }
            core_document::MenuScope::TreeComponent(component) => {
                let c = component.0.to_string();
                match id {
                    "asm.menu.component_rigid" | "asm.menu.component_flexible" => Some((
                        "asm.component_set",
                        serde_json::json!({
                            "component": c,
                            "flexible": id == "asm.menu.component_flexible",
                        }),
                        if id == "asm.menu.component_rigid" {
                            "Make component rigid"
                        } else {
                            "Make component flexible"
                        },
                    )),
                    "asm.menu.component_sub" => Some((
                        "asm.component",
                        serde_json::json!({"bodies": [], "parent": c}),
                        "New component",
                    )),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((command, args, label)) = component_call {
            let args = commands::object(args);
            match commands::run(command, &args, ctx) {
                Ok(result) => {
                    ctx.record(command, args, result);
                    ctx.request(HostRequest::JournalLabel(label.into()));
                }
                Err(err) => ctx.log_warn(format!("{label}: {err}")),
            }
            return true;
        }
        let core_document::MenuScope::TreeFeature(state) = scope else {
            return false;
        };
        match id {
            "asm.restore_state" => {
                self.restore_state(ctx, *state);
                true
            }
            "asm.update_state" => {
                let now = states::capture(ctx.document);
                if ctx
                    .document
                    .update_feature_data(*state, now.to_json())
                    .is_ok()
                {
                    ctx.document.clear_feature_dirty(*state);
                    ctx.record(
                        "asm.save_state",
                        commands::object(serde_json::json!({"state": state.0.to_string()})),
                        serde_json::json!(state.0.to_string()),
                    );
                    ctx.request(HostRequest::JournalLabel("Save assembly state".into()));
                }
                true
            }
            _ => false,
        }
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
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        let tool = |id: &str, label: &str, icon: &'static str| {
            ToolDescriptor::new_action(id, label, Some("joints"))
                .icon(icon)
                .row(1)
        };
        for joint in JointTool::ALL {
            context.register_tool(
                tool(joint.command(), joint.label(), joint.icon()).shortcut(joint.shortcut()),
            );
        }
        context.register_tool(tool("asm.couple", "Couple joints", "involute-gear").shortcut("K"));
        context.register_tool(tool("asm.move", "Move body", "move-geometry").shortcut("G"));
        context.register_action(
            core_document::ActionDescriptor::new(
                handles::MODE_ACTION,
                "Move body: switch the handles between moving and turning",
            )
            .category("joints")
            .shortcut("Shift+G"),
        );
        context.register_tool(
            tool("asm.interference", "Check interference", "check-geometry").shortcut("I"),
        );
        context.register_tool(tool("asm.explode", "Exploded view", "scale-geometry").shortcut("E"));
        context.register_tool(
            tool("asm.collisions", "Stop drags at collisions", "boolean").shortcut("C"),
        );
        context.register_tool(tool("asm.parts", "Parts list", "file-document").shortcut("B"));
        context.register_tool(tool("asm.mass", "Mass and centre of mass", "measure").shortcut("W"));
        context.register_tool(tool("asm.group", "Rigid group", "tree-group").shortcut("U"));
        context.register_tool(tool("asm.copy", "Insert linked copies", "clone").shortcut("Y"));
        context
            .register_tool(tool("asm.replace", "Replace body", "carbon-copy").shortcut("Shift+Y"));
        context.register_tool(tool("asm.ground", "Ground body", "constraint-lock").shortcut("F"));
        context.register_tool(tool("asm.solve", "Solve joints", "refresh").shortcut("S"));
        context.register_tool(
            tool("asm.motion", "Motion over time", "polar-pattern").shortcut("Shift+M"),
        );
        context.register_tool(
            tool("asm.save_state", "Save assembly state", "save").shortcut("Shift+E"),
        );
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

    /// Whether the assembly is fully placed, or how many motions its
    /// joints leave open; the selected body's, in words.
    fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        let free = self.freedom_now(ctx);
        if free.is_empty() {
            return None;
        }
        let pal = ctx.sketch_palette;
        let open: usize = free.iter().map(|(_, m)| m.len()).sum();
        let extra = self.redundant_now(ctx).len();
        let extra = match extra {
            0 => String::new(),
            1 => ", 1 joint redundant".to_string(),
            n => format!(", {n} joints redundant"),
        };
        let state = if open == 0 {
            (pal.fully_constrained, format!("Fully placed{extra}"))
        } else {
            (
                pal.constraint,
                format!(
                    "{open} motion{} free{extra}",
                    if open == 1 { "" } else { "s" }
                ),
            )
        };
        let selection = Self::body_to_move(ctx).and_then(|body| {
            let (_, motions) = free.iter().find(|(b, _)| *b == body)?;
            let name = ctx
                .document
                .bodies()
                .iter()
                .find(|b| b.id == body)
                .map(|b| b.name.clone())
                .unwrap_or_default();
            Some(if motions.is_empty() {
                format!("{name}: fully placed")
            } else {
                let words: Vec<String> = motions.iter().map(Motion::describe).collect();
                format!("{name}: {}", words.join(", "))
            })
        });
        Some(core_document::StatusItems {
            state: Some(state),
            selection,
            coords: None,
            mode: None,
        })
    }

    /// A check or a measuring running away from the window, its answer
    /// shown once it comes, or a motion study or exploded view playing.
    fn busy(&self) -> bool {
        self.checking.is_some()
            || self.measuring.is_some()
            || self.sweeping.is_some()
            || matches!(&self.task, Some(Task::Explode { steps, .. }) if steps.playing)
            || matches!(&self.task, Some(Task::Motion(studying)) if studying.playing)
            || self.measuring_volumes()
    }

    fn tool_toggled(&self, tool_id: &str) -> bool {
        tool_id == "asm.collisions" && !self.collisions_off
    }

    fn is_tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        match tool_id {
            id if JointTool::of_command(id).is_some() => ctx.document.bodies().len() >= 2,
            "asm.interference" | "asm.explode" => ctx.document.bodies().len() >= 2,
            "asm.parts" | "asm.mass" => !ctx.document.bodies().is_empty(),
            "asm.group" => ctx.document.bodies().len() >= 2,
            "asm.save_state" => !ctx.document.bodies().is_empty(),
            "asm.motion" => joints(ctx.document).iter().any(|j| {
                matches!(
                    j.feature.kind,
                    JointKind::Hinge { .. } | JointKind::Slider { .. }
                )
            }),
            "asm.copy" => Self::body_to_move(ctx).is_some(),
            "asm.replace" => Self::body_to_move(ctx).is_some() && ctx.document.bodies().len() >= 2,
            "asm.collisions" => true,
            "asm.move" | "asm.ground" => Self::body_to_move(ctx).is_some(),
            "asm.solve" => !joints(ctx.document).is_empty(),
            "asm.couple" => {
                let all = joints(ctx.document);
                all.iter().any(|a| {
                    all.iter().any(|b| {
                        a.id != b.id && Gearing::suiting(&a.feature.kind, &b.feature.kind).is_some()
                    })
                })
            }
            _ => false,
        }
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if let Some(result) = self.handle_input(event, ctx) {
            return result;
        }
        match event {
            WorkbenchInputEvent::ToolActivated => {}
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos,
            } if tool.is_none() => {
                self.take_hold(ctx, *viewport_pos);
                return InputResult::ignored();
            }
            WorkbenchInputEvent::MouseMove { viewport_pos } => {
                return self.drag_to(ctx, *viewport_pos);
            }
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                ..
            } => return self.let_go(ctx),
            _ => return InputResult::ignored(),
        }
        match tool {
            Some(id) if JointTool::of_command(id).is_some() => {
                let kind = JointTool::of_command(id).unwrap_or(JointTool::Mate);
                self.task = None;
                self.seen = None;
                self.picking = Some(Picking::new(kind, None));
                // A face already selected is the first pick.
                self.take_pick(ctx);
            }
            Some("asm.interference") => {
                self.picking = None;
                // A selected body is checked against the rest.
                let around = ctx.selected_body_id.map(BodyId);
                self.check_interference(ctx, around);
            }
            Some("asm.couple") => self.make_coupling(ctx),
            Some("asm.explode") => {
                self.picking = None;
                let placements = all_placements(ctx);
                explode(ctx, &placements, 1.0);
                self.task = Some(Task::Explode {
                    placements,
                    spread: 1.0,
                    steps: Box::new(Stepping {
                        shift: [0.0, 0.0, 20.0],
                        ..Stepping::default()
                    }),
                });
            }
            Some("asm.collisions") => {
                self.collisions_off = !self.collisions_off;
                ctx.log_info(if self.collisions_off {
                    "Drags go through other bodies"
                } else {
                    "Drags stop where bodies would collide"
                });
            }
            Some("asm.parts") => {
                self.picking = None;
                self.task = Some(Task::Parts);
            }
            Some("asm.copy") => match Self::body_to_move(ctx) {
                Some(body) => {
                    self.picking = None;
                    self.task = Some(Task::Copies {
                        body,
                        count: 1,
                        step: commands::copy_step(ctx.document, body).to_array(),
                        around: None,
                        mirror: None,
                    });
                }
                None => ctx.log_warn("Select a body to copy"),
            },
            Some("asm.motion") => {
                self.picking = None;
                self.task = Some(Task::Motion(Box::new(Studying {
                    study: None,
                    draft: MotionStudy::default(),
                    frames: None,
                    frame: 0,
                    playing: false,
                    clock: 0.0,
                    placements: all_placements(ctx),
                    traces: Vec::new(),
                    tracing: false,
                })));
            }
            Some("asm.save_state") => {
                let name = commands::next_name(ctx.document, "State");
                match states::save(ctx.document, name.clone()) {
                    Ok(id) => {
                        ctx.record(
                            "asm.save_state",
                            commands::object(serde_json::json!({"name": name})),
                            serde_json::json!(id.0.to_string()),
                        );
                        ctx.log_info(format!(
                            "Saved {name}: double-click it in the tree to return to it"
                        ));
                        ctx.request(HostRequest::JournalLabel("Save assembly state".into()));
                    }
                    Err(why) => ctx.log_warn(format!("Could not save the state: {why}")),
                }
            }
            Some("asm.replace") => match Self::body_to_move(ctx) {
                Some(old) => {
                    self.picking = None;
                    self.group_seen = ctx.selected_body_id;
                    self.task = Some(Task::Replace { old, new: None });
                }
                None => ctx.log_warn("Select the body to replace"),
            },
            Some("asm.group") => {
                self.picking = None;
                self.group_seen = ctx.selected_body_id;
                self.task = Some(Task::Group {
                    editing: None,
                    members: ctx.selected_body_id.map(BodyId).into_iter().collect(),
                });
            }
            Some("asm.mass") => {
                self.picking = None;
                self.measure_mass(ctx, commands::DEFAULT_DENSITY);
            }
            Some("asm.move") => match Self::body_to_move(ctx) {
                Some(body) => {
                    self.picking = None;
                    self.task = Some(Task::Move {
                        body,
                        placements: all_placements(ctx),
                    });
                }
                None => ctx.log_warn("Select a body to move"),
            },
            Some("asm.ground") => match Self::body_to_move(ctx) {
                Some(body) => {
                    let grounded = joints(ctx.document)
                        .iter()
                        .any(|j| j.body == body && j.feature.kind == JointKind::Ground);
                    let made = commands::set_grounded(ctx, body, !grounded);
                    ctx.record(
                        "asm.ground",
                        commands::object(serde_json::json!({
                            "body": body.0.to_string(),
                            "grounded": !grounded,
                        })),
                        made.map_or(serde_json::Value::Null, |id| {
                            serde_json::json!(id.0.to_string())
                        }),
                    );
                    self.solve_and_apply(ctx);
                    ctx.request(HostRequest::JournalLabel(
                        if grounded {
                            "Unground body"
                        } else {
                            "Ground body"
                        }
                        .into(),
                    ));
                }
                None => ctx.log_warn("Select a body to ground"),
            },
            Some("asm.solve") => {
                self.solve_and_apply(ctx);
                if let Some(Ok(message)) = &self.verdict {
                    ctx.log_info(message.clone());
                    // A solve that left joints apart would stop a replay.
                    ctx.record(
                        "asm.solve",
                        core_document::CommandArgs::new(),
                        serde_json::json!(message),
                    );
                }
                ctx.request(HostRequest::JournalLabel("Solve joints".into()));
            }
            _ => return InputResult::ignored(),
        }
        InputResult::consumed()
    }

    fn cancel_pointer_gesture(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if let Some(body) = self.handled_body(ctx) {
            self.handles.cancel(ctx, body);
        }
        self.handles.reset();
        if let Some(grab) = self.grab.take() {
            restore_placements(ctx, &grab.placements);
        }
    }

    fn on_frame(&mut self, dt: f32, ctx: &mut WorkbenchRuntimeContext) {
        self.collect_interference(ctx);
        self.collect_mass(ctx);
        self.collect_sweep(ctx);
        let mut advanced = false;
        if let Some(Task::Motion(studying)) = &mut self.task
            && studying.playing
            && let Some(count) = studying.frames.as_ref().map(Vec::len).filter(|n| *n > 0)
        {
            // Frames at the study's own pace, round again at the end.
            studying.clock += dt.min(0.1);
            let step = studying.draft.step.max(1e-3);
            while studying.clock >= step {
                studying.clock -= step;
                studying.frame = (studying.frame + 1) % count;
                advanced = true;
            }
        }
        if advanced {
            self.show_frame(ctx);
        }
        if let Some(Task::Explode { steps, .. }) = &mut self.task
            && steps.playing
            && let Some(count) = steps
                .view
                .and_then(|id| exploded::view_of(ctx.document, id))
                .map(|v| v.steps.len() as f32)
                .filter(|n| *n > 0.0)
        {
            // A step a second, round again once through.
            steps.at += dt.min(0.1);
            if steps.at > count {
                steps.at = 0.0;
            }
            self.show_steps(ctx);
        }
        if self.picking.is_some() {
            self.take_pick(ctx);
            return;
        }
        self.take_group_pick(ctx);
        // A joint's settings stay open while it is selected; picking
        // something else closes them, keeping what was set. They open on a
        // double click (`edit_feature`), never on selection alone.
        if let Some(Task::Joint { id, .. }) = &self.task
            && Self::selected_joint(ctx) != Some(*id)
        {
            self.task = None;
        }
        if let Some(Task::Coupling { id, .. }) = &self.task
            && Self::selected_of(ctx, COUPLING_KIND) != Some(*id)
        {
            self.task = None;
        }
    }

    fn edit_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        let kind = ctx
            .document
            .get_feature_meta(id)
            .map(|n| n.workbench_id.as_str().to_string());
        if self.picking.is_some() {
            return;
        }
        if kind.as_deref() == Some(parts::PARTS_KIND) {
            self.task = Some(Task::Parts);
            return;
        }
        if kind.as_deref() == Some(STATE_KIND) {
            self.restore_state(ctx, id);
            return;
        }
        if kind.as_deref() == Some(MOTION_KIND) {
            let draft = ctx
                .document
                .get_feature_data(id)
                .and_then(|d| MotionStudy::from_json(d).ok())
                .unwrap_or_default();
            self.put_back_motion(ctx);
            self.task = Some(Task::Motion(Box::new(Studying {
                study: Some(id),
                draft,
                frames: None,
                frame: 0,
                playing: false,
                clock: 0.0,
                placements: all_placements(ctx),
                traces: Vec::new(),
                tracing: false,
            })));
            return;
        }
        if kind.as_deref() == Some(EXPLODED_KIND) {
            self.put_back_explosion(ctx);
            let placements = all_placements(ctx);
            let at = exploded::view_of(ctx.document, id).map_or(0.0, |v| v.steps.len() as f32);
            self.task = Some(Task::Explode {
                placements,
                spread: 0.0,
                steps: Box::new(Stepping {
                    view: Some(id),
                    at,
                    shift: [0.0, 0.0, 20.0],
                    ..Stepping::default()
                }),
            });
            self.show_steps(ctx);
            return;
        }
        if kind.as_deref() == Some(GROUP_KIND) {
            let members = ctx
                .document
                .get_feature_data(id)
                .and_then(|d| RigidGroup::from_json(d).ok())
                .map(|g| g.bodies())
                .unwrap_or_default();
            self.group_seen = ctx.selected_body_id;
            self.task = Some(Task::Group {
                editing: Some(id),
                members,
            });
            return;
        }
        if kind.as_deref() == Some(COUPLING_KIND) {
            self.task = Some(Task::Coupling {
                id,
                before: ctx.document.get_feature_data(id).cloned(),
                placements: all_placements(ctx),
            });
            self.verdict = None;
            return;
        }
        if kind.as_deref() != Some(JOINT_KIND) {
            return;
        }
        self.task = Some(Task::Joint {
            id,
            before: ctx.document.get_feature_data(id).cloned(),
            placements: all_placements(ctx),
        });
        self.verdict = None;
    }

    fn task(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        if let Some(picking) = &self.picking {
            return Some(core_document::TaskInfo {
                title: picking.kind.label().to_string(),
                icon: picking.kind.icon(),
                confirmable: false,
                stepwise: false,
            });
        }
        match self.task.as_ref()? {
            Task::Joint { id, .. } => {
                let kind = ctx
                    .document
                    .get_feature_data(*id)
                    .and_then(|d| JointFeature::from_json(d).ok())
                    .map(|j| j.kind)?;
                Some(core_document::TaskInfo {
                    title: kind.label().to_string(),
                    icon: kind.icon(),
                    confirmable: true,
                    stepwise: false,
                })
            }
            Task::Coupling { id, .. } => Some(core_document::TaskInfo {
                title: ctx
                    .document
                    .get_feature_data(*id)
                    .and_then(|d| Coupling::from_json(d).ok())
                    .map_or("Coupling", |c| c.gearing.label())
                    .to_string(),
                icon: "involute-gear",
                confirmable: true,
                stepwise: false,
            }),
            Task::Move { .. } => Some(core_document::TaskInfo {
                title: "Move body".to_string(),
                icon: "move-geometry",
                confirmable: true,
                stepwise: false,
            }),
            Task::Interference { .. } => Some(core_document::TaskInfo {
                title: "Interference".to_string(),
                icon: "check-geometry",
                confirmable: false,
                stepwise: false,
            }),
            Task::Explode { .. } => Some(core_document::TaskInfo {
                title: "Exploded view".to_string(),
                icon: "scale-geometry",
                confirmable: false,
                stepwise: false,
            }),
            Task::Parts => Some(core_document::TaskInfo {
                title: "Parts list".to_string(),
                icon: "file-document",
                confirmable: false,
                stepwise: false,
            }),
            Task::Replace { .. } => Some(core_document::TaskInfo {
                title: "Replace body".to_string(),
                icon: "carbon-copy",
                confirmable: true,
                stepwise: false,
            }),
            Task::Copies { .. } => Some(core_document::TaskInfo {
                title: "Insert linked copies".to_string(),
                icon: "clone",
                confirmable: true,
                stepwise: false,
            }),
            Task::Group { .. } => Some(core_document::TaskInfo {
                title: "Rigid group".to_string(),
                icon: "tree-group",
                confirmable: true,
                stepwise: false,
            }),
            Task::Motion(_) => Some(core_document::TaskInfo {
                title: "Motion over time".to_string(),
                icon: "polar-pattern",
                confirmable: false,
                stepwise: false,
            }),
            Task::Mass { .. } => Some(core_document::TaskInfo {
                title: "Mass".to_string(),
                icon: "measure",
                confirmable: false,
                stepwise: false,
            }),
        }
    }

    /// The material each clash shares, drawn over everything.
    fn get_overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::OverlayMesh> {
        let Some(Task::Interference {
            found: Some(found), ..
        }) = &self.task
        else {
            return Vec::new();
        };
        found
            .clashes
            .iter()
            .map(|clash| {
                core_document::OverlayMesh::on_top(
                    (*clash.mesh).clone(),
                    ctx.sketch_palette.conflict,
                    0.6,
                )
            })
            .collect()
    }

    /// Each pair nearer than the clearance, joined between its nearest
    /// points; the joint drawn, the move handles, the motion's traced
    /// points and the explode lines.
    fn get_screen_space_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let mut lines: Vec<core_document::ScreenSpaceOverlay> = self
            .near_on_screen(ctx)
            .into_iter()
            .map(|(a, b, _)| {
                core_document::ScreenSpaceOverlay::new(a, b, ctx.sketch_palette.conflict, 2.0)
            })
            .collect();
        if let Some((joint, _, _)) = self.joint_drawing(ctx) {
            lines.extend(joint);
        }
        if let Some(body) = self.handled_body(ctx) {
            lines.extend(self.handles.drawing(ctx, body).overlays);
        }
        if let Some(Task::Motion(studying)) = &self.task
            && let Some(frames) = &studying.frames
        {
            for (body, point) in &studying.traces {
                let path = motion::trace(frames, *body, *point);
                let px: Vec<Option<(f32, f32)>> = path
                    .iter()
                    .map(|(_, p, _)| ctx.world_to_viewport(*p))
                    .collect();
                for w in px.windows(2) {
                    if let (Some(a), Some(b)) = (w[0], w[1]) {
                        lines.push(core_document::ScreenSpaceOverlay::new(
                            [a.0, a.1],
                            [b.0, b.1],
                            ctx.sketch_palette.selected,
                            1.5,
                        ));
                    }
                }
            }
        }
        for (from, to) in self.explode_lines(ctx) {
            if let (Some(a), Some(b)) = (ctx.world_to_viewport(from), ctx.world_to_viewport(to)) {
                lines.push(
                    core_document::ScreenSpaceOverlay::new(
                        [a.0, a.1],
                        [b.0, b.1],
                        ctx.sketch_palette.preview,
                        1.0,
                    )
                    .dashed(5.0, 4.0),
                );
            }
        }
        lines
    }

    /// Each clash found, marked where it is; the joint drawn's ends, the
    /// centre of mass and the move handles' dots.
    fn get_screen_space_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceMark> {
        let mut marks: Vec<core_document::ScreenSpaceMark> = self
            .clashes_on_screen(ctx)
            .map(|(pos, _)| {
                core_document::ScreenSpaceMark::icon(
                    pos,
                    "warning",
                    18.0,
                    ctx.sketch_palette.conflict,
                )
            })
            .collect();
        if let Some((_, dots, _)) = self.joint_drawing(ctx) {
            marks.extend(
                dots.into_iter().map(|p| {
                    core_document::ScreenSpaceMark::dot(p, 4.0, ctx.sketch_palette.selected)
                }),
            );
        }
        if let Some(pos) = self.centre_of_mass_on_screen(ctx) {
            marks.push(core_document::ScreenSpaceMark::crosshair(
                pos,
                14.0,
                ctx.sketch_palette.selected,
            ));
            marks.push(core_document::ScreenSpaceMark::dot(
                pos,
                3.0,
                ctx.sketch_palette.selected,
            ));
        }
        if let Some(body) = self.handled_body(ctx) {
            marks.extend(self.handles.drawing(ctx, body).marks);
        }
        marks
    }

    /// How much each clash shares, beside its mark; the centre of mass,
    /// each near pair's distance, the joint drawn's name and the move
    /// handles' letters and value.
    fn get_screen_space_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let com = self.centre_of_mass_on_screen(ctx).map(|[x, y]| {
            core_document::ScreenSpaceLabel::new(
                [x + 14.0, y - 14.0],
                "Centre of mass".to_string(),
                ctx.sketch_palette.selected,
                11.0,
            )
            .pill()
        });
        let near: Vec<core_document::ScreenSpaceLabel> = self
            .near_on_screen(ctx)
            .into_iter()
            .map(|(a, b, d)| {
                core_document::ScreenSpaceLabel::new(
                    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0 - 14.0],
                    format!("{d:.2} mm"),
                    ctx.sketch_palette.conflict,
                    12.0,
                )
                .pill()
                .mono()
            })
            .collect();
        self.clashes_on_screen(ctx)
            .map(|([x, y], clash)| {
                core_document::ScreenSpaceLabel::new(
                    [x, y + 20.0],
                    format!("{:.1} mm³", clash.volume_mm3),
                    ctx.sketch_palette.conflict,
                    12.0,
                )
                .pill()
                .mono()
            })
            .chain(com)
            .chain(near)
            .chain(self.joint_drawing(ctx).and_then(|(_, dots, name)| {
                let [a, b] = dots.as_slice().try_into().ok()?;
                let [a, b]: [[f32; 2]; 2] = [a, b];
                Some(
                    core_document::ScreenSpaceLabel::new(
                        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0 - 14.0],
                        name,
                        ctx.sketch_palette.selected,
                        11.0,
                    )
                    .pill(),
                )
            }))
            .chain(
                self.handled_body(ctx)
                    .into_iter()
                    .flat_map(|body| self.handles.drawing(ctx, body).labels),
            )
            .collect()
    }

    /// The move handles' cones, squares and swept angle.
    fn get_screen_space_polygons(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpacePolygon> {
        self.handled_body(ctx)
            .map(|body| self.handles.drawing(ctx, body).polygons)
            .unwrap_or_default()
    }

    #[cfg(feature = "egui")]
    fn ui_task_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        self.draw_task_panel(ui, ctx, request)
    }

    fn viewport_hud(&self, _ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        let picking = self.picking.as_ref()?;
        Some(ViewportHud {
            tool: Some(ToolHint {
                icon: picking.kind.icon(),
                name: picking.kind.label().to_string(),
                prompt: picking.prompt().to_string(),
                keys: vec![("Esc".to_string(), "cancel")],
            }),
            ..ViewportHud::default()
        })
    }

    fn finish_editing(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.cancel_pointer_gesture(ctx);
        self.put_back_motion(ctx);
        self.picking = None;
        self.checking = None;
        self.measuring = None;
        self.put_back_explosion(ctx);
        self.task = None;
    }

    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.cancel_pointer_gesture(ctx);
        self.put_back_motion(ctx);
        self.picking = None;
        self.checking = None;
        self.measuring = None;
        self.put_back_explosion(ctx);
        self.task = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, FaceRef, ImportedGeometry, TriMesh};
    use kernel_api::FaceSurface;
    use serde_json::json;
    use std::sync::Arc;

    /// Two bodies, each a flat square facing up at its own height.
    pub(crate) fn scene() -> (Document, BodyId, BodyId) {
        let mut doc = Document::new("t");
        let base = doc.create_body(Some("Base".into()));
        let part = doc.create_body(Some("Part".into()));
        for body in [base, part] {
            doc.set_imported_geometry(
                body,
                ImportedGeometry {
                    mesh: Arc::new(TriMesh {
                        positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
                        normals: vec![[0.0, 0.0, 1.0]; 3],
                        indices: vec![0, 1, 2],
                        ..TriMesh::default()
                    }),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: None,
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
        }
        doc.set_body_placement(
            part,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(30.0, 0.0, 40.0)),
        );
        (doc, base, part)
    }

    pub(crate) fn face_up(z: f32) -> FaceRef {
        FaceRef {
            name: 0,
            point: [2.0, 2.0, z],
            normal: [0.0, 0.0, 1.0],
            surface: Some(FaceSurface::Plane {
                origin: [0.0, 0.0, z],
                normal: [0.0, 0.0, 1.0],
            }),
        }
    }

    pub(crate) fn frame(
        wb: &mut AssemblyWorkbench,
        doc: &mut Document,
        pick: Option<(BodyId, FaceRef)>,
    ) {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.selected_body_id = pick.map(|(b, _)| b.0);
        ctx.selected_face = pick.map(|(_, f)| f);
        wb.on_frame(0.016, &mut ctx);
    }

    /// A datum plane selected in the tree while a joint is picked is taken
    /// as its body's face; the origin's planes are offered as the second.
    #[test]
    fn a_datum_or_the_origin_is_the_other_end() {
        let (mut doc, base, part) = scene();
        let datum = doc
            .add_feature_in_body(
                core_document::DatumFeature {
                    shape: core_document::DatumShape::Plane { size: 10.0 },
                    attachment: core_document::datum::DatumAttachment::BasePlane(
                        core_document::BasePlane::XY,
                    ),
                    offset: core_document::datum::AttachmentOffset {
                        translation: [0.0, 0.0, 7.0],
                        ..Default::default()
                    },
                },
                "Datum".into(),
                Some(base),
            )
            .unwrap();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.mate"),
                &mut ctx,
            );
        }
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            ctx.active_document_object = Some(datum);
            wb.on_frame(0.016, &mut ctx);
        }
        assert!(wb.picking.is_none(), "the datum made the joint");
        let mate = joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .unwrap();
        assert_eq!(mate.feature.other_body, base);
        let placed = doc.body_placement(part);
        assert!((placed.point([2.0, 2.0, 0.0])[2] - 7.0).abs() < 1e-3);

        // The origin's XY plane as the second end.
        let (mut doc, _, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.mate"),
                &mut ctx,
            );
        }
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        let (_, xy) = ORIGIN[0];
        wb.picked(&mut ctx, WORLD, xy, None, 0);
        drop(ctx);
        let mate = joints(&doc).into_iter().next().unwrap();
        assert_eq!(mate.feature.other_body, WORLD);
        assert!((doc.body_placement(part).point([2.0, 2.0, 0.0])[2]).abs() < 1e-3);
    }

    /// A joint's faces picked again keep the joint: its name, and it is
    /// still the only mate; the part moves to the new face.
    #[test]
    fn a_joint_s_faces_are_picked_again_in_place() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.mate"),
                &mut ctx,
            );
        }
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
        let mate = joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .unwrap();
        wb.start_repick(mate.id, JointTool::Mate);
        let placed = doc.body_placement(part);
        // The part's face where the mate put it, onto a face 5 up.
        let top = placed.point([2.0, 2.0, 0.0]);
        frame(&mut wb, &mut doc, Some((part, face_up(top[2]))));
        frame(&mut wb, &mut doc, Some((base, face_up(5.0))));
        assert!(wb.picking.is_none());
        let all: Vec<_> = joints(&doc)
            .into_iter()
            .filter(|j| j.feature.kind != JointKind::Ground)
            .collect();
        assert_eq!(all.len(), 1, "the same joint");
        assert_eq!(all[0].id, mate.id);
        assert_eq!(all[0].name, "Mate 1");
        let Anchor::Plane { point, .. } = all[0].feature.fixed else {
            panic!()
        };
        assert!((point[2] - 5.0).abs() < 1e-4, "{point:?}");
        assert!(matches!(wb.task, Some(Task::Joint { id, .. }) if id == mate.id));
    }

    /// A joint shows under the body it holds to as well as its own; the
    /// ground it made does not.
    #[test]
    fn a_joint_is_linked_to_the_body_it_holds_to() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        let anchor = Anchor::Plane {
            point: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        };
        wb.make_joint(
            &mut ctx,
            part,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Mate {
                    flip: false,
                    offset: 0.0,
                },
                moving: anchor,
                other_body: base,
                fixed: anchor,
            },
        );
        drop(ctx);
        let mate = joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .unwrap();
        assert_eq!(wb.linked_features(&doc, base), [mate.id]);
        assert!(wb.linked_features(&doc, part).is_empty());
    }

    /// A selected joint draws its two ends, their directions and the link,
    /// and names itself between them; nothing is drawn with none selected.
    #[test]
    fn a_selected_joint_is_drawn_in_the_view() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        let eye = glam::Vec3::new(60.0, -80.0, 90.0);
        let view = glam::camera::rh::view::look_at_mat4(eye, glam::Vec3::ZERO, glam::Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective(0.8, 800.0 / 600.0, 0.1, 1000.0);
        ctx.view_proj = Some((proj * view).to_cols_array_2d());
        let anchor = Anchor::Plane {
            point: [2.0, 2.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        };
        wb.make_joint(
            &mut ctx,
            part,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Mate {
                    flip: false,
                    offset: 0.0,
                },
                moving: anchor,
                other_body: base,
                fixed: anchor,
            },
        );
        let (lines, dots, name) = wb.joint_drawing(&ctx).expect("the new joint is drawn");
        assert_eq!(dots.len(), 2);
        assert_eq!(
            lines.len(),
            11,
            "a normal and a square at each end, and the link"
        );
        assert_eq!(name, "Mate 1");
        wb.task = None;
        ctx.active_document_object = None;
        assert!(wb.joint_drawing(&ctx).is_none());
    }

    /// Picking fades every body but the one under the cursor and the one
    /// picked first; nothing fades otherwise.
    #[test]
    fn bodies_fade_while_a_joint_is_picked() {
        let (mut doc, base, part) = scene();
        let third = doc.create_body(None);
        let mut wb = AssemblyWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        assert!(wb.faded_bodies(&ctx).is_empty());
        wb.picking = Some(Picking {
            kind: JointTool::Mate,
            first: None,
            repick: None,
            tab: None,
            slot: None,
        });
        ctx.hovered_body_id = Some(base.0);
        let mut faded = wb.faded_bodies(&ctx);
        faded.sort();
        let mut want = vec![part, third];
        want.sort();
        assert_eq!(faded, want);
        wb.picking = Some(Picking {
            kind: JointTool::Mate,
            first: Some((
                part,
                Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
                None,
                0,
            )),
            repick: None,
            tab: None,
            slot: None,
        });
        assert_eq!(wb.faded_bodies(&ctx), vec![third]);
    }

    #[test]
    fn two_picks_make_a_mate_that_moves_the_first_body_onto_the_second() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.mate"),
                &mut ctx,
            );
        }
        assert!(wb.picking.is_some());
        // The part's own face, as the scene shows it (placed at z = 40).
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        // The same pick again is not a second pick.
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        assert!(wb.picking.as_ref().unwrap().first.is_some());
        frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
        assert!(wb.picking.is_none());
        let joints = joints(&doc);
        assert_eq!(joints.len(), 2, "the mate, and the base grounded");
        assert!(
            joints
                .iter()
                .any(|j| j.body == base && j.feature.kind == JointKind::Ground),
            "the first joint grounds the body it holds to"
        );
        let joints: Vec<_> = joints
            .into_iter()
            .filter(|j| j.feature.kind != JointKind::Ground)
            .collect();
        assert_eq!(joints[0].body, part);
        assert_eq!(joints[0].name, "Mate 1");
        // The part's face now lies on the base's, turned to face it.
        let placed = doc.body_placement(part);
        assert!((placed.point([-28.0, 2.0, 0.0])[2]).abs() < 1e-3);
        assert!((placed.direction([0.0, 0.0, 1.0])[2] + 1.0).abs() < 1e-4);
        assert!(matches!(wb.task, Some(Task::Joint { before: None, .. })));
    }

    /// One frame of the task panel, as the host runs it; what it recorded.
    pub(crate) fn task_frame(
        wb: &mut AssemblyWorkbench,
        doc: &mut Document,
        request: core_document::TaskRequest,
    ) -> Vec<core_document::Recorded> {
        let egui_ctx = egui::Context::default();
        ui_kit::apply_theme(&egui_ctx);
        let mut recorded = Vec::new();
        let mut output = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.ui_task_panel(ui, &mut ctx, request);
            recorded = core_document::HookOutcome::take(&mut ctx).recorded;
        });
        output.textures_delta.clear();
        recorded
    }

    #[test]
    fn a_mate_made_by_picks_records_as_the_command_and_replays_to_the_same_place() {
        let (mut doc, base, part) = scene();
        let before = doc.clone();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.mate"),
                &mut ctx,
            );
        }
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
        let recorded = task_frame(
            &mut wb,
            &mut doc,
            core_document::TaskRequest {
                accept: true,
                cancel: false,
            },
        );
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].id, "asm.mate");

        let mut replay = before;
        let mut ctx = WorkbenchRuntimeContext::new(&mut replay, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        AssemblyWorkbench::default()
            .run_command(&recorded[0].id, &recorded[0].args, &mut ctx)
            .unwrap();
        let (a, b) = (replay.body_placement(part), doc.body_placement(part));
        for i in 0..3 {
            assert!(
                (a.translation[i] - b.translation[i]).abs() < 1e-4,
                "{a:?} {b:?}"
            );
        }
        for i in 0..4 {
            assert!((a.rotation[i] - b.rotation[i]).abs() < 1e-4, "{a:?} {b:?}");
        }
    }

    #[test]
    fn the_couple_tool_ties_two_hinges_records_as_the_command_and_follows_a_ratio() {
        let (mut doc, base, part) = scene();
        let third = doc.create_body(Some("Third".into()));
        let run = |doc: &mut Document, id: &str, args: serde_json::Value| {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            AssemblyWorkbench::default()
                .run_command(id, &commands::object(args), &mut ctx)
                .unwrap()
        };
        run(&mut doc, "asm.ground", json!({"body": base.0.to_string()}));
        let axis = |x: f32| json!({"axis": {"point": [x, 0.0, 0.0], "direction": [0.0, 0.0, 1.0]}});
        let mut hinges = Vec::new();
        for (body, x) in [(part, 0.0), (third, 30.0)] {
            let placed = doc.body_placement(body).point([0.0; 3]);
            let id = run(
                &mut doc,
                "asm.hinge",
                json!({
                    "body": body.0.to_string(),
                    "face": {"axis": {"point": placed, "direction": [0.0, 0.0, 1.0]}},
                    "other": base.0.to_string(),
                    "other_face": axis(x),
                }),
            );
            hinges.push(FeatureId(
                uuid::Uuid::parse_str(id.as_str().unwrap()).unwrap(),
            ));
        }
        let before = doc.clone();

        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            assert!(wb.is_tool_enabled("asm.couple", &ctx));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.couple"),
                &mut ctx,
            );
        }
        let Some(Task::Coupling { id, .. }) = wb.task.clone() else {
            panic!("the tool opens the coupling's settings: {:?}", wb.task);
        };
        let coupling = Coupling::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        assert_eq!(coupling.gearing, Gearing::Gears, "gears suit two hinges");
        let recorded = task_frame(
            &mut wb,
            &mut doc,
            core_document::TaskRequest {
                accept: true,
                cancel: false,
            },
        );
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].id, "asm.couple");
        let mut replay = before;
        let made = run(
            &mut replay,
            &recorded[0].id,
            recorded[0].args.clone().into(),
        );
        let made = FeatureId(uuid::Uuid::parse_str(made.as_str().unwrap()).unwrap());
        let again = Coupling::from_json(replay.get_feature_data(made).unwrap()).unwrap();
        assert_eq!(
            (again.gearing, again.driver, again.driven, again.ratio),
            (
                coupling.gearing,
                coupling.driver,
                coupling.driven,
                coupling.ratio
            )
        );

        run(
            &mut doc,
            "asm.set",
            json!({"joint": id.0.to_string(), "ratio": 0.5}),
        );
        run(
            &mut doc,
            "asm.set",
            json!({"joint": coupling.driver.0.to_string(), "drive": 60.0}),
        );
        let travel = |doc: &Document, joint: FeatureId| {
            let j = joints(doc).into_iter().find(|j| j.id == joint).unwrap();
            let at = |b: BodyId| -> Rigid { doc.body_placement(b).into() };
            j.feature
                .travel(&at(j.body), &at(j.feature.other_body))
                .unwrap()
        };
        let driven = travel(&doc, coupling.driven);
        assert!(
            (driven + 30.0).abs() < 1e-2,
            "half the turn, the other way: {driven}"
        );
        assert!(hinges.contains(&coupling.driver) && hinges.contains(&coupling.driven));
    }

    #[test]
    fn a_body_dragged_by_the_mouse_turns_on_its_hinge_and_is_recorded() {
        use glam::{Mat4, Vec3};
        let mut doc = Document::new("t");
        let frame = doc.create_body(None);
        let door = doc.create_body(None);
        let pin = Anchor::Axis {
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        };
        doc.add_feature_in_body(
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointTool::Hinge.joint(
                    &pin,
                    &Rigid::from(BodyPlacement::default()),
                    &pin,
                    &Rigid::from(BodyPlacement::default()),
                    0.0,
                ),
                moving: pin,
                other_body: frame,
                fixed: pin,
            },
            "Hinge 1".into(),
            Some(door),
        )
        .unwrap();
        let proj = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            800.0 / 600.0,
            0.1,
            1000.0,
        );
        let view =
            glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 100.0), Vec3::ZERO, Vec3::Y);
        let vp = (Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d();
        let mut wb = AssemblyWorkbench::default();
        let mut send = |doc: &mut Document, event: WorkbenchInputEvent| {
            let mut ctx =
                WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
            ctx.view_proj = Some(vp);
            ctx.hovered_body_id = Some(door.0);
            ctx.hovered_world_pos = Some([10.0, 0.0, 0.0]);
            let result = wb.on_input(&event, None, &mut ctx);
            let outcome = core_document::HookOutcome::take(&mut ctx);
            (result, outcome.requests, outcome.recorded)
        };
        let screen = |p: [f32; 3]| {
            core_document::runtime::world_to_viewport(vp, (0, 0, 800, 600), p).unwrap()
        };
        let press = screen([10.0, 0.0, 0.0]);
        let (result, ..) = send(
            &mut doc,
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: press,
            },
        );
        assert!(!result.consumed, "the host still sees the press");
        for step in 1..=10 {
            let turn = step as f32 * 9f32.to_radians();
            let at = screen([10.0 * turn.cos(), 10.0 * turn.sin(), 0.0]);
            send(
                &mut doc,
                WorkbenchInputEvent::MouseMove { viewport_pos: at },
            );
        }
        let (_, requests, recorded) = send(
            &mut doc,
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                viewport_pos: screen([0.0, 10.0, 0.0]),
            },
        );
        let x = doc.body_placement(door).direction([1.0, 0.0, 0.0]);
        assert!(
            (x[1].atan2(x[0]).to_degrees() - 90.0).abs() < 2.0,
            "a quarter turn: {x:?}"
        );
        assert!(doc.body_placement(door).translation[2].abs() < 1e-3);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id, "asm.place");
        assert!(requests.contains(&HostRequest::JournalLabel("Drag body".into())));
    }

    /// Linked copies go in a row beside the body, count as the same part,
    /// and one with no joints is dragged straight across the view.
    #[test]
    fn copies_go_in_a_row_and_a_free_one_is_dragged() {
        use glam::{Mat4, Vec3};
        let (mut doc, base, _) = scene();
        doc.set_imported_brep_data(base, b"ogeom base".to_vec(), Vec::new());
        let made = {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            let args = serde_json::json!({"body": base.0.to_string(), "count": 2});
            commands::run("asm.copy", args.as_object().unwrap(), &mut ctx).unwrap()
        };
        let made: Vec<BodyId> = made
            .as_array()
            .unwrap()
            .iter()
            .map(|v| BodyId(uuid::Uuid::parse_str(v.as_str().unwrap()).unwrap()))
            .collect();
        assert_eq!(made.len(), 2);
        let x = |b: BodyId, doc: &Document| doc.body_placement(b).translation[0];
        assert!(
            (x(made[0], &doc) - 11.0).abs() < 1e-3,
            "a width and a tenth on"
        );
        assert!((x(made[1], &doc) - 22.0).abs() < 1e-3);
        let parts = parts_list(&doc, &[]);
        assert!(
            parts.iter().any(|p| p.bodies.len() == 3),
            "one part, three of it"
        );

        let copy = made[0];
        let proj = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            800.0 / 600.0,
            0.1,
            1000.0,
        );
        let view =
            glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 100.0), Vec3::ZERO, Vec3::Y);
        let vp = (Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d();
        let screen = |p: [f32; 3]| {
            core_document::runtime::world_to_viewport(vp, (0, 0, 800, 600), p).unwrap()
        };
        let mut wb = AssemblyWorkbench::default();
        let mut send = |doc: &mut Document, event: WorkbenchInputEvent| {
            let mut ctx =
                WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 100.0], [0.0; 3], (0, 0, 800, 600));
            ctx.view_proj = Some(vp);
            ctx.hovered_body_id = Some(copy.0);
            ctx.hovered_world_pos = Some([12.0, 2.0, 0.0]);
            wb.on_input(&event, None, &mut ctx);
            core_document::HookOutcome::take(&mut ctx).recorded
        };
        send(
            &mut doc,
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: screen([12.0, 2.0, 0.0]),
            },
        );
        send(
            &mut doc,
            WorkbenchInputEvent::MouseMove {
                viewport_pos: screen([32.0, 12.0, 0.0]),
            },
        );
        let recorded = send(
            &mut doc,
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                viewport_pos: screen([32.0, 12.0, 0.0]),
            },
        );
        let t = doc.body_placement(copy).translation;
        assert!(
            (t[0] - 31.0).abs() < 1e-2 && (t[1] - 10.0).abs() < 1e-2,
            "{t:?}"
        );
        assert_eq!(recorded.len(), 1);
    }

    /// A kernel for which every pair shares a little.
    struct AlwaysShared;

    impl kernel_api::KernelQueries for AlwaysShared {
        fn project_edge(
            &self,
            _: &[u8],
            _: [f64; 3],
            _: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("projection".into()))
        }

        fn overlap(
            &self,
            _: &[u8],
            _: &[u8],
            _: &[[f64; 4]; 4],
        ) -> kernel_api::KernelResult<Option<kernel_api::Overlap>> {
            Ok(Some(kernel_api::Overlap {
                volume_mm3: 2.0,
                centre_mm: [1.0, 1.0, 0.0],
                mesh: TriMesh {
                    positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    indices: vec![0, 1, 2],
                    ..TriMesh::default()
                },
            }))
        }
    }

    static ALWAYS_SHARED: AlwaysShared = AlwaysShared;

    /// A kernel whose every solid is 1000 mm³ about its own (1, 2, 3).
    struct Litre;

    impl kernel_api::KernelQueries for Litre {
        fn project_edge(
            &self,
            _: &[u8],
            _: [f64; 3],
            _: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("projection".into()))
        }

        fn measure(&self, _: &[u8]) -> kernel_api::KernelResult<kernel_api::PhysicalProperties> {
            Ok(kernel_api::PhysicalProperties {
                volume_mm3: Some(1000.0),
                area_mm2: 600.0,
                centre_mm: [1.0, 2.0, 3.0],
                approximate: false,
            })
        }
    }

    static LITRE: Litre = Litre;

    /// Two bodies weighed: the centre between their placed centres, the
    /// mass at the density asked; the tool shows the same away from the
    /// window.
    #[test]
    fn the_mass_tool_weighs_the_bodies_where_they_sit() {
        let (mut doc, base, part) = scene();
        for body in [base, part] {
            doc.set_imported_brep_data(body, b"shape".to_vec(), Vec::new());
        }
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.kernel = Some(&LITRE);
        let args = serde_json::json!({"density": 2.0});
        let got = commands::run("asm.mass", args.as_object().unwrap(), &mut ctx).unwrap();
        assert!((got["mass"].as_f64().unwrap() - 4.0).abs() < 1e-9, "{got}");
        // The part sits at (30, 0, 40): the centres are (1, 2, 3) and
        // (31, 2, 43).
        let centre: Vec<f64> = serde_json::from_value(got["centre"].clone()).unwrap();
        assert!((centre[0] - 16.0).abs() < 1e-4 && (centre[2] - 23.0).abs() < 1e-4);

        let mut wb = AssemblyWorkbench::default();
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("asm.mass"),
            &mut ctx,
        );
        assert!(wb.busy());
        let started = web_time::Instant::now();
        while wb.measuring.is_some() {
            assert!(started.elapsed().as_secs() < 5, "the measuring finishes");
            std::thread::sleep(std::time::Duration::from_millis(5));
            wb.on_frame(0.016, &mut ctx);
        }
        assert!(!wb.busy());
        let Some(Task::Mass {
            found: Some(report),
            ..
        }) = &wb.task
        else {
            panic!("the report is shown")
        };
        assert_eq!(report.bodies.len(), 2);
    }

    #[test]
    fn an_interference_check_runs_away_from_the_window_and_draws_what_is_shared() {
        let (mut doc, base, part) = scene();
        for body in [base, part] {
            doc.set_imported_brep_data(body, b"shape".to_vec(), Vec::new());
        }
        doc.set_body_placement(part, BodyPlacement::default());
        let mut wb = AssemblyWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.kernel = Some(&ALWAYS_SHARED);
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("asm.interference"),
            &mut ctx,
        );
        assert!(matches!(
            wb.task,
            Some(Task::Interference { found: None, .. })
        ));
        let started = web_time::Instant::now();
        while wb.checking.is_some() {
            assert!(started.elapsed().as_secs() < 5, "the check finishes");
            std::thread::sleep(std::time::Duration::from_millis(5));
            wb.on_frame(0.016, &mut ctx);
        }
        let Some(Task::Interference {
            found: Some(found), ..
        }) = &wb.task
        else {
            panic!("the answer is shown")
        };
        assert_eq!(found.clashes.len(), 1);
        let drawn = wb.get_overlay_meshes(&ctx, None);
        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].on_top && drawn[0].opacity < 1.0);
        wb.finish_editing(&mut ctx);
        assert!(wb.get_overlay_meshes(&ctx, None).is_empty());
    }

    /// With a body selected, the check asks only about the pairs it is in.
    #[test]
    fn a_selected_body_is_checked_against_the_others() {
        let (mut doc, base, part) = scene();
        let third = doc.create_body(Some("Third".into()));
        let geometry = doc.imported_geometry(base).cloned().unwrap();
        doc.set_imported_geometry(third, geometry);
        for body in [base, part, third] {
            doc.set_imported_brep_data(body, b"shape".to_vec(), Vec::new());
        }
        doc.set_body_placement(part, BodyPlacement::default());
        assert_eq!(interference::plan(&doc, None).pairs(), 3);
        assert_eq!(interference::plan(&doc, None).around(part).pairs(), 2);
        let mut wb = AssemblyWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.kernel = Some(&ALWAYS_SHARED);
        ctx.selected_body_id = Some(part.0);
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("asm.interference"),
            &mut ctx,
        );
        assert!(matches!(
            wb.task,
            Some(Task::Interference { around: Some(b), .. }) if b == part
        ));
        assert_eq!(wb.interference_progress().map(|(_, total)| total), Some(2));
    }

    /// A kernel for 10 mm cubes that are never turned: what two share is
    /// where their boxes overlap.
    struct Cubes;

    impl kernel_api::KernelQueries for Cubes {
        fn project_edge(
            &self,
            _: &[u8],
            _: [f64; 3],
            _: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            Err(kernel_api::KernelError::Unsupported("projection".into()))
        }

        fn overlap(
            &self,
            _: &[u8],
            _: &[u8],
            b_in_a: &[[f64; 4]; 4],
        ) -> kernel_api::KernelResult<Option<kernel_api::Overlap>> {
            let span = |k: usize| (10.0 - b_in_a[k][3].abs()).max(0.0);
            let volume = span(0) * span(1) * span(2);
            Ok((volume > 1e-6).then(|| kernel_api::Overlap {
                volume_mm3: volume,
                centre_mm: [0.0; 3],
                mesh: TriMesh::default(),
            }))
        }
    }

    static CUBES: Cubes = Cubes;

    /// A slider swept from where the cube stands (20 mm off) down to
    /// nothing runs into the other cube for the steps under 10 mm.
    #[test]
    fn a_motion_is_checked_for_collisions_step_by_step() {
        use glam::Vec3;
        let mut doc = Document::new("t");
        let cube = || TriMesh {
            positions: vec![[0.0; 3], [10.0, 0.0, 0.0], [0.0, 10.0, 10.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        };
        let [base, part] = [doc.create_body(None), doc.create_body(None)];
        for body in [base, part] {
            doc.set_imported_geometry(
                body,
                ImportedGeometry {
                    mesh: Arc::new(cube()),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: Some(([0.0; 3], [10.0; 3])),
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
            doc.set_imported_brep_data(body, b"cube".to_vec(), Vec::new());
        }
        doc.set_body_placement(
            part,
            BodyPlacement::new(glam::Quat::IDENTITY, Vec3::new(20.0, 0.0, 0.0)),
        );
        let rail = Anchor::Axis {
            point: [0.0, 5.0, 5.0],
            direction: [1.0, 0.0, 0.0],
        };
        let at = |b: BodyId| Rigid::from(doc.body_placement(b));
        let kind = JointTool::Slider.joint(&rail, &at(part), &rail, &at(base), 0.0);
        let slider = doc
            .add_feature_in_body(
                JointFeature {
                    second: None,
                    shape: Vec::new(),
                    ends: [0.0; 2],
                    names: [0; 2],
                    kind,
                    moving: rail,
                    other_body: base,
                    fixed: rail,
                },
                "Slider 1".into(),
                Some(part),
            )
            .unwrap();
        let check = sweep_check::plan(&doc, slider, 20.0, 0.0, 5).expect("the part moves");
        let found = check
            .run(
                &CUBES,
                &std::sync::atomic::AtomicUsize::new(0),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        let at: Vec<f32> = found.iter().map(|c| c.at).collect();
        assert_eq!(at, [5.0, 0.0], "{found:?}");
        assert!((found[0].volume_mm3 - 500.0).abs() < 1e-6);
        assert_eq!(
            doc.body_placement(part).translation[0],
            20.0,
            "the document is not moved"
        );

        let mut wb = AssemblyWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.kernel = Some(&CUBES);
        wb.check_sweep(&mut ctx, slider, (20.0, 0.0));
        let started = web_time::Instant::now();
        while wb.sweeping.is_some() {
            assert!(started.elapsed().as_secs() < 5);
            std::thread::sleep(std::time::Duration::from_millis(5));
            wb.on_frame(0.016, &mut ctx);
        }
        let Some((joint, Ok(found))) = &wb.motion_clashes else {
            panic!("the answer is kept")
        };
        assert_eq!(*joint, slider);
        assert!(!found.is_empty());
    }

    /// Two 10 mm cubes, the second on a slider along X, 20 mm along; the
    /// second dragged from its middle to `to`, and where it ends up.
    fn slide_cube_to(to: [f32; 3], collisions_off: bool) -> f32 {
        use glam::{Mat4, Vec3};
        let mut doc = Document::new("t");
        let cube = || TriMesh {
            positions: vec![[0.0; 3], [10.0, 0.0, 0.0], [0.0, 10.0, 10.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..TriMesh::default()
        };
        let [base, part] = [doc.create_body(None), doc.create_body(None)];
        for body in [base, part] {
            doc.set_imported_geometry(
                body,
                ImportedGeometry {
                    mesh: Arc::new(cube()),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: Some(([0.0; 3], [10.0; 3])),
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
            doc.set_imported_brep_data(body, b"cube".to_vec(), Vec::new());
        }
        doc.set_body_placement(
            part,
            BodyPlacement::new(glam::Quat::IDENTITY, Vec3::new(20.0, 0.0, 0.0)),
        );
        let rail = Anchor::Axis {
            point: [0.0, 5.0, 5.0],
            direction: [1.0, 0.0, 0.0],
        };
        let at = |b: BodyId| Rigid::from(doc.body_placement(b));
        let kind = JointTool::Slider.joint(&rail, &at(part), &rail, &at(base), 0.0);
        doc.add_feature_in_body(
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind,
                moving: rail,
                other_body: base,
                fixed: rail,
            },
            "Slider 1".into(),
            Some(part),
        )
        .unwrap();
        let proj = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            800.0 / 600.0,
            0.1,
            1000.0,
        );
        let view = glam::camera::rh::view::look_at_mat4(
            Vec3::new(10.0, 5.0, 100.0),
            Vec3::new(10.0, 5.0, 0.0),
            Vec3::Y,
        );
        let vp = (Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d();
        let screen = |p: [f32; 3]| {
            core_document::runtime::world_to_viewport(vp, (0, 0, 800, 600), p).unwrap()
        };
        let mut wb = AssemblyWorkbench {
            collisions_off,
            ..AssemblyWorkbench::default()
        };
        let grab = [25.0, 5.0, 10.0];
        let mut send = |doc: &mut Document, event: WorkbenchInputEvent| {
            let mut ctx = WorkbenchRuntimeContext::new(
                doc,
                [10.0, 5.0, 100.0],
                [10.0, 5.0, 0.0],
                (0, 0, 800, 600),
            );
            ctx.view_proj = Some(vp);
            ctx.kernel = Some(&CUBES);
            ctx.hovered_body_id = Some(part.0);
            ctx.hovered_world_pos = Some(grab);
            wb.on_input(&event, None, &mut ctx);
        };
        send(
            &mut doc,
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                viewport_pos: screen(grab),
            },
        );
        for step in 1..=8 {
            let t = step as f32 / 8.0;
            let p = glam::Vec3::from_array(grab).lerp(glam::Vec3::from_array(to), t);
            send(
                &mut doc,
                WorkbenchInputEvent::MouseMove {
                    viewport_pos: screen(p.to_array()),
                },
            );
        }
        send(
            &mut doc,
            WorkbenchInputEvent::MouseRelease {
                button: core_document::MouseButton::Left,
                viewport_pos: screen(to),
            },
        );
        doc.body_placement(part).translation[0]
    }

    #[test]
    fn a_dragged_body_stops_where_it_would_collide() {
        let stopped = slide_cube_to([-5.0, 5.0, 10.0], false);
        assert!(
            (10.0..10.5).contains(&stopped),
            "against the other cube's face: {stopped}"
        );
        let through = slide_cube_to([-5.0, 5.0, 10.0], true);
        assert!(
            through < 0.0,
            "with collisions off it goes through: {through}"
        );
        let clear = slide_cube_to([40.0, 5.0, 10.0], false);
        assert!(clear > 30.0, "away from it nothing stops it: {clear}");
    }

    #[test]
    fn a_sweep_is_recorded_frame_by_frame_without_touching_the_document() {
        let mut doc = Document::new("t");
        let frame = doc.create_body(None);
        let door = doc.create_body(None);
        let pin = Anchor::Axis {
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        };
        let at = Rigid::from(BodyPlacement::default());
        let hinge = doc
            .add_feature_in_body(
                JointFeature {
                    second: None,
                    shape: Vec::new(),
                    ends: [0.0; 2],
                    names: [0; 2],
                    kind: JointTool::Hinge.joint(&pin, &at, &pin, &at, 0.0),
                    moving: pin,
                    other_body: frame,
                    fixed: pin,
                },
                "Hinge 1".into(),
                Some(door),
            )
            .unwrap();
        let seq = doc.mutation_seq();
        let frames = sweep_frames(&doc, hinge, -90.0, 90.0, 8);
        assert_eq!(frames.len(), 8);
        let angle = |frame: &Vec<(BodyId, BodyPlacement)>| {
            let (_, placed) = frame.iter().find(|(b, _)| *b == door).unwrap();
            let x = placed.direction([1.0, 0.0, 0.0]);
            x[1].atan2(x[0]).to_degrees()
        };
        assert!(
            (angle(&frames[0]) + 90.0).abs() < 0.1,
            "{}",
            angle(&frames[0])
        );
        assert!(
            (angle(&frames[4]) - 90.0).abs() < 0.1,
            "{}",
            angle(&frames[4])
        );
        assert_eq!(doc.mutation_seq(), seq, "the document is left alone");
        assert!(doc.body_placement(door).is_identity());
    }

    #[test]
    fn an_exploded_view_spreads_the_bodies_and_puts_them_back() {
        let (mut doc, base, part) = scene();
        let before = [base, part].map(|b| doc.body_placement(b));
        let mut wb = AssemblyWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("asm.explode"),
            &mut ctx,
        );
        let apart = |ctx: &WorkbenchRuntimeContext| {
            let [a, b] = [base, part]
                .map(|body| glam::Vec3::from_array(ctx.document.body_placement(body).translation));
            a.distance(b)
        };
        let was = glam::Vec3::from_array(before[0].translation)
            .distance(glam::Vec3::from_array(before[1].translation));
        assert!((apart(&ctx) - 2.0 * was).abs() < 1e-3, "twice as far apart");
        wb.finish_editing(&mut ctx);
        assert_eq!([base, part].map(|b| ctx.document.body_placement(b)), before);
    }

    #[test]
    fn a_hole_s_rim_is_an_axis_for_a_hinge() {
        let (mut doc, base, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        let rim = |body: BodyId, center: [f32; 3]| core_document::EdgeRef {
            faces: [0, 0],
            point: [center[0] + 3.0, center[1], center[2]],
            direction: [0.0, 1.0, 0.0],
            length_mm: 18.85,
            body: body.0,
            circle: Some(core_document::EdgeCircle {
                center,
                normal: [0.0, 0.0, 1.0],
                radius: 3.0,
            }),
        };
        let edge_frame = |wb: &mut AssemblyWorkbench, doc: &mut Document, edge| {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            ctx.selected_edges = vec![edge];
            wb.on_frame(0.016, &mut ctx);
        };
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.hinge"),
                &mut ctx,
            );
        }
        // The part's rim where the scene shows it (placed at x + 30, z + 40).
        edge_frame(&mut wb, &mut doc, rim(part, [35.0, 5.0, 40.0]));
        edge_frame(&mut wb, &mut doc, rim(base, [5.0, 5.0, 0.0]));
        assert!(wb.picking.is_none(), "two rims make the hinge");
        let joint = joints(&doc)
            .into_iter()
            .find(|j| j.feature.kind != JointKind::Ground)
            .expect("a hinge");
        assert!(matches!(joint.feature.kind, JointKind::Hinge { .. }));
        let origin = doc.body_placement(part).point([0.0; 3]);
        assert!(
            origin[0].abs() < 1e-3 && origin[1].abs() < 1e-3 && origin[2].abs() < 1e-3,
            "the rims on one axis at one height: {origin:?}"
        );
    }

    #[test]
    fn a_round_face_is_asked_for_where_an_alignment_needs_one() {
        let (mut doc, _, part) = scene();
        let mut wb = AssemblyWorkbench::default();
        {
            let mut ctx =
                WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
            wb.on_input(
                &WorkbenchInputEvent::ToolActivated,
                Some("asm.align"),
                &mut ctx,
            );
        }
        frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
        assert!(
            wb.picking.as_ref().unwrap().first.is_none(),
            "a flat face is no axis"
        );
    }
}

#[cfg(all(test, feature = "egui"))]
mod icon_coverage {
    use super::*;

    #[test]
    fn the_bench_its_tools_and_its_joints_name_icons_in_the_set() {
        let wb = AssemblyWorkbench::default();
        assert!(ui_kit::icon::exists(wb.descriptor().icon));
        let mut context = WorkbenchContext::default();
        wb.configure(&mut context);
        for tool in context.tools() {
            let icon = tool.icon.expect("every tool has an icon");
            assert!(ui_kit::icon::exists(icon), "{icon}");
        }
        for tool in JointTool::ALL {
            let kind = tool.joint(
                &Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
                &Rigid::from(BodyPlacement::default()),
                &Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
                &Rigid::from(BodyPlacement::default()),
                1.0,
            );
            assert!(ui_kit::icon::exists(kind.icon()), "{}", kind.icon());
            assert!(ui_kit::icon::exists(tool.icon()), "{}", tool.icon());
            assert_eq!(JointTool::of_kind(&kind), Some(tool));
        }
        assert!(ui_kit::icon::exists(JointKind::Ground.icon()));
    }
}
