//! A mesh body's history, welded once and edited in order. Every vertex
//! keeps the start vertices it came from, through reduction too: a marking
//! selects a vertex when it marks all of them, so a collapsed vertex is
//! marked only when everything merged into it was, and a vertex an edit
//! added, with none, never is.

use kernel_api::{
    KernelError, KernelResult, MeshChainAnswer, MeshFairing, MeshOp, MeshRegion, MeshStep,
    MeshStepReport, TriMesh,
};
use ogeom::math::{Direction, Plane};
use ogeom::mesh::{Cap, Editable, Fairing, Fill, Side, Smooth, Target};
use ogeom::topo::Triangulation;
use std::collections::{HashMap, HashSet};

use crate::convert::{array, point, vector};
use crate::mesh_intake::{Degenerate, Intake, Weld, intake};

/// Apply each step atomically; a refusal leaves the preceding mesh.
pub(crate) fn build(start: &TriMesh, ops: &[MeshOp]) -> KernelResult<MeshChainAnswer> {
    let Intake {
        tri,
        vertex: welded,
        ..
    } = intake(start, Weld::InPlace, Degenerate::Drop)?;
    let mut mesh =
        Editable::from(&tri, 0.0).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
    let mut sources: Vec<_> = (0..start.positions.len() as u32).map(Some).collect();
    let mut answer = MeshChainAnswer::default();
    for (k, op) in ops.iter().enumerate() {
        let before = mesh.clone();
        match step(&mut mesh, op, &welded, &mut sources) {
            Ok((note, report)) => answer.steps.push(MeshStep {
                faces: (0..mesh.face_count() as u32)
                    .filter(|f| mesh.face(*f).is_some())
                    .count(),
                note,
                report,
            }),
            Err(why) => {
                mesh = before;
                answer.failed = Some((k, why));
                break;
            }
        }
    }
    answer.mesh = crate::mesh::display_mesh(&mesh.to_triangulation(), &crate::mesh::Paint::None);
    let at = source_positions(&mesh, &sources);
    answer.sources = answer
        .mesh
        .positions
        .iter()
        .map(|p| at.get(&p.map(f32::to_bits)).copied().flatten())
        .collect();
    Ok(answer)
}

pub(crate) fn triangulation(mesh: &TriMesh) -> Result<Triangulation, String> {
    let tri = intake(mesh, Weld::InPlace, Degenerate::Drop)
        .map_err(|e| e.to_string())?
        .tri;
    Editable::from(&tri, 0.0)
        .map(|m| m.to_triangulation())
        .map_err(|e| e.to_string())
}

fn vertices_in(
    mesh: &Editable,
    region: &MeshRegion,
    welded: &[u32],
    sources: &[Option<u32>],
) -> Vec<bool> {
    let marked: HashSet<u32> = match region {
        MeshRegion::Vertices(list) => list
            .iter()
            .filter_map(|v| welded.get(*v as usize).copied())
            .collect(),
        _ => HashSet::new(),
    };
    (0..mesh.vertex_count() as u32)
        .map(|v| match region {
            MeshRegion::All => true,
            MeshRegion::Vertices(_) => {
                let from = mesh.source_vertices(v);
                !from.is_empty()
                    && from.iter().all(|s| {
                        sources
                            .get(*s as usize)
                            .copied()
                            .flatten()
                            .is_some_and(|s| marked.contains(&s))
                    })
            }
            MeshRegion::Box { min, max } => mesh.position(v).is_some_and(|p| {
                let p = array(p);
                (0..3).all(|k| p[k] >= min[k] && p[k] <= max[k])
            }),
            MeshRegion::Ball { centre, radius } => mesh
                .position(v)
                .is_some_and(|p| (p - point(centre)).magnitude() <= *radius),
        })
        .collect()
}

fn faces_in(
    mesh: &Editable,
    region: &MeshRegion,
    welded: &[u32],
    sources: &[Option<u32>],
) -> Vec<u32> {
    let inside = vertices_in(mesh, region, welded, sources);
    (0..mesh.face_count() as u32)
        .filter(|f| {
            mesh.face(*f)
                .is_some_and(|t| t.iter().all(|v| inside[*v as usize]))
        })
        .collect()
}

fn source_positions(mesh: &Editable, sources: &[Option<u32>]) -> HashMap<[u32; 3], Option<u32>> {
    (0..mesh.vertex_count() as u32)
        .filter_map(|v| {
            let p = mesh.position(v)?;
            let source = mesh
                .source_vertex(v)
                .and_then(|s| sources.get(s as usize).copied().flatten());
            Some((
                [p.x as f32, p.y as f32, p.z as f32].map(f32::to_bits),
                source,
            ))
        })
        .collect()
}

