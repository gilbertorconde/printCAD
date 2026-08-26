//! STEP import via `ogeom::io::step` → `kernel_api::ImportedModel`.
//!
//! Bodies come back one per solid in file order. With
//! `persist_brep_snapshot` set (the default), each body carries a native
//! snapshot blob plus its per-face color table and meshing is deferred to the
//! kernel worker; otherwise (or on the legacy full-mesh path) meshes are
//! produced inline.

use std::path::Path;
use std::time::Instant;

use kernel_api::{
    ImportedBody, ImportedModel, ImportedNode, ImportedNodeKind, KernelError, KernelResult,
    LengthUnit, TessellationSettings, TriMesh,
};

use ogeom::core::parallel::map_ordered;
use ogeom::doc::{Document, ProductId, ProductKind};
use ogeom::math::{Point, Transform, Vector};
use ogeom::topo::{explore, Filter, Model, Shape, ShapeType};
use tracing::{info, warn};

use crate::{progress, tess};

pub fn import_step(
    path: &Path,
    detail: &TessellationSettings,
    force_inline_mesh: bool,
) -> KernelResult<ImportedModel> {
    let total = Instant::now();
    let bytes = std::fs::read(path)
        .map_err(|e| KernelError::Import(format!("failed to read {}: {e}", path.display())))?;
    // Part 21 is nominally ASCII, but exporters routinely write Latin-1 bytes
    // inside string literals — product names, authors. Decoding lossily keeps
    // those files importable; a replacement character can only ever land in a
    // name, never in geometry.
    let text = String::from_utf8_lossy(&bytes);

    let parse_start = Instant::now();
    progress::context("Reading STEP");
    let import = ogeom::io::step::read_step(&text, tess::tolerances())
        .map_err(|e| KernelError::Import(format!("STEP read failed: {e}")))?;
    let parse_ms = parse_start.elapsed().as_secs_f64() * 1000.0;

    for warning in &import.report.warnings {
        warn!(target: "printcad.kernel", "STEP import warning: {warning}");
    }
    if !import.report.untrimmed_faces.is_empty() {
        // The structured form of the warnings above: STEP entity ids of faces
        // whose boundary could not be trimmed to the surface, so their bodies
        // will draw with gaps. Named so a user can find them in the source
        // file rather than guess from prose.
        let faces = &import.report.untrimmed_faces;
        let shown = faces
            .iter()
            .take(16)
            .map(u64::to_string)
            .collect::<Vec<_>>();
        warn!(
            target: "printcad.kernel",
            count = faces.len(),
            "faces read without a complete trim and will draw with gaps; \
             STEP entity ids: #{}{}",
            shown.join(", #"),
            if faces.len() > shown.len() { ", …" } else { "" }
        );
    }
    if !import.report.skipped.is_empty() {
        info!(
            target: "printcad.kernel",
            skipped = ?import.report.skipped,
            "STEP entities not visited by the reader"
        );
    }

    let source_unit = unit_from_scale(import.report.scale_mm);
    let document = &import.document;
    let model = document.model();

    // Mesh here, from the model already in memory. Deferring it would mean
    // parsing every snapshot back afterwards, and re-parsing costs several
    // times what the meshing itself does — the round trip through text was
    // the dominant cost of a large import.
    let want_mesh = true;
    let want_blob = detail.persist_brep_snapshot && !force_inline_mesh;

    let sources = body_sources(document, &import.solids);

    // Each body's work only reads the model — colours, the snapshot blob and
    // bounds — so the bodies go wide. Serializing the snapshot dominates by
    // two orders of magnitude, which is what makes this worth threading.
    let loop_start = Instant::now();
    progress::context(format_args!("Preparing {} bodies", sources.len()));
    // Workers finish out of order; a shared counter keeps the announced
    // progress monotone regardless of which body lands when.
    let done = std::sync::atomic::AtomicU64::new(0);
    let body_count = sources.len() as u64;
    let computed = map_ordered(&sources, |i, source| -> KernelResult<ImportedBody> {
        progress::checkpoint().map_err(KernelError::Import)?;
        progress::stage_at(
            "bodies",
            done.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            body_count,
        );
        let shape = &source.shape;
        let face_colors = face_color_table(document, model, source.part, shape);

        let brep_blob = if want_blob {
            tess::write_blob(model, shape)?
        } else {
            Vec::new()
        };
        let mesh = if want_mesh {
            tess::mesh_shape_with(model, shape, &face_colors, detail, tess::Faces::Inline)
                .unwrap_or_else(|e| {
                    warn!(target: "printcad.kernel", body = i, "inline mesh failed: {e}");
                    TriMesh::default()
                })
        } else {
            TriMesh::default()
        };

        let bounds_mm = tess::robust_bounds(model, shape).map(|(lo, hi)| {
            (
                [lo.x as f32, lo.y as f32, lo.z as f32],
                [hi.x as f32, hi.y as f32, hi.z as f32],
            )
        });

        Ok(ImportedBody {
            name: source.name.clone(),
            mesh,
            brep_blob,
            face_colors,
            bounds_mm,
        })
    });
    let bodies = computed.into_iter().collect::<KernelResult<Vec<_>>>()?;
    let loop_ms = loop_start.elapsed().as_secs_f64() * 1000.0;

    let nodes = nodes_from_document(document, bodies.len());

    info!(
        "Imported STEP `{}`: {} bodies, source unit {:?}, parse {:.1} ms, \
         bodies {:.1} ms, total {:.1} ms",
        path.display(),
        bodies.len(),
        source_unit,
        parse_ms,
        loop_ms,
        total.elapsed().as_secs_f64() * 1000.0,
    );

    Ok(ImportedModel {
        bodies,
        nodes,
        source_unit,
    })
}

