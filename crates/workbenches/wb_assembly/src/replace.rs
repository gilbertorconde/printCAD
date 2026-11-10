//! Replacing a component: another body takes a body's place and its
//! joints. The new body is put where the old one sits; each joint end on
//! the old body is found again on the new one, on the face of the same
//! kind that lies nearest (a flat face facing the same way, a round face
//! on a parallel axis), and the old body is hidden.

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature};
use kernel_api::FaceSurface;

use crate::joint::{Anchor, JointKind};
use crate::solve::joints;

/// How closely two directions must agree to be taken as parallel.
const PARALLEL: f32 = 0.999;

/// What a replacement did with the old body's joints.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Replaced {
    /// Joints whose ends were found on the new body.
    pub kept: Vec<String>,
    /// Joints with an end no face of the new body matches: they go to the
    /// new body as they were, and may be left apart.
    pub unmatched: Vec<String>,
}

/// The new body's faces as anchors could take them, in its own frame.
fn anchors_of(document: &Document, body: BodyId) -> Vec<Anchor> {
    let Some((mesh, _)) = document.local_geometry(body) else {
        return Vec::new();
    };
    mesh.face_surfaces
        .iter()
        .filter_map(|surface| match *surface {
            FaceSurface::Plane { origin, normal } => Some(Anchor::Plane {
                point: origin,
                normal,
            }),
            FaceSurface::Sphere { center, .. } => Some(Anchor::Point { point: center }),
            other => other
                .axis()
                .map(|(point, direction)| Anchor::Axis { point, direction }),
        })
        .collect()
}

/// The anchor among `candidates` of the same kind as `old`, parallel to
/// it, that lies nearest: a flat face's plane nearest its point, an axis
/// nearest its line. Its point is the one nearest `old`'s, its direction
/// turned to agree with `old`'s.
pub(crate) fn nearest_like(old: &Anchor, candidates: &[Anchor]) -> Option<Anchor> {
    let (p, d) = old.parts();
    let (p, d) = (p.as_vec3(), d.as_vec3());
    // A point is found again as the nearest point.
    if let Anchor::Point { .. } = old {
        return candidates
            .iter()
            .filter(|c| matches!(c, Anchor::Point { .. }))
            .min_by(|a, b| {
                let da = (a.parts().0.as_vec3() - p).length();
                let db = (b.parts().0.as_vec3() - p).length();
                da.total_cmp(&db)
            })
            .copied();
    }
    candidates
        .iter()
        .filter_map(|c| {
            let (q, e) = c.parts();
            let (q, e) = (q.as_vec3(), e.as_vec3());
            let along = e.dot(d);
            // A flat face faces the same way; an axis may run either way.
            let flat = matches!(old, Anchor::Plane { .. });
            if (flat && along < PARALLEL) || along.abs() < PARALLEL {
                return None;
            }
            let e = e * along.signum();
            match (old, c) {
                (Anchor::Plane { .. }, Anchor::Plane { .. }) => {
                    let off = (p - q).dot(e);
                    let on = p - e * off;
                    Some((
                        off.abs(),
                        Anchor::Plane {
                            point: on.to_array(),
                            normal: e.to_array(),
                        },
                    ))
                }
                (Anchor::Axis { .. }, Anchor::Axis { .. }) => {
                    let on = q + e * (p - q).dot(e);
                    Some((
                        (p - on).length(),
                        Anchor::Axis {
                            point: on.to_array(),
                            direction: e.to_array(),
                        },
                    ))
                }
                _ => None,
            }
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, anchor)| anchor)
}

/// `new` takes `old`'s place and joints; `old` is hidden.
pub fn replace(document: &mut Document, old: BodyId, new: BodyId) -> Result<Replaced, String> {
    if old == new {
        return Err("a body cannot replace itself".into());
    }
    for body in [old, new] {
        if document.body(body).is_none() {
            return Err("both must be bodies of this document".into());
        }
    }
    let placement = document.body_placement(old);
    document.set_body_placement(new, placement);
    let faces = anchors_of(document, new);
    let mut report = Replaced::default();
    let affected: Vec<crate::Joint> = joints(document)
        .into_iter()
        .filter(|j| {
            j.body == old || (j.feature.other_body == old && j.feature.kind != JointKind::Ground)
        })
        .collect();
    for joint in affected {
        let mut feature = joint.feature.clone();
        let mut matched = true;
        let mut refind = |anchor: &mut Anchor| match nearest_like(anchor, &faces) {
            Some(found) => *anchor = found,
            None => matched = false,
        };
        if joint.body == old && feature.kind != JointKind::Ground {
            refind(&mut feature.moving);
        }
        if feature.other_body == old {
            refind(&mut feature.fixed);
            feature.other_body = new;
        }
        if joint.body == old {
            document.move_feature_to_body(joint.id, new)?;
        }
        document
            .update_feature_data(joint.id, feature.to_json())
            .map_err(|e| e.to_string())?;
        document.clear_feature_dirty(joint.id);
        if matched {
            report.kept.push(joint.name);
        } else {
            report.unmatched.push(joint.name);
        }
    }
    // Rigid groups the old body is in take the new one.
    let groups: Vec<(FeatureId, crate::RigidGroup)> = crate::group::groups(document)
        .into_iter()
        .filter(|(_, _, g)| g.bodies().contains(&old))
        .map(|(id, _, g)| (id, g))
        .collect();
    for (id, mut group) in groups {
        for member in &mut group.members {
            if member.body == old {
                member.body = new;
            }
        }
        document
            .update_feature_data(id, group.to_json())
            .map_err(|e| e.to_string())?;
        document.clear_feature_dirty(id);
    }
    document.set_body_visible(old, false);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_parallel_face_of_the_same_kind_is_found() {
        let old = Anchor::Plane {
            point: [3.0, 4.0, 10.0],
            normal: [0.0, 0.0, 1.0],
        };
        let faces = [
            Anchor::Plane {
                point: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, -1.0],
            },
            Anchor::Plane {
                point: [0.0, 0.0, 12.0],
                normal: [0.0, 0.0, 1.0],
            },
            Anchor::Plane {
                point: [0.0, 0.0, 10.0],
                normal: [1.0, 0.0, 0.0],
            },
            Anchor::Axis {
                point: [3.0, 4.0, 0.0],
                direction: [0.0, 0.0, 1.0],
            },
        ];
        let Some(Anchor::Plane { point, normal }) = nearest_like(&old, &faces) else {
            panic!("a flat face is found")
        };
        assert_eq!(normal, [0.0, 0.0, 1.0]);
        assert!((point[2] - 12.0).abs() < 1e-5 && (point[0] - 3.0).abs() < 1e-5);
        let axis = Anchor::Axis {
            point: [5.0, 4.0, 3.0],
            direction: [0.0, 0.0, -1.0],
        };
        let Some(Anchor::Axis { point, direction }) = nearest_like(&axis, &faces) else {
            panic!("an axis is found")
        };
        assert_eq!(direction, [0.0, 0.0, -1.0], "turned to agree");
        assert!((point[0] - 3.0).abs() < 1e-5 && (point[2] - 3.0).abs() < 1e-5);
    }
}
