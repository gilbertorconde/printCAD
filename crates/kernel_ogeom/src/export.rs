//! Export: the document's bodies written as STEP, STL or 3MF.
//!
//! STEP carries the exact shapes, one part per body, gathered into one
//! kernel model, and the bodies' notes as drawn callouts: a leader from
//! the point to the label, named by the note's text and carrying it as
//! text, naming the faces the note is about on its own part. The mesh
//! formats carry triangles: a body with a shape is meshed afresh at the export's own tolerance (a print wants a finer mesh
//! than the viewport), and a mesh body goes out as the mesh it is. Each
//! body's mesh is welded by position, so a closed solid arrives as a
//! closed mesh rather than as faces that merely touch.

use std::collections::HashMap;
use std::path::Path;

use kernel_api::{KernelError, KernelResult, TessellationSettings, TriMesh};
use ogeom::math::{Point, Transform, Vector};
use ogeom::topo::{Model, Shape, Triangulation};

use crate::{progress, tess};

/// A file format export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Step,
    /// STEP with every surface and curve a NURBS: exact where one is, fitted
    /// within [`NURBS_TOLERANCE`] where not, for a reader that takes no other.
    StepNurbs,
    Stl,
    ThreeMf,
}

/// How far a surface fitted for a NURBS-only STEP may stray, in millimetres.
pub const NURBS_TOLERANCE: f64 = 1e-3;

impl ExportFormat {
    pub const ALL: [ExportFormat; 4] = [
        ExportFormat::Step,
        ExportFormat::StepNurbs,
        ExportFormat::Stl,
        ExportFormat::ThreeMf,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Step => "STEP",
            ExportFormat::StepNurbs => "STEP (NURBS)",
            ExportFormat::Stl => "STL",
            ExportFormat::ThreeMf => "3MF",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Step | ExportFormat::StepNurbs => "step",
            ExportFormat::Stl => "stl",
            ExportFormat::ThreeMf => "3mf",
        }
    }

    /// Whether the format carries triangles, and so a mesh tolerance.
    pub fn is_mesh(self) -> bool {
        matches!(self, ExportFormat::Stl | ExportFormat::ThreeMf)
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
    /// What a mesh file writes of the body's faces once meshed, in the
    /// body's own frame, before it is placed (a surface texture pressed
    /// in); `None` writes them as meshed.
    pub finish: Option<&'a (dyn Fn(TriMesh) -> TriMesh + Sync)>,
}

/// A note written with a body to STEP: its text, the point its leader
/// starts at and where its label stands, in the body's own frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportNote {
    /// Which of the bodies it is on, by index.
    pub body: usize,
    pub text: String,
    pub at: [f32; 3],
    pub label: [f32; 3],
    /// Points on the faces the note is about, in the body's own frame:
    /// the callout names the face of the body's shape each lies on, so a
    /// reader ties the note to that face of that part.
    pub faces: Vec<[f64; 3]>,
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
    export_with_notes(bodies, &[], format, detail)
}