fn unit_from_scale(scale_mm: f64) -> Option<LengthUnit> {
    const EPS: f64 = 1e-6;
    let close = |target: f64| (scale_mm - target).abs() < EPS * target.max(1.0);
    if close(1.0) {
        Some(LengthUnit::Millimetre)
    } else if close(10.0) {
        Some(LengthUnit::Centimetre)
    } else if close(1000.0) {
        Some(LengthUnit::Metre)
    } else if close(25.4) {
        Some(LengthUnit::Inch)
    } else if close(304.8) {
        Some(LengthUnit::Foot)
    } else {
        None
    }
}

/// One body to import: a shape, and who it belongs to.
struct BodySource {
    /// World-space shape. For an assembly this is the part's shape carried
    /// down through every placement above it.
    shape: Shape,
    /// The part product it came from, for colour inheritance.
    part: Option<ProductId>,
    name: Option<String>,
}

/// The bodies an import should produce.
///
/// **Placed occurrences, not `import.solids`.** The raw solids are one per
/// `MANIFOLD_SOLID_BREP` in part-local coordinates, so an assembly built from
/// them puts every part at its own origin — parts that look right on their
/// own, scattered in relation to each other. `occurrences_of` carries each
/// part's shape down through the placements above it, which is the whole
/// point of the product tree.
///
/// Files with no product structure still import: their solids stand in as
/// occurrences at identity.
fn body_sources(document: &Document, solids: &[Shape]) -> Vec<BodySource> {
    let mut out = Vec::new();
    for root in document.roots() {
        let Ok(occurrences) = document.occurrences_of(root) else {
            continue;
        };
        for occurrence in occurrences {
            // The trailing path segment is the instance name where the file
            // gave one, which is what a user recognises in the tree.
            let name = occurrence
                .path
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            out.push(BodySource {
                shape: occurrence.shape,
                part: Some(occurrence.part),
                name,
            });
        }
    }
    if out.is_empty() {
        out.extend(solids.iter().map(|shape| BodySource {
            shape: shape.clone(),
            part: None,
            name: None,
        }));
    }
    out
}

/// Per-face RGB in face-exploration order, colour inheritance resolved
/// through the owning product; white where nothing is coloured.
fn face_color_table(
    document: &Document,
    model: &Model,
    product: Option<ProductId>,
    solid: &Shape,
) -> Vec<[f32; 3]> {
    let Ok(faces) = explore(model, solid, Filter::OfType(ShapeType::Face)) else {
        return Vec::new();
    };
    let mut any_colored = false;
    let colors: Vec<[f32; 3]> = faces
        .iter()
        .map(|face| {
            let colour = match product {
                Some(id) => document.resolved_colour(id, face),
                None => document.colour_of(face),
            };
            match colour {
                Some(c) => {
                    any_colored = true;
                    [c.r as f32, c.g as f32, c.b as f32]
                }
                None => tess::WHITE,
            }
        })
        .collect();
    if any_colored {
        colors
    } else {
        // No colour data at all: leave the table empty so the renderer's
        // body tint applies instead of forcing white.
        Vec::new()
    }
}

