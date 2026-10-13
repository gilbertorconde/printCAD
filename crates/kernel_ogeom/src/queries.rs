//! The kernel's answers to the questions a workbench asks while it runs
//! (`kernel_api::KernelQueries`).

use kernel_api::{
    CentreLine, FaceProbe, KernelError, KernelQueries, KernelResult, MedialPath, MedialRegion,
    Narrowest, Overlap, PathFrame, PathFrames, ProbeAnswer, Profile, ProfilePlane, ProjectedEdge,
    ShapeProbe, TessellationSettings, TriMesh,
};
use ogeom::algo::{MedialGraph, linear_properties, medial_graph, volume_properties};
use ogeom::algo::{ProjectedCurve, distance_between_shapes, project_edge_onto_plane};
use ogeom::core::Tolerances;
use ogeom::geom::Curve3d as _;
use ogeom::math::{Direction, Frame, Plane, Point, Vector};
use ogeom::mesh::{Deflection, discretize};
use ogeom::topo::{EdgeRepr, Model, NodeData, Orientation, Shape, ShapeType, explore_unique};

use crate::ops::dressup::nearest_of;
use crate::tess;

/// The ogeom kernel's answers. It holds nothing, so one serves every caller.
pub struct OgeomQueries;

/// The instance the application hands its workbenches.
pub static QUERIES: OgeomQueries = OgeomQueries;

fn other(message: impl std::fmt::Display) -> KernelError {
    KernelError::Other(anyhow::anyhow!("{message}"))
}

/// How far the foot found for a point asked its curvature may stand from
/// it and still be taken: the points lie on the surface, so a foot further
/// off is a wrong one (a ball's pole beside it) and the surface is searched
/// again, more finely (mm).
const CURVATURE_REACH: f64 = 1e-3;

/// Seeds per direction for each search of a surface afresh, coarse first.
const CURVATURE_SEEDS: [usize; 2] = [16, 64];

/// A shared solid smaller than this, in mm³, is taken for two faces that
/// touch: what a boolean of flush faces leaves behind.
const TOUCHING_MM3: f64 = 1e-6;

impl KernelQueries for OgeomQueries {
    fn continuity(&self, brep: &[u8]) -> KernelResult<Vec<kernel_api::EdgeContinuity>> {
        // Stations along each edge where the faces are compared.
        const STATIONS: usize = 9;
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        // The kernel compares faces in their own coordinates: a placed face
        // (a pad's far end is its sketch's face moved along) is restated
        // where it stands first, or its edges are measured where it was.
        let root = ogeom::algo::baked_where_placed(&mut model, &root, tol)
            .map_err(other)?
            .shape;
        // A curvature the kernel could not read comes back infinite.
        let read = |k: f64| k.is_finite().then_some(k);
        let mut found: Vec<(Shape, f64, f64, Option<f64>)> = Vec::new();
        for face in explore_unique(&model, &root, ShapeType::Face).map_err(other)? {
            let contacts =
                ogeom::fillet::analyse_blend(&model, &root, &face, STATIONS, tol).map_err(other)?;
            for contact in contacts {
                let bend = read(contact.curvature_error);
                match found.iter_mut().find(|(e, ..)| e.is_same(&contact.edge)) {
                    // Each shared edge is met from both its faces.
                    Some((_, gap, angle, curvature)) => {
                        *gap = gap.max(contact.gap);
                        *angle = angle.max(contact.tangency_error);
                        *curvature = match (*curvature, bend) {
                            (Some(a), Some(b)) => Some(a.max(b)),
                            (a, b) => a.or(b),
                        };
                    }
                    None => found.push((contact.edge, contact.gap, contact.tangency_error, bend)),
                }
            }
        }
        found
            .into_iter()
            .map(|(edge, gap, angle, curvature)| {
                let point = edge_samples(&model, &edge, &[0.5])?
                    .first()
                    .copied()
                    .ok_or_else(|| other("an edge has no middle"))?;
                Ok(kernel_api::EdgeContinuity {
                    point: [point.x, point.y, point.z],
                    gap,
                    angle_deg: angle.to_degrees(),
                    curvature,
                })
            })
            .collect()
    }

