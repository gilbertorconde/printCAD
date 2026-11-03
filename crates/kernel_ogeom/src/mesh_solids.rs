//! Mesh steps that take or make solids (a boolean with a bounded solid, a
//! shell), a solid trimmed to where a scan breaks off, and the mesh
//! writers.

use kernel_api::{
    BooleanOp, KernelError, KernelResult, MeshOperand, MeshSide, MeshStepReport, TriMesh,
    TrimAnswer, TrimRequest,
};
use ogeom::algo::distance::{self, DistanceOptions};
use ogeom::mesh::{Bvh, Deflection, Editable, Grid, Side, Thicken};
use ogeom::topo::{Model, Shape, Triangulation};

use crate::tess;

/// The operand's solid in the frame it is used in.
fn operand(value: &MeshOperand) -> Result<(Model, Shape), String> {
    match value {
        MeshOperand::Shape { brep, transform } => {
            let (mut model, shape) = tess::read_blob(brep.as_bytes()).map_err(|e| e.to_string())?;
            let motion = crate::ops::pattern::rigid_of(transform)
                .ok_or("the operand's placement is not rigid")?;
            let shape = ogeom::algo::transformed(&mut model, &shape, motion)
                .map_err(|e| e.to_string())?
                .shape;
            Ok((model, shape))
        }
        MeshOperand::Primitive { kind, placement } => {
            let mut model = Model::new();
            let shape = crate::ops::primitive::build_tool(&mut model, kind, placement)?;
            Ok((model, shape))
        }
    }
}

/// The volume a closed mesh bounds, by its triangles' signed tetrahedra.
fn volume(mesh: &Triangulation) -> f64 {
    mesh.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|v| mesh.positions[v as usize]);
            a.to_vector().dot(b.to_vector().cross(c.to_vector())) / 6.0
        })
        .sum()
}

pub(crate) fn boolean(
    mesh: &Editable,
    value: &MeshOperand,
    op: BooleanOp,
    chord: f64,
) -> Result<(Triangulation, MeshStepReport), String> {
    if !mesh.rims().is_empty() {
        return Err("the mesh is open: fill holes first".into());
    }
    let operation = match op {
        BooleanOp::Cut => ogeom::mesh::BoolKind::Difference,
        BooleanOp::Fuse => ogeom::mesh::BoolKind::Union,
        BooleanOp::Common => ogeom::mesh::BoolKind::Intersection,
        BooleanOp::NewSolid => return Err("the operation is subtract, add or intersect".into()),
    };
    let (model, shape) = operand(value)?;
    let result = ogeom::mesh::boolean_with_shape(
        &mesh.to_triangulation(),
        &model,
        &shape,
        operation,
        chord,
        tess::tolerances(),
    )
    .map_err(|e| e.to_string())?;
    if result.mesh.triangles.is_empty() {
        return Err("nothing is left".into());
    }
    let report = MeshStepReport::Boolean {
        cut_triangles: result.cut.iter().filter(|v| **v).count(),
        volume: volume(&result.mesh),
    };
    Ok((result.mesh, report))
}

pub(crate) fn shell(
    mesh: &Editable,
    thickness: f64,
    side: MeshSide,
) -> Result<(Triangulation, MeshStepReport), String> {
    let hollow = mesh.rims().is_empty();
    let tri = mesh.to_triangulation();
    let result = if hollow {
        ogeom::mesh::hollow(&tri, thickness, &Grid::default(), tess::tolerances())
            .map_err(|e| e.to_string())?
            .mesh
    } else {
        let side = match side {
            MeshSide::Inward => Thicken::Inward,
            MeshSide::Outward => Thicken::Outward,
            MeshSide::Both => Thicken::Both,
        };
        ogeom::mesh::thicken(&tri, thickness, side)
            .map_err(|e| e.to_string())?
            .mesh
    };
    let report = MeshStepReport::Shell {
        hollow,
        volume: volume(&result),
    };
    Ok((result, report))
}

pub(crate) fn write(mesh: &TriMesh, format: &str) -> Result<Vec<u8>, String> {
    use ogeom::io;
    let mut tri = crate::mesh_edit::triangulation(mesh)?;
    // The text writers carry a normal per vertex: each vertex takes the
    // area-weighted sum of its triangles' normals.
    let mut normals = vec![ogeom::math::Vector::new(0.0, 0.0, 0.0); tri.positions.len()];
    for t in &tri.triangles {
        let [a, b, c] = t.map(|v| tri.positions[v as usize]);
        let n = (b - a).cross(c - a);
        for v in t {
            normals[*v as usize] += n;
        }
    }
    tri.normals = normals
        .into_iter()
        .map(|n| n.normalized(tess::tolerances()).unwrap_or(n))
        .collect();
    let export = io::mesh_formats::ExportMesh::plain(&tri);
    match format.to_ascii_lowercase().as_str() {
        "stl" => io::stl::write(&tri, io::stl::Encoding::Binary).map_err(|e| e.to_string()),
        "3mf" => Ok(io::threemf::write_3mf(&[io::threemf::Object {
            mesh: &tri,
            name: None,
        }])),
        "ply" => Ok(io::write_ply(&export).into_bytes()),
        "obj" => Ok(io::write_obj(&[export]).into_bytes()),
        _ => Err("the format is stl, 3mf, ply or obj".into()),
    }
}