/// Rebuild the assembly tree as `ImportedNode`s. Products become
/// Assembly/Part nodes; assembly children become Instance nodes carrying the
/// placement, with the instanced product's subtree repeated beneath them.
/// Falls back to a flat "Body N" list when the document has no product tree.
fn nodes_from_document(document: &Document, body_count: usize) -> Vec<ImportedNode> {
    let mut nodes = Vec::new();
    let mut next_id = 1u64;
    // Bodies were produced by `occurrences_of`, whose flattening is a preorder
    // walk of the product tree. Mirroring that walk here means the n-th part
    // leaf we reach is the n-th body — an index correspondence rather than a
    // fragile match on names or shapes.
    let mut next_body = 0usize;

    #[expect(clippy::too_many_arguments)]
    fn walk(
        document: &Document,
        product_id: ProductId,
        parent: Option<u64>,
        next_id: &mut u64,
        next_body: &mut usize,
        nodes: &mut Vec<ImportedNode>,
        body_count: usize,
        depth: usize,
    ) {
        if depth > 64 {
            warn!(target: "printcad.kernel", "assembly tree deeper than 64 levels; truncated");
            return;
        }
        let Some(product) = document.get(product_id) else {
            return;
        };
        let id = *next_id;
        *next_id += 1;
        match &product.kind {
            ProductKind::Part { .. } => {
                let body_index = (*next_body < body_count).then(|| {
                    let index = *next_body;
                    *next_body += 1;
                    index
                });
                nodes.push(ImportedNode {
                    id,
                    parent_id: parent,
                    name: Some(product.name.clone()),
                    kind: ImportedNodeKind::Part,
                    visible: true,
                    body_index,
                    local_transform: None,
                });
            }
            ProductKind::Assembly { children } => {
                nodes.push(ImportedNode {
                    id,
                    parent_id: parent,
                    name: Some(product.name.clone()),
                    kind: ImportedNodeKind::Assembly,
                    visible: true,
                    body_index: None,
                    local_transform: None,
                });
                for instance in children {
                    let inst_id = *next_id;
                    *next_id += 1;
                    // Informational only: the geometry already carries this
                    // placement, because the body is the placed occurrence.
                    let local_transform = instance
                        .location
                        .composed(document.model().datums())
                        .ok()
                        .map(|t| matrix_rows(&t));
                    nodes.push(ImportedNode {
                        id: inst_id,
                        parent_id: Some(id),
                        name: instance.name.clone(),
                        kind: ImportedNodeKind::Instance,
                        visible: true,
                        body_index: None,
                        local_transform,
                    });
                    walk(
                        document,
                        instance.product,
                        Some(inst_id),
                        next_id,
                        next_body,
                        nodes,
                        body_count,
                        depth + 1,
                    );
                }
            }
        }
    }

    for root in document.roots() {
        walk(
            document,
            root,
            None,
            &mut next_id,
            &mut next_body,
            &mut nodes,
            body_count,
            0,
        );
    }

    if nodes.is_empty() && body_count > 0 {
        nodes.extend((0..body_count).map(|idx| ImportedNode {
            id: (idx + 1) as u64,
            parent_id: None,
            name: Some(format!("Body {}", idx + 1)),
            kind: ImportedNodeKind::Part,
            visible: true,
            body_index: Some(idx),
            local_transform: None,
        }));
    }
    nodes
}

/// Row-major 4×4 from a similarity transform, extracted through the public
/// apply surface so the private matrix layout stays private.
fn matrix_rows(t: &Transform) -> [[f32; 4]; 4] {
    let cx = t.apply_vector(Vector::new(1.0, 0.0, 0.0));
    let cy = t.apply_vector(Vector::new(0.0, 1.0, 0.0));
    let cz = t.apply_vector(Vector::new(0.0, 0.0, 1.0));
    let o = t.apply(Point::new(0.0, 0.0, 0.0));
    [
        [cx.x as f32, cy.x as f32, cz.x as f32, o.x as f32],
        [cx.y as f32, cy.y as f32, cz.y as f32, o.y as f32],
        [cx.z as f32, cy.z as f32, cz.z as f32, o.z as f32],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