    fn curvature(
        &self,
        brep: &[u8],
        faces: &[kernel_api::FacePoints],
    ) -> KernelResult<Vec<Vec<Option<[f64; 2]>>>> {
        use ogeom::geom::Surface as _;
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        let mut out = Vec::with_capacity(faces.len());
        for (inside, points) in faces {
            let face = nearest_of(
                &mut model,
                &root,
                ShapeType::Face,
                Point::new(inside[0], inside[1], inside[2]),
            )
            .map_err(other)?;
            let Some(NodeData::Face(data)) = model.node(&face).map(|n| n.data()) else {
                out.push(vec![None; points.len()]);
                continue;
            };
            let Some(surface) = model.geometry().surface(data.surface).cloned() else {
                out.push(vec![None; points.len()]);
                continue;
            };
            let placement = face.transform(model.datums()).map_err(other)?;
            let back = placement.inverse().map_err(other)?;
            let scale = placement.scale_factor();
            let mut guess: Option<(f64, f64)> = None;
            let mut read = Vec::with_capacity(points.len());
            for (p, n) in points {
                let target = back.apply(Point::new(p[0], p[1], p[2]));
                let facing = back.apply_vector(Vector::new(n[0], n[1], n[2]));
                // From the last point's foot first, the surface searched
                // afresh where that lands far off or on a pole.
                let at = |foot: ogeom::geom::SurfaceProjection| {
                    let (u, v) = foot.parameters;
                    surface
                        .curvature_at(u, v, tol)
                        .ok()
                        .map(|c| (foot.parameters, c))
                };
                let found = guess
                    .and_then(|g| {
                        ogeom::geom::project_on_surface_from(&surface, target, g, tol).ok()
                    })
                    .filter(|f| f.distance <= CURVATURE_REACH)
                    .and_then(at)
                    .or_else(|| {
                        let mut best: Option<ogeom::geom::SurfaceProjection> = None;
                        for seeds in CURVATURE_SEEDS {
                            if let Ok(foot) =
                                ogeom::geom::project_on_surface(&surface, target, seeds, tol)
                                && best.is_none_or(|b| foot.distance < b.distance)
                            {
                                best = Some(foot);
                            }
                            if best.is_some_and(|b| b.distance <= CURVATURE_REACH) {
                                break;
                            }
                        }
                        best.and_then(at)
                    });
                let Some((parameters, c)) = found else {
                    read.push(None);
                    continue;
                };
                guess = Some(parameters);
                // The surface's curvatures are signed against its own
                // normal: turned round where the asked one faces away.
                let (max, min) = (c.max / scale, c.min / scale);
                read.push(Some(if c.normal.vector().dot(facing) < 0.0 {
                    [-min, -max]
                } else {
                    [max, min]
                }));
            }
            out.push(read);
        }
        Ok(out)
    }

    fn face_edges(&self, brep: &[u8], near: [f64; 3]) -> KernelResult<Vec<([f64; 3], [f64; 3])>> {
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        let face = nearest_of(
            &mut model,
            &root,
            ShapeType::Face,
            Point::new(near[0], near[1], near[2]),
        )
        .map_err(other)?;
        // A seam turns up twice in its face's boundary, once each way.
        let all = ogeom::topo::explore(&model, &face, ogeom::topo::Filter::OfType(ShapeType::Edge))
            .map_err(other)?;
        let mut out = Vec::new();
        for edge in explore_unique(&model, &face, ShapeType::Edge).map_err(other)? {
            if all.iter().filter(|e| e.is_same(&edge)).count() > 1 {
                continue;
            }
            let Ok(points) = edge_points(&model, &edge, Deflection::default(), tol) else {
                continue;
            };
            if points.len() < 2 {
                continue;
            }
            let i = (points.len() - 1) / 2;
            let (a, b) = (points[i], points[i + 1]);
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length <= 1e-12 {
                continue;
            }
            let mid = [
                (a[0] + b[0]) / 2.0,
                (a[1] + b[1]) / 2.0,
                (a[2] + b[2]) / 2.0,
            ];
            out.push((mid, d.map(|v| v / length)));
        }
        Ok(out)
    }

    fn recognize_holes(
        &self,
        brep: &[u8],
    ) -> KernelResult<(Vec<kernel_api::RecognizedHole>, usize)> {
        crate::holes::recognize_holes(brep)
    }

    fn gap(&self, a: &[u8], b: &[u8], b_in_a: &[[f64; 4]; 4]) -> KernelResult<kernel_api::Gap> {
        let tol = tess::tolerances();
        let (mut model, first) = tess::read_blob(a)?;
        let second = crate::chain::absorb_shape(&mut model, b).map_err(other)?;
        let second = crate::ops::pattern::moved(&mut model, &second, b_in_a).map_err(other)?;
        let found = distance_between_shapes(
            &model,
            &first,
            &second,
            ogeom::intersect::ExtremaOptions::default(),
            tol,
        )
        .map_err(other)?;
        let pair = found
            .pairs
            .first()
            .ok_or_else(|| other("the shapes have nothing to measure between"))?;
        let at = |p: Point| [p.x, p.y, p.z];
        Ok(kernel_api::Gap {
            distance_mm: found.distance,
            on_a: at(pair.point_a),
            on_b: at(pair.point_b),
        })
    }

    fn mirror(&self, brep: &[u8], point: [f64; 3], normal: [f64; 3]) -> KernelResult<Vec<u8>> {
        let n = Vector::new(normal[0], normal[1], normal[2]);
        let len = n.dot(n).sqrt();
        if len < 1e-12 {
            return Err(other("a mirror plane needs a normal"));
        }
        let n = n * (1.0 / len);
        let d = 2.0 * (point[0] * n.x + point[1] * n.y + point[2] * n.z);
        let c = [n.x, n.y, n.z];
        let mut m = [[0.0; 4]; 4];
        for (i, row) in m.iter_mut().enumerate().take(3) {
            for (j, cell) in row.iter_mut().enumerate().take(3) {
                *cell = f64::from(u8::from(i == j)) - 2.0 * c[i] * c[j];
            }
            row[3] = d * c[i];
        }
        m[3][3] = 1.0;
        let (mut model, root) = tess::read_blob(brep)?;
        let shape = crate::ops::pattern::mirrored(&mut model, &root, &m).map_err(other)?;
        tess::write_blob(&model, &shape)
    }

    fn measure(&self, brep: &[u8]) -> KernelResult<kernel_api::PhysicalProperties> {
        crate::health::measure_blob(brep)
    }