/// The value at `share` of the sorted `values`.
fn at_share(values: &[f64], share: f64) -> f64 {
    values[((values.len() - 1) as f64 * share).round() as usize]
}

pub(crate) fn trim_to_scan(request: &TrimRequest) -> KernelResult<TrimAnswer> {
    let invalid = |e: ogeom::core::OgeomError| KernelError::InvalidInput(e.to_string());
    if !(request.depth >= 0.0 && request.gap > 0.0) {
        return Err(KernelError::InvalidInput(
            "the depth is a distance and the gap more than nothing".into(),
        ));
    }
    let (mut model, shape) = tess::read_blob(&request.solid)?;
    let tol = tess::tolerances();
    let scan = crate::deviation::welded(&request.scan);
    let (low, high) = tess::robust_bounds(&model, &shape)
        .ok_or_else(|| KernelError::InvalidInput("the solid has no extent".into()))?;
    let chord = low.distance(high) / 2000.0;
    let solid = ogeom::mesh::triangulate(
        &model,
        &shape,
        Deflection::with_chord(chord).map_err(invalid)?,
        tol,
    )
    .map_err(invalid)?;
    let solid_index = Bvh::build(&solid).map_err(invalid)?;
    let signed = DistanceOptions {
        max: f64::INFINITY,
        facing_limit: None,
        signed: true,
    };
    let depth: Vec<f64> = distance::to_mesh(&solid_index, &scan.positions, None, &signed)
        .map_err(invalid)?
        .into_iter()
        .map(|c| c.map_or(f64::INFINITY, |c| c.distance))
        .collect();
    let region: Vec<bool> = scan
        .triangles
        .iter()
        .map(|t| t.iter().all(|v| depth[*v as usize] < -request.depth))
        .collect();
    if !region.contains(&true) {
        return Err(KernelError::InvalidInput(
            "the scan reaches no deeper than the break depth into the solid".into(),
        ));
    }
    let keep = if request.away {
        Side::Above
    } else {
        Side::Below
    };
    let built = ogeom::boolean::cut_by_mesh_region(
        &mut model,
        &shape,
        &scan,
        &region,
        request.gap,
        keep,
        tol,
    )
    .map_err(|e| KernelError::Other(anyhow::anyhow!("the break does not trim the solid: {e}")))?;
    let piece = ogeom::mesh::triangulate(
        &model,
        &built.shape,
        Deflection::with_chord(chord).map_err(invalid)?,
        tol,
    )
    .map_err(invalid)?;

    // The break's points measure the gap to the piece; its triangles whose
    // corners all lie within reach of the piece are where the two mate.
    let reach = request.gap * 1.5 + chord;
    let piece_index = Bvh::build(&piece).map_err(invalid)?;
    let gap_at: Vec<Option<f64>> = piece_index
        .nearest_many(&scan.positions, reach)
        .into_iter()
        .map(|hit| hit.map(|h| h.distance))
        .collect();
    let mut mating_area = 0.0;
    let mut mated = std::collections::BTreeSet::new();
    for (t, _) in scan.triangles.iter().zip(&region).filter(|(_, r)| **r) {
        if t.iter().all(|v| gap_at[*v as usize].is_some()) {
            let [a, b, c] = t.map(|v| scan.positions[v as usize]);
            mating_area += (b - a).cross(c - a).magnitude() / 2.0;
            mated.extend(t.iter().copied());
        }
    }
    let mut gaps: Vec<f64> = mated
        .into_iter()
        .filter_map(|v| gap_at[v as usize])
        .collect();
    gaps.sort_by(f64::total_cmp);
    let (gap_median, gap_range) = if gaps.is_empty() {
        (f64::NAN, [f64::NAN; 2])
    } else {
        (
            at_share(&gaps, 0.5),
            [at_share(&gaps, 0.05), at_share(&gaps, 0.95)],
        )
    };
    let brep = ogeom::io::brep::write(&model, &built.shape, tol)
        .map_err(invalid)?
        .into_bytes();
    Ok(TrimAnswer {
        brep,
        volume: volume(&piece),
        mating_area,
        gap_median,
        gap_range,
    })
}
