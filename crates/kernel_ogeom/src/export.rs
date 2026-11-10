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

use crate::convert::{point, vector};
use crate::{progress, tess};

mod pmi;

/// Semantic annotations of an exported body, with probes in its own frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportPmi {
    pub body: usize,
    pub annotations: kernel_api::SemanticPmi,
}

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
    /// What a mesh file writes scaled by, per axis of the document's frame,
    /// about the body's middle where it sits: a printer's shrinkage made up
    /// for. STEP writes the body as drawn.
    pub scale: Option<[f64; 3]>,
    /// How much larger a mesh file draws the solid's round holes and
    /// bosses in radius, mm, `[holes, bosses]`: a printer that prints
    /// them off made up for, before the scale. STEP writes the body as
    /// drawn, and a body without a solid as it is.
    pub rounds: Option<[f64; 2]>,
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
    /// Semantic annotations not written, by body and name: none of their
    /// probes landed on a face of the body's shape.
    pub annotations_left_out: Vec<String>,
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
    export_with_annotations(bodies, notes, &[], format, detail)
}

/// Write drawn notes and face-associated semantic PMI where the format
/// carries annotations (STEP). Mesh formats write geometry alone.
pub fn export_with_annotations(
    bodies: &[ExportBody<'_>],
    notes: &[ExportNote],
    semantic: &[ExportPmi],
    format: ExportFormat,
    detail: &TessellationSettings,
) -> KernelResult<Exported> {
    match format {
        ExportFormat::Step => export_step(bodies, notes, semantic, false),
        ExportFormat::StepNurbs => export_step(bodies, notes, semantic, true),
        ExportFormat::Stl | ExportFormat::ThreeMf => export_mesh(bodies, format, detail),
    }
}

/// A rigid row-major 4×4 matrix.
type Matrix = [[f64; 4]; 4];

fn export_step(
    bodies: &[ExportBody<'_>],
    notes: &[ExportNote],
    semantic: &[ExportPmi],
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
    let mut annotated_parts = std::collections::HashSet::new();
    for (index, body) in bodies.iter().enumerate() {
        progress::checkpoint()
            .map_err(|e| KernelError::Other(anyhow::anyhow!("export stopped: {e}")))?;
        let Some(blob) = body.brep else {
            skipped.push(body.name.clone());
            part_of.push(None);
            continue;
        };
        // Semantic PMI belongs to this body, not every copy of its shape.
        let annotated = semantic
            .iter()
            .any(|p| p.body == index && !p.annotations.is_empty());
        let shared = (!annotated)
            .then(|| {
                shapes
                    .iter()
                    .enumerate()
                    .position(|(i, (b, _, _))| *b == blob && !annotated_parts.contains(&i))
            })
            .flatten();
        let part = match shared {
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
        if annotated {
            annotated_parts.insert(part);
        }
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
            point(p)
        };
        let items = note
            .faces
            .iter()
            .filter_map(|&p| {
                let probe = if single { place(p) } else { point(p) };
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
    let left_out = pmi::write(&mut document, bodies, semantic, &part_of, &parts, single);
    let text = ogeom::io::write_step(&document, tess::tolerances())
        .map_err(|e| KernelError::Other(anyhow::anyhow!("STEP writing failed: {e}")))?;
    Ok(Exported {
        bytes: text.into_bytes(),
        written,
        skipped,
        triangles: 0,
        annotations_left_out: left_out,
    })
}

/// The face of `shapes` nearest `probe`, when it lies within a pick's
/// reach of it.
fn face_at(model: &mut Model, shapes: &[Shape], probe: Point) -> Option<ogeom::topo::TShapeId> {
    let pick = crate::probe::at_point(probe, 0);
    shapes
        .iter()
        .filter_map(|shape| {
            crate::probe::face_within(model, shape, &pick, crate::probe::Reach::Pick)
                .ok()
                .flatten()
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(face, _)| face.node())
}

/// `mesh` scaled by `scale` per axis about the middle of its bounds; its
/// normals turned to stay square to its faces.
fn scaled_mesh(mesh: &mut TriMesh, scale: [f64; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &mesh.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let mid: [f64; 3] = std::array::from_fn(|k| (f64::from(lo[k]) + f64::from(hi[k])) / 2.0);
    for p in &mut mesh.positions {
        *p = std::array::from_fn(|k| (mid[k] + (f64::from(p[k]) - mid[k]) * scale[k]) as f32);
    }
    for n in &mut mesh.normals {
        let m: [f64; 3] = std::array::from_fn(|k| f64::from(n[k]) / scale[k]);
        let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt().max(1e-12);
        *n = m.map(|c| (c / len) as f32);
    }
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
                let mesh = match body.rounds.filter(|r| r.iter().any(|d| *d != 0.0)) {
                    Some([hole, boss]) => {
                        let (mut model, root) = tess::read_blob(blob)?;
                        let shape = crate::rounds::offset_rounds(&mut model, &root, hole, boss)
                            .map_err(|e| {
                                KernelError::Other(anyhow::anyhow!("{}: {e}", body.name))
                            })?;
                        tess::mesh_shape_with(&model, &shape, &[], detail, tess::Faces::Wide)?
                    }
                    None => tess::tessellate_blob(blob, &[], detail, tess::Faces::Wide)?,
                };
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
        let mut mesh = mesh;
        if let Some(scale) = body.scale {
            scaled_mesh(&mut mesh, scale);
        }
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
        annotations_left_out: Vec::new(),
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

/// A STEP file of one part, `part`, carrying `items` as construction
/// geometry and no solid.
pub fn construction_step(
    part: &str,
    items: &[kernel_api::ConstructionItem],
) -> KernelResult<Vec<u8>> {
    let document = construction_document(part, items)?;
    let text = ogeom::io::write_step(&document, tess::tolerances())
        .map_err(|e| KernelError::Other(anyhow::anyhow!("STEP writing failed: {e}")))?;
    Ok(text.into_bytes())
}

/// An IGES file carrying `items` as construction geometry, with writer warnings.
pub fn construction_iges(
    part: &str,
    items: &[kernel_api::ConstructionItem],
) -> KernelResult<kernel_api::IgesExport> {
    let document = construction_document(part, items)?;
    let written = ogeom::io::iges::write_iges_reported(&document, tess::tolerances())
        .map_err(|e| KernelError::Other(anyhow::anyhow!("IGES writing failed: {e}")))?;
    Ok(kernel_api::IgesExport {
        text: written.text,
        warnings: written.warnings,
    })
}

fn construction_document(
    part: &str,
    items: &[kernel_api::ConstructionItem],
) -> KernelResult<ogeom::doc::Document> {
    use kernel_api::ConstructionShape as S;
    use ogeom::doc::ConstructionGeometry as G;
    use ogeom::geom::{BSplineCurve, CircleCurve, Curve, PlaneSurface};
    use ogeom::math::{Circle, Direction, Frame, KnotVector, Plane};
    if items.is_empty() {
        return Err(KernelError::InvalidInput("nothing to write".into()));
    }
    let tol = tess::tolerances();
    let fail =
        |name: &str, e: ogeom::core::OgeomError| KernelError::InvalidInput(format!("{name}: {e}"));
    let point = |p: [f64; 3]| point(p);
    let direction = |v: [f64; 3]| Direction::new(vector(v), tol);
    let mut model = Model::new();
    let nothing = ogeom::algo::make_compound(&mut model, &[])
        .map_err(|e| fail(part, e))?
        .shape;
    let mut document = ogeom::doc::Document::over(model);
    let product = document.add_part(part.to_string(), nothing);
    for item in items {
        let name = item.name.as_str();
        let geometry = match &item.shape {
            S::Point(p) => G::Point(point(*p)),
            S::Axis { origin, z, x } => G::Axis(
                Frame::new(
                    point(*origin),
                    direction(*z).map_err(|e| fail(name, e))?,
                    direction(*x).map_err(|e| fail(name, e))?,
                    tol,
                )
                .map_err(|e| fail(name, e))?,
            ),
            S::Polyline { points, closed } => {
                let mut at: Vec<Point> = points.iter().map(|p| point(*p)).collect();
                if *closed && let Some(first) = at.first().copied() {
                    at.push(first);
                }
                let n = at.len();
                if n < 2 {
                    continue;
                }
                // Degree one, a knot at every point.
                let mut knots = vec![0.0];
                knots.extend((0..n).map(|i| i as f64));
                knots.push((n - 1) as f64);
                let curve = BSplineCurve::new(
                    KnotVector::new(knots, 1).map_err(|e| fail(name, e))?,
                    at,
                    tol,
                )
                .map_err(|e| fail(name, e))?;
                G::Curve {
                    curve: Curve::from(curve),
                    range: (0.0, (n - 1) as f64),
                }
            }
            S::Circle {
                centre,
                normal,
                radius,
            } => {
                let z = direction(*normal).map_err(|e| fail(name, e))?;
                let circle = Circle::new(Frame::about(point(*centre), z), *radius, tol)
                    .map_err(|e| fail(name, e))?;
                G::Curve {
                    curve: Curve::from(CircleCurve::new(circle)),
                    range: (0.0, std::f64::consts::TAU),
                }
            }
            S::Plane {
                origin,
                normal,
                x_axis,
                u,
                v,
            } => {
                let frame = Frame::new(
                    point(*origin),
                    direction(*normal).map_err(|e| fail(name, e))?,
                    direction(*x_axis).map_err(|e| fail(name, e))?,
                    tol,
                )
                .map_err(|e| fail(name, e))?;
                let surface = PlaneSurface::over(
                    Plane::new(frame),
                    (u[0].min(-1e3), u[1].max(1e3)),
                    (v[0].min(-1e3), v[1].max(1e3)),
                )
                .map_err(|e| fail(name, e))?;
                G::Surface {
                    surface: surface.into(),
                    u: (u[0], u[1]),
                    v: (v[0], v[1]),
                }
            }
        };
        document
            .add_construction(product, name, geometry)
            .map_err(|e| fail(name, e))?;
    }
    Ok(document)
}
