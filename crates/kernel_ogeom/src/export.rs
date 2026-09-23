//! Export: the document's bodies written as STEP, STL or 3MF.
//!
//! STEP carries the exact shapes, one part per body, gathered into one
//! kernel model. The mesh formats carry triangles: a body with a shape is
//! meshed afresh at the export's own tolerance (a print wants a finer mesh
//! than the viewport), and a mesh body goes out as the mesh it is. 3MF
//! objects are welded by position, so each closed solid arrives as a
//! closed mesh rather than as faces that merely touch.

use std::collections::HashMap;
use std::path::Path;

use kernel_api::{KernelError, KernelResult, TessellationSettings, TriMesh};
use ogeom::math::{Point, Vector};
use ogeom::topo::{Model, Triangulation};

use crate::{progress, tess};

/// A file format export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Step,
    Stl,
    ThreeMf,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] =
        [ExportFormat::Step, ExportFormat::Stl, ExportFormat::ThreeMf];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Step => "STEP",
            ExportFormat::Stl => "STL",
            ExportFormat::ThreeMf => "3MF",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Step => "step",
            ExportFormat::Stl => "stl",
            ExportFormat::ThreeMf => "3mf",
        }
    }

    /// Whether the format carries triangles, and so a mesh tolerance.
    pub fn is_mesh(self) -> bool {
        self != ExportFormat::Step
    }

    /// The format a file name asks for, by extension.
    pub fn of_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "step" | "stp" => Some(ExportFormat::Step),
            "stl" => Some(ExportFormat::Stl),
            "3mf" => Some(ExportFormat::ThreeMf),
            _ => None,
        }
    }
}

/// One body to write: its name, its shape snapshot when it has one, and
/// the mesh the viewport draws.
pub struct ExportBody<'a> {
    pub name: String,
    pub brep: Option<&'a [u8]>,
    /// Where the snapshot's shape sits in the document, when the body is
    /// placed: a rigid row-major 4×4 matrix. The viewport's mesh is placed
    /// already.
    pub transform: Option<[[f64; 4]; 4]>,
    pub mesh: &'a TriMesh,
}

/// What an export wrote.
#[derive(Debug)]
pub struct Exported {
    pub bytes: Vec<u8>,
    /// Bodies written.
    pub written: usize,
    /// Bodies the format could not carry, by name: a mesh body in STEP.
    pub skipped: Vec<String>,
    /// Triangles written, for the mesh formats.
    pub triangles: usize,
}