    fn overlap(&self, a: &[u8], b: &[u8], b_in_a: &[[f64; 4]; 4]) -> KernelResult<Option<Overlap>> {
        let tol = tess::tolerances();
        let (mut model, first) = tess::read_blob(a)?;
        let second = crate::chain::absorb_shape(&mut model, b).map_err(other)?;
        let second = crate::ops::pattern::moved(&mut model, &second, b_in_a).map_err(other)?;
        let pieces = crate::ops::common_pieces(&mut model, &first, &second).map_err(other)?;
        let (mut volume, mut moment) = (0.0, Vector::new(0.0, 0.0, 0.0));
        for piece in &pieces {
            let measured = volume_properties(&model, piece, Deflection::default(), tol)
                .map_err(|e| other(format!("measuring the shared solid failed: {e}")))?;
            let mass = measured.mass.abs();
            volume += mass;
            moment += Vector::new(measured.centre.x, measured.centre.y, measured.centre.z) * mass;
        }
        if volume <= TOUCHING_MM3 {
            return Ok(None);
        }
        let shared = crate::ops::wrap_pieces(&mut model, pieces).map_err(other)?;
        let mesh = tess::mesh_shape(&model, &shared, &[], &TessellationSettings::default())?;
        let centre = moment / volume;
        Ok(Some(Overlap {
            volume_mm3: volume,
            centre_mm: [centre.x, centre.y, centre.z],
            mesh,
        }))
    }

    fn project_edge(
        &self,
        brep: &[u8],
        near: [f64; 3],
        plane: &ProfilePlane,
    ) -> KernelResult<ProjectedEdge> {
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        let probe = Point::new(near[0], near[1], near[2]);
        let (edge, distance) =
            crate::ops::dressup::nearest_with_distance(&mut model, &root, ShapeType::Edge, probe)
                .map_err(other)?;
        // A pick names an edge only within reach of it, as a fillet's does.
        if distance > crate::ops::dressup::pick_reach(&model, &root) {
            return Err(other(format!(
                "no edge near ({:.3}, {:.3}, {:.3}): the nearest is {distance:.3} mm away",
                near[0], near[1], near[2]
            )));
        }
        let plane = plane_of(plane)?;
        let projected = project_edge_onto_plane(&model, &edge, &plane, tol).map_err(other)?;
        in_plane(projected)
    }

    fn edge_frames(
        &self,
        brep: &[u8],
        near: [f64; 3],
        stations: &[f64],
    ) -> KernelResult<PathFrames> {
        let (mut model, root) = tess::read_blob(brep)?;
        let probe = Point::new(near[0], near[1], near[2]);
        let (edge, distance) =
            crate::ops::dressup::nearest_with_distance(&mut model, &root, ShapeType::Edge, probe)
                .map_err(other)?;
        if distance > crate::ops::dressup::pick_reach(&model, &root) {
            return Err(other(format!(
                "no edge near ({:.3}, {:.3}, {:.3}): the nearest is {distance:.3} mm away",
                near[0], near[1], near[2]
            )));
        }
        edge_frames_of(&model, &edge, stations)
    }

    fn wire_frames(
        &self,
        plane: &ProfilePlane,
        wire: &kernel_api::ProfileWire,
        stations: &[f64],
    ) -> KernelResult<PathFrames> {
        let curves = crate::profile::wire_curves(plane, wire).map_err(other)?;
        frames_along(&curves, None, stations)
    }

    fn pattern_transforms(
        &self,
        brep: Option<&[u8]>,
        steps: &[kernel_api::PatternStep],
    ) -> KernelResult<Vec<[[f64; 4]; 4]>> {
        let (mut model, root) = match brep {
            Some(brep) => {
                let (model, root) = tess::read_blob(brep)?;
                (model, Some(root))
            }
            None => (Model::new(), None),
        };
        crate::ops::pattern_steps::transforms(&mut model, root.as_ref(), steps).map_err(other)
    }

    fn section_face(
        &self,
        brep: &[u8],
        near: [f64; 3],
        plane: &ProfilePlane,
    ) -> KernelResult<Vec<ProjectedEdge>> {
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        let face = nearest_of(
            &mut model,
            &root,
            ShapeType::Face,
            Point::new(near[0], near[1], near[2]),
        )
        .map_err(other)?;
        let plane = plane_of(plane)?;
        face_section(&mut model, &root, &face, &plane)?
            .iter()
            .map(|edge| {
                // A section edge lies in the plane: projecting it onto the
                // plane names the curve it is there.
                in_plane(project_edge_onto_plane(&model, edge, &plane, tol).map_err(other)?)
            })
            .collect()
    }

    fn section(
        &self,
        brep: &[u8],
        plane: &ProfilePlane,
        near: Option<[f64; 3]>,
    ) -> KernelResult<Vec<kernel_api::SectionRun>> {
        let (mut model, root) = tess::read_blob(brep)?;
        let plane = plane_of(plane)?;
        let edges = match near {
            Some(near) => {
                let face = nearest_of(
                    &mut model,
                    &root,
                    ShapeType::Face,
                    Point::new(near[0], near[1], near[2]),
                )
                .map_err(other)?;
                face_section(&mut model, &root, &face, &plane)?
            }
            None => shape_section(&mut model, &root, &plane)?,
        };
        section_runs(&model, &edges, &plane)
    }

