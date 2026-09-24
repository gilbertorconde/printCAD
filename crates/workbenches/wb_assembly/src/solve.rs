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

use crate::joint::{JOINT_KIND, JointFeature, JointKind, Rigid};

/// A joint as the solver reads it.
#[derive(Debug, Clone)]
pub struct Joint {
    pub id: FeatureId,
    pub name: String,
    /// The body it moves.
    pub body: BodyId,
    pub feature: JointFeature,
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
    /// joints are the ones left apart.
    Conflict { body: BodyId, joints: Vec<String> },
}

/// The joints the solver reads: both bodies there and different, or a
/// ground.
fn usable(document: &Document) -> Vec<Joint> {
    let exists = |body: BodyId| document.bodies().iter().any(|b| b.id == body);
    joints(document)
        .into_iter()
        .filter(|j| {
            exists(j.body)
                && (j.feature.kind == JointKind::Ground
                    || (exists(j.feature.other_body) && j.feature.other_body != j.body))
        })
        .collect()
}

/// The bodies the solver moves: those that own a joint and are not
/// grounded. Every other body stays where it is.
fn free_bodies(all: &[Joint]) -> BTreeSet<BodyId> {
    let grounded: BTreeSet<BodyId> = all
        .iter()
        .filter(|j| j.feature.kind == JointKind::Ground)
        .map(|j| j.body)
        .collect();
    all.iter()
        .filter(|j| j.feature.kind != JointKind::Ground)
        .map(|j| j.body)
        .filter(|b| !grounded.contains(b))
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
    let starts: HashMap<BodyId, Rigid> = document
        .bodies()
        .iter()
        .map(|b| (b.id, Rigid::from(b.placement)))
        .collect();
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

    // One body at a time, against what is placed.
    let mut placed: BTreeSet<BodyId> = placements
        .keys()
        .copied()
        .filter(|b| !free.contains(b))
        .collect();
    let mut pending: Vec<BodyId> = free.iter().copied().collect();
    while !pending.is_empty() {
        let holds_to = |body: BodyId| -> Vec<&Joint> {
            holding
                .iter()
                .copied()
                .filter(|j| j.body == body && placed.contains(&j.feature.other_body))
                .collect()
        };
        // Ready: every joint holds to something placed. In a ring none is;
        // the one with the most joints placed goes first.
        let next = pending
            .iter()
            .copied()
            .find(|b| holding.iter().filter(|j| j.body == *b).count() == holds_to(*b).len())
            .or_else(|| pending.iter().copied().max_by_key(|b| holds_to(*b).len()))
            .expect("pending is not empty");
        let joints_now = holds_to(next);
        if !joints_now.is_empty() {
            let at = place(placements[&next], &joints_now, &placements);
            placements.insert(next, at);
        }
        placed.insert(next);
        pending.retain(|b| *b != next);
    }

    // Every free body together, against every joint.
    refine(&mut placements, &free, &holding);

    // Joints still apart: the first free body with one, and every one at
    // either end of it.
    for body in &free {
        let left_apart: Vec<String> = holding
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
        if !left_apart.is_empty() {
            return Err(SolveError::Conflict {
                body: *body,
                joints: left_apart,
            });
        }
    }
    let mut moved = Vec::new();
    for body in &free {
        // Rounding is no move: an assembly that already holds records
        // nothing when solved again.
        let before = BodyPlacement::from(starts[body]);
        let after = BodyPlacement::from(placements[body]);
        if !after.after(&before.inverse()).is_identity() {
            moved.push((*body, after));
        }
    }
    Ok(moved)
}

/// Whether dragging `body` moves it: it has joints and is not grounded.
pub fn draggable(document: &Document, body: BodyId) -> bool {
    free_bodies(&usable(document)).contains(&body)
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
    let starts: HashMap<BodyId, Rigid> = document
        .bodies()
        .iter()
        .map(|b| (b.id, Rigid::from(b.placement)))
        .collect();
    let mut placements = starts.clone();
    let pull = Pull {
        body,
        point: DVec3::from_array(point.map(f64::from)),
        target: DVec3::from_array(target.map(f64::from)),
    };
    refine_pulled(&mut placements, &free, &holding, Some(&pull));
    // The pull traded a little of each joint for reach; let them close.
    refine(&mut placements, &free, &holding);
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

/// Every joint's residuals with the bodies at `placements`.
fn residuals_of(placements: &HashMap<BodyId, Rigid>, joints: &[&Joint]) -> Vec<f64> {
    let mut r = Vec::new();
    for j in joints {
        j.feature.residuals(
            &placements[&j.body],
            &placements[&j.feature.other_body],
            &mut r,
        );
    }
    r
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
    pull: Option<&Pull>,
) -> Vec<f64> {
    let mut r = residuals_of(placements, joints);
    if let Some(pull) = pull {
        let at = &placements[&pull.body];
        let apart = at.rotation * pull.point + at.translation - pull.target;
        r.extend(apart.to_array().map(|c| c * PULL_WEIGHT));
    }
    r
}

/// Damped least squares over every free body's six freedoms at once.
fn refine(placements: &mut HashMap<BodyId, Rigid>, free: &BTreeSet<BodyId>, joints: &[&Joint]) {
    refine_pulled(placements, free, joints, None);
}

/// [`refine`], with a body's point pulled toward a target as well.
fn refine_pulled(
    placements: &mut HashMap<BodyId, Rigid>,
    free: &BTreeSet<BodyId>,
    joints: &[&Joint],
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
    let mut r0 = residuals_with(placements, joints, pull);
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
            let rp = residuals_with(&apply(placements, &pivots, &x), joints, pull);
            x[k] = -h;
            let rm = residuals_with(&apply(placements, &pivots, &x), joints, pull);
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
            let rt = residuals_with(&trial, joints, pull);
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
    /// A turn about a line through `through`, along `axis`.
    Turn { axis: [f64; 3], through: [f64; 3] },
    /// A slide along `direction`.
    Slide { direction: [f64; 3] },
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
        match self {
            Motion::Turn { axis, .. } => format!("turn about {}", name(*axis)),
            Motion::Slide { direction } => format!("slide along {}", name(*direction)),
        }
    }
}

