//! Placing bodies so their joints hold.
//!
//! A joint moves the body it belongs to against another. Bodies are solved
//! one at a time, each after every body it is joined to, so a chain settles
//! from its grounded end: a body with no joints of its own stays where it is.
//! Each body is moved as little as its joints allow, by damped least squares
//! over its six degrees of freedom from where it sits now, so what a joint
//! leaves free (a slide along a mated face, a turn about an aligned axis)
//! keeps its current value.

use std::collections::{BTreeSet, HashMap};

use core_document::{BodyId, BodyPlacement, Document, FeatureId};
use glam::{DQuat, DVec3};

use crate::coupling::{Coupling, Link, links};
use crate::joint::{JOINT_KIND, JointFeature, JointKind, Rigid, WORLD};

/// Every body's placement in double precision, and the world's, which
/// never moves.
pub(crate) fn rigid_placements(document: &Document) -> HashMap<BodyId, Rigid> {
    document
        .bodies()
        .iter()
        .map(|b| (b.id, Rigid::from(b.placement)))
        .chain([(WORLD, Rigid::from(BodyPlacement::default()))])
        .collect()
}

/// A joint as the solver reads it.
#[derive(Debug, Clone)]
pub struct Joint {
    pub id: FeatureId,
    pub name: String,
    /// The body it moves.
    pub body: BodyId,
    pub feature: JointFeature,
}

impl Joint {
    /// The body across the joint from `body`.
    fn other_end(&self, body: BodyId) -> BodyId {
        if self.body == body {
            self.feature.other_body
        } else {
            self.body
        }
    }
}

/// Every joint that is not suppressed, in history order.
pub fn joints(document: &Document) -> Vec<Joint> {
    let mut found: Vec<(u64, Joint)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == JOINT_KIND && !node.suppressed)
        .filter_map(|(id, node)| {
            // With its formulas' values in.
            let feature = serde_json::from_value(document.feature_values(*id)?.clone()).ok()?;
            Some((
                node.seq,
                Joint {
                    id: *id,
                    name: node.name.clone(),
                    body: node.body?,
                    feature,
                },
            ))
        })
        .collect();
    found.sort_by_key(|(seq, joint)| (*seq, joint.id));
    found.into_iter().map(|(_, joint)| joint).collect()
}

/// Why the joints could not all hold.
#[derive(Debug, Clone, PartialEq)]
pub enum SolveError {
    /// A body's joints ask for more than one place at once; the named
    /// joints are the ones left apart. `moves` places the bodies joined
    /// neither to it nor to another such body: a conflict holds up only
    /// what it is joined to.
    Conflict {
        body: BodyId,
        joints: Vec<String>,
        moves: Vec<(BodyId, BodyPlacement)>,
    },
}

/// The joints the solver reads, rigid groups' and rigid components' holds
/// among them: both bodies there and different, or a ground. A joint
/// between two bodies of one rigid component rests.
fn usable(document: &Document) -> Vec<Joint> {
    let exists = |body: BodyId| body == WORLD || document.bodies().iter().any(|b| b.id == body);
    let own: Vec<Joint> = joints(document)
        .into_iter()
        .chain(crate::group::holds(document))
        .map(|j| Joint {
            feature: j.feature.with_ends_moved(),
            ..j
        })
        .filter(|j| {
            exists(j.body)
                && (j.feature.kind == JointKind::Ground
                    || (exists(j.feature.other_body)
                        && j.feature.other_body != j.body
                        && !crate::components::together(document, j.body, j.feature.other_body)))
        })
        .collect();
    let holds = crate::components::holds(document, &own);
    own.into_iter().chain(holds).collect()
}

/// The bodies the solver moves: those that own a joint and are not
/// grounded, and the body at the other end of a joint a grounded body
/// owns, which moves for it. Every other body stays where it is.
fn free_bodies(all: &[Joint]) -> BTreeSet<BodyId> {
    let grounded: BTreeSet<BodyId> = all
        .iter()
        .filter(|j| j.feature.kind == JointKind::Ground)
        .map(|j| j.body)
        .collect();
    all.iter()
        .filter(|j| j.feature.kind != JointKind::Ground)
        .map(|j| {
            if grounded.contains(&j.body) {
                j.feature.other_body
            } else {
                j.body
            }
        })
        .filter(|b| *b != WORLD && !grounded.contains(b))
        .collect()
}

/// The placement every jointed body takes. Bodies whose joints already
/// hold are left out, as are bodies without joints.
///
/// Each free body is first placed on its own against the bodies placed
/// before it, a joint at a time turning it round where it faces the wrong
/// way; bodies joined in a ring take their turn once most of their joints
/// have something placed to hold to. Then every free body is refined
/// together against every joint, which is what closes a ring.
pub fn solve(document: &Document) -> Result<Vec<(BodyId, BodyPlacement)>, SolveError> {
    let all = usable(document);
    let free = free_bodies(&all);
    let starts: HashMap<BodyId, Rigid> = rigid_placements(document);
    let mut placements = starts.clone();
    // Every joint with a free body at either end: one a grounded body
    // owns still holds, by moving the body at its other end.
    let holding: Vec<&Joint> = all
        .iter()
        .filter(|j| {
            j.feature.kind != JointKind::Ground
                && (free.contains(&j.body) || free.contains(&j.feature.other_body))
        })
        .collect();

    let tied = tied(document, &all, &free, &starts);

    // One body at a time, against what is placed.
    let mut placed: BTreeSet<BodyId> = placements
        .keys()
        .copied()
        .filter(|b| !free.contains(b))
        .collect();
    let mut pending: Vec<BodyId> = free.iter().copied().collect();
    while !pending.is_empty() {
        // A body's joints: its own, and those a body the solver does not
        // move holds it by.
        let held_by = |j: &Joint, body: BodyId| {
            j.body == body || (j.feature.other_body == body && !free.contains(&j.body))
        };
        let holds_to = |body: BodyId| -> Vec<&Joint> {
            holding
                .iter()
                .copied()
                .filter(|j| held_by(j, body) && placed.contains(&j.other_end(body)))
                .collect()
        };
        // Ready: every joint holds to something placed. In a ring none is;
        // the one with the most joints placed goes first.
        let next = pending
            .iter()
            .copied()
            .find(|b| holding.iter().filter(|j| held_by(j, *b)).count() == holds_to(*b).len())
            .or_else(|| pending.iter().copied().max_by_key(|b| holds_to(*b).len()))
            .expect("pending is not empty");
        let joints_now = holds_to(next);
        if !joints_now.is_empty() {
            let at = place(next, placements[&next], &joints_now, &placements);
            placements.insert(next, at);
        }
        placed.insert(next);
        pending.retain(|b| *b != next);
    }

    // Every free body together, against every joint and coupling.
    refine(&mut placements, &free, &holding, &tied);

    // Joints still apart: each free body with one, and every one at either
    // end of it.
    let mut conflict: Option<(BodyId, Vec<String>)> = None;
    let mut apart: BTreeSet<BodyId> = BTreeSet::new();
    for body in &free {
        let mut left_apart: Vec<String> = holding
            .iter()
            .filter(|j| j.body == *body || j.feature.other_body == *body)
            .filter(|j| {
                worst(
                    &j.feature,
                    &placements[&j.body],
                    &placements[&j.feature.other_body],
                ) > HOLDS_MM
            })
            .map(|j| j.name.clone())
            .collect();
        left_apart.extend(
            tied.iter()
                .filter(|l| l.driven.body == *body)
                .filter(|l| {
                    let mut r = Vec::new();
                    l.residual(&placements, &mut r);
                    r.iter().any(|v| v.abs() > HOLDS_MM)
                })
                .map(|l| l.name.clone()),
        );
        if !left_apart.is_empty() {
            apart.insert(*body);
            if conflict.is_none() {
                conflict = Some((*body, left_apart));
            }
        }
    }
    // A joint between two bodies the solver moves neither of (both
    // grounded, say) holds only if they already sit as it asks.
    if conflict.is_none() {
        let stuck: Vec<&Joint> = all
            .iter()
            .filter(|j| {
                j.feature.kind != JointKind::Ground
                    && !free.contains(&j.body)
                    && !free.contains(&j.feature.other_body)
                    && worst(
                        &j.feature,
                        &placements[&j.body],
                        &placements[&j.feature.other_body],
                    ) > HOLDS_MM
            })
            .collect();
        if let Some(first) = stuck.first() {
            conflict = Some((
                first.body,
                stuck
                    .iter()
                    .filter(|j| j.body == first.body)
                    .map(|j| j.name.clone())
                    .collect(),
            ));
        }
    }
    // A body joined, however far round, to one whose joints are apart
    // keeps its place; the rest of the assembly is placed.
    let held_up = joined_to(&apart, &free, &holding, &tied);
    let mut moved = Vec::new();
    for body in free.iter().filter(|b| !held_up.contains(b)) {
        // Rounding is no move: an assembly that already holds records
        // nothing when solved again.
        let before = BodyPlacement::from(starts[body]);
        let after = BodyPlacement::from(placements[body]);
        if !after.after(&before.inverse()).is_identity() {
            moved.push((*body, after));
        }
    }
    match conflict {
        Some((body, joints)) => Err(SolveError::Conflict {
            body,
            joints,
            moves: moved,
        }),
        None => Ok(moved),
    }
}

