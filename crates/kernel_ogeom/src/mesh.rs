//! Mesh files as bodies, and a mesh body turned into a B-rep.
//!
//! STL, OBJ and 3MF carry triangles and nothing else, so a mesh imports as
//! a body with no shape snapshot: it draws, hides and picks, and takes no
//! features. Its triangles weld where their normals agree within the
//! crease angle, so curved regions shade smoothly, and its outline is the
//! edges only one welded triangle uses: sharp creases and holes. When asked,
//! the kernel builds a B-rep from the mesh's own connectivity, coplanar
//! triangles merged into planar faces, and the body becomes an ordinary
//! imported solid.

use std::path::Path;

use kernel_api::{
    ImportReport, ImportedBody, ImportedModel, KernelError, KernelResult, LengthUnit,
    MeshSolidResult, TessellationSettings, TriMesh,
};
use ogeom::algo::{MeshSolidOptions, solid_from_mesh};
use ogeom::math::{Point, Vector};
use ogeom::topo::{Filter, Model, ShapeType, Triangulation, explore};

use crate::{health, progress, tess};

/// Two triangles whose normals differ by more than this meet at a crease:
/// the shading breaks there and the outline is drawn.
const CREASE_DEG: f32 = 30.0;

/// One mesh a file holds: its name, its triangles, and its colour when it
/// has one throughout.
type Piece = (Option<String>, Triangulation, Option<[f32; 3]>);

/// The mesh formats an import reads, by extension.
pub fn is_mesh_file(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("stl" | "obj" | "3mf" | "ply" | "glb" | "gltf" | "wrl" | "vrml")
    )
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Read a mesh file into bodies: one for an STL, OBJ or PLY, one per build
/// item of a 3MF, one per placed mesh of a glTF or VRML scene. Coordinates
/// are taken as millimetres, as STL, OBJ and PLY conventionally have them
/// (and 3MF's unit, which the reader applies); glTF and VRML are in metres
/// by their standards, and are scaled.
pub fn import_mesh(path: &Path) -> KernelResult<ImportedModel> {
    let bytes = std::fs::read(path)
        .map_err(|e| KernelError::Import(format!("failed to read {}: {e}", path.display())))?;
    let tol = tess::tolerances();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mesh".to_string());
    let failed = |what: &str, e: ogeom::core::OgeomError| {
        KernelError::Import(format!("{what} read failed: {e}"))
    };
    progress::context("Reading mesh");
    let (pieces, warnings): (Vec<Piece>, Vec<String>) = match extension(path).as_deref() {
        Some("stl") => (
            vec![(
                Some(stem),
                ogeom::io::stl::read(&bytes, tol).map_err(|e| failed("STL", e))?,
                None,
            )],
            Vec::new(),
        ),
        Some("obj") => (
            vec![(
                Some(stem),
                ogeom::io::mesh_formats::read_obj(&String::from_utf8_lossy(&bytes))
                    .map_err(|e| failed("OBJ", e))?,
                None,
            )],
            Vec::new(),
        ),
        Some("3mf") => {
            let import = ogeom::io::read_3mf(&bytes, tol).map_err(|e| failed("3MF", e))?;
            let pieces = import
                .objects
                .into_iter()
                .enumerate()
                .map(|(i, object)| {
                    let name = object.name.or_else(|| Some(format!("{stem} {}", i + 1)));
                    let colour = object
                        .colour
                        .map(|c| [c[0], c[1], c[2]].map(|v| srgb_to_linear(v as f32)));
                    (name, object.mesh, colour)
                })
                .collect();
            (pieces, import.warnings)
        }
        Some("ply") => (
            vec![(
                Some(stem),
                ogeom::io::mesh_formats::read_ply(&String::from_utf8_lossy(&bytes))
                    .map_err(|e| failed("PLY", e))?,
                None,
            )],
            Vec::new(),
        ),
        Some("glb") => (
            scene_pieces(
                ogeom::io::read_glb(&bytes).map_err(|e| failed("glTF", e))?,
                &stem,
                false,
            ),
            Vec::new(),
        ),
        Some("gltf") => (
            scene_pieces(
                ogeom::io::read_gltf(&String::from_utf8_lossy(&bytes))
                    .map_err(|e| failed("glTF", e))?,
                &stem,
                false,
            ),
            Vec::new(),
        ),
        Some("wrl" | "vrml") => (
            scene_pieces(
                ogeom::io::read_vrml(&String::from_utf8_lossy(&bytes))
                    .map_err(|e| failed("VRML", e))?,
                &stem,
                true,
            ),
            Vec::new(),
        ),
        _ => {
            return Err(KernelError::Import(format!(
                "{} is not a mesh file this reads (STL, OBJ, 3MF, PLY, glTF, VRML)",
                path.display()
            )));
        }
    };

    let bodies = pieces
        .into_iter()
        .map(|(name, mesh, colour)| {
            let mesh = display_mesh(&mesh, colour);
            ImportedBody {
                name,
                bounds_mm: mesh.bounds(),
                mesh,
                brep_blob: Vec::new(),
                face_colors: Vec::new(),
                health: None,
            }
        })
        .filter(|b| !b.mesh.indices.is_empty())
        .collect();
    Ok(ImportedModel {
        bodies,
        report: ImportReport {
            kernel: format!("ogeom {}", ogeom::VERSION),
            warnings,
            ..ImportReport::default()
        },
        nodes: Vec::new(),
        source_unit: Some(LengthUnit::Millimetre),
    })
}