fn step(
    mesh: &mut Editable,
    op: &MeshOp,
    welded: &[u32],
    sources: &mut Vec<Option<u32>>,
) -> Result<(String, Option<MeshStepReport>), String> {
    let mut measured = None;
    let note = match op {
        MeshOp::RemoveIslands { smaller_than } => {
            let pieces = mesh.components();
            let total = pieces.len();
            let small: Vec<_> = pieces
                .into_iter()
                .filter(|c| c.bounds.diagonal() < *smaller_than)
                .collect();
            if small.len() == total && total > 0 {
                return Err("every piece is smaller than that: nothing would be left".into());
            }
            let faces: Vec<u32> = small.iter().flat_map(|c| c.faces.iter().copied()).collect();
            mesh.remove_faces(&faces);
            format!(
                "{} of {total} pieces removed, {} faces",
                small.len(),
                faces.len()
            )
        }
        MeshOp::Delete { region } => {
            let faces = faces_in(mesh, region, welded, sources);
            if faces.is_empty() {
                return Err("no face lies in the region".into());
            }
            mesh.remove_faces(&faces);
            format!("{} faces removed", faces.len())
        }
        MeshOp::Flip { region } => {
            let faces = faces_in(mesh, region, welded, sources);
            mesh.flip_faces(&faces);
            format!("{} faces turned over", faces.len())
        }
        MeshOp::Fill {
            holes_up_to,
            near,
            fairing,
        } => {
            if !holes_up_to.is_finite()
                || *holes_up_to <= 0.0
                || near.is_some_and(|p| p.iter().any(|v| !v.is_finite()))
            {
                return Err("hole size must be positive and the rim point finite".into());
            }
            let mut rims = mesh.rims();
            if let Some(p) = near {
                let point = point(p);
                let distance = |rim: &ogeom::mesh::Rim| {
                    rim.edges
                        .iter()
                        .filter_map(|h| {
                            let [a, b] = mesh.edge(*h)?;
                            let (a, b) = (mesh.position(a)?, mesh.position(b)?);
                            let ab = b - a;
                            let t = ((point - a).dot(ab) / ab.dot(ab).max(f64::MIN_POSITIVE))
                                .clamp(0.0, 1.0);
                            Some((point - (a + ab * t)).magnitude())
                        })
                        .fold(f64::INFINITY, f64::min)
                };
                rims.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
                rims.truncate(1);
            }
            rims.retain(|r| r.span <= *holes_up_to);
            if near.is_some() && rims.is_empty() {
                return Err("no hole near that point fits the size limit".into());
            }
            let mut count = 0;
            for rim in rims {
                let report = mesh
                    .fill(
                        rim.id(),
                        &Fill {
                            fairing: match fairing {
                                MeshFairing::Curved => Fairing::Curved,
                                MeshFairing::Flat => Fairing::Flat,
                            },
                            refine: true,
                        },
                    )
                    .map_err(|e| e.to_string())?;
                count += report.loops;
            }
            format!("{count} holes filled")
        }
        MeshOp::Smooth {
            region,
            bumps_up_to,
            move_at_most,
            keep_edges_deg,
        } => {
            let mask = vertices_in(mesh, region, welded, sources);
            let report = mesh
                .smooth(&Smooth {
                    size: *bumps_up_to,
                    max_move: *move_at_most,
                    keep_edges: keep_edges_deg.map(f64::to_radians),
                    region: (!matches!(region, MeshRegion::All)).then_some(mask.as_slice()),
                })
                .map_err(|e| e.to_string())?;
            measured = Some(MeshStepReport::Smooth {
                max: report.max,
                rms: report.rms,
            });
            format!("{} vertices held at the movement limit", report.limited)
        }
        MeshOp::Offset { region, distance } => {
            let mask = vertices_in(mesh, region, welded, sources);
            let report = mesh
                .offset(
                    *distance,
                    (!matches!(region, MeshRegion::All)).then_some(mask.as_slice()),
                )
                .map_err(|e| e.to_string())?;
            format!(
                "{} folds repaired, {} vertices merged",
                report.folded, report.merged
            )
        }
        MeshOp::Boolean {
            operand,
            operation,
            chord,
        } => {
            let (tri, report) = crate::mesh_solids::boolean(mesh, operand, *operation, *chord)?;
            *mesh = Editable::from(&tri, 0.0).map_err(|e| e.to_string())?;
            *sources = vec![None; tri.positions.len()];
            measured = Some(report);
            "closed mesh boolean".into()
        }
        MeshOp::Shell { thickness, side } => {
            let (tri, report) = crate::mesh_solids::shell(mesh, *thickness, *side)?;
            *mesh = Editable::from(&tri, 0.0).map_err(|e| e.to_string())?;
            *sources = vec![None; tri.positions.len()];
            measured = Some(report);
            "closed wall".into()
        }
        MeshOp::Reduce {
            tolerance,
            triangle_limit,
        } => {
            let result = mesh
                .simplify(
                    Target::Verified {
                        tolerance: *tolerance,
                        share: 0.999,
                        max_triangles: Some(*triangle_limit),
                    },
                    crate::tess::tolerances(),
                )
                .map_err(|e| e.to_string())?;
            let d = result
                .deviation
                .ok_or("the reduction returned no verification")?;
            measured = Some(MeshStepReport::Reduce {
                within_share: d.within_share,
                p999: d.distance_at_share,
                furthest: array(d.furthest.0),
                max: d.furthest.1,
                rms: d.rms,
                target_met: result.target_met,
            });
            if !result.target_met {
                "triangle limit could not be reached safely"
            } else if d.distance_at_share > *tolerance {
                "triangle limit reached; tolerance exceeded"
            } else {
                "triangle limit and tolerance met"
            }
            .into()
        }
        MeshOp::Cut {
            origin,
            normal,
            keep_above,
            close,
            bridge,
        } => {
            let n = Direction::new(vector(normal), crate::tess::tolerances())
                .map_err(|e| format!("the plane has no direction: {e}"))?;
            let plane = Plane::through(point(origin), n);
            let report = mesh
                .cut(
                    &plane,
                    if *keep_above {
                        Side::Above
                    } else {
                        Side::Below
                    },
                    close.then_some(Cap {
                        bridge: bridge.max(0.0),
                    }),
                )
                .map_err(|e| e.to_string())?;
            measured = Some(MeshStepReport::Cut {
                capped_loops: report.capped_loops,
                bridged: report.bridged,
                open_loops: report.open,
            });
            format!(
                "{} loops capped ({} bridged), {} left open",
                report.capped_loops, report.bridged, report.open
            )
        }
    };
    Ok((note, measured))
}