/// `from` and every free body joined to them, however far round, by a
/// joint or a coupling between two free bodies.
fn joined_to(
    from: &BTreeSet<BodyId>,
    free: &BTreeSet<BodyId>,
    holding: &[&Joint],
    tied: &[Link],
) -> BTreeSet<BodyId> {
    let mut reached = from.clone();
    loop {
        let before = reached.len();
        let pairs = holding
            .iter()
            .map(|j| (j.body, j.feature.other_body))
            .chain(tied.iter().map(|l| (l.driven.body, l.driver.body)));
        for (a, b) in pairs {
            if !(free.contains(&a) && free.contains(&b)) {
                continue;
            }
            if reached.contains(&a) || reached.contains(&b) {
                reached.insert(a);
                reached.insert(b);
            }
        }
        if reached.len() == before {
            return reached;
        }
    }
}

/// Whether dragging `body` moves it: it has joints and is not grounded,
/// and a rigid component it sits in is joined to something outside it.
pub fn draggable(document: &Document, body: BodyId) -> bool {
    let all = usable(document);
    free_bodies(&all).contains(&body) && crate::components::joined_outside(document, body, &all)
}

/// `body` dragged so its `point` (in its own frame) follows `target` (in
/// the world) as far as its joints let it, every joint holding; the bodies
/// that moved and where. A body the solver does not move (grounded, or
/// with no joints of its own) is not dragged.
pub fn drag(
    document: &Document,
    body: BodyId,
    point: [f32; 3],
    target: [f32; 3],
) -> Vec<(BodyId, BodyPlacement)> {
    let all = usable(document);
    let free = free_bodies(&all);
    if !free.contains(&body) {
        return Vec::new();
    }
    let holding: Vec<&Joint> = all
        .iter()
        .filter(|j| {
            j.feature.kind != JointKind::Ground
                && (free.contains(&j.body) || free.contains(&j.feature.other_body))
        })
        .collect();
    let starts: HashMap<BodyId, Rigid> = rigid_placements(document);
    let tied = tied(document, &all, &free, &starts);
    let mut placements = starts.clone();
    let pull = Pull {
        body,
        point: DVec3::from_array(point.map(f64::from)),
        target: DVec3::from_array(target.map(f64::from)),
    };
    refine_pulled(&mut placements, &free, &holding, &tied, Some(&pull));
    // The pull traded a little of each joint for reach; let them close.
    refine(&mut placements, &free, &holding, &tied);
    free.iter()
        .filter_map(|b| {
            let before = BodyPlacement::from(starts[b]);
            let after = BodyPlacement::from(placements[b]);
            (!after.after(&before.inverse()).is_identity()).then_some((*b, after))
        })
        .collect()
}

/// How closely a joint must hold, in millimetres (and the equivalent turn).
pub const HOLDS_MM: f64 = 1e-3;

fn worst(joint: &JointFeature, moving: &Rigid, fixed: &Rigid) -> f64 {
    let mut r = Vec::new();
    joint.residuals(moving, fixed, &mut r);
    r.iter().fold(0.0, |m: f64, v| m.max(v.abs()))
}

/// Where each free body's turns pivot: the middle of its anchors, so a
/// turn does not throw it across the room.
fn pivots(
    placements: &HashMap<BodyId, Rigid>,
    free: &BTreeSet<BodyId>,
    joints: &[&Joint],
) -> HashMap<BodyId, DVec3> {
    free.iter()
        .map(|body| {
            let at = placements[body];
            let points: Vec<DVec3> = joints
                .iter()
                .flat_map(|j| {
                    let mut p = Vec::new();
                    if j.body == *body {
                        p.push(j.feature.moving.placed(&at).0);
                    }
                    if j.feature.other_body == *body {
                        p.push(j.feature.fixed.placed(&at).0);
                    }
                    p
                })
                .collect();
            let middle = if points.is_empty() {
                at.translation
            } else {
                points.iter().copied().sum::<DVec3>() / points.len() as f64
            };
            (*body, middle)
        })
        .collect()
}

/// `at` turned by the rotation vector `turn` about `pivot`, then moved by
/// `step`.
fn stepped(at: Rigid, pivot: DVec3, turn: DVec3, step: DVec3) -> Rigid {
    let q = DQuat::from_scaled_axis(turn);
    Rigid {
        rotation: (q * at.rotation).normalize(),
        translation: q * (at.translation - pivot) + pivot + step,
    }
}

/// Every joint's residuals, then every coupling's, with the bodies at
/// `placements`.
fn residuals_of(placements: &HashMap<BodyId, Rigid>, joints: &[&Joint], tied: &[Link]) -> Vec<f64> {
    let mut r = Vec::new();
    for j in joints {
        j.feature.residuals(
            &placements[&j.body],
            &placements[&j.feature.other_body],
            &mut r,
        );
    }
    for link in tied {
        link.residual(placements, &mut r);
    }
    r
}

/// The couplings a solve holds: both joints usable, and a body the solve
/// moves at either end of the driven one.
fn tied(
    document: &Document,
    all: &[Joint],
    free: &BTreeSet<BodyId>,
    starts: &HashMap<BodyId, Rigid>,
) -> Vec<Link> {
    links(document, all, starts)
        .into_iter()
        .filter(|l| free.contains(&l.driven.body) || free.contains(&l.driven.feature.other_body))
        .collect()
}

/// The couplings whose driver `moves` carry past a whole turn, with their
/// counts moved: to store alongside the moves, before they are made.
pub fn counted_couplings(
    document: &Document,
    moves: &[(BodyId, BodyPlacement)],
) -> Vec<(FeatureId, Coupling)> {
    let starts: HashMap<BodyId, Rigid> = rigid_placements(document);
    let mut after = starts.clone();
    for (body, placement) in moves {
        after.insert(*body, Rigid::from(*placement));
    }
    links(document, &usable(document), &starts)
        .into_iter()
        .filter_map(|l| Some((l.id, l.counted(&after)?)))
        .collect()
}

/// A point of a body pulled toward a target, gently: joints give way to it
/// only where they leave the body free to follow.
#[derive(Debug, Clone, Copy)]
struct Pull {
    body: BodyId,
    /// The point, in the body's own frame.
    point: DVec3,
    target: DVec3,
}

/// How much a pull counts against a joint.
const PULL_WEIGHT: f64 = 0.1;

/// Every joint's residuals, and the pull's, with the bodies at
/// `placements`.
fn residuals_with(
    placements: &HashMap<BodyId, Rigid>,
    joints: &[&Joint],
    tied: &[Link],
    pull: Option<&Pull>,
) -> Vec<f64> {
    let mut r = residuals_of(placements, joints, tied);
    if let Some(pull) = pull {
        let at = &placements[&pull.body];
        let apart = at.rotation * pull.point + at.translation - pull.target;
        r.extend(apart.to_array().map(|c| c * PULL_WEIGHT));
    }
    r
}

/// Damped least squares over every free body's six freedoms at once.
fn refine(
    placements: &mut HashMap<BodyId, Rigid>,
    free: &BTreeSet<BodyId>,
    joints: &[&Joint],
    tied: &[Link],
) {
    refine_pulled(placements, free, joints, tied, None);
}