    fn face_loops(
        &self,
        brep: &[u8],
        near: [f64; 3],
    ) -> KernelResult<Vec<Vec<kernel_api::LoopEdge>>> {
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        let face = nearest_of(
            &mut model,
            &root,
            ShapeType::Face,
            Point::new(near[0], near[1], near[2]),
        )
        .map_err(other)?;
        // A face keeps its outer wire first, each carrying the face's
        // orientation.
        let mut loops = Vec::new();
        for wire in model.children_of(&face).map_err(other)? {
            let edges = model.ordered_children_of(&wire).map_err(other)?;
            let mut walked = Vec::with_capacity(edges.len());
            for edge in &edges {
                // An edge with no curve (a cone's apex) has nothing to walk.
                let Ok(points) = edge_points(&model, edge, Deflection::default(), tol) else {
                    continue;
                };
                walked.push(kernel_api::LoopEdge {
                    points,
                    seam: edges.iter().filter(|e| e.is_same(edge)).count() > 1,
                });
            }
            if !walked.is_empty() {
                loops.push(walked);
            }
        }
        Ok(loops)
    }

    fn face_of(&self, brep: &[u8], near: [f64; 3]) -> KernelResult<Vec<u8>> {
        let (mut model, root) = tess::read_blob(brep)?;
        let at = Point::new(near[0], near[1], near[2]);
        let face = crate::ops::sweep::face_at(&mut model, &root, at)
            .map_err(other)?
            .ok_or_else(|| {
                other(format!(
                    "no face of the solid lies at ({:.1}, {:.1}, {:.1})",
                    at.x, at.y, at.z
                ))
            })?;
        let copy = ogeom::algo::copied(&mut model, &face)
            .map_err(|e| other(format!("copying the face failed: {e}")))?
            .shape;
        tess::write_blob(&model, &copy)
    }

    fn edge_of(&self, brep: &[u8], near: [f64; 3]) -> KernelResult<Vec<u8>> {
        let (mut model, root) = tess::read_blob(brep)?;
        let probe = Point::new(near[0], near[1], near[2]);
        let (edge, distance) =
            crate::ops::dressup::nearest_with_distance(&mut model, &root, ShapeType::Edge, probe)
                .map_err(other)?;
        if distance > crate::ops::dressup::pick_reach(&model, &root) {
            return Err(other(format!(
                "no edge near ({:.3}, {:.3}, {:.3}): the nearest is {distance:.3} mm away",
                near[0], near[1], near[2]
            )));
        }
        let copy = ogeom::algo::copied(&mut model, &edge)
            .map_err(|e| other(format!("copying the edge failed: {e}")))?
            .shape;
        tess::write_blob(&model, &copy)
    }

    fn read_dxf(&self, text: &str) -> KernelResult<kernel_api::Drawing2d> {
        crate::dxf::read_dxf(text)
    }

    fn profile_mesh(&self, profile: &Profile) -> KernelResult<TriMesh> {
        let mut model = Model::new();
        let built = crate::profile::build_profile(&mut model, profile).map_err(other)?;
        let detail = TessellationSettings {
            generate_boundary_edges: false,
            ..TessellationSettings::default()
        };
        let mut out = TriMesh::default();
        for face in &built.faces {
            let mesh = tess::mesh_shape(&model, face, &[], &detail)?;
            let base = out.positions.len() as u32;
            out.positions.extend_from_slice(&mesh.positions);
            out.normals.extend_from_slice(&mesh.normals);
            out.indices.extend(mesh.indices.iter().map(|i| i + base));
        }
        Ok(out)
    }

    fn medial_axis(&self, profile: &Profile, tolerance: f64) -> KernelResult<Vec<MedialRegion>> {
        let tol = tess::tolerances();
        let mut model = Model::new();
        let built = crate::profile::build_profile(&mut model, profile).map_err(other)?;
        built
            .faces
            .iter()
            .zip(&built.groups)
            .map(|(face, group)| {
                let graph = medial_graph(&model, face, tolerance, tol)
                    .map_err(|e| other(format!("the medial axis failed: {e}")))?;
                medial_region(
                    &graph,
                    &profile.plane,
                    group.wire_indices.clone(),
                    tolerance,
                    tol,
                )
            })
            .collect()
    }

    fn centre_line(
        &self,
        brep: &[u8],
        from: &FaceProbe,
        to: &FaceProbe,
        tolerance: f64,
    ) -> KernelResult<CentreLine> {
        let tol = tess::tolerances();
        let (mut model, root) = tess::read_blob(brep)?;
        let start = face_named(&mut model, &root, from)?;
        let end = face_named(&mut model, &root, to)?;
        let solid = crate::ops::solids_of(&model, &root)
            .into_iter()
            .find(|solid| {
                explore_unique(&model, solid, ShapeType::Face)
                    .is_ok_and(|faces| faces.iter().any(|f| f.is_same(&start)))
            })
            .ok_or_else(|| other("the first face is on no solid"))?;
        let path = ogeom::offset::middle_path(&mut model, &solid, &start, &end, tolerance, tol)
            .map_err(|e| other(format!("no centre line between those faces: {e}")))?;
        let wire = path.built.shape;
        let deflection = Deflection {
            chord: tolerance * 0.25,
            ..Deflection::default()
        };
        let length = linear_properties(&model, &wire, deflection, tol)
            .map_err(|e| other(format!("measuring the centre line failed: {e}")))?
            .mass;
        let edges = explore_unique(&model, &wire, ShapeType::Edge).map_err(other)?;
        let mut points: Vec<[f64; 3]> = Vec::new();
        for edge in &edges {
            let mut run = edge_points(&model, edge, deflection, tol)?;
            // Each edge runs on from where the last one stopped.
            if let (Some(last), Some(first), Some(tail)) = (points.last(), run.first(), run.last())
                && distance(*last, *tail) < distance(*last, *first)
            {
                run.reverse();
            }
            if !points.is_empty() && !run.is_empty() {
                run.remove(0);
            }
            points.extend(run);
        }
        if let (Some(first), Some(last)) = (points.first(), points.last())
            && distance(*last, from.point) < distance(*first, from.point)
        {
            points.reverse();
        }
        let straight = edges.len() == 1 && is_line(&model, &edges[0]);
        Ok(CentreLine {
            points,
            length,
            deviation: path.deviation,
            straight,
        })
    }