/// Write `bodies` as `format`; `detail` sets the mesh formats' tolerance.
pub fn export(
    bodies: &[ExportBody<'_>],
    format: ExportFormat,
    detail: &TessellationSettings,
) -> KernelResult<Exported> {
    match format {
        ExportFormat::Step => export_step(bodies),
        ExportFormat::Stl | ExportFormat::ThreeMf => export_mesh(bodies, format, detail),
    }
}

fn export_step(bodies: &[ExportBody<'_>]) -> KernelResult<Exported> {
    progress::context("Writing STEP");
    let mut model = Model::new();
    let mut parts = Vec::new();
    let mut skipped = Vec::new();
    for body in bodies {
        progress::checkpoint()
            .map_err(|e| KernelError::Other(anyhow::anyhow!("export stopped: {e}")))?;
        let Some(blob) = body.brep else {
            skipped.push(body.name.clone());
            continue;
        };
        let text = std::str::from_utf8(blob).map_err(|_| {
            KernelError::InvalidInput(format!("{}: snapshot is not UTF-8", body.name))
        })?;
        let absorbed = ogeom::io::native::read_into(&mut model, text).map_err(|e| {
            KernelError::InvalidInput(format!("{}: snapshot failed to parse: {e}", body.name))
        })?;
        for shape in absorbed.shapes {
            let shape = match &body.transform {
                Some(matrix) => crate::ops::pattern::moved(&mut model, &shape, matrix)
                    .map_err(|e| KernelError::Other(anyhow::anyhow!("{}: {e}", body.name)))?,
                None => shape,
            };
            parts.push((body.name.clone(), shape));
        }
    }
    if parts.is_empty() {
        return Err(KernelError::InvalidInput(
            "no body has a shape to write as STEP".into(),
        ));
    }
    let written = bodies.len() - skipped.len();
    let mut document = ogeom::doc::Document::over(model);
    for (name, shape) in parts {
        document.add_part(name, shape);
    }
    let text = ogeom::io::write_step(&document, tess::tolerances())
        .map_err(|e| KernelError::Other(anyhow::anyhow!("STEP writing failed: {e}")))?;
    Ok(Exported {
        bytes: text.into_bytes(),
        written,
        skipped,
        triangles: 0,
    })
}

fn export_mesh(
    bodies: &[ExportBody<'_>],
    format: ExportFormat,
    detail: &TessellationSettings,
) -> KernelResult<Exported> {
    progress::context(match format {
        ExportFormat::Stl => "Writing STL",
        _ => "Writing 3MF",
    });
    let mut meshes = Vec::with_capacity(bodies.len());
    for body in bodies {
        progress::checkpoint()
            .map_err(|e| KernelError::Other(anyhow::anyhow!("export stopped: {e}")))?;
        let mesh = match body.brep {
            Some(blob) => {
                let mut mesh = tess::tessellate_blob(blob, &[], detail, tess::Faces::Wide)?;
                if let Some(m) = &body.transform {
                    moved_mesh(&mut mesh, m);
                }
                mesh
            }
            None => body.mesh.clone(),
        };
        if !mesh.indices.is_empty() {
            meshes.push((body.name.clone(), welded(&mesh)));
        }
    }
    if meshes.is_empty() {
        return Err(KernelError::InvalidInput(
            "no body has triangles to write".into(),
        ));
    }
    let triangles = meshes.iter().map(|(_, t)| t.triangles.len()).sum();
    let bytes = match format {
        ExportFormat::Stl => {
            let mut soup = Triangulation::new();
            for (_, mesh) in &meshes {
                let base = soup.positions.len() as u32;
                soup.positions.extend_from_slice(&mesh.positions);
                soup.normals.extend_from_slice(&mesh.normals);
                soup.parameters.extend_from_slice(&mesh.parameters);
                soup.triangles
                    .extend(mesh.triangles.iter().map(|t| t.map(|i| i + base)));
            }
            ogeom::io::stl::write(&soup, ogeom::io::stl::Encoding::Binary)
                .map_err(|e| KernelError::Other(anyhow::anyhow!("STL writing failed: {e}")))?
        }
        _ => {
            let objects: Vec<ogeom::io::threemf::Object<'_>> = meshes
                .iter()
                .map(|(name, mesh)| ogeom::io::threemf::Object {
                    mesh,
                    name: Some(name.clone()),
                })
                .collect();
            ogeom::io::threemf::write_3mf(&objects)
        }
    };
    Ok(Exported {
        bytes,
        written: meshes.len(),
        skipped: Vec::new(),
        triangles,
    })
}

/// Move a mesh's points and normals by a rigid row-major matrix.
fn moved_mesh(mesh: &mut TriMesh, m: &[[f64; 4]; 4]) {
    let apply = |v: [f32; 3], w: f64| -> [f32; 3] {
        let v = v.map(f64::from);
        std::array::from_fn(|r| {
            (m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2] + m[r][3] * w) as f32
        })
    };
    for p in &mut mesh.positions {
        *p = apply(*p, 1.0);
    }
    for n in &mut mesh.normals {
        *n = apply(*n, 0.0);
    }
}

/// The mesh with coincident vertices merged, whatever face they came from:
/// a solid's faces share their boundary, and the file should say so.
fn welded(mesh: &TriMesh) -> Triangulation {
    const QUANTIZE: f64 = 1.0e5;
    let key = |p: [f32; 3]| p.map(|c| (f64::from(c) * QUANTIZE).round() as i64);
    let mut index: HashMap<[i64; 3], u32> = HashMap::with_capacity(mesh.positions.len());
    let mut positions = Vec::new();
    let remap: Vec<u32> = mesh
        .positions
        .iter()
        .map(|&p| {
            *index.entry(key(p)).or_insert_with(|| {
                positions.push(Point::new(
                    f64::from(p[0]),
                    f64::from(p[1]),
                    f64::from(p[2]),
                ));
                (positions.len() - 1) as u32
            })
        })
        .collect();
    let mut normals = vec![Vector::new(0.0, 0.0, 0.0); positions.len()];
    let triangles: Vec<[u32; 3]> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| t.map(|i| remap[i as usize]))
        // A sliver whose corners weld together is no triangle.
        .filter(|[a, b, c]| a != b && b != c && a != c)
        .collect();
    for t in &triangles {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        let n = (b - a).cross(c - a);
        for i in t {
            normals[*i as usize] += n;
        }
    }
    let normals = normals
        .into_iter()
        .map(|n| {
            let len = n.dot(n).sqrt();
            if len > 0.0 {
                n * (1.0 / len)
            } else {
                Vector::new(0.0, 0.0, 1.0)
            }
        })
        .collect();
    let count = positions.len();
    Triangulation {
        positions,
        normals,
        parameters: vec![(0.0, 0.0); count],
        triangles,
        deflection_met: true,
    }
}