/// [`refine`], with a body's point pulled toward a target as well.
fn refine_pulled(
    placements: &mut HashMap<BodyId, Rigid>,
    free: &BTreeSet<BodyId>,
    joints: &[&Joint],
    tied: &[Link],
    pull: Option<&Pull>,
) {
    let bodies: Vec<BodyId> = free.iter().copied().collect();
    let n = bodies.len() * 6;
    if n == 0 || joints.is_empty() {
        return;
    }
    let apply = |base: &HashMap<BodyId, Rigid>, pivots: &HashMap<BodyId, DVec3>, x: &[f64]| {
        let mut out = base.clone();
        for (i, body) in bodies.iter().enumerate() {
            let v = &x[i * 6..i * 6 + 6];
            out.insert(
                *body,
                stepped(
                    base[body],
                    pivots[body],
                    DVec3::new(v[0], v[1], v[2]),
                    DVec3::new(v[3], v[4], v[5]),
                ),
            );
        }
        out
    };
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut r0 = residuals_with(placements, joints, tied, pull);
    let mut c0 = cost(&r0);
    let mut damping = 1e-3;
    for _ in 0..200 {
        if c0 < 1e-16 {
            break;
        }
        let pivots = pivots(placements, free, joints);
        // The Jacobian by central differences, a column per freedom.
        let h = 1e-7;
        let mut jac = vec![0.0f64; r0.len() * n];
        let mut x = vec![0.0; n];
        for k in 0..n {
            x[k] = h;
            let rp = residuals_with(&apply(placements, &pivots, &x), joints, tied, pull);
            x[k] = -h;
            let rm = residuals_with(&apply(placements, &pivots, &x), joints, tied, pull);
            x[k] = 0.0;
            for row in 0..r0.len() {
                jac[row * n + k] = (rp[row] - rm[row]) / (2.0 * h);
            }
        }
        let mut jtj = vec![0.0f64; n * n];
        let mut jtr = vec![0.0f64; n];
        for row in 0..r0.len() {
            let jr = &jac[row * n..row * n + n];
            for a in 0..n {
                if jr[a] == 0.0 {
                    continue;
                }
                jtr[a] += jr[a] * r0[row];
                for b in 0..n {
                    jtj[a * n + b] += jr[a] * jr[b];
                }
            }
        }
        let mut accepted = false;
        for _ in 0..12 {
            let mut m = jtj.clone();
            for a in 0..n {
                // On the identity as well as the diagonal: a freedom the
                // joints leave open takes no step.
                m[a * n + a] += damping * (jtj[a * n + a] + 1.0);
            }
            let Some(step) = solve_dense(m, jtr.iter().map(|v| -v).collect(), n) else {
                damping *= 10.0;
                continue;
            };
            let trial = apply(placements, &pivots, &step);
            let rt = residuals_with(&trial, joints, tied, pull);
            let ct = cost(&rt);
            if ct < c0 {
                *placements = trial;
                r0 = rt;
                c0 = ct;
                damping = (damping / 3.0).max(1e-12);
                accepted = true;
                break;
            }
            damping *= 10.0;
        }
        if !accepted {
            break;
        }
    }
}

/// `m · x = b` for an `n`×`n` system by Gaussian elimination with
/// pivoting.
fn solve_dense(mut m: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for col in 0..n {
        let pivot =
            (col..n).max_by(|&i, &j| m[i * n + col].abs().total_cmp(&m[j * n + col].abs()))?;
        if m[pivot * n + col].abs() < 1e-18 {
            return None;
        }
        if pivot != col {
            for k in 0..n {
                m.swap(col * n + k, pivot * n + k);
            }
            b.swap(col, pivot);
        }
        for row in col + 1..n {
            let f = m[row * n + col] / m[col * n + col];
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                m[row * n + k] -= f * m[col * n + k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|k| m[row * n + k] * x[k]).sum();
        x[row] = (b[row] - s) / m[row * n + row];
    }
    Some(x)
}

/// A motion a body's joints leave it, the bodies it is joined to held
/// still.
#[derive(Debug, Clone, PartialEq)]
pub enum Motion {
    /// A turn about a line through `through`, along `axis`. `at_limit`: a
    /// hinge's or an alignment's limit holds it at one end, so it turns one way only.
    Turn {
        axis: [f64; 3],
        through: [f64; 3],
        at_limit: bool,
    },
    /// A slide along `direction`. `at_limit`: a slider's or an alignment's limit,
    /// or a path's end, holds it at one end, so it slides one way only.
    Slide { direction: [f64; 3], at_limit: bool },
}

impl Motion {
    /// In words: "turn about Z", "slide along (0.6, 0.8, 0)".
    pub fn describe(&self) -> String {
        let name = |v: [f64; 3]| {
            for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                if (v[i].abs() - 1.0).abs() < 1e-3 {
                    return axis.to_string();
                }
            }
            format!("({:.2}, {:.2}, {:.2})", v[0], v[1], v[2])
        };
        let (words, at_limit) = match self {
            Motion::Turn { axis, at_limit, .. } => {
                (format!("turn about {}", name(*axis)), at_limit)
            }
            Motion::Slide {
                direction,
                at_limit,
            } => (format!("slide along {}", name(*direction)), at_limit),
        };
        if *at_limit {
            format!("{words} (one way, at its limit)")
        } else {
            words
        }
    }

    fn mark_at_limit(&mut self) {
        match self {
            Motion::Turn { at_limit, .. } | Motion::Slide { at_limit, .. } => *at_limit = true,
        }
    }
}

/// How near an end of its limits, in degrees or millimetres, a hinge, a
/// slider or an alignment rests on it, or a point on a path's end.
const AT_LIMIT: f64 = 1e-3;

/// The lines along which a limit holds a motion at one end, for `body`:
/// each the line of a turn (`true`) or of a slide (`false`), in the world.
/// A hinge's axis, a slider's, an alignment's for either of its motions,
/// and a path's end, where the point runs back along the last segment
/// only.
fn limits_reached(
    joints: &[Joint],
    placements: &HashMap<BodyId, Rigid>,
    body: BodyId,
) -> Vec<(bool, DVec3)> {
    let mut found = Vec::new();
    for j in joints
        .iter()
        .filter(|j| j.body == body || j.feature.other_body == body)
    {
        let (Some(moving), Some(fixed)) = (
            placements.get(&j.body),
            placements.get(&j.feature.other_body),
        ) else {
            continue;
        };
        let axis = j.feature.fixed.placed(fixed).1;
        // A drive held between limits, with where its motion is now.
        let resting = |drive: crate::joint::Drive, now: f64| {
            drive.to.is_none()
                && drive.limits.is_some_and(|[low, high]| {
                    let near = |end: f32| (now - f64::from(end)).abs() <= AT_LIMIT;
                    near(low) || near(high)
                })
        };
        match j.feature.kind {
            JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } => {
                let hinge = matches!(j.feature.kind, JointKind::Hinge { .. });
                if let Some(now) = j.feature.travel(moving, fixed)
                    && resting(drive, now)
                {
                    found.push((hinge, axis));
                }
            }
            JointKind::Align { turn, slide, .. } => {
                if let Some((turned, slid)) = j.feature.align_travel(moving, fixed) {
                    if resting(turn, turned) {
                        found.push((true, axis));
                    }
                    if resting(slide, slid) {
                        found.push((false, axis));
                    }
                }
            }
            JointKind::Path => {
                if let Some((_, tangent, end)) = path_tangent(&j.feature, moving, fixed)
                    && end
                {
                    found.push((false, tangent));
                }
            }
            _ => {}
        }
    }
    found
}

/// Where a path joint's point stands on its line, the other body at
/// `fixed`: the nearest point, the line's direction there (at a corner,
/// the segment after it), and whether that is an end of an open line.
fn path_tangent(
    feature: &JointFeature,
    moving: &Rigid,
    fixed: &Rigid,
) -> Option<(DVec3, DVec3, bool)> {
    let line: Vec<DVec3> = feature
        .shape
        .iter()
        .map(|p| fixed.rotation * DVec3::from_array(p.map(f64::from)) + fixed.translation)
        .collect();
    let (pm, _) = feature.moving.placed(moving);
    let (i, q) = line
        .windows(2)
        .enumerate()
        .map(|(i, w)| (i, crate::shapes::nearest_on_segment(pm, w[0], w[1])))
        .min_by(|a, b| a.1.distance(pm).total_cmp(&b.1.distance(pm)))?;
    let closed = line.len() > 2 && line[0].distance(line[line.len() - 1]) < 1e-6;
    let last = line.len() - 2;
    // At the segment's far end, the one after it carries on.
    let i = if q.distance(line[i + 1]) < 1e-6 && i < last {
        i + 1
    } else {
        i
    };
    let tangent = (line[i + 1] - line[i]).try_normalize()?;
    let end = !closed
        && ((i == 0 && q.distance(line[0]) < 1e-6)
            || (i == last && q.distance(line[i + 1]) < 1e-6));
    Some((q, tangent, end))
}