    fn probe(&self, brep: &[u8], probe: &ShapeProbe) -> KernelResult<ProbeAnswer> {
        let (mut model, root) = tess::read_blob(brep)?;
        crate::probe::answer(&mut model, &root, probe).map_err(other)
    }
}

/// A profile plane as the kernel's.
fn plane_of(plane: &ProfilePlane) -> KernelResult<Plane> {
    let tol = tess::tolerances();
    let direction = |v: [f64; 3]| Direction::new(Vector::new(v[0], v[1], v[2]), tol);
    let frame = Frame::from_axes(
        Point::new(plane.origin[0], plane.origin[1], plane.origin[2]),
        direction(plane.x_axis).map_err(other)?,
        direction(plane.y_axis).map_err(other)?,
        direction(plane.normal).map_err(other)?,
        tol,
    )
    .map_err(other)?;
    Ok(Plane::new(frame))
}

/// How far off the face a section edge may run and still be its, in mm.
const ON_FACE_MM: f64 = 1e-4;

/// The edges where `face` crosses `plane`: its solid sectioned by the half
/// space the plane bounds, and of the section the edges that run on the
/// face.
fn face_section(
    model: &mut Model,
    root: &Shape,
    face: &Shape,
    plane: &Plane,
) -> KernelResult<Vec<Shape>> {
    let tol = tess::tolerances();
    let solid = crate::ops::solids_of(model, root)
        .into_iter()
        .find(|solid| {
            explore_unique(model, solid, ShapeType::Face)
                .is_ok_and(|faces| faces.iter().any(|f| f.is_same(face)))
        })
        .ok_or_else(|| other("the face is on no solid"))?;
    if misses(model, face, plane)? {
        return Ok(Vec::new());
    }
    let edges = solid_section(model, &solid, plane)?;
    let mut on_face = Vec::new();
    for edge in edges {
        // Along the edge, away from its ends, where an edge of the next
        // face would meet this one.
        let mut runs_on = true;
        for p in edge_samples(model, &edge, &[0.25, 0.5, 0.75])? {
            let vertex = model.add_vertex(ogeom::topo::VertexData::new(p));
            let gap = distance_between_shapes(
                model,
                &vertex,
                face,
                ogeom::intersect::ExtremaOptions::default(),
                tol,
            )
            .map_err(other)?
            .distance;
            runs_on &= gap <= ON_FACE_MM;
        }
        if runs_on {
            on_face.push(edge);
        }
    }
    Ok(on_face)
}

/// The edges where `plane` crosses `root`: each of its solids sectioned by
/// the half space the plane bounds, or, a shape with no solid, its faces
/// cut by the plane.
fn shape_section(model: &mut Model, root: &Shape, plane: &Plane) -> KernelResult<Vec<Shape>> {
    let tol = tess::tolerances();
    if misses(model, root, plane)? {
        return Ok(Vec::new());
    }
    let solids = crate::ops::solids_of(model, root);
    if solids.is_empty() {
        let section = ogeom::boolean::section_face(model, root, plane, tol)
            .map_err(|e| other(format!("the shape could not be cut by the plane: {e}")))?
            .shape;
        return explore_unique(model, &section, ShapeType::Edge).map_err(other);
    }
    let mut edges = Vec::new();
    for solid in solids {
        if !misses(model, &solid, plane)? {
            edges.extend(solid_section(model, &solid, plane)?);
        }
    }
    Ok(edges)
}

/// `plane` clear of the box round `shape`, so it crosses none of it.
fn misses(model: &Model, shape: &Shape, plane: &Plane) -> KernelResult<bool> {
    let tol = tess::tolerances();
    let frame = plane.frame();
    let (origin, normal) = (frame.origin(), frame.z().vector());
    let sides: Vec<f64> = ogeom::algo::shape_bounds(model, shape, tol)
        .map_err(other)?
        .corners()
        .into_iter()
        .map(|corner| normal.dot(corner - origin))
        .collect();
    let reach = tol.confusion();
    Ok(sides.iter().all(|d| *d > reach) || sides.iter().all(|d| *d < -reach))
}

/// The edges where `solid`'s boundary crosses `plane`.
fn solid_section(model: &mut Model, solid: &Shape, plane: &Plane) -> KernelResult<Vec<Shape>> {
    let tol = tess::tolerances();
    let frame = plane.frame();
    let (origin, normal) = (frame.origin(), frame.z().vector());
    let surface = ogeom::geom::PlaneSurface::over(*plane, (-1.0e6, 1.0e6), (-1.0e6, 1.0e6))
        .map_err(|e| other(format!("the section plane: {e}")))?;
    let cutter =
        ogeom::algo::make_natural_face(model, ogeom::geom::SurfaceGeometry::Plane(surface))
            .map_err(|e| other(format!("the section plane: {e}")))?
            .shape;
    let below = origin - normal;
    let half = ogeom::algo::make_half_space(model, &cutter, below, tol)
        .map_err(|e| other(format!("the section plane: {e}")))?
        .shape;
    let section = ogeom::boolean::section(model, solid, &half, tol)
        .map_err(|e| other(format!("the solid could not be cut by the plane: {e}")))?
        .shape;
    explore_unique(model, &section, ShapeType::Edge).map_err(other)
}

