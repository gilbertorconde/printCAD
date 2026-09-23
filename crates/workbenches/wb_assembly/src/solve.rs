//! Placing bodies so their joints hold.
//!
//! A joint moves the body it belongs to against another. Bodies are solved
//! one at a time, each after every body it is joined to, so a chain settles
//! from its grounded end: a body with no joints of its own stays where it is.
//! Each body is moved as little as its joints allow, by damped least squares
//! over its six degrees of freedom from where it sits now, so what a joint
//! leaves free (a slide along a mated face, a turn about an aligned axis)
//! keeps its current value.

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
            let feature = serde_json::from_value(node.data.clone()).ok()?;
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
    /// Bodies joined to each other in a ring: each would have to wait for
    /// the next.
    Loop(Vec<BodyId>),
    /// A body's joints ask for more than one place at once; the named
    /// joints are the ones left apart.
    Conflict { body: BodyId, joints: Vec<String> },
}

/// The placement every jointed body takes. Bodies whose joints already
/// hold are left out, as are bodies without joints.
pub fn solve(document: &Document) -> Result<Vec<(BodyId, BodyPlacement)>, SolveError> {
    let all = joints(document);
    let exists = |body: BodyId| document.bodies().iter().any(|b| b.id == body);
    let mut by_body: BTreeMap<BodyId, Vec<&Joint>> = BTreeMap::new();
    for joint in &all {
        if exists(joint.body)
            && exists(joint.feature.other_body)
            && joint.body != joint.feature.other_body
        {
            by_body.entry(joint.body).or_default().push(joint);
        }
    }
    let order = order(&by_body)?;
    let mut placements: HashMap<BodyId, Rigid> = document
        .bodies()
        .iter()
        .map(|b| (b.id, Rigid::from(b.placement)))
        .collect();
    let mut moved = Vec::new();
    for body in order {
        let body_joints = &by_body[&body];
        let start = placements[&body];
        let placed = place(start, body_joints, &placements);
        let left_apart: Vec<String> = body_joints
            .iter()
            .filter(|j| worst(&j.feature, &placed, &placements[&j.feature.other_body]) > HOLDS_MM)
            .map(|j| j.name.clone())
            .collect();
        if !left_apart.is_empty() {
            return Err(SolveError::Conflict {
                body,
                joints: left_apart,
            });
        }
        // Rounding is no move: an assembly that already holds records
        // nothing when solved again.
        let before = BodyPlacement::from(start);
        let after = BodyPlacement::from(placed);
        if !after.after(&before.inverse()).is_identity() {
            moved.push((body, after));
        }
        placements.insert(body, placed);
    }
    Ok(moved)
}

/// How closely a joint must hold, in millimetres (and the equivalent turn).
pub const HOLDS_MM: f64 = 1e-3;

fn worst(joint: &JointFeature, moving: &Rigid, fixed: &Rigid) -> f64 {
    let mut r = Vec::new();
    joint.residuals(moving, fixed, &mut r);
    r.iter().fold(0.0, |m: f64, v| m.max(v.abs()))
}

/// Jointed bodies after every body they are joined to.
fn order(by_body: &BTreeMap<BodyId, Vec<&Joint>>) -> Result<Vec<BodyId>, SolveError> {
    let mut done: BTreeSet<BodyId> = BTreeSet::new();
    let mut out = Vec::new();
    let mut pending: Vec<BodyId> = by_body.keys().copied().collect();
    while !pending.is_empty() {
        let ready: Vec<BodyId> = pending
            .iter()
            .copied()
            .filter(|body| {
                by_body[body].iter().all(|j| {
                    let other = j.feature.other_body;
                    !by_body.contains_key(&other) || done.contains(&other)
                })
            })
            .collect();
        if ready.is_empty() {
            return Err(SolveError::Loop(pending));
        }
        for body in ready {
            done.insert(body);
            out.push(body);
        }
        pending.retain(|b| !done.contains(b));
    }
    Ok(out)
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
    if let Some(joint) = joints.first() {
        let (_, dm) = joint.feature.moving.placed(&current);
        let (_, df) = joint
            .feature
            .fixed
            .placed(&others[&joint.feature.other_body]);
        let target = match joint.feature.kind {
            JointKind::Mate { flip: false, .. } => -df,
            JointKind::Mate { flip: true, .. } => df,
            // An axis has no way round: the nearer of its two directions.
            JointKind::Align => {
                if dm.dot(df) < 0.0 {
                    -df
                } else {
                    df
                }
            }
        };
        if dm.length_squared() > 0.0 && target.length_squared() > 0.0 {
            current = turned(current, DQuat::from_rotation_arc(dm, target));
        }
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
    use crate::joint::{Anchor, JointKind};
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
    fn a_chain_settles_from_its_grounded_end_and_a_loop_is_refused() {
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
        // a on c closes a ring.
        add_joint(&mut doc, a, mate(c));
        assert!(matches!(solve(&doc), Err(SolveError::Loop(_))));
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
}
