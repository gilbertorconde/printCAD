//! Constraint tools taking edges and vertices of the model as operands.
//!
//! A constraint or dimension tool pressed while the selection does not
//! fit it yet, but could, stays armed: clicks on the sketch add elements,
//! and a click that lands on no element is the host's, which picks the
//! solid edge under it. Each picked edge is an operand: the edge itself,
//! or the end of it the click was at (a vertex). Once what is picked
//! fits the tool and holds two items (or Enter is pressed), the model
//! picks are projected into the sketch as guides, reusing what the sketch
//! already holds of them, and the constraint goes on, as one edit: one
//! undo step, recorded as the `sketch.external` calls and the
//! `sketch.constrain` that do the same.

use std::collections::HashSet;

use core_document::{BodyId, InputResult, WorkbenchInputEvent, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::boundary::widen3;
use crate::constrain::{self, SelectionShape};
use crate::sketch::{ExternalSource, GeometryElement, Sketch, SketchPlane, Vec2D};
use crate::{SketchFeature, SketchWorkbench, commands};

/// A constraint tool waiting for its operands.
#[derive(Debug, Clone)]
pub(crate) struct Armed {
    /// The tool's id as it was pressed, variant and all.
    pub tool: String,
    /// The constraint tool it is (`dimension` or one of
    /// [`constrain::TOOLS`]).
    pub which: String,
    /// The operands, in the order they were picked.
    order: Vec<Operand>,
    /// The model's edges and vertices picked, by [`Operand::Model`].
    picks: Vec<ModelPick>,
    /// The host's edge picks already looked at, by body and point.
    seen: HashSet<(Uuid, [u32; 3])>,
    /// The last left press landed on sketch geometry, so its release is
    /// the sketcher's too.
    press_hit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operand {
    Sketch(Uuid),
    Model(usize),
}

/// An edge of the model picked as an operand.
#[derive(Debug, Clone, Copy)]
struct ModelPick {
    /// The edge, in its body's frame, as a guide.
    source: ExternalSource,
    /// Where it was picked, in sketch coordinates.
    at: Vec2D,
    /// The operand is the end of the edge the pick was at, not the edge.
    vertex: bool,
}

/// One `sketch.external` call an edit made: the edge and what it added.
struct Projected {
    source: ExternalSource,
    made: serde_json::Value,
}

/// The kinds of element one more pick can add, for whether a selection
/// can still grow into one a tool takes.
#[derive(Clone, Copy)]
enum Kind {
    Point,
    Line,
    Circle,
    Arc,
    Ellipse,
}

const KINDS: [Kind; 5] = [
    Kind::Point,
    Kind::Line,
    Kind::Circle,
    Kind::Arc,
    Kind::Ellipse,
];

fn with_one_more(shape: &SelectionShape, kind: Kind) -> SelectionShape {
    let mut shape = shape.clone();
    let id = Uuid::new_v4();
    shape.all.push(id);
    shape.picked.push(id);
    match kind {
        Kind::Point => shape.points.push(id),
        Kind::Line => shape.lines.push(id),
        Kind::Circle => shape.circles.push(id),
        Kind::Arc => {
            shape.circles.push(id);
            shape.arcs.push(id);
        }
        Kind::Ellipse => shape.ellipses.push(id),
    }
    shape
}

/// Whether the tool `which` takes `shape` as it is.
fn fits(which: &str, shape: &SelectionShape) -> bool {
    if which == "dimension" {
        constrain::dimension_for(shape).is_some()
    } else {
        constrain::fits(which, shape)
    }
}

/// Whether one more pick of some kind makes `shape` fit `which`.
fn fits_with_one_more(which: &str, shape: &SelectionShape) -> bool {
    KINDS
        .iter()
        .any(|kind| fits(which, &with_one_more(shape, *kind)))
}

/// Whether `shape`, as it is or with one or two more picks, fits `which`.
pub(crate) fn completable(which: &str, shape: &SelectionShape) -> bool {
    fits(which, shape)
        || KINDS.iter().any(|kind| {
            let more = with_one_more(shape, *kind);
            fits(which, &more) || fits_with_one_more(which, &more)
        })
}

/// The ids an element stands on: a point itself, a curve's points.
fn points_of(sketch: &Sketch, id: Uuid) -> Vec<Uuid> {
    match sketch.get_geometry(id) {
        Some(GeometryElement::Point(p)) => vec![p.id],
        Some(other) => Sketch::curve_point_ids(other),
        None => Vec::new(),
    }
}

fn distance(sketch: &Sketch, point: Uuid, at: Vec2D) -> f64 {
    sketch
        .point_position(point)
        .map_or(f64::INFINITY, |p| (p.to_glam() - at.to_glam()).length())
}

/// How far `at` lies from element `id`: a line as its segment, anything
/// else by its nearest point.
fn distance_to_element(sketch: &Sketch, id: Uuid, at: Vec2D) -> f64 {
    if let Some(GeometryElement::Line(l)) = sketch.get_geometry(id)
        && let (Some(a), Some(b)) = (sketch.point_position(l.start), sketch.point_position(l.end))
    {
        let (a, b, p) = (a.to_glam(), b.to_glam(), at.to_glam());
        let along = b - a;
        let t = if along.length_squared() > 0.0 {
            ((p - a).dot(along) / along.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        return (a + along * t - p).length();
    }
    points_of(sketch, id)
        .into_iter()
        .map(|p| distance(sketch, p, at))
        .fold(f64::INFINITY, f64::min)
}

/// How close an external point must come to a projected vertex to be the
/// same one, mm.
const SAME_POINT_MM: f64 = 1e-3;

/// Bring `pick` into `sketch`: the elements the sketch already holds of
/// its edge, else its projection added as a guide. Returns the operand's
/// id, and the projection when one was added.
fn bring_in(
    ctx: &WorkbenchRuntimeContext,
    plane: &SketchPlane,
    sketch: &mut Sketch,
    pick: &ModelPick,
) -> Result<(Uuid, Option<Projected>), String> {
    let source = crate::with_edge_names(ctx.document, pick.source);
    let curves = crate::project_source(ctx, plane, &source)?;
    let held = crate::external::groups(sketch)
        .into_iter()
        .map(|(_, group)| group)
        .find(|group| crate::external::already_holds(sketch, group, &curves));
    // A vertex another projection already ends at is taken as it is.
    if pick.vertex && held.is_none() {
        let mut probe = sketch.clone();
        let probed = add_projection(&mut probe, &curves, source);
        if let Some(end) = nearest_point(&probe, &probed, pick.at)
            && let Some(at) = probe.point_position(end)
            && let Some(existing) = sketch
                .geometry
                .iter()
                .filter_map(|g| match g {
                    GeometryElement::Point(p) if sketch.is_external(p.id) => Some(p.id),
                    _ => None,
                })
                .find(|p| distance(sketch, *p, at) <= SAME_POINT_MM)
        {
            return Ok((existing, None));
        }
    }
    let (elements, projected) = match held {
        Some(group) => (group, None),
        None => {
            let before = commands::ids_of(sketch);
            let made = add_projection(sketch, &curves, source);
            let projected = Projected {
                source,
                made: commands::made_since(sketch, &before),
            };
            (made, Some(projected))
        }
    };
    let operand = if pick.vertex {
        nearest_point(sketch, &elements, pick.at)
    } else {
        elements.iter().copied().min_by(|a, b| {
            distance_to_element(sketch, *a, pick.at)
                .total_cmp(&distance_to_element(sketch, *b, pick.at))
        })
    };
    operand
        .map(|id| (id, projected))
        .ok_or_else(|| "the edge came to nothing in the sketch".to_string())
}

/// Add `curves` to `sketch` as external geometry from `source`; the
/// elements made.
fn add_projection(
    sketch: &mut Sketch,
    curves: &[kernel_api::ProjectedEdge],
    source: ExternalSource,
) -> Vec<Uuid> {
    let before: HashSet<Uuid> = sketch.external.keys().copied().collect();
    for curve in curves {
        crate::external::add(sketch, curve, source);
    }
    sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| sketch.external.contains_key(id) && !before.contains(id))
        .collect()
}

/// The point of `elements` nearest `at`.
fn nearest_point(sketch: &Sketch, elements: &[Uuid], at: Vec2D) -> Option<Uuid> {
    elements
        .iter()
        .flat_map(|id| points_of(sketch, *id))
        .min_by(|a, b| distance(sketch, *a, at).total_cmp(&distance(sketch, *b, at)))
}

/// What `armed` comes to in `sketch`: every model pick brought in, the
/// operands as sketch ids in the order picked, and the projections made.
fn materialize(
    ctx: &WorkbenchRuntimeContext,
    plane: &SketchPlane,
    sketch: &mut Sketch,
    order: &[Operand],
    picks: &[ModelPick],
) -> Result<(Vec<Uuid>, Vec<Projected>), String> {
    let mut ids = Vec::new();
    let mut projected = Vec::new();
    let mut brought: Vec<Option<Uuid>> = vec![None; picks.len()];
    for operand in order {
        let id = match *operand {
            Operand::Sketch(id) => id,
            Operand::Model(i) => match brought[i] {
                Some(id) => id,
                None => {
                    let (id, made) = bring_in(ctx, plane, sketch, &picks[i])?;
                    projected.extend(made);
                    brought[i] = Some(id);
                    id
                }
            },
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok((ids, projected))
}

fn shape_of(sketch: &Sketch, ids: &[Uuid]) -> SelectionShape {
    let set: HashSet<Uuid> = ids.iter().copied().collect();
    SelectionShape::picked_in(sketch, &set, ids)
}

impl SketchWorkbench {
    /// Arm the constraint tool `tool` (`which`) on the current selection,
    /// when picks could still make it fit. Whether it armed.
    pub(crate) fn arm_constraint(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        tool: &str,
        which: &str,
        shape: &SelectionShape,
    ) -> bool {
        if !completable(which, shape) {
            return false;
        }
        self.armed = Some(Armed {
            tool: tool.to_string(),
            which: which.to_string(),
            order: shape.picked.iter().copied().map(Operand::Sketch).collect(),
            picks: Vec::new(),
            seen: ctx
                .selected_edges
                .iter()
                .map(|e| (e.body, e.point.map(f32::to_bits)))
                .collect(),
            press_hit: false,
        });
        ctx.log_info(
            "Pick what to constrain: sketch elements, or edges and vertices of the model \
             (Enter applies, Esc cancels)",
        );
        true
    }

    /// Whether the armed constraint tool is `tool_id` (its base id).
    pub(crate) fn armed_as(&self, tool_id: &str) -> bool {
        self.armed
            .as_ref()
            .is_some_and(|a| core_document::base_tool_id(&a.tool) == tool_id)
    }

    /// The name the viewport card gives an armed tool.
    pub(crate) fn armed_hint(&self) -> Option<String> {
        self.armed.as_ref().map(|a| {
            let name = a.which.replace('_', " ");
            let mut name: Vec<char> = name.chars().collect();
            if let Some(first) = name.first_mut() {
                *first = first.to_ascii_uppercase();
            }
            name.into_iter().collect()
        })
    }

    /// An event while a constraint tool is armed; `None` leaves it to the
    /// sketcher's usual handling.
    pub(crate) fn armed_input(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        event: &WorkbenchInputEvent,
    ) -> Option<InputResult> {
        use core_document::{KeyCode, MouseButton};
        let armed = self.armed.as_mut()?;
        match event {
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            } => {
                let feature = self.get_active_sketch(ctx)?;
                let cursor = Self::cursor_to_sketch(ctx, &feature.plane, *viewport_pos)?;
                let tol = Self::snap_tolerance(ctx, &feature.plane);
                let Some(id) = crate::snap::hit_test(&feature.sketch, cursor, tol) else {
                    // Not the sketch's: the host picks the edge under it.
                    if let Some(armed) = self.armed.as_mut() {
                        armed.press_hit = false;
                    }
                    return Some(InputResult::ignored());
                };
                let armed = self.armed.as_mut()?;
                armed.press_hit = true;
                let operand = Operand::Sketch(id);
                if let Some(at) = armed.order.iter().position(|o| *o == operand) {
                    armed.order.remove(at);
                    self.selected.remove(&id);
                } else {
                    armed.order.push(operand);
                    self.selected.insert(id);
                }
                self.fire_armed(ctx, false);
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                ..
            } => Some(if armed.press_hit {
                InputResult::consumed()
            } else {
                InputResult::ignored()
            }),
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Right,
                ..
            }
            | WorkbenchInputEvent::KeyPress {
                key: KeyCode::Escape,
            } => {
                self.armed = None;
                ctx.log_info("Constraint cancelled");
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::KeyPress {
                key: KeyCode::Enter,
            } => {
                self.fire_armed(ctx, true);
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }

    /// Take the edges the host picked since the last frame as operands of
    /// the armed tool.
    pub(crate) fn take_armed_picks(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(armed) = self.armed.as_mut() else {
            return;
        };
        let fresh: Vec<core_document::EdgeRef> = ctx
            .selected_edges
            .iter()
            .filter(|e| armed.seen.insert((e.body, e.point.map(f32::to_bits))))
            .copied()
            .collect();
        if fresh.is_empty() {
            return;
        }
        let Some(feature) = self.get_active_sketch(ctx) else {
            return;
        };
        for edge in fresh {
            if self.add_model_pick(ctx, &feature, &edge) {
                self.fire_armed(ctx, false);
            }
            if self.armed.is_none() {
                return;
            }
        }
    }

    /// Add `edge` as an operand: its end when the pick was at one and the
    /// tool takes a point there, else the edge. Whether it was taken.
    fn add_model_pick(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &SketchFeature,
        edge: &core_document::EdgeRef,
    ) -> bool {
        let Some(armed) = self.armed.clone() else {
            return false;
        };
        let plane = feature.plane;
        let local = edge.moved(&ctx.document.body_placement(BodyId(edge.body)).inverse());
        let source = ExternalSource {
            body: edge.body,
            point: widen3(local.point),
            direction: widen3(local.direction),
            section: false,
            defining: false,
            reference: None,
            faces: [0; 2],
        };
        let world = glam::DVec3::from_array(widen3(edge.point));
        let rel = world - glam::DVec3::from_array(plane.origin);
        let at = Vec2D::new(
            rel.dot(glam::DVec3::from_array(plane.x_axis)),
            rel.dot(glam::DVec3::from_array(plane.y_axis)),
        );
        let edge_pick = ModelPick {
            source,
            at,
            vertex: false,
        };
        // Whether the pick was at an end of the edge, as the sketch sees it.
        let tol = Self::snap_tolerance(ctx, &plane);
        let mut probe = feature.sketch.clone();
        let near_end = match bring_in(ctx, &plane, &mut probe, &edge_pick) {
            Ok((curve, _)) => {
                let ends: Vec<Uuid> = match probe.get_geometry(curve) {
                    Some(GeometryElement::Point(p)) => vec![p.id],
                    Some(GeometryElement::Line(l)) => vec![l.start, l.end],
                    Some(GeometryElement::Arc(a)) => vec![a.start, a.end],
                    Some(GeometryElement::Ellipse(e)) => {
                        e.arc.map(|a| vec![a.start, a.end]).unwrap_or_default()
                    }
                    Some(GeometryElement::Conic(c)) => vec![c.start, c.end],
                    Some(GeometryElement::BSpline(b)) if !b.periodic => b
                        .control_points
                        .first()
                        .into_iter()
                        .chain(b.control_points.last())
                        .copied()
                        .collect(),
                    _ => Vec::new(),
                };
                ends.iter().any(|p| distance(&probe, *p, at) <= tol)
            }
            Err(why) => {
                ctx.log_warn(format!("Could not bring that edge in: {why}"));
                return false;
            }
        };
        let roles: &[bool] = if near_end { &[true, false] } else { &[false] };
        let index = armed.picks.len();
        for vertex in roles {
            let pick = ModelPick {
                vertex: *vertex,
                ..edge_pick
            };
            let mut picks = armed.picks.clone();
            picks.push(pick);
            let mut order = armed.order.clone();
            order.push(Operand::Model(index));
            let mut scratch = feature.sketch.clone();
            let Ok((ids, _)) = materialize(ctx, &plane, &mut scratch, &order, &picks) else {
                continue;
            };
            if completable(&armed.which, &shape_of(&scratch, &ids)) {
                if let Some(armed) = self.armed.as_mut() {
                    armed.picks = picks;
                    armed.order = order;
                }
                return true;
            }
        }
        ctx.log_warn(format!(
            "That edge does not fit the {} constraint with what is picked",
            armed.which.replace('_', " ")
        ));
        false
    }

    /// Apply the armed tool once what is picked fits it and holds two
    /// items, or, `now`, whatever fits.
    fn fire_armed(&mut self, ctx: &mut WorkbenchRuntimeContext, now: bool) {
        let Some(armed) = self.armed.clone() else {
            return;
        };
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            self.armed = None;
            return;
        };
        let plane = feature.plane;
        let (items, projected) =
            match materialize(ctx, &plane, &mut feature.sketch, &armed.order, &armed.picks) {
                Ok(done) => done,
                Err(why) => {
                    ctx.log_warn(format!("Could not bring that edge in: {why}"));
                    return;
                }
            };
        let shape = shape_of(&feature.sketch, &items);
        let ready = fits(&armed.which, &shape)
            && (now || items.len() >= 2 || !fits_with_one_more(&armed.which, &shape));
        if !ready {
            if now {
                ctx.log_warn(format!(
                    "The {} constraint does not fit what is picked yet",
                    armed.which.replace('_', " ")
                ));
            }
            return;
        }
        let which = if armed.which == "dimension" {
            match constrain::dimension_in(&shape, &feature.sketch) {
                Some(which) => which,
                None => return,
            }
        } else {
            armed.which.as_str()
        };
        let first = self
            .active_sketch_id
            .map(|sketch_id| {
                projected
                    .iter()
                    .map(|p| {
                        let args = commands::args(serde_json::json!({
                            "sketch": sketch_id.0.to_string(),
                            "edges": [{
                                "body": p.source.body.to_string(),
                                "point": p.source.point,
                                "direction": p.source.direction,
                            }],
                            "counts": false,
                        }));
                        ("sketch.external", args, p.made.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !self.constrain_items(ctx, feature, which, &items, first) {
            ctx.log_warn(format!(
                "The {which} constraint does not fit what is picked"
            ));
            return;
        }
        // The picks were the tool's operands, not a selection to keep.
        self.armed = None;
        self.selected.clear();
        if !projected.is_empty() {
            ctx.log_info(match projected.len() {
                1 => "Added 1 external element as a guide".to_string(),
                n => format!("Added {n} external elements as guides"),
            });
        }
    }
}