/// How near two ends of section curves come and still be one point, mm.
const JOINED_MM: f64 = 1e-3;

/// `edges` (all in `plane`) chained end to end into runs, each curve in
/// the plane's coordinates and drawn along its run.
fn section_runs(
    model: &Model,
    edges: &[Shape],
    plane: &Plane,
) -> KernelResult<Vec<kernel_api::SectionRun>> {
    let tol = tess::tolerances();
    let mut curves: Vec<kernel_api::SectionCurve> = Vec::new();
    for edge in edges {
        let points = edge_points(model, edge, Deflection::default(), tol)?;
        if points.len() < 2 {
            continue;
        }
        // A section edge lies in the plane: projecting it onto the plane
        // names the curve it is there.
        let curve = in_plane(project_edge_onto_plane(model, edge, plane, tol).map_err(other)?)?;
        curves.push(kernel_api::SectionCurve {
            curve,
            points,
            reversed: false,
        });
    }
    let near = |a: Option<&[f64; 3]>, b: Option<&[f64; 3]>| {
        a.zip(b).is_some_and(|(a, b)| distance(*a, *b) <= JOINED_MM)
    };
    let mut runs = Vec::new();
    while let Some(first) = curves.pop() {
        let mut run = std::collections::VecDeque::from([first]);
        let closes = |run: &std::collections::VecDeque<kernel_api::SectionCurve>| {
            near(
                run.front().and_then(|c| c.points.first()),
                run.back().and_then(|c| c.points.last()),
            )
        };
        // On from the tail, then back from the head, until nothing meets
        // the end or the run closes.
        for forward in [true, false] {
            while !closes(&run) {
                let end = if forward {
                    run.back().and_then(|c| c.points.last())
                } else {
                    run.front().and_then(|c| c.points.first())
                };
                let meets = |c: &kernel_api::SectionCurve| {
                    let (start, finish) = (c.points.first(), c.points.last());
                    if forward {
                        (near(start, end), near(finish, end))
                    } else {
                        (near(finish, end), near(start, end))
                    }
                };
                let Some((i, turned)) =
                    curves.iter().enumerate().find_map(|(i, c)| match meets(c) {
                        (true, _) => Some((i, false)),
                        (_, true) => Some((i, true)),
                        _ => None,
                    })
                else {
                    break;
                };
                let mut next = curves.swap_remove(i);
                if turned {
                    next.points.reverse();
                    next.reversed = !next.reversed;
                }
                if forward {
                    run.push_back(next);
                } else {
                    run.push_front(next);
                }
            }
        }
        let closed = closes(&run);
        runs.push(kernel_api::SectionRun {
            curves: run.into(),
            closed,
        });
    }
    Ok(runs)
}

/// Points of `edge` at fractions of its parameter range, in the model's
/// frame.
pub(crate) fn edge_samples(
    model: &Model,
    edge: &Shape,
    fractions: &[f64],
) -> KernelResult<Vec<Point>> {
    let tol = tess::tolerances();
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return Err(other("not an edge"));
    };
    let Some(EdgeRepr::Curve3d { curve, range, .. }) = data.curve3d() else {
        return Err(other("an edge has no curve"));
    };
    let geometry = model
        .geometry()
        .curve(*curve)
        .ok_or_else(|| other("an edge's curve is missing"))?;
    let placement = edge.transform(model.datums()).map_err(other)?;
    fractions
        .iter()
        .map(|f| {
            let t = range.0 + (range.1 - range.0) * f;
            let p = geometry.point_at(t, tol).map_err(other)?;
            Ok(placement.apply(p))
        })
        .collect()
}