/// What each jointed body may still do where it sits: the motions its
/// joints leave open, the bodies around it held still. A body with none
/// is fully placed.
pub fn freedom(document: &Document) -> Vec<(BodyId, Vec<Motion>)> {
    // A limit stops a motion only at its ends: the motion is still there.
    let all: Vec<Joint> = usable(document)
        .into_iter()
        .map(|mut j| {
            if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } =
                &mut j.feature.kind
            {
                drive.limits = None;
            }
            j
        })
        .collect();
    let free = free_bodies(&all);
    let holding: Vec<&Joint> = all
        .iter()
        .filter(|j| j.feature.kind != JointKind::Ground)
        .collect();
    let placements: HashMap<BodyId, Rigid> = document
        .bodies()
        .iter()
        .map(|b| (b.id, Rigid::from(b.placement)))
        .collect();
    let pivots = pivots(&placements, &free, &holding);
    free.iter()
        .map(|body| {
            let own: Vec<&Joint> = holding
                .iter()
                .copied()
                .filter(|j| j.body == *body || j.feature.other_body == *body)
                .collect();
            // This body's six columns, turns first, by central
            // differences: both sides go through the same steps (a stored
            // rotation is normalised on the way), so nothing but the
            // motion differs.
            let h = 1e-6;
            let at = |x: [f64; 6]| {
                let mut moved = placements.clone();
                moved.insert(
                    *body,
                    stepped(
                        placements[body],
                        pivots[body],
                        DVec3::new(x[0], x[1], x[2]),
                        DVec3::new(x[3], x[4], x[5]),
                    ),
                );
                residuals_of(&moved, &own)
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
                        }
                    } else {
                        Motion::Slide {
                            direction: tidy(slide.normalize_or_zero()),
                        }
                    }
                })
                .collect();
            (*body, motions)
        })
        .collect()
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

/// The placement nearest `start` at which `joints` hold, the bodies they
/// hold against sitting at `others`.
fn place(start: Rigid, joints: &[&Joint], others: &HashMap<BodyId, Rigid>) -> Rigid {
    // Turns pivot about the moving anchors' middle, so a turn does not
    // throw the body across the room.
    let pivot = {
        let points: Vec<DVec3> = joints
            .iter()
            .map(|j| j.feature.moving.placed(&start).0)
            .collect();
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
        let (_, dm) = joint.feature.moving.placed(&current);
        let (_, df) = joint
            .feature
            .fixed
            .placed(&others[&joint.feature.other_body]);
        let target = match joint.feature.kind {
            JointKind::Mate { flip: false, .. } => -df,
            JointKind::Mate { flip: true, .. } => df,
            // An axis has no way round: the nearer of its two directions.
            JointKind::Align | JointKind::Hinge { .. } | JointKind::Parallel => {
                if dm.dot(df) < 0.0 { -df } else { df }
            }
            // A held turn is the whole rotation, not only a direction.
            JointKind::Slider { turn, .. } | JointKind::Fixed { turn, .. } => {
                let fixed = &others[&joint.feature.other_body];
                let want = (fixed.rotation * crate::joint::quat(turn)).normalize();
                return Some((want * current.rotation.inverse()).normalize());
            }
            JointKind::Angle { .. }
            | JointKind::Ground
            | JointKind::Perpendicular
            | JointKind::Distance { .. }
            | JointKind::Tangent { .. } => return None,
        };
        (dm.length_squared() > 0.0 && target.length_squared() > 0.0)
            .then(|| DQuat::from_rotation_arc(dm, target))
    });
    // With only angles, the first one turns the body to its angle, in the
    // plane of the two normals (or about any line square to them when they
    // start parallel, where the angle has no slope to follow).
    let first_turn = first_turn.or_else(|| {
        joints.iter().find_map(|joint| {
            let JointKind::Angle { degrees } = joint.feature.kind else {
                return None;
            };
            let (_, dm) = joint.feature.moving.placed(&current);
            let (_, df) = joint
                .feature
                .fixed
                .placed(&others[&joint.feature.other_body]);
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
            joint
                .feature
                .residuals(p, &others[&joint.feature.other_body], &mut r);
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
                kind: JointKind::Align,
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
            Err(SolveError::Conflict { body, joints }) => {
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
            kind: JointKind::Mate { flip, offset },
            moving,
            other_body: other,
            fixed,
        }
    }

    fn holds(doc: &Document) -> bool {
        let placements: HashMap<BodyId, Rigid> = doc
            .bodies()
            .iter()
            .map(|b| (b.id, Rigid::from(b.placement)))
            .collect();
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
                kind: JointKind::Align,
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
        let Motion::Turn { axis, through } = motions[0] else {
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
        let Motion::Turn { axis, through } = motions[0] else {
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
        let Motion::Slide { direction } = motions[0] else {
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
}