/// The meshes of a scene in metres (glTF, VRML), each a piece in
/// millimetres named after its node or the file. A VRML colour is sRGB and
/// taken to linear; a glTF one is linear already.
fn scene_pieces(meshes: Vec<ogeom::io::ImportedMesh>, stem: &str, srgb: bool) -> Vec<Piece> {
    let count = meshes.len();
    meshes
        .into_iter()
        .enumerate()
        .map(|(i, scene)| {
            let mut mesh = scene.mesh;
            for p in &mut mesh.positions {
                *p = ogeom::math::Point::new(p.x * 1000.0, p.y * 1000.0, p.z * 1000.0);
            }
            let name = scene.name.or_else(|| {
                Some(if count == 1 {
                    stem.to_string()
                } else {
                    format!("{stem} {}", i + 1)
                })
            });
            let colour = scene.colour.map(|c| {
                let rgb = [c[0] as f32, c[1] as f32, c[2] as f32];
                if srgb { rgb.map(srgb_to_linear) } else { rgb }
            });
            (name, mesh, colour)
        })
        .collect()
}

/// The render mesh of a triangle mesh: every triangle flat-shaded and
/// welded to its neighbours where their normals agree within the crease
/// angle, the outline where they do not, and at the mesh's own holes.
pub fn display_mesh(tri: &Triangulation, colour: Option<[f32; 3]>) -> TriMesh {
    let mut mesh = TriMesh::default();
    let mut vertex_face = Vec::with_capacity(tri.triangles.len() * 3);
    let mut face_normals = Vec::with_capacity(tri.triangles.len());
    for t in &tri.triangles {
        let [a, b, c] = t.map(|i| tri.positions[i as usize]);
        let n = (b - a).cross(c - a);
        let len = n.magnitude();
        if len <= 1e-12 {
            continue;
        }
        let n = [(n.x / len) as f32, (n.y / len) as f32, (n.z / len) as f32];
        let face = face_normals.len() as u32;
        face_normals.push(n);
        let base = mesh.positions.len() as u32;
        for p in [a, b, c] {
            mesh.positions.push([p.x as f32, p.y as f32, p.z as f32]);
            mesh.normals.push(n);
            mesh.colors.push(colour.unwrap_or(tess::WHITE));
            vertex_face.push(face);
        }
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    tess::weld_vertices(
        &mut mesh,
        &vertex_face,
        &face_normals,
        CREASE_DEG.to_radians().cos(),
    );
    mesh.edges = crease_outline(&mesh);
    if colour.is_none() {
        mesh.colors.clear();
    }
    mesh
}

/// The edges only one welded triangle uses, each drawn once: at a crease
/// the two sides keep their own vertices, so the same edge comes back from
/// each side, and one copy is enough.
fn crease_outline(mesh: &TriMesh) -> Vec<u32> {
    const QUANTIZE: f32 = 1.0e4;
    let key = |i: u32| {
        let p = mesh.positions[i as usize];
        p.map(|v| (v * QUANTIZE).round() as i64)
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for pair in tess::extract_boundary_edges(&mesh.indices)
        .as_chunks::<2>()
        .0
    {
        let (a, b) = (key(pair[0]), key(pair[1]));
        if seen.insert(if a <= b { (a, b) } else { (b, a) }) {
            out.extend_from_slice(pair);
        }
    }
    out
}

/// Build a B-rep from a mesh body's render mesh and mesh it as any shape.
/// Coplanar triangles merge into planar faces; a mesh that does not close
/// comes back as its open shells, `closed` false, with the reason.
pub fn solid_of_mesh(
    mesh: &TriMesh,
    detail: &TessellationSettings,
) -> KernelResult<MeshSolidResult> {
    progress::context("Converting the mesh to a solid");
    let tri = Triangulation {
        positions: mesh
            .positions
            .iter()
            .map(|p| Point::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
            .collect(),
        normals: mesh
            .normals
            .iter()
            .map(|n| Vector::new(f64::from(n[0]), f64::from(n[1]), f64::from(n[2])))
            .collect(),
        parameters: vec![(0.0, 0.0); mesh.positions.len()],
        triangles: mesh.indices.as_chunks::<3>().0.to_vec(),
        deflection_met: true,
    };
    let tol = tess::tolerances();
    let mut model = Model::new();
    let built = solid_from_mesh(&mut model, &tri, &MeshSolidOptions::default(), tol)
        .map_err(|e| KernelError::Other(anyhow::anyhow!("the mesh did not convert: {e}")))?;
    let report = &built.report;

    let faces = explore(&model, &built.shape, Filter::OfType(ShapeType::Face))
        .map_err(|e| KernelError::Other(anyhow::anyhow!("exploring faces failed: {e}")))?;
    let uniform = mesh
        .colors
        .first()
        .filter(|first| mesh.colors.iter().all(|c| c == *first))
        .copied();
    let face_colors = uniform.map_or_else(Vec::new, |c| vec![c; faces.len()]);

    progress::context("Meshing the solid");
    let shaped = tess::mesh_shape_with(
        &model,
        &built.shape,
        &face_colors,
        detail,
        tess::Faces::Wide,
    )
    .map_err(|e| KernelError::Other(anyhow::anyhow!("tessellation failed: {e}")))?;
    let bounds_mm = tess::robust_bounds(&model, &built.shape).map(|(lo, hi)| {
        (
            [lo.x as f32, lo.y as f32, lo.z as f32],
            [hi.x as f32, hi.y as f32, hi.z as f32],
        )
    });

    let mut summary = vec![format!(
        "{} triangles into {} faces",
        report.triangles, report.faces
    )];
    let mut note = |n: usize, what: &str| {
        if n > 0 {
            summary.push(format!("{n} {what}"));
        }
    };
    note(report.vertices_welded, "vertices welded");
    note(report.degenerate_dropped, "degenerate triangles dropped");
    note(report.duplicates_dropped, "duplicate triangles dropped");
    note(report.windings_flipped, "windings flipped");
    note(report.edges_used_once, "hole edges");
    note(report.edges_used_more, "non-manifold edges");
    note(report.orientation_conflicts, "orientation conflicts");
    if report.shells > 1 {
        summary.push(format!("{} pieces", report.shells));
    }

    Ok(MeshSolidResult {
        brep_blob: tess::write_blob(&model, &built.shape)?,
        face_colors,
        mesh: shaped,
        bounds_mm,
        health: health::diagnose(&model, &built.shape),
        closed: built.closed,
        summary,
    })
}

/// sRGB, as 3MF writes colours, to the linear RGB the renderer shades with.
fn srgb_to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