/// What each jointed body may still do where it sits: the motions its
/// joints leave open, the bodies around it held still. A body with none
/// is fully placed.
pub fn freedom(document: &Document) -> Vec<(BodyId, Vec<Motion>)> {
    let limited = usable(document);
    let placements: HashMap<BodyId, Rigid> = rigid_placements(document);
    // A limit stops a motion only at its ends: the motion is still there,
    // marked one way where a limit holds it. A path's point runs along the
    // line through where it stands, a corner or an end of the path too.
    let all: Vec<Joint> = limited
        .clone()
        .into_iter()
        .map(|mut j| {
            match &mut j.feature.kind {
                JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } => {
                    drive.limits = None;
                }
                JointKind::Align { turn, slide, .. } => {
                    turn.limits = None;
                    slide.limits = None;
                }
                JointKind::Path => {
                    if let (Some(moving), Some(fixed)) = (
                        placements.get(&j.body),
                        placements.get(&j.feature.other_body),
                    ) && let Some((q, tangent, _)) = path_tangent(&j.feature, moving, fixed)
                    {
                        let local = |p: DVec3| {
                            let p = fixed.rotation.inverse() * (p - fixed.translation);
                            p.as_vec3().to_array()
                        };
                        j.feature.shape =
                            vec![local(q - tangent * 10.0), local(q + tangent * 10.0)];
                    }
                }
                _ => {}
            }
            j
        })
        .collect();
    let free = free_bodies(&all);
    let holding: Vec<&Joint> = all
        .iter()
        .filter(|j| j.feature.kind != JointKind::Ground)
        .collect();
    let pivots = pivots(&placements, &free, &holding);
    // A coupling holds the body its driven joint moves: that body follows
    // the driver, which keeps its own motion.
    let tied = links(document, &all, &placements);
    free.iter()
        .map(|body| {
            let own: Vec<&Joint> = holding
                .iter()
                .copied()
                .filter(|j| j.body == *body || j.feature.other_body == *body)
                .collect();
            let own_tied: Vec<Link> = tied
                .iter()
                .filter(|l| l.driven.body == *body)
                .cloned()
                .collect();
            let jtj = normal_matrix(&placements, &pivots, *body, &own, &own_tied);
            let (values, vectors) = eigen6(jtj);

            let scale = values.iter().fold(1.0f64, |m, v| m.max(*v));
            let motions = (0..6)
                .filter(|i| values[*i] < 1e-6 * scale)
                .map(|i| {
                    let v = vectors[i];
                    let turn = DVec3::new(v[0], v[1], v[2]);
                    let slide = DVec3::new(v[3], v[4], v[5]);
                    let pivot = pivots[body];
                    if turn.length() > 1e-3 {
                        // Turning by ω about the pivot and sliding by v is a
                        // turn about the line where the two agree.
                        let axis = turn.normalize();
                        let through = pivot + turn.cross(slide) / turn.length_squared();
                        Motion::Turn {
                            axis: tidy(axis),
                            through: through.to_array(),
                            at_limit: false,
                        }
                    } else {
                        Motion::Slide {
                            direction: tidy(slide.normalize_or_zero()),
                            at_limit: false,
                        }
                    }
                })
                .collect::<Vec<Motion>>();
            let mut motions = motions;
            for (hinge, line) in limits_reached(&limited, &placements, *body) {
                let along = |v: &[f64; 3]| DVec3::from_array(*v).dot(line).abs() > 0.999;
                if let Some(motion) = motions.iter_mut().find(|m| match m {
                    Motion::Turn { axis, .. } => hinge && along(axis),
                    Motion::Slide { direction, .. } => !hinge && along(direction),
                }) {
                    motion.mark_at_limit();
                }
            }
            (*body, motions)
        })
        .collect()
}

/// `JᵀJ` of `body`'s six motions against `own` joints and `tied`
/// couplings, where the bodies are placed: its null space is what they
/// leave free.
fn normal_matrix(
    placements: &HashMap<BodyId, Rigid>,
    pivots: &HashMap<BodyId, DVec3>,
    body: BodyId,
    own: &[&Joint],
    tied: &[Link],
) -> [[f64; 6]; 6] {
    // This body's six columns, turns first, by central differences: both
    // sides go through the same steps (a stored rotation is normalised on
    // the way), so nothing but the motion differs.
    let h = 1e-6;
    let at = |x: [f64; 6]| {
        let mut moved = placements.clone();
        moved.insert(
            body,
            stepped(
                placements[&body],
                pivots[&body],
                DVec3::new(x[0], x[1], x[2]),
                DVec3::new(x[3], x[4], x[5]),
            ),
        );
        residuals_of(&moved, own, tied)
    };
    let mut columns = vec![[0.0f64; 6]; at([0.0; 6]).len()];
    for k in 0..6 {
        let (mut plus, mut minus) = ([0.0; 6], [0.0; 6]);
        plus[k] = h;
        minus[k] = -h;
        let (rp, rm) = (at(plus), at(minus));
        for (row, (a, b)) in columns.iter_mut().zip(rp.iter().zip(&rm)) {
            row[k] = (a - b) / (2.0 * h);
        }
    }
    let mut jtj = [[0.0f64; 6]; 6];
    for row in &columns {
        for a in 0..6 {
            for b in 0..6 {
                jtj[a][b] += row[a] * row[b];
            }
        }
    }
    jtj
}

/// How many of a body's six motions `JᵀJ` holds.
fn rank(jtj: [[f64; 6]; 6]) -> usize {
    let (values, _) = eigen6(jtj);
    let scale = values.iter().fold(1.0f64, |m, v| m.max(*v));
    values.iter().filter(|v| **v >= 1e-6 * scale).count()
}

/// Joints that hold, but hold nothing the body's other joints do not:
/// each such joint could go without freeing any motion. Each with the
/// body it moves.
pub fn redundant(document: &Document) -> Vec<(FeatureId, String)> {
    let all = usable(document);
    let free = free_bodies(&all);
    let holding: Vec<&Joint> = all
        .iter()
        .filter(|j| j.feature.kind != JointKind::Ground)
        .collect();
    let placements: HashMap<BodyId, Rigid> = rigid_placements(document);
    let pivots = pivots(&placements, &free, &holding);
    let tied = links(document, &all, &placements);
    let mut found: Vec<(FeatureId, String)> = Vec::new();
    for body in &free {
        let own: Vec<&Joint> = holding
            .iter()
            .copied()
            .filter(|j| j.body == *body || j.feature.other_body == *body)
            .collect();
        let own_tied: Vec<Link> = tied
            .iter()
            .filter(|l| l.driven.body == *body)
            .cloned()
            .collect();
        let whole = rank(normal_matrix(&placements, &pivots, *body, &own, &own_tied));
        for joint in &own {
            if found.iter().any(|(id, _)| *id == joint.id)
                || worst(
                    &joint.feature,
                    &placements[&joint.body],
                    &placements[&joint.feature.other_body],
                ) > HOLDS_MM
            {
                continue;
            }
            let rest: Vec<&Joint> = own.iter().copied().filter(|j| j.id != joint.id).collect();
            if rank(normal_matrix(&placements, &pivots, *body, &rest, &own_tied)) == whole {
                found.push((joint.id, joint.name.clone()));
            }
        }
    }
    found
}

/// Joints and couplings that do not hold where the bodies sit now, in
/// history order: what a solve left apart, each by its id and name.
pub fn unmet(document: &Document) -> Vec<(FeatureId, String)> {
    let all = usable(document);
    let placements: HashMap<BodyId, Rigid> = rigid_placements(document);
    let is_joint = |id: FeatureId| {
        document
            .get_feature_meta(id)
            .is_some_and(|n| n.workbench_id.as_str() == JOINT_KIND)
    };
    let apart = all
        .iter()
        .filter(|j| j.feature.kind != JointKind::Ground && is_joint(j.id))
        .filter(|j| {
            worst(
                &j.feature,
                &placements[&j.body],
                &placements[&j.feature.other_body],
            ) > HOLDS_MM
        })
        .map(|j| (j.id, j.name.clone()));
    let tied = links(document, &all, &placements)
        .into_iter()
        .filter(|l| {
            let mut r = Vec::new();
            l.residual(&placements, &mut r);
            r.iter().any(|v| v.abs() > HOLDS_MM)
        })
        .map(|l| (l.id, l.name));
    apart.chain(tied).collect()
}

/// A direction pointing its larger part the positive way, noise dropped.
fn tidy(v: DVec3) -> [f64; 3] {
    let biggest = v
        .to_array()
        .into_iter()
        .fold(0.0f64, |m, c| if c.abs() > m.abs() { c } else { m });
    let v = if biggest < 0.0 { -v } else { v };
    v.to_array().map(|c| if c.abs() < 1e-9 { 0.0 } else { c })
}