/// Write `bodies` as `format`, with `notes` on them where the format
/// carries them (STEP); `detail` sets the mesh formats' tolerance.
pub fn export_with_notes(
    bodies: &[ExportBody<'_>],
    notes: &[ExportNote],
    format: ExportFormat,
    detail: &TessellationSettings,
) -> KernelResult<Exported> {
    match format {
        ExportFormat::Step => export_step(bodies, notes, false),
        ExportFormat::StepNurbs => export_step(bodies, notes, true),
        ExportFormat::Stl | ExportFormat::ThreeMf => export_mesh(bodies, format, detail),
    }
}

/// A rigid row-major 4×4 matrix.
type Matrix = [[f64; 4]; 4];

fn export_step(
    bodies: &[ExportBody<'_>],
    notes: &[ExportNote],
    nurbs: bool,
) -> KernelResult<Exported> {
    progress::context("Writing STEP");
    let mut model = Model::new();
    let mut skipped = Vec::new();
    // Each distinct shape once, as a part in its own frame; each body an
    // instance of its part where the body sits.
    let mut shapes: Vec<(&[u8], String, Vec<Shape>)> = Vec::new();
    let mut placed: Vec<(usize, String, Option<Matrix>)> = Vec::new();
    // Each body's part, by the body's index.
    let mut part_of: Vec<Option<usize>> = Vec::with_capacity(bodies.len());
    for body in bodies {
        progress::checkpoint()
            .map_err(|e| KernelError::Other(anyhow::anyhow!("export stopped: {e}")))?;
        let Some(blob) = body.brep else {
            skipped.push(body.name.clone());
            part_of.push(None);
            continue;
        };
        let part = match shapes.iter().position(|(b, _, _)| *b == blob) {
            Some(part) => part,
            None => {
                let text = std::str::from_utf8(blob).map_err(|_| {
                    KernelError::InvalidInput(format!("{}: snapshot is not UTF-8", body.name))
                })?;
                let absorbed = ogeom::io::native::read_into(&mut model, text).map_err(|e| {
                    KernelError::InvalidInput(format!(
                        "{}: snapshot failed to parse: {e}",
                        body.name
                    ))
                })?;
                let mut own = Vec::new();
                for shape in absorbed.shapes {
                    own.push(if nurbs {
                        ogeom::algo::to_nurbs_within(
                            &mut model,
                            &shape,
                            NURBS_TOLERANCE,
                            tess::tolerances(),
                        )
                        .map(|built| built.shape)
                        .map_err(|e| {
                            KernelError::Other(anyhow::anyhow!(
                                "{}: converting to NURBS failed: {e}",
                                body.name
                            ))
                        })?
                    } else {
                        shape
                    });
                }
                shapes.push((blob, body.name.clone(), own));
                shapes.len() - 1
            }
        };
        placed.push((part, body.name.clone(), body.transform));
        part_of.push(Some(part));
    }
    if placed.is_empty() {
        return Err(KernelError::InvalidInput(
            "no body has a shape to write as STEP".into(),
        ));
    }
    let written = placed.len();
    let mut document = ogeom::doc::Document::over(model);
    let fail = |e: ogeom::core::OgeomError| {
        KernelError::Other(anyhow::anyhow!("STEP writing failed: {e}"))
    };
    // The shapes each part is written as: where its body sits when there
    // is one body, else in the part's own frame.
    let single = placed.len() == 1;
    let mut parts: Vec<Vec<Shape>> = shapes.iter().map(|(_, _, own)| own.clone()).collect();
    if single {
        // One body: the part itself, where it sits.
        let (part, name, transform) = placed.remove(0);
        let mut written = Vec::new();
        for shape in &shapes[part].2 {
            let shape = match &transform {
                Some(matrix) => crate::ops::pattern::moved(document.model_mut(), shape, matrix)
                    .map_err(|e| KernelError::Other(anyhow::anyhow!("{name}: {e}")))?,
                None => shape.clone(),
            };
            document.add_part(name.clone(), shape.clone());
            written.push(shape);
        }
        parts[part] = written;
    } else {
        let root = document.add_assembly("Assembly");
        let mut products = Vec::with_capacity(shapes.len());
        for (_, name, own) in &shapes {
            let product = match own.as_slice() {
                [one] => document.add_part(name.clone(), one.clone()),
                many => {
                    let group = document.add_assembly(name.clone());
                    for (i, shape) in many.iter().enumerate() {
                        let piece = document.add_part(format!("{name} {}", i + 1), shape.clone());
                        document
                            .add_instance(group, piece, Transform::IDENTITY, None)
                            .map_err(fail)?;
                    }
                    group
                }
            };
            products.push(product);
        }
        for (part, name, transform) in placed {
            let at = match &transform {
                Some(matrix) => crate::ops::pattern::rigid_of(matrix).ok_or_else(|| {
                    KernelError::InvalidInput(format!("{name}: its placement is not rigid"))
                })?,
                None => Transform::IDENTITY,
            };
            document
                .add_instance(root, products[part], at, Some(name))
                .map_err(fail)?;
        }
    }
    // Each note where its body sits: the drawing's coordinates are the
    // document's, the bodies' placements applied. Its words go as the
    // callout's text as well as its name, and the faces it is about as the
    // topology it describes, which ties it to its own part.
    for note in notes {
        let (Some(body), Some(Some(part))) = (bodies.get(note.body), part_of.get(note.body)) else {
            continue;
        };
        let place = |p: [f64; 3]| {
            let p = match &body.transform {
                Some(m) => std::array::from_fn(|r| {
                    m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3]
                }),
                None => p,
            };
            Point::new(p[0], p[1], p[2])
        };
        let items = note
            .faces
            .iter()
            .filter_map(|&p| {
                let probe = if single {
                    place(p)
                } else {
                    Point::new(p[0], p[1], p[2])
                };
                face_at(document.model_mut(), &parts[*part], probe)
            })
            .collect();
        document.pmi_mut().callouts.push(ogeom::doc::Callout {
            name: note.text.clone(),
            plane: None,
            polylines: vec![vec![
                place(note.at.map(f64::from)),
                place(note.label.map(f64::from)),
            ]],
            annotates: None,
            items,
            text: Some(note.text.clone()),
        });
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

/// The face of `shapes` nearest `probe`, when it lies within a pick's
/// reach of it.
fn face_at(model: &mut Model, shapes: &[Shape], probe: Point) -> Option<ogeom::topo::TShapeId> {
    let mut best: Option<(f64, Shape)> = None;
    for shape in shapes {
        let Ok((face, distance)) = crate::ops::dressup::nearest_with_distance(
            model,
            shape,
            ogeom::topo::ShapeType::Face,
            probe,
        ) else {
            continue;
        };
        if distance <= crate::ops::dressup::pick_reach(model, shape)
            && best.as_ref().is_none_or(|(d, _)| distance < *d)
        {
            best = Some((distance, face));
        }
    }
    best.map(|(_, face)| face.node())
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
                let mesh = tess::tessellate_blob(blob, &[], detail, tess::Faces::Wide)?;
                let mut mesh = match body.finish {
                    Some(finish) => finish(mesh),
                    None => mesh,
                };
                if let Some(m) = &body.transform {
                    moved_mesh(&mut mesh, m);
                }
                mesh
            }
            None => match body.finish {
                Some(finish) => finish(body.mesh.clone()),
                None => body.mesh.clone(),
            },
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