/// A curve projected onto a plane, in the plane's own coordinates: the
/// exact curve it is there.
fn in_plane(projected: ProjectedCurve) -> KernelResult<ProjectedEdge> {
    Ok(match projected {
        ProjectedCurve::Point(p) => ProjectedEdge::Point([p.x, p.y]),
        ProjectedCurve::Line { start, end } => ProjectedEdge::Line {
            start: [start.x, start.y],
            end: [end.x, end.y],
        },
        ProjectedCurve::Circle {
            centre,
            radius,
            range,
        } => ProjectedEdge::Circle {
            centre: [centre.x, centre.y],
            radius,
            range,
        },
        ProjectedCurve::Ellipse {
            centre,
            major,
            ratio,
            range,
        } => ProjectedEdge::Ellipse {
            centre: [centre.x, centre.y],
            major: [major.x, major.y],
            ratio,
            range,
        },
        ProjectedCurve::Parabola {
            vertex,
            axis,
            focal,
            range,
        } => ProjectedEdge::Parabola {
            vertex: [vertex.x, vertex.y],
            axis: [axis.x, axis.y],
            focal,
            range,
        },
        ProjectedCurve::Hyperbola {
            centre,
            major,
            minor,
            range,
        } => ProjectedEdge::Hyperbola {
            centre: [centre.x, centre.y],
            major: [major.x, major.y],
            minor: [minor.x, minor.y],
            range,
        },
        // The kernel hands every spline over clamped, so a profile drawn on
        // it is the same curve as the edge it came from.
        ProjectedCurve::BSpline { curve, .. } => {
            if !curve.knots().is_clamped() {
                return Err(other("the kernel projected a spline with unclamped knots"));
            }
            let control: Vec<_> = curve
                .control_points()
                .iter()
                .map(|w| (w.point(), w.weight))
                .collect();
            ProjectedEdge::Spline {
                degree: curve.knots().degree() as u32,
                knots: curve.knots().knots().to_vec(),
                control_points: control.iter().map(|(p, _)| [p.x, p.y]).collect(),
                weights: if curve.is_rational() {
                    control.iter().map(|(_, w)| *w).collect()
                } else {
                    Vec::new()
                },
            }
        }
    })
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The face of `root` a probe names: the face its name finds, else the
/// nearest to its point, and among faces as near (a point on the edge
/// between them), the one whose outward normal agrees best with the probe's.
pub(crate) fn face_named(
    model: &mut Model,
    root: &Shape,
    probe: &FaceProbe,
) -> KernelResult<Shape> {
    let tol = tess::tolerances();
    let point = Point::new(probe.point[0], probe.point[1], probe.point[2]);
    if let Some(face) = crate::naming::find_face(model, root, probe.name, point) {
        return Ok(face);
    }
    let normal = Vector::new(probe.normal[0], probe.normal[1], probe.normal[2]);
    let vertex = model.add_vertex(ogeom::topo::VertexData::new(point));
    let faces = explore_unique(model, root, ShapeType::Face).map_err(other)?;
    let mut near: Vec<(f64, Shape)> = Vec::new();
    for face in faces {
        let Ok(d) = distance_between_shapes(
            model,
            &vertex,
            &face,
            ogeom::intersect::ExtremaOptions::default(),
            tol,
        ) else {
            continue;
        };
        near.push((d.distance, face));
    }
    let best = near.iter().map(|(d, _)| *d).fold(f64::INFINITY, f64::min);
    let agreement = |face: &Shape| {
        crate::ops::sweep::face_plane(model, face).map_or(0.0, |(_, n)| {
            let outward = if face.orientation() == Orientation::Reversed {
                -n.vector()
            } else {
                n.vector()
            };
            outward.dot(normal)
        })
    };
    near.into_iter()
        .filter(|(d, _)| *d <= best + 1e-6)
        .map(|(_, face)| (agreement(&face), face))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, face)| face)
        .ok_or_else(|| other("no face near the picked point"))
}

/// Frames at `stations` (fractions of its length) along `edge`, placed,
/// walked from its curve's start.
pub(crate) fn edge_frames_of(
    model: &Model,
    edge: &Shape,
    stations: &[f64],
) -> KernelResult<PathFrames> {
    let (curve, placement) = edge_curve(model, edge)?;
    frames_along(&[curve], Some(&placement), stations)
}

/// `edge`'s curve over the range it covers, and where the edge places it.
pub(crate) fn edge_curve(
    model: &Model,
    edge: &Shape,
) -> KernelResult<(crate::profile::SegmentCurve, ogeom::math::Transform)> {
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return Err(other("the shape holds no edge there"));
    };
    let Some(EdgeRepr::Curve3d { curve, range, .. }) = data.curve3d() else {
        return Err(other("the edge has no curve"));
    };
    let geometry = model
        .geometry()
        .curve(*curve)
        .ok_or_else(|| other("the edge's curve is missing"))?
        .clone();
    let placement = edge.transform(model.datums()).map_err(other)?;
    Ok(((geometry, *range), placement))
}

/// Frames at `stations` (fractions of the whole length) along `curves`
/// walked one after another, each over its range from its start, moved by
/// `placement` when there is one.
pub(crate) fn frames_along(
    curves: &[crate::profile::SegmentCurve],
    placement: Option<&ogeom::math::Transform>,
    stations: &[f64],
) -> KernelResult<PathFrames> {
    use ogeom::algo::{curve_length, parameter_at_length};
    let tol = tess::tolerances();
    let lengths = curves
        .iter()
        .map(|(curve, range)| curve_length(curve, *range, tol))
        .collect::<Result<Vec<f64>, _>>()
        .map_err(other)?;
    let length: f64 = lengths.iter().sum();
    if curves.is_empty() || length <= tol.confusion() {
        return Err(other("the path has no length"));
    }
    let place = |p: Point| placement.map_or(p, |t| t.apply(p));
    let turn = |v: Vector| placement.map_or(v, |t| t.apply_vector(v));
    let ends = |i: usize, u: f64| curves[i].0.point_at(u, tol).map(place).map_err(other);
    let start = ends(0, curves[0].1.0)?;
    let last = curves.len() - 1;
    let end = ends(last, curves[last].1.1)?;
    let closed = start.distance(end) <= tol.confusion() * 10.0;
    let mut frames = Vec::with_capacity(stations.len());
    for &station in stations {
        if !(0.0..=1.0).contains(&station) {
            return Err(other(format!(
                "station {station} is not a fraction of the path from 0 to 1"
            )));
        }
        let mut target = station * length;
        let mut index = 0;
        while index < last && target > lengths[index] {
            target -= lengths[index];
            index += 1;
        }
        let (curve, range) = &curves[index];
        let target = target.clamp(0.0, lengths[index]);
        let u = parameter_at_length(curve, *range, target, tol).map_err(other)?;
        let point = place(curve.point_at(u, tol).map_err(other)?);
        let d = turn(curve.d1_at(u, tol).map_err(other)?);
        let speed = d.magnitude();
        if speed <= 1e-12 {
            return Err(other(format!(
                "the path has no direction at station {station}"
            )));
        }
        frames.push(PathFrame {
            point: [point.x, point.y, point.z],
            tangent: [d.x / speed, d.y / speed, d.z / speed],
        });
    }
    Ok(PathFrames {
        length,
        closed,
        frames,
    })
}