/// The eigenvalues and eigenvectors of a symmetric 6×6 matrix, by Jacobi
/// rotations; rows and columns by index, as the algebra reads.
#[allow(clippy::needless_range_loop)]
fn eigen6(mut a: [[f64; 6]; 6]) -> ([f64; 6], [[f64; 6]; 6]) {
    let mut v = [[0.0f64; 6]; 6];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..100 {
        let mut off = 0.0;
        for p in 0..6 {
            for q in p + 1..6 {
                off += a[p][q] * a[p][q];
            }
        }
        if off < 1e-30 {
            break;
        }
        for p in 0..6 {
            for q in p + 1..6 {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..6 {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..6 {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let values = [a[0][0], a[1][1], a[2][2], a[3][3], a[4][4], a[5][5]];
    // Columns of `v` are the vectors; hand them back as rows.
    let mut vectors = [[0.0f64; 6]; 6];
    for (i, vector) in vectors.iter_mut().enumerate() {
        for (k, c) in vector.iter_mut().enumerate() {
            *c = v[k][i];
        }
    }
    (values, vectors)
}

/// The placement of `body` nearest `start` at which `joints` hold, the
/// bodies they hold against sitting at `others`. A joint may belong to
/// `body` or hold it from the other end.
fn place(body: BodyId, start: Rigid, joints: &[&Joint], others: &HashMap<BodyId, Rigid>) -> Rigid {
    let owns = |joint: &Joint| joint.body == body;
    // The anchor on `body` and the one across the joint.
    let mine = |joint: &Joint| {
        if owns(joint) {
            joint.feature.moving
        } else {
            joint.feature.fixed
        }
    };
    let theirs = |joint: &Joint| {
        if owns(joint) {
            (joint.feature.fixed, &others[&joint.feature.other_body])
        } else {
            (joint.feature.moving, &others[&joint.body])
        }
    };
    // Turns pivot about the body's anchors' middle, so a turn does not
    // throw the body across the room.
    let pivot = {
        let points: Vec<DVec3> = joints.iter().map(|j| mine(j).placed(&start).0).collect();
        points.iter().copied().sum::<DVec3>() / points.len().max(1) as f64
    };
    let turned = |at: Rigid, turn: DQuat| Rigid {
        rotation: (turn * at.rotation).normalize(),
        translation: turn * (at.translation - pivot) + pivot,
    };
    let mut current = start;
    // First turn the body so its first joint's directions agree: where two
    // faces start facing the same way and must face each other, the least
    // squares below would sit on the half-turn's flat top and never start.
    // An angle names no single direction, so the first joint that does
    // sets the turn.
    let first_turn = joints.iter().find_map(|joint| {
        let (_, dm) = mine(joint).placed(&current);
        let (anchor, at) = theirs(joint);
        let (_, df) = anchor.placed(at);
        let target = match joint.feature.kind {
            JointKind::Mate { flip: false, .. } => -df,
            JointKind::Mate { flip: true, .. } => df,
            // An axis has no way round: the nearer of its two directions.
            JointKind::Align { .. } | JointKind::Hinge { .. } | JointKind::Parallel => {
                if dm.dot(df) < 0.0 { -df } else { df }
            }
            // A held turn is the whole rotation, not only a direction: the
            // moving body's is the fixed one's turned by it.
            JointKind::Slider { turn, .. } | JointKind::Fixed { turn, .. } => {
                let turn = crate::joint::quat(turn);
                let want = if owns(joint) {
                    at.rotation * turn
                } else {
                    at.rotation * turn.inverse()
                };
                return Some((want.normalize() * current.rotation.inverse()).normalize());
            }
            JointKind::Angle { .. }
            | JointKind::Ground
            | JointKind::Perpendicular
            | JointKind::Distance { .. }
            | JointKind::Tangent { .. }
            | JointKind::Ball
            | JointKind::Universal
            | JointKind::Slot
            | JointKind::Path
            | JointKind::Cam { .. }
            | JointKind::Width => return None,
        };
        (dm.length_squared() > 0.0 && target.length_squared() > 0.0)
            .then(|| DQuat::from_rotation_arc(dm, target))
    });
    // With only angles, the first one turns the body to its angle, in the
    // plane of the two normals (or about any line square to them when they
    // start parallel, where the angle has no slope to follow). Square faces
    // are an angle of 90 degrees.
    let first_turn = first_turn.or_else(|| {
        joints.iter().find_map(|joint| {
            let degrees = match joint.feature.kind {
                JointKind::Angle { degrees } => degrees,
                JointKind::Perpendicular => 90.0,
                _ => return None,
            };
            let (_, dm) = mine(joint).placed(&current);
            let (anchor, at) = theirs(joint);
            let (_, df) = anchor.placed(at);
            let now = dm.cross(df).length().atan2(dm.dot(df));
            let axis = df
                .cross(dm)
                .try_normalize()
                .unwrap_or_else(|| dm.any_orthonormal_vector());
            Some(DQuat::from_axis_angle(
                axis,
                f64::from(degrees).to_radians() - now,
            ))
        })
    });
    if let Some(turn) = first_turn {
        current = turned(current, turn);
    }
    let step_to = |at: Rigid, x: &[f64; 6]| {
        let moved = turned(at, DQuat::from_scaled_axis(DVec3::new(x[0], x[1], x[2])));
        Rigid {
            translation: moved.translation + DVec3::new(x[3], x[4], x[5]),
            ..moved
        }
    };
    let residuals = |p: &Rigid| {
        let mut r = Vec::new();
        for joint in joints {
            if owns(joint) {
                joint
                    .feature
                    .residuals(p, &others[&joint.feature.other_body], &mut r);
            } else {
                joint.feature.residuals(&others[&joint.body], p, &mut r);
            }
        }
        r
    };
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut r0 = residuals(&current);
    let mut c0 = cost(&r0);
    let mut damping = 1e-3;
    for _ in 0..200 {
        if c0 < 1e-16 {
            break;
        }
        // The Jacobian by central differences about the current placement.
        let h = 1e-7;
        let mut jac = vec![[0.0f64; 6]; r0.len()];
        for k in 0..6 {
            let mut plus = [0.0; 6];
            let mut minus = [0.0; 6];
            plus[k] = h;
            minus[k] = -h;
            let rp = residuals(&step_to(current, &plus));
            let rm = residuals(&step_to(current, &minus));
            for (row, (a, b)) in jac.iter_mut().zip(rp.iter().zip(&rm)) {
                row[k] = (a - b) / (2.0 * h);
            }
        }
        let mut jtj = [[0.0f64; 6]; 6];
        let mut jtr = [0.0f64; 6];
        for (row, r) in jac.iter().zip(&r0) {
            for a in 0..6 {
                jtr[a] += row[a] * r;
                for b in 0..6 {
                    jtj[a][b] += row[a] * row[b];
                }
            }
        }
        let mut accepted = false;
        for _ in 0..12 {
            let mut m = jtj;
            for (a, row) in m.iter_mut().enumerate() {
                // Damping on the identity as well as the diagonal keeps a
                // freedom the joints leave open at zero step.
                row[a] += damping * (jtj[a][a] + 1.0);
            }
            let Some(step) = solve6(m, jtr.map(|v| -v)) else {
                damping *= 10.0;
                continue;
            };
            let trial = step_to(current, &step);
            let rt = residuals(&trial);
            let ct = cost(&rt);
            if ct < c0 {
                current = trial;
                r0 = rt;
                c0 = ct;
                damping = (damping / 3.0).max(1e-12);
                accepted = true;
                break;
            }
            damping *= 10.0;
        }
        if !accepted {
            break;
        }
    }
    current
}

/// `m · x = b` for a 6×6 system by Gaussian elimination with pivoting.
fn solve6(mut m: [[f64; 6]; 6], mut b: [f64; 6]) -> Option<[f64; 6]> {
    for col in 0..6 {
        let pivot = (col..6).max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))?;
        if m[pivot][col].abs() < 1e-18 {
            return None;
        }
        m.swap(col, pivot);
        b.swap(col, pivot);
        let pivot_row = m[col];
        for row in col + 1..6 {
            let f = m[row][col] / pivot_row[col];
            for (value, above) in m[row][col..].iter_mut().zip(&pivot_row[col..]) {
                *value -= f * above;
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; 6];
    for row in (0..6).rev() {
        let s: f64 = (row + 1..6).map(|k| m[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / m[row][row];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::joint::{Anchor, JointKind, Rigid};
    use core_document::Document;
    use glam::{Quat, Vec3};

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3)
    }

    fn add_joint(doc: &mut Document, body: BodyId, feature: JointFeature) {
        doc.add_feature_in_body(feature, "joint".into(), Some(body))
            .unwrap();
    }

    fn apply(doc: &mut Document) {
        for (body, placement) in solve(doc).unwrap() {
            doc.set_body_placement(body, placement);
        }
    }

    #[test]
    fn a_mate_sets_one_face_on_the_other_and_keeps_the_slide() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        // The part starts turned over and off to the side.
        doc.set_body_placement(
            part,
            BodyPlacement::new(Quat::from_rotation_x(0.3), Vec3::new(40.0, 5.0, 30.0)),
        );
        // The part's underside (z = 0, facing -Z) onto the base's top
        // (z = 10, facing +Z).
        add_joint(
            &mut doc,
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
                moving: Anchor::Plane {
                    point: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, -1.0],
                },
                other_body: base,
                fixed: Anchor::Plane {
                    point: [0.0, 0.0, 10.0],
                    normal: [0.0, 0.0, 1.0],
                },
            },
        );
        apply(&mut doc);
        let placed = doc.body_placement(part);
        assert!((placed.point([0.0, 0.0, 0.0])[2] - 10.0).abs() < 1e-3);
        assert!(close(placed.direction([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]));
        // Sliding on the face is left as it was: the part stays off to the
        // side rather than jumping to the origin.
        assert!((placed.offset().x - 40.0).abs() < 1.0, "{placed:?}");
        // Solving again finds nothing to move.
        assert!(solve(&doc).unwrap().is_empty());
    }

    #[test]
    fn an_offset_leaves_a_gap_and_a_flip_turns_the_part_over() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        add_joint(
            &mut doc,
            part,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Mate {
                    flip: true,
                    offset: 2.5,
                },
                moving: Anchor::Plane {
                    point: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, -1.0],
                },
                other_body: base,
                fixed: Anchor::Plane {
                    point: [0.0, 0.0, 10.0],
                    normal: [0.0, 0.0, 1.0],
                },
            },
        );
        apply(&mut doc);
        let placed = doc.body_placement(part);
        assert!(close(placed.direction([0.0, 0.0, -1.0]), [0.0, 0.0, 1.0]));
        assert!((placed.point([0.0, 0.0, 0.0])[2] - 12.5).abs() < 1e-3);
    }

    #[test]
    fn an_align_puts_a_pin_on_the_hole_s_axis() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let pin = doc.create_body(None);
        doc.set_body_placement(
            pin,
            BodyPlacement::new(Quat::from_rotation_y(0.5), Vec3::new(3.0, -7.0, 0.0)),
        );
        add_joint(
            &mut doc,
            pin,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::align(),
                moving: Anchor::Axis {
                    point: [0.0, 0.0, 0.0],
                    direction: [0.0, 0.0, 1.0],
                },
                other_body: base,
                fixed: Anchor::Axis {
                    point: [20.0, 10.0, 0.0],
                    direction: [0.0, 0.0, 1.0],
                },
            },
        );
        apply(&mut doc);
        let placed = doc.body_placement(pin);
        let axis = placed.direction([0.0, 0.0, 1.0]);
        assert!(axis[2].abs() > 0.9999, "{axis:?}");
        let on_axis = placed.point([0.0, 0.0, 5.0]);
        assert!((on_axis[0] - 20.0).abs() < 1e-3 && (on_axis[1] - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_chain_settles_from_its_grounded_end_and_a_ring_closes_or_is_named() {
        let mut doc = Document::new("t");
        let a = doc.create_body(None);
        let b = doc.create_body(None);
        let c = doc.create_body(None);
        let mate = |other| JointFeature {
            second: None,
            shape: Vec::new(),
            ends: [0.0; 2],
            names: [0; 2],
            kind: JointKind::Mate {
                flip: true,
                offset: 5.0,
            },
            moving: Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
            other_body: other,
            fixed: Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
        };
        // c sits on b, which sits on a: b first, then c.
        add_joint(&mut doc, c, mate(b));
        add_joint(&mut doc, b, mate(a));
        apply(&mut doc);
        assert!((doc.body_placement(b).offset().z - 5.0).abs() < 1e-3);
        assert!((doc.body_placement(c).offset().z - 10.0).abs() < 1e-3);
        // Grounding a lets it take a joint of its own: a ring that can
        // close does, one that cannot is named.
        let ground = JointFeature {
            second: None,
            shape: Vec::new(),
            ends: [0.0; 2],
            names: [0; 2],
            kind: JointKind::Ground,
            moving: Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
            other_body: a,
            fixed: Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
        };
        add_joint(&mut doc, a, ground);
        let mut on_a = mate(a);
        if let JointKind::Mate { offset, .. } = &mut on_a.kind {
            *offset = 10.0;
        }
        add_joint(&mut doc, c, on_a.clone());
        apply(&mut doc);
        assert!((doc.body_placement(c).offset().z - 10.0).abs() < 1e-3);
        assert!(solve(&doc).unwrap().is_empty(), "the ring holds");
        if let JointKind::Mate { offset, .. } = &mut on_a.kind {
            *offset = 12.0;
        }
        add_joint(&mut doc, c, on_a);
        assert!(matches!(solve(&doc), Err(SolveError::Conflict { .. })));
    }

    #[test]
    fn an_angle_turns_a_face_to_the_angle_asked_and_leaves_where_it_sits() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let flap = doc.create_body(None);
        doc.set_body_placement(
            flap,
            BodyPlacement::new(Quat::IDENTITY, Vec3::new(10.0, 20.0, 30.0)),
        );
        let up = Anchor::Plane {
            point: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        };
        let joint = JointFeature {
            second: None,
            shape: Vec::new(),
            ends: [0.0; 2],
            names: [0; 2],
            kind: JointKind::Angle { degrees: 90.0 },
            moving: up,
            other_body: base,
            fixed: up,
        };
        add_joint(&mut doc, flap, joint.clone());
        apply(&mut doc);
        let placed = Rigid::from(doc.body_placement(flap));
        let angle = up.angle_to(&placed, &up, &Rigid::from(BodyPlacement::IDENTITY));
        assert!((angle - 90.0).abs() < 1e-3, "{angle}");
        // Only the turn is held: the flap is still about where it was.
        assert!((doc.body_placement(flap).offset() - Vec3::new(10.0, 20.0, 30.0)).length() < 1.0);
    }

    #[test]
    fn an_angle_with_a_mate_holds_both() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        doc.set_body_placement(
            part,
            BodyPlacement::new(Quat::from_rotation_z(0.2), Vec3::new(5.0, 5.0, 10.0)),
        );
        // The part's underside on the base's top.
        add_joint(
            &mut doc,
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
                moving: Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, -1.0],
                },
                other_body: base,
                fixed: Anchor::Plane {
                    point: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                },
            },
        );
        // Its +X side at 30 degrees to the base's +X side.
        let side = Anchor::Plane {
            point: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
        };
        add_joint(
            &mut doc,
            part,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Angle { degrees: 30.0 },
                moving: side,
                other_body: base,
                fixed: side,
            },
        );
        apply(&mut doc);
        let placed = Rigid::from(doc.body_placement(part));
        let origin = Rigid::from(BodyPlacement::IDENTITY);
        assert!((side.angle_to(&placed, &side, &origin) - 30.0).abs() < 1e-3);
        let under = Anchor::Plane {
            point: [0.0; 3],
            normal: [0.0, 0.0, -1.0],
        };
        let (point, normal) = under.placed(&placed);
        assert!(point.z.abs() < 1e-3 && (normal.z + 1.0).abs() < 1e-6);
    }

    #[test]
    fn joints_that_cannot_both_hold_are_named() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        let on = |z: f32| JointFeature {
            second: None,
            shape: Vec::new(),
            ends: [0.0; 2],
            names: [0; 2],
            kind: JointKind::Mate {
                flip: true,
                offset: 0.0,
            },
            moving: Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
            other_body: base,
            fixed: Anchor::Plane {
                point: [0.0, 0.0, z],
                normal: [0.0, 0.0, 1.0],
            },
        };
        add_joint(&mut doc, part, on(0.0));
        add_joint(&mut doc, part, on(10.0));
        match solve(&doc) {
            Err(SolveError::Conflict { body, joints, .. }) => {
                assert_eq!(body, part);
                assert_eq!(joints.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    fn plane(point: [f32; 3], normal: [f32; 3]) -> Anchor {
        Anchor::Plane { point, normal }
    }

    fn mate(moving: Anchor, other: BodyId, fixed: Anchor, flip: bool, offset: f32) -> JointFeature {
        JointFeature {
            second: None,
            shape: Vec::new(),
            ends: [0.0; 2],
            names: [0; 2],
            kind: JointKind::Mate { flip, offset },
            moving,
            other_body: other,
            fixed,
        }
    }

    fn holds(doc: &Document) -> bool {
        let placements: HashMap<BodyId, Rigid> = rigid_placements(doc);
        joints(doc).iter().all(|j| {
            j.feature.kind == JointKind::Ground
                || worst(
                    &j.feature,
                    &placements[&j.body],
                    &placements[&j.feature.other_body],
                ) <= HOLDS_MM
        })
    }

    #[test]
    fn a_ring_the_bodies_close_together_holds() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let b = doc.create_body(None);
        let c = doc.create_body(None);
        doc.set_body_placement(
            b,
            BodyPlacement::new(Quat::IDENTITY, Vec3::new(10.0, 4.0, 7.0)),
        );
        doc.set_body_placement(
            c,
            BodyPlacement::new(Quat::IDENTITY, Vec3::new(-5.0, -6.0, 2.0)),
        );
        // Both stand on the base.
        for body in [b, c] {
            add_joint(
                &mut doc,
                body,
                mate(
                    plane([0.0; 3], [0.0, 0.0, -1.0]),
                    base,
                    plane([0.0; 3], [0.0, 0.0, 1.0]),
                    false,
                    0.0,
                ),
            );
        }
        // b's side 3 mm from c's, along X; c's front against b's, along Y:
        // each needs where the other ends up.
        add_joint(
            &mut doc,
            b,
            mate(
                plane([0.0; 3], [1.0, 0.0, 0.0]),
                c,
                plane([0.0; 3], [-1.0, 0.0, 0.0]),
                false,
                3.0,
            ),
        );
        add_joint(
            &mut doc,
            c,
            mate(
                plane([0.0; 3], [0.0, 1.0, 0.0]),
                b,
                plane([0.0; 3], [0.0, -1.0, 0.0]),
                false,
                0.0,
            ),
        );
        apply(&mut doc);
        assert!(holds(&doc), "every joint holds");
        assert!(
            solve(&doc).unwrap().is_empty(),
            "and solving again moves nothing"
        );
    }

    #[test]
    fn the_freedom_left_is_the_motions_the_joints_do_not_hold() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        add_joint(
            &mut doc,
            part,
            mate(
                plane([0.0; 3], [0.0, 0.0, -1.0]),
                base,
                plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]),
                false,
                0.0,
            ),
        );
        apply(&mut doc);
        let free = freedom(&doc);
        let motions = &free.iter().find(|(b, _)| *b == part).unwrap().1;
        let mut words: Vec<String> = motions.iter().map(Motion::describe).collect();
        words.sort();
        assert_eq!(
            words,
            ["slide along X", "slide along Y", "turn about Z"],
            "{motions:?}"
        );

        // A pin in a hole, resting on the face: only its turn is left,
        // about the pin's own axis.
        add_joint(
            &mut doc,
            part,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::align(),
                moving: Anchor::Axis {
                    point: [5.0, 0.0, 0.0],
                    direction: [0.0, 0.0, 1.0],
                },
                other_body: base,
                fixed: Anchor::Axis {
                    point: [20.0, 3.0, 0.0],
                    direction: [0.0, 0.0, 1.0],
                },
            },
        );
        apply(&mut doc);
        let free = freedom(&doc);
        let motions = &free.iter().find(|(b, _)| *b == part).unwrap().1;
        assert_eq!(motions.len(), 1, "{motions:?}");
        let Motion::Turn { axis, through, .. } = motions[0] else {
            panic!("a turn: {motions:?}")
        };
        assert!((axis[2] - 1.0).abs() < 1e-3);
        assert!(
            (through[0] - 20.0).abs() < 1e-2 && (through[1] - 3.0).abs() < 1e-2,
            "{through:?}"
        );
    }

    #[test]
    fn eigen_finds_a_known_null() {
        let (a, b) = (2.0, -1.5);
        let mut m = [[0.0f64; 6]; 6];
        m[0][0] = 1.0;
        m[1][1] = 1.0;
        m[0][2] = a;
        m[2][0] = a;
        m[1][2] = b;
        m[2][1] = b;
        m[2][2] = a * a + b * b;
        m[3][3] = 5.0;
        m[4][4] = 7.0;
        m[5][5] = 9.0;
        let (values, _) = eigen6(m);
        assert!(values.iter().any(|v| v.abs() < 1e-9), "{values:?}");
    }

    /// Each body's joints solved, every joint holding, and the motions
    /// left to `part`.
    fn free_after_solving(doc: &mut Document, part: BodyId) -> Vec<Motion> {
        apply(doc);
        assert!(holds(doc), "every joint holds");
        freedom(doc)
            .into_iter()
            .find(|(b, _)| *b == part)
            .map(|(_, m)| m)
            .unwrap_or_default()
    }

    fn axis(point: [f32; 3], direction: [f32; 3]) -> Anchor {
        Anchor::Axis { point, direction }
    }

    fn rigid(doc: &Document, body: BodyId) -> Rigid {
        doc.body_placement(body).into()
    }

    /// A joint the way its tool makes it, from the bodies where they sit.
    fn made(
        doc: &Document,
        tool: crate::JointTool,
        body: BodyId,
        moving: Anchor,
        other: BodyId,
        fixed: Anchor,
    ) -> JointFeature {
        JointFeature {
            second: None,
            shape: Vec::new(),
            ends: [0.0; 2],
            names: [0; 2],
            kind: tool.joint(&moving, &rigid(doc, body), &fixed, &rigid(doc, other), 0.0),
            moving,
            other_body: other,
            fixed,
        }
    }

    fn tilted(doc: &mut Document, body: BodyId) {
        doc.set_body_placement(
            body,
            BodyPlacement::new(
                Quat::from_rotation_x(0.4) * Quat::from_rotation_y(-0.3),
                Vec3::new(35.0, -12.0, 20.0),
            ),
        );
    }

    #[test]
    fn a_hinge_leaves_only_the_turn_about_its_axis_at_its_height() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        tilted(&mut doc, part);
        add_joint(
            &mut doc,
            part,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Hinge {
                    offset: 5.0,
                    zero: DQuat::IDENTITY.to_array(),
                    drive: Default::default(),
                },
                moving: axis([0.0; 3], [0.0, 0.0, 1.0]),
                other_body: base,
                fixed: axis([20.0, 3.0, 0.0], [0.0, 0.0, 1.0]),
            },
        );
        let motions = free_after_solving(&mut doc, part);
        assert_eq!(motions.len(), 1, "{motions:?}");
        let Motion::Turn { axis, through, .. } = motions[0] else {
            panic!("a turn: {motions:?}")
        };
        assert!((axis[2].abs() - 1.0).abs() < 1e-3, "{axis:?}");
        assert!(
            (through[0] - 20.0).abs() < 1e-2 && (through[1] - 3.0).abs() < 1e-2,
            "{through:?}"
        );
        let origin = doc.body_placement(part).point([0.0; 3]);
        assert!((origin[2] - 5.0).abs() < 1e-3, "{origin:?}");
    }

    #[test]
    fn a_slider_leaves_only_the_slide_and_keeps_the_turn_it_was_made_with() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        doc.set_body_placement(
            part,
            BodyPlacement::new(Quat::from_rotation_z(0.5), Vec3::new(8.0, 9.0, 4.0)),
        );
        let joint = made(
            &doc,
            crate::JointTool::Slider,
            part,
            axis([0.0; 3], [0.0, 0.0, 1.0]),
            base,
            axis([20.0, 3.0, 0.0], [0.0, 0.0, 1.0]),
        );
        add_joint(&mut doc, part, joint);
        let motions = free_after_solving(&mut doc, part);
        assert_eq!(motions.len(), 1, "{motions:?}");
        let Motion::Slide { direction, .. } = motions[0] else {
            panic!("a slide: {motions:?}")
        };
        assert!((direction[2].abs() - 1.0).abs() < 1e-3, "{direction:?}");
        let placed = doc.body_placement(part);
        let x = placed.direction([1.0, 0.0, 0.0]);
        assert!(
            (x[1].atan2(x[0]) - 0.5).abs() < 1e-3,
            "still turned as made: {x:?}"
        );
        let origin = placed.point([0.0; 3]);
        assert!(
            close([origin[0], origin[1], 0.0], [20.0, 3.0, 0.0]),
            "{origin:?}"
        );
    }

    #[test]
    fn a_fixed_body_follows_the_other_and_is_left_nothing() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        tilted(&mut doc, part);
        let joint = made(
            &doc,
            crate::JointTool::Fixed,
            part,
            plane([0.0; 3], [0.0, 0.0, 1.0]),
            base,
            plane([0.0; 3], [0.0, 0.0, 1.0]),
        );
        add_joint(&mut doc, part, joint);
        assert!(solve(&doc).unwrap().is_empty(), "made where it sits");
        let turn = Quat::from_rotation_z(1.1);
        let step = Vec3::new(-5.0, 30.0, 2.0);
        doc.set_body_placement(base, BodyPlacement::new(turn, step));
        let before = doc.body_placement(part);
        let motions = free_after_solving(&mut doc, part);
        assert!(motions.is_empty(), "{motions:?}");
        let after = doc.body_placement(part);
        for p in [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 7.0, 3.0]] {
            let want = BodyPlacement::new(turn, step).point(before.point(p));
            assert!(close(after.point(p), want), "{p:?}");
        }
    }

    #[test]
    fn parallel_perpendicular_and_distance_hold_only_what_they_say() {
        let cases = [
            (JointKind::Parallel, 4),
            (JointKind::Perpendicular, 5),
            (JointKind::Distance { offset: 7.0 }, 5),
        ];
        for (kind, free) in cases {
            let mut doc = Document::new("t");
            let base = doc.create_body(None);
            let part = doc.create_body(None);
            tilted(&mut doc, part);
            add_joint(
                &mut doc,
                part,
                JointFeature {
                    second: None,
                    shape: Vec::new(),
                    ends: [0.0; 2],
                    names: [0; 2],
                    kind,
                    moving: plane([0.0; 3], [0.0, 0.0, -1.0]),
                    other_body: base,
                    fixed: plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]),
                },
            );
            let motions = free_after_solving(&mut doc, part);
            assert_eq!(motions.len(), free, "{kind:?}: {motions:?}");
        }
    }

    #[test]
    fn a_round_face_rests_on_a_flat_one_a_radius_up() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let part = doc.create_body(None);
        tilted(&mut doc, part);
        // A roller along the part's X, radius 4, on the base's top.
        add_joint(
            &mut doc,
            part,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Tangent { radius: 4.0 },
                moving: axis([0.0; 3], [1.0, 0.0, 0.0]),
                other_body: base,
                fixed: plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]),
            },
        );
        let motions = free_after_solving(&mut doc, part);
        assert_eq!(motions.len(), 4, "roll, spin and two slides: {motions:?}");
        let placed = doc.body_placement(part);
        assert!((placed.point([0.0; 3])[2] - 14.0).abs() < 1e-3);
        assert!(placed.direction([1.0, 0.0, 0.0])[2].abs() < 1e-4);
    }

    #[test]
    fn a_dragged_door_swings_on_its_hinge_toward_the_mouse() {
        let mut doc = Document::new("t");
        let frame = doc.create_body(None);
        let door = doc.create_body(None);
        add_joint(
            &mut doc,
            door,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Hinge {
                    offset: 0.0,
                    zero: DQuat::IDENTITY.to_array(),
                    drive: Default::default(),
                },
                moving: axis([0.0; 3], [0.0, 0.0, 1.0]),
                other_body: frame,
                fixed: axis([0.0; 3], [0.0, 0.0, 1.0]),
            },
        );
        // The handle, 10 mm out along X, pulled round to Y.
        for (body, placement) in drag(&doc, door, [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]) {
            doc.set_body_placement(body, placement);
        }
        assert!(holds(&doc), "the hinge holds");
        let handle = doc.body_placement(door).point([10.0, 0.0, 0.0]);
        assert!(
            close(handle, [0.0, 10.0, 0.0]),
            "swung round, not slid: {handle:?}"
        );
        // Pulled out of reach, it goes only as far as the hinge lets it.
        for (body, placement) in drag(&doc, door, [10.0, 0.0, 0.0], [-40.0, 0.0, 25.0]) {
            doc.set_body_placement(body, placement);
        }
        assert!(holds(&doc));
        let handle = doc.body_placement(door).point([10.0, 0.0, 0.0]);
        assert!(close(handle, [-10.0, 0.0, 0.0]), "{handle:?}");
        // The frame has no joints of its own: nothing drags it.
        assert!(drag(&doc, frame, [0.0; 3], [5.0, 5.0, 5.0]).is_empty());
    }

    fn ground(doc: &mut Document, body: BodyId) {
        let anchor = plane([0.0; 3], [0.0, 0.0, 1.0]);
        add_joint(
            doc,
            body,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Ground,
                moving: anchor,
                other_body: body,
                fixed: anchor,
            },
        );
    }

    #[test]
    fn a_grounded_body_s_joint_moves_the_body_at_its_other_end() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let lid = doc.create_body(None);
        // The base starts turned and off to the side.
        doc.set_body_placement(
            base,
            BodyPlacement::new(Quat::from_rotation_x(2.5), Vec3::new(30.0, 4.0, 9.0)),
        );
        ground(&mut doc, lid);
        // The lid's underside (z = 0) on the base's top (z = 5).
        add_joint(
            &mut doc,
            lid,
            mate(
                plane([0.0; 3], [0.0, 0.0, -1.0]),
                base,
                plane([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]),
                false,
                0.0,
            ),
        );
        apply(&mut doc);
        assert!(holds(&doc), "the mate holds");
        assert!(doc.body_placement(lid).is_identity(), "the lid stays put");
        let placed = doc.body_placement(base);
        assert!(
            (placed.point([0.0, 0.0, 5.0])[2]).abs() < 1e-3,
            "{placed:?}"
        );
        assert!(close(placed.direction([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]));

        // Both grounded, nothing can move: the joint is named, not held.
        ground(&mut doc, base);
        doc.set_body_placement(
            base,
            BodyPlacement::new(Quat::IDENTITY, Vec3::new(0.0, 0.0, -9.0)),
        );
        match solve(&doc) {
            Err(SolveError::Conflict { joints, .. }) => assert_eq!(joints, ["joint"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_limit_or_a_path_s_corner_leaves_the_motion_it_rests_on() {
        let mut doc = Document::new("t");
        let base = doc.create_body(None);
        let pin = doc.create_body(None);
        let rider = doc.create_body(None);
        // A pin in a hole, resting on the near end of its slide's limits.
        let mut kind = JointKind::align();
        if let JointKind::Align { slide, .. } = &mut kind {
            slide.limits = Some([0.0, 3.0]);
        }
        add_joint(
            &mut doc,
            pin,
            JointFeature {
                second: None,
                shape: Vec::new(),
                ends: [0.0; 2],
                names: [0; 2],
                kind,
                moving: axis([0.0; 3], [0.0, 0.0, 1.0]),
                other_body: base,
                fixed: axis([5.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            },
        );
        let motions = free_after_solving(&mut doc, pin);
        assert_eq!(motions.len(), 2, "{motions:?}");
        assert!(
            motions
                .iter()
                .any(|m| matches!(m, Motion::Slide { at_limit: true, .. })),
            "{motions:?}"
        );

        // A point on a square's corner runs on along the next side, and
        // turns every way.
        add_joint(
            &mut doc,
            rider,
            JointFeature {
                second: None,
                shape: vec![
                    [0.0, 0.0, 10.0],
                    [10.0, 0.0, 10.0],
                    [10.0, 10.0, 10.0],
                    [0.0, 10.0, 10.0],
                    [0.0, 0.0, 10.0],
                ],
                ends: [0.0; 2],
                names: [0; 2],
                kind: JointKind::Path,
                moving: Anchor::Point { point: [0.0; 3] },
                other_body: base,
                fixed: Anchor::Point {
                    point: [10.0, 0.0, 10.0],
                },
            },
        );
        doc.set_body_placement(
            rider,
            BodyPlacement::new(Quat::IDENTITY, Vec3::new(10.0, 0.0, 10.0)),
        );
        let motions = free_after_solving(&mut doc, rider);
        assert_eq!(motions.len(), 4, "{motions:?}");
        assert!(
            motions.iter().any(|m| matches!(
                m,
                Motion::Slide { direction, at_limit: false } if (direction[1].abs() - 1.0).abs() < 1e-3
            )),
            "{motions:?}"
        );
    }
}