/// Points along an edge, placed, from its start to its end as the edge
/// runs.
fn edge_points(
    model: &Model,
    edge: &Shape,
    deflection: Deflection,
    tol: Tolerances,
) -> KernelResult<Vec<[f64; 3]>> {
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return Err(other("the centre line holds no edge"));
    };
    let Some(EdgeRepr::Curve3d { curve, range, .. }) = data.curve3d() else {
        return Err(other("the centre line's edge has no curve"));
    };
    let geometry = model
        .geometry()
        .curve(*curve)
        .ok_or_else(|| other("the centre line's curve is missing"))?;
    let placement = edge.transform(model.datums()).map_err(other)?;
    let line = discretize(geometry, *range, deflection, tol).map_err(other)?;
    let mut points: Vec<[f64; 3]> = line
        .points
        .iter()
        .map(|p| {
            let p = placement.apply(*p);
            [p.x, p.y, p.z]
        })
        .collect();
    if edge.orientation() == Orientation::Reversed {
        points.reverse();
    }
    Ok(points)
}

fn is_line(model: &Model, edge: &Shape) -> bool {
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return false;
    };
    let Some(EdgeRepr::Curve3d { curve, .. }) = data.curve3d() else {
        return false;
    };
    matches!(
        model.geometry().curve(*curve),
        Some(ogeom::geom::Curve::Line(_))
    )
}

/// Points taken along each medial branch.
const MEDIAL_SAMPLES: usize = 48;

/// A medial graph read into the profile plane's coordinates, with the
/// narrowest place found.
///
/// The clearance runs down to nothing where a branch ends in a convex
/// corner, and down to a fillet's radius where it ends at a rounded one,
/// without the region getting thinner there, so the branch ends that meet
/// the boundary never count. What counts is a branch point (where three
/// or more branches meet, or a full circle's centre, where none do) and
/// every local minimum inside a branch: a neck, or a strip of constant
/// width.
fn medial_region(
    graph: &MedialGraph,
    plane: &ProfilePlane,
    wires: Vec<usize>,
    tolerance: f64,
    tol: Tolerances,
) -> KernelResult<MedialRegion> {
    let flat = |p: Point| {
        let d = [
            p.x - plane.origin[0],
            p.y - plane.origin[1],
            p.z - plane.origin[2],
        ];
        let dot = |a: [f64; 3]| d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
        [dot(plane.x_axis), dot(plane.y_axis)]
    };
    let mut degree = vec![0usize; graph.vertices.len()];
    for branch in &graph.branches {
        for &end in &branch.ends {
            if let Some(d) = degree.get_mut(end) {
                *d += 1;
            }
        }
    }
    let mut narrowest: Option<Narrowest> = None;
    let mut consider = |at: [f64; 2], clearance: f64| {
        if clearance > tolerance && narrowest.is_none_or(|n| clearance < n.clearance) {
            narrowest = Some(Narrowest { at, clearance });
        }
    };
    for (vertex, &d) in graph.vertices.iter().zip(&degree) {
        if d != 1 {
            consider(flat(vertex.point), vertex.clearance);
        }
    }
    let mut paths = Vec::with_capacity(graph.branches.len());
    for (index, branch) in graph.branches.iter().enumerate() {
        let (a, b) = branch.range;
        let at = |i: usize| a + (b - a) * i as f64 / MEDIAL_SAMPLES as f64;
        let mut points = Vec::with_capacity(MEDIAL_SAMPLES + 1);
        let mut clearance = Vec::with_capacity(MEDIAL_SAMPLES + 1);
        for i in 0..=MEDIAL_SAMPLES {
            let t = at(i);
            let p = branch.curve.point_at(t, tol).map_err(other)?;
            points.push(flat(p));
            clearance.push(graph.clearance_at(index, t, tol).map_err(other)?);
        }
        for i in 1..MEDIAL_SAMPLES {
            if clearance[i] <= clearance[i - 1] && clearance[i] <= clearance[i + 1] {
                let t = least_between(graph, index, at(i - 1), at(i + 1), tol);
                let p = branch.curve.point_at(t, tol).map_err(other)?;
                let c = graph.clearance_at(index, t, tol).map_err(other)?;
                if c <= clearance[i] {
                    consider(flat(p), c);
                } else {
                    consider(points[i], clearance[i]);
                }
            }
        }
        let boundary = |end: usize| degree.get(end) == Some(&1);
        paths.push(MedialPath {
            points,
            clearance,
            boundary_ends: [boundary(branch.ends[0]), boundary(branch.ends[1])],
        });
    }
    Ok(MedialRegion {
        wires,
        paths,
        narrowest,
    })
}

/// Where the clearance along a branch is least between two parameters
/// that bracket a local minimum, by golden-section search.
fn least_between(
    graph: &MedialGraph,
    branch: usize,
    mut lo: f64,
    mut hi: f64,
    tol: Tolerances,
) -> f64 {
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    let f = |t: f64| graph.clearance_at(branch, t, tol).unwrap_or(f64::INFINITY);
    for _ in 0..40 {
        let m1 = hi - (hi - lo) * ratio;
        let m2 = lo + (hi - lo) * ratio;
        if f(m1) <= f(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    (lo + hi) / 2.0
}
