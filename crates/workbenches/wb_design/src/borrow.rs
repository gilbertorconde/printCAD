//! Geometry one body borrows from another (`DesignFeature::Borrow`): a
//! sketch, or faces and edges of a solid, placed in the borrowing body's
//! frame where the two bodies sit. A live borrow follows its source (the
//! sketch as it is solved now, the solid as it is built now, the bodies as
//! they are placed now); a frozen one keeps the snapshot taken when it was
//! frozen, stored with it.
//!
//! Features of the borrowing body take a borrowed sketch as their profile
//! (through `build::load_sketch`), a borrowed face as the face they stop on
//! and a borrowed edge as the axis they turn about or the way they run.

use std::collections::HashMap;

use core_document::{BodyId, BodyPlacement, Document, FeatureId, WorkbenchFeature};
use kernel_api::{FaceSurface, KernelQueries, TriMesh};
use wb_sketch::SketchFeature;

use crate::feature::{
    BorrowOptions, BorrowSource, BorrowedRef, DesignFeature, EdgePick, FacePick, FrozenBorrow,
    FrozenEdge, FrozenFace,
};

/// A borrow feature as the build reads it.
pub(crate) struct Borrow {
    /// The body that borrows.
    pub body: BodyId,
    pub source: BorrowSource,
    pub frozen: Option<FrozenBorrow>,
    pub options: BorrowOptions,
}

impl Borrow {
    /// What the borrow's offset does, on top of where the bodies put it.
    fn offset(&self) -> BodyPlacement {
        self.options.placement()
    }

    /// Whether it lends a sketch: a borrowed sketch, or edges it fills.
    fn lends_sketch(&self) -> bool {
        match &self.frozen {
            Some(frozen) => {
                frozen.sketch.is_some() || (self.options.fill && !frozen.edges.is_empty())
            }
            None => match &self.source {
                BorrowSource::Sketch(_) => true,
                BorrowSource::Solid { .. } => self.options.fill,
            },
        }
    }
}

/// `rows` (a row-major rigid transform) done after by `then`.
fn then_rows(then: &BodyPlacement, rows: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let a = glam::DMat4::from_cols_array_2d(&then.rows()).transpose();
    let b = glam::DMat4::from_cols_array_2d(&rows).transpose();
    (a * b).transpose().to_cols_array_2d()
}

/// The borrow `id` is, when it is one.
pub(crate) fn borrow_of(document: &Document, id: FeatureId) -> Option<Borrow> {
    let node = document.get_feature_meta(id)?;
    if node.workbench_id.as_str() != "wb.design" {
        return None;
    }
    match DesignFeature::from_json(document.feature_values(id)?).ok()? {
        DesignFeature::Borrow {
            source,
            frozen,
            options,
        } => Some(Borrow {
            body: node.body?,
            source,
            frozen,
            options,
        }),
        _ => None,
    }
}

/// The body the source lives in: the sketch's body, or the solid's. `None`
/// for a sketch that belongs to no body, which lies in the document's frame.
pub(crate) fn source_body(document: &Document, source: &BorrowSource) -> Option<BodyId> {
    match source {
        BorrowSource::Sketch(sketch) => document.get_feature_meta(*sketch)?.body,
        BorrowSource::Solid { body, .. } => Some(*body),
    }
}

/// What carries a point of `source`'s frame (the document's when `None`)
/// into `using`'s frame, where the two bodies sit.
pub(crate) fn relative(
    document: &Document,
    using: BodyId,
    source: Option<BodyId>,
) -> BodyPlacement {
    let into_using = document.body_placement(using).inverse();
    match source {
        Some(source) => into_using.after(&document.body_placement(source)),
        None => into_using,
    }
}

/// Why `source` cannot be borrowed by `using`, if it cannot.
pub(crate) fn refusal(document: &Document, using: BodyId, source: &BorrowSource) -> Option<String> {
    match source {
        BorrowSource::Sketch(sketch) => {
            let Some(node) = document.get_feature_meta(*sketch) else {
                return Some("the borrowed sketch is not in this document".into());
            };
            if node.workbench_id.as_str() != "wb.sketch" {
                return Some("what it borrows is not a sketch".into());
            }
            (node.body == Some(using))
                .then(|| "the sketch is this body's own; a feature can take it directly".into())
        }
        BorrowSource::Solid { body, .. } => {
            if *body == using {
                return Some("a body cannot borrow from itself; pick another body".into());
            }
            (!document.bodies().iter().any(|b| b.id == *body))
                .then(|| "the body it borrows from is not in this document".into())
        }
    }
}

/// A sketch feature's plane (and its sketch's copy of it) moved by
/// `placement`: the same sketch, seen from another frame.
fn placed_sketch(mut feature: SketchFeature, placement: &BodyPlacement) -> SketchFeature {
    use wb_sketch::boundary::{place_direction, place_point};
    let place = |plane: &mut wb_sketch::sketch::SketchPlane| {
        plane.origin = place_point(placement, plane.origin);
        plane.normal = place_direction(placement, plane.normal);
        plane.x_axis = place_direction(placement, plane.x_axis);
        plane.y_axis = place_direction(placement, plane.y_axis);
    };
    place(&mut feature.plane);
    place(&mut feature.sketch.plane);
    // The copy's plane is where it lies here, whatever it follows at home.
    feature.support = None;
    feature
}

/// The source sketch as it is now, in `using`'s frame.
fn live_sketch(
    document: &Document,
    using: BodyId,
    sketch: FeatureId,
) -> Result<SketchFeature, String> {
    let data = document
        .feature_values(sketch)
        .ok_or("the borrowed sketch is not in this document")?;
    let feature =
        SketchFeature::from_json(data).map_err(|e| format!("invalid borrowed sketch: {e}"))?;
    let home = document.get_feature_meta(sketch).and_then(|n| n.body);
    Ok(placed_sketch(feature, &relative(document, using, home)))
}

/// The sketch the borrow `id` lends, in its borrowing body's frame; `None`
/// when `id` is not a borrow.
pub(crate) fn sketch(document: &Document, id: FeatureId) -> Option<Result<SketchFeature, String>> {
    let borrow = borrow_of(document, id)?;
    Some(borrowed_sketch(document, &borrow))
}

fn borrowed_sketch(document: &Document, borrow: &Borrow) -> Result<SketchFeature, String> {
    if borrow.options.fill && !matches!(borrow.source, BorrowSource::Sketch(_)) {
        let (_, edges) = seen(document, borrow);
        return filled(&edges);
    }
    if let Some(frozen) = &borrow.frozen {
        let data = frozen
            .sketch
            .as_ref()
            .ok_or("this borrow was frozen with no sketch in it")?;
        let feature =
            SketchFeature::from_json(data).map_err(|e| format!("invalid frozen sketch: {e}"))?;
        return Ok(placed_sketch(feature, &borrow.offset()));
    }
    let BorrowSource::Sketch(sketch) = &borrow.source else {
        return Err("this borrow lends faces and edges, not a sketch".into());
    };
    if let Some(why) = refusal(document, borrow.body, &borrow.source) {
        return Err(why);
    }
    Ok(placed_sketch(
        live_sketch(document, borrow.body, *sketch)?,
        &borrow.offset(),
    ))
}

/// Whether the borrow `id` lends a sketch.
pub(crate) fn lends_sketch(document: &Document, id: FeatureId) -> bool {
    borrow_of(document, id).is_some_and(|b| b.lends_sketch())
}

/// How far apart two points of borrowed edges may be and still be one.
const JOIN_MM: f64 = 1e-3;

/// The face borrowed `edges` bound, as a sketch on their plane: straight
/// pieces as lines, a piece that keeps to one circle as that circle or an
/// arc of it. Refused when they leave the plane or do not close.
pub(crate) fn filled(edges: &[FrozenEdge]) -> Result<SketchFeature, String> {
    use glam::DVec3;
    use wb_sketch::boundary::widen3;
    use wb_sketch::sketch::{
        Arc, Circle, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D,
    };
    let points: Vec<DVec3> = edges
        .iter()
        .flat_map(|e| e.outline.iter().map(|p| DVec3::from_array(widen3(*p))))
        .collect();
    let Some(&p0) = points.first() else {
        return Err("fill needs borrowed edges".into());
    };
    let size = points
        .iter()
        .map(|p| p.distance(p0))
        .fold(0.0, f64::max)
        .max(1.0);
    // The plane: the first point and the two directions from it that
    // span the most.
    let far = points
        .iter()
        .copied()
        .max_by(|a, b| a.distance(p0).total_cmp(&b.distance(p0)))
        .unwrap_or(p0);
    let u = far - p0;
    let normal = points
        .iter()
        .map(|p| u.cross(*p - p0))
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .unwrap_or(DVec3::Z);
    if normal.length() < 1e-6 * size * size {
        return Err("the borrowed edges lie in a line, which bounds no face".into());
    }
    let normal = normal / normal.length();
    if points
        .iter()
        .any(|p| (*p - p0).dot(normal).abs() > 1e-3 * size)
    {
        return Err("the borrowed edges do not lie in one plane".into());
    }
    let x_axis = u / u.length();
    let plane = SketchPlane::from_frame(p0.to_array(), normal.to_array(), x_axis.to_array());
    let flat = |p: [f32; 3]| {
        let d = DVec3::from_array(widen3(p)) - DVec3::from_array(plane.origin);
        Vec2D::new(
            d.dot(DVec3::from_array(plane.x_axis)),
            d.dot(DVec3::from_array(plane.y_axis)),
        )
    };
    let mut sketch = Sketch::new("Filled");
    sketch.plane = plane;
    let mut ids: Vec<(Vec2D, uuid::Uuid)> = Vec::new();
    let mut point_at = |sketch: &mut Sketch, p: Vec2D| {
        if let Some((_, id)) = ids
            .iter()
            .find(|(q, _)| (*q - p).to_glam().length() < JOIN_MM)
        {
            return *id;
        }
        let id = sketch.add_geometry(GeometryElement::Point(Point::new(p)));
        ids.push((p, id));
        id
    };
    for edge in edges {
        let run = chain(&edge.outline)
            .into_iter()
            .map(flat)
            .collect::<Vec<_>>();
        if run.len() < 2 {
            continue;
        }
        match round(&run) {
            Some((c, r)) if run.len() >= 5 => {
                let closed = (run[0] - run[run.len() - 1]).to_glam().length() < JOIN_MM;
                let centre = sketch.add_geometry(GeometryElement::Point(Point::new(c)));
                if closed {
                    sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, r)));
                } else {
                    let (a, b) = (
                        point_at(&mut sketch, run[0]),
                        point_at(&mut sketch, run[run.len() - 1]),
                    );
                    let turn = (run[1] - c).to_glam().perp_dot((run[2] - c).to_glam())
                        + (run[0] - c).to_glam().perp_dot((run[1] - c).to_glam());
                    let (start, end) = if turn > 0.0 { (a, b) } else { (b, a) };
                    sketch.add_geometry(GeometryElement::Arc(Arc::new(centre, start, end, r)));
                }
            }
            _ => {
                for w in run.windows(2) {
                    if (w[1] - w[0]).to_glam().length() < JOIN_MM {
                        continue;
                    }
                    let (a, b) = (point_at(&mut sketch, w[0]), point_at(&mut sketch, w[1]));
                    sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
                }
            }
        }
    }
    match wb_sketch::profile::extract_wires(&sketch) {
        Ok(wires) if !wires.is_empty() => Ok(SketchFeature::new(sketch, plane)),
        _ => Err("the borrowed edges do not close into a loop".into()),
    }
}

/// Pairs of points (an edge's pieces) as one run from end to end.
fn chain(pairs: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut pieces: Vec<([f32; 3], [f32; 3])> = pairs
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (p[0], p[1]))
        .collect();
    let Some(first) = pieces.pop() else {
        return Vec::new();
    };
    let near = |a: [f32; 3], b: [f32; 3]| f64::from(length(sub(a, b))) < JOIN_MM;
    let mut run = vec![first.0, first.1];
    loop {
        let tail = *run.last().expect("a run");
        let head = run[0];
        if let Some(i) = pieces
            .iter()
            .position(|(a, b)| near(*a, tail) || near(*b, tail))
        {
            let (a, b) = pieces.remove(i);
            run.push(if near(a, tail) { b } else { a });
        } else if let Some(i) = pieces
            .iter()
            .position(|(a, b)| near(*a, head) || near(*b, head))
        {
            let (a, b) = pieces.remove(i);
            run.insert(0, if near(a, head) { b } else { a });
        } else {
            break;
        }
    }
    run
}

/// The circle every point of `run` keeps to, when there is one.
fn round(run: &[wb_sketch::sketch::Vec2D]) -> Option<(wb_sketch::sketch::Vec2D, f64)> {
    use wb_sketch::sketch::Vec2D;
    let (a, b, c) = (
        run[0].to_glam(),
        run[run.len() / 3].to_glam(),
        run[2 * run.len() / 3].to_glam(),
    );
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-9 {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    let centre = glam::DVec2::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    );
    let r = (a - centre).length();
    run.iter()
        .all(|p| ((p.to_glam() - centre).length() - r).abs() < 1e-3 * r.max(1.0))
        .then_some((Vec2D::from_glam(centre), r))
}

/// A borrowed face, ready for the kernel: a shape, what moves it into the
/// borrowing body's frame, and where the face was picked in that frame.
pub(crate) struct KernelFace {
    pub shape: Vec<u8>,
    pub transform: Option<[[f64; 4]; 4]>,
    pub point: [f64; 3],
}

/// The borrow `r` names, or why there is none.
fn referenced(document: &Document, r: &BorrowedRef) -> Result<Borrow, String> {
    borrow_of(document, r.borrow).ok_or_else(|| "the borrowed geometry it uses is gone".to_string())
}

/// The source body of a live borrow of faces and edges, checked: it lends
/// only when it is not itself built from the borrowing body.
fn live_solid_source(document: &Document, borrow: &Borrow) -> Result<BodyId, String> {
    let BorrowSource::Solid { body, .. } = &borrow.source else {
        return Err("this borrow lends a sketch, not faces or edges".into());
    };
    if let Some(why) = refusal(document, borrow.body, &borrow.source) {
        return Err(why);
    }
    if crate::build::bodies_reach(document, *body, borrow.body) {
        return Err(
            "the body it borrows from is built from this one in turn; one of the two has to go"
                .into(),
        );
    }
    Ok(*body)
}

/// The borrowed face `r` names, for the kernel to stop on.
pub(crate) fn kernel_face(document: &Document, r: &BorrowedRef) -> Result<KernelFace, String> {
    let borrow = referenced(document, r)?;
    if let Some(frozen) = &borrow.frozen {
        let face = frozen
            .faces
            .get(r.index)
            .ok_or("the borrow has no such face; pick it again")?;
        let offset = borrow.offset();
        return Ok(KernelFace {
            shape: face.shape.clone().into_bytes(),
            transform: Some(then_rows(&offset, face.transform)),
            point: offset.point(face.pick.point).map(f64::from),
        });
    }
    let source = live_solid_source(document, &borrow)?;
    let BorrowSource::Solid { faces, .. } = &borrow.source else {
        unreachable!("checked above");
    };
    let pick = faces
        .get(r.index)
        .ok_or("the borrow has no such face; pick it again")?;
    let shape = document
        .imported_brep_blob(source)
        .ok_or("the body it borrows from has no solid yet")?
        .to_vec();
    let placement = borrow
        .offset()
        .after(&relative(document, borrow.body, Some(source)));
    Ok(KernelFace {
        shape,
        transform: (!placement.is_identity()).then(|| placement.rows()),
        point: placement.point(pick.point).map(f64::from),
    })
}

/// The borrowed faces `refs` name, for the kernel to stop on: faces of
/// one shape, moved alike, go together.
pub(crate) fn kernel_faces(
    document: &Document,
    refs: &[BorrowedRef],
) -> Result<Vec<kernel_api::FacesOf>, String> {
    let mut out: Vec<kernel_api::FacesOf> = Vec::new();
    for r in refs {
        let face = kernel_face(document, r)?;
        let transform = face.transform.map(Box::new);
        match out
            .iter_mut()
            .find(|f| f.shape == face.shape && f.transform == transform)
        {
            Some(group) => group.points.push(face.point),
            None => out.push(kernel_api::FacesOf {
                shape: face.shape,
                transform,
                points: vec![face.point],
            }),
        }
    }
    Ok(out)
}

/// Borrowed edges as a pipe's path: the solid they are on, moved into the
/// borrowing body's frame, and where each is in that frame. They must all
/// come from one live borrow of a solid.
pub(crate) fn kernel_edges(
    document: &Document,
    refs: &[BorrowedRef],
) -> Result<kernel_api::PipePath, String> {
    let first = refs.first().ok_or("the path names no borrowed edges")?;
    if refs.iter().any(|r| r.borrow != first.borrow) {
        return Err("a path's borrowed edges must all come from one borrow".into());
    }
    let borrow = referenced(document, first)?;
    if borrow.frozen.is_some() {
        return Err("a frozen borrow keeps no solid for a path to run along; let it follow".into());
    }
    let source = live_solid_source(document, &borrow)?;
    let shape = document
        .imported_brep_blob(source)
        .ok_or("the body it borrows from has no solid yet")?
        .to_vec();
    let placement = borrow
        .offset()
        .after(&relative(document, borrow.body, Some(source)));
    let edges = refs
        .iter()
        .map(|r| {
            edge(document, r).map(|pick| kernel_api::EdgeProbe {
                point: pick.point.map(f64::from),
                direction: pick.direction.map(f64::from),
                faces: [0, 0],
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(kernel_api::PipePath::EdgesOf {
        shape,
        transform: (!placement.is_identity()).then(|| Box::new(placement.rows())),
        edges,
    })
}

/// The borrowed edge `r` names, as a point of it and its direction there,
/// in the borrowing body's frame.
pub(crate) fn edge(document: &Document, r: &BorrowedRef) -> Result<EdgePick, String> {
    let borrow = referenced(document, r)?;
    if let Some(frozen) = &borrow.frozen {
        let offset = borrow.offset();
        return frozen
            .edges
            .get(r.index)
            .map(|e| moved_edge(e.clone(), &offset).pick)
            .ok_or_else(|| "the borrow has no such edge; pick it again".to_string());
    }
    let source = live_solid_source(document, &borrow)?;
    let BorrowSource::Solid { edges, .. } = &borrow.source else {
        unreachable!("checked above");
    };
    let pick = edges
        .get(r.index)
        .ok_or("the borrow has no such edge; pick it again")?;
    let (mesh, _) = document
        .local_geometry(source)
        .ok_or("the body it borrows from has no solid yet")?;
    let found =
        mesh_edge(&mesh, pick).ok_or("no edge of the source solid lies where it was picked")?;
    let placement = borrow
        .offset()
        .after(&relative(document, borrow.body, Some(source)));
    Ok(EdgePick {
        faces: [0, 0],
        point: placement.point(found.pick.point),
        direction: placement.direction(found.pick.direction),
    })
}

/// A face of a borrow, as the borrowing body sees it.
pub(crate) struct SeenFace {
    pub pick: FacePick,
    pub surface: Option<FaceSurface>,
    /// Pairs of points.
    pub outline: Vec<[f32; 3]>,
}

/// A body's faces and edges as the borrow lends them, in the borrowing
/// body's frame, drawn from the source's mesh (live) or the snapshot.
pub(crate) fn seen(document: &Document, borrow: &Borrow) -> (Vec<SeenFace>, Vec<FrozenEdge>) {
    if let Some(frozen) = &borrow.frozen {
        let offset = borrow.offset();
        let faces = frozen
            .faces
            .iter()
            .map(|f| {
                let face = SeenFace {
                    pick: f.pick,
                    surface: f.surface,
                    outline: f.outline.clone(),
                };
                moved_face(face, &offset)
            })
            .collect();
        let edges = frozen
            .edges
            .iter()
            .map(|e| moved_edge(e.clone(), &offset))
            .collect();
        return (faces, edges);
    }
    let BorrowSource::Solid {
        body: source,
        faces,
        edges,
    } = &borrow.source
    else {
        return (Vec::new(), Vec::new());
    };
    let Some((mesh, _)) = document.local_geometry(*source) else {
        return (Vec::new(), Vec::new());
    };
    let placement = borrow
        .offset()
        .after(&relative(document, borrow.body, Some(*source)));
    let seen_faces = faces
        .iter()
        .filter_map(|pick| mesh_face(&mesh, pick))
        .map(|face| moved_face(face, &placement))
        .collect();
    let mut found: Vec<FrozenEdge> = edges
        .iter()
        .filter_map(|pick| mesh_edge(&mesh, pick))
        .collect();
    if borrow.options.whole {
        found.extend(all_edges(&mesh));
    }
    let seen_edges = found
        .into_iter()
        .map(|edge| moved_edge(edge, &placement))
        .collect();
    (seen_faces, seen_edges)
}

/// Every edge of a solid's mesh, each whole: the reference a whole borrowed
/// solid draws.
fn all_edges(mesh: &TriMesh) -> Vec<FrozenEdge> {
    let count = mesh.edges.len() / 2;
    let mut by_edge: HashMap<u32, Vec<[f32; 3]>> = HashMap::new();
    let mut order = Vec::new();
    for s in 0..count {
        let key = mesh.edge_ids.get(s).copied().unwrap_or(s as u32);
        let entry = by_edge.entry(key).or_insert_with(|| {
            order.push(key);
            Vec::new()
        });
        entry.push(mesh.positions[mesh.edges[s * 2] as usize]);
        entry.push(mesh.positions[mesh.edges[s * 2 + 1] as usize]);
    }
    order
        .into_iter()
        .filter_map(|key| {
            let outline = by_edge.remove(&key)?;
            let (a, b) = (outline[0], outline[1]);
            let d = sub(b, a);
            let len = length(d).max(f32::EPSILON);
            Some(FrozenEdge {
                pick: EdgePick {
                    faces: [0, 0],
                    point: a,
                    direction: d.map(|v| v / len),
                },
                outline,
            })
        })
        .collect()
}

fn moved_face(face: SeenFace, placement: &BodyPlacement) -> SeenFace {
    SeenFace {
        pick: FacePick {
            name: face.pick.name,
            point: placement.point(face.pick.point),
            normal: placement.direction(face.pick.normal),
        },
        surface: face
            .surface
            .map(|s| s.moved(|p| placement.point(p), |d| placement.direction(d))),
        outline: face.outline.iter().map(|p| placement.point(*p)).collect(),
    }
}

fn moved_edge(edge: FrozenEdge, placement: &BodyPlacement) -> FrozenEdge {
    FrozenEdge {
        pick: EdgePick {
            faces: [0, 0],
            point: placement.point(edge.pick.point),
            direction: placement.direction(edge.pick.direction),
        },
        outline: edge.outline.iter().map(|p| placement.point(*p)).collect(),
    }
}

/// The snapshot a borrow keeps once frozen: its source as it is now, in
/// `using`'s frame. Faces are taken out of the source's solid by the
/// kernel, so they need it.
pub fn freeze(
    document: &Document,
    kernel: Option<&dyn KernelQueries>,
    using: BodyId,
    source: &BorrowSource,
    options: &BorrowOptions,
) -> Result<FrozenBorrow, String> {
    if let Some(why) = refusal(document, using, source) {
        return Err(why);
    }
    match source {
        BorrowSource::Sketch(sketch) => {
            let placed = live_sketch(document, using, *sketch)?;
            Ok(FrozenBorrow {
                sketch: Some(placed.to_json()),
                ..FrozenBorrow::default()
            })
        }
        BorrowSource::Solid { body, faces, edges } => {
            let (mesh, _) = document
                .local_geometry(*body)
                .ok_or("the body it borrows from has no solid yet")?;
            let placement = relative(document, using, Some(*body));
            let mut frozen = FrozenBorrow::default();
            if !faces.is_empty() {
                let kernel = kernel.ok_or("freezing faces needs the geometry kernel")?;
                let brep = document
                    .imported_brep_blob(*body)
                    .ok_or("the body it borrows from has no solid yet")?;
                for pick in faces {
                    let shape = kernel
                        .face_of(brep, pick.point.map(f64::from))
                        .map_err(|e| format!("taking a face out of the source: {e}"))?;
                    let shape = String::from_utf8(shape)
                        .map_err(|_| "the kernel gave a face snapshot that is not text")?;
                    let seen = mesh_face(&mesh, pick)
                        .map(|face| moved_face(face, &placement))
                        .unwrap_or_else(|| SeenFace {
                            pick: FacePick {
                                name: 0,
                                point: placement.point(pick.point),
                                normal: placement.direction(pick.normal),
                            },
                            surface: None,
                            outline: Vec::new(),
                        });
                    frozen.faces.push(FrozenFace {
                        pick: FacePick {
                            name: 0,
                            point: placement.point(pick.point),
                            normal: placement.direction(pick.normal),
                        },
                        shape,
                        transform: placement.rows(),
                        surface: seen.surface,
                        outline: seen.outline,
                    });
                }
            }
            for pick in edges {
                let edge = mesh_edge(&mesh, pick)
                    .ok_or("no edge of the source solid lies where an edge was picked")?;
                frozen.edges.push(moved_edge(edge, &placement));
            }
            if options.whole {
                frozen.edges.extend(
                    all_edges(&mesh)
                        .into_iter()
                        .map(|e| moved_edge(e, &placement)),
                );
            }
            Ok(frozen)
        }
    }
}

/// What a live borrow in `using` follows, summed up: where its source sits
/// relative to it and what the source is now (the sketch's data, the
/// solid's geometry revision). A change means what stands on it rebuilds.
pub(crate) fn inputs(document: &Document, using: BodyId, source: &BorrowSource) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match source {
        BorrowSource::Sketch(sketch) => document
            .feature_values(*sketch)
            .map(core_document::data_revision)
            .hash(&mut hasher),
        BorrowSource::Solid { body, .. } => document
            .imported_geometry(*body)
            .map(|g| g.revision)
            .hash(&mut hasher),
    }
    let placement = relative(document, using, source_body(document, source));
    for row in placement.rows() {
        for value in row {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// The borrows of `body`, each with its data, in history order.
pub(crate) fn borrows_of_body(document: &Document, body: BodyId) -> Vec<(FeatureId, Borrow)> {
    crate::build::design_features_of_body(document, body)
        .into_iter()
        .filter_map(|(id, feature)| match feature {
            DesignFeature::Borrow {
                source,
                frozen,
                options,
            } => Some((
                id,
                Borrow {
                    body,
                    source,
                    frozen,
                    options,
                },
            )),
            _ => None,
        })
        .collect()
}

/// The faces `body` borrows, each named for a choice list.
#[cfg(feature = "egui")]
pub(crate) fn faces_of_body(document: &Document, body: BodyId) -> Vec<(BorrowedRef, String)> {
    let mut out = Vec::new();
    for (id, borrow) in borrows_of_body(document, body) {
        let name = feature_name(document, id);
        let count = match &borrow.frozen {
            Some(frozen) => frozen.faces.len(),
            None => match &borrow.source {
                BorrowSource::Solid { faces, .. } => faces.len(),
                BorrowSource::Sketch(_) => 0,
            },
        };
        for index in 0..count {
            out.push((
                BorrowedRef { borrow: id, index },
                format!("{name}: face {}", index + 1),
            ));
        }
    }
    out
}

/// The edges `body` borrows, each named for a choice list.
#[cfg(feature = "egui")]
pub(crate) fn edges_of_body(document: &Document, body: BodyId) -> Vec<(BorrowedRef, String)> {
    let mut out = Vec::new();
    for (id, borrow) in borrows_of_body(document, body) {
        let name = feature_name(document, id);
        let count = match &borrow.frozen {
            Some(frozen) => frozen.edges.len(),
            None => match &borrow.source {
                BorrowSource::Solid { edges, .. } => edges.len(),
                BorrowSource::Sketch(_) => 0,
            },
        };
        for index in 0..count {
            out.push((
                BorrowedRef { borrow: id, index },
                format!("{name}: edge {}", index + 1),
            ));
        }
    }
    out
}

fn feature_name(document: &Document, id: FeatureId) -> String {
    document
        .get_feature_meta(id)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "Borrowed".into())
}

/// The flat faces `body` borrows, each named for a menu: what a sketch of
/// the body can be mapped onto.
pub(crate) fn flat_faces_of_body(document: &Document, body: BodyId) -> Vec<(BorrowedRef, String)> {
    let mut out = Vec::new();
    for (id, borrow) in borrows_of_body(document, body) {
        let name = feature_name(document, id);
        for (index, face) in seen(document, &borrow).0.iter().enumerate() {
            if flat(face).is_some() {
                out.push((
                    BorrowedRef { borrow: id, index },
                    format!("{name}: face {}", index + 1),
                ));
            }
        }
    }
    out
}

/// Put the sketch `sketch` on the flat face `face` a borrow of its body
/// lends, keeping its geometry in its own coordinates: the sketch then
/// follows the face as the borrow finds it, and leaves any datum, mode or
/// face it stood on before.
pub(crate) fn map_sketch(
    document: &mut Document,
    sketch: FeatureId,
    face: BorrowedRef,
) -> Result<(), String> {
    use core_document::WorkbenchFeature;
    let node = document
        .get_feature_meta(sketch)
        .ok_or("the sketch is not in this document")?;
    if node.workbench_id.as_str() != "wb.sketch" {
        return Err("only a sketch is mapped onto a face".into());
    }
    let body = node.body.ok_or("the sketch belongs to no body")?;
    let borrow = borrow_of(document, face.borrow).ok_or("that feature is not a borrow")?;
    if borrow.body != body {
        return Err(
            "the borrow is another body's; a sketch takes faces its own body borrows".into(),
        );
    }
    let (point, normal) = lent_face(document, face.borrow, face.index)
        .ok_or("the borrow lends no flat face there")?;
    let data = document
        .get_feature_data(sketch)
        .ok_or("the sketch is not in this document")?;
    let mut feature = SketchFeature::from_json(data).map_err(|e| format!("invalid sketch: {e}"))?;
    let (point, normal) = (
        wb_sketch::boundary::widen3(point),
        wb_sketch::boundary::widen3(normal),
    );
    let plane = wb_sketch::sketch::SketchPlane::from_face(point, normal);
    feature.plane = plane;
    feature.sketch.plane = plane;
    feature.support = None;
    feature.attached = None;
    feature.face = Some(wb_sketch::FaceSupport {
        point,
        normal,
        name: 0,
        placed: plane,
        lent_by: Some(wb_sketch::LentFace {
            borrow: face.borrow,
            index: face.index,
        }),
    });
    document
        .update_feature_data(sketch, feature.to_json())
        .map_err(|e| e.to_string())?;
    document.set_feature_dependencies(sketch, feature.dependencies());
    document.mark_feature_dirty(sketch);
    Ok(())
}

/// The first flat face the borrow `id` lends, in world space, for a sketch
/// to be drawn on, and which of the borrow's faces it is.
pub(crate) fn flat_face_in_world(
    document: &Document,
    id: FeatureId,
) -> Option<(core_document::FaceRef, usize)> {
    let borrow = borrow_of(document, id)?;
    let placement = document.body_placement(borrow.body);
    let (index, point, normal) = (0..)
        .zip(seen(document, &borrow).0)
        .find_map(|(index, face)| flat(&face).map(|(p, n)| (index, p, n)))?;
    Some((
        core_document::FaceRef {
            name: 0,
            point: placement.point(point),
            normal: placement.direction(normal),
            surface: None,
        },
        index,
    ))
}

/// Where a lent face lies, when it is flat: a point on it and its normal.
fn flat(face: &SeenFace) -> Option<([f32; 3], [f32; 3])> {
    match face.surface {
        Some(FaceSurface::Plane { normal, .. }) => Some((face.pick.point, normal)),
        Some(_) => None,
        None => Some((face.pick.point, face.pick.normal)),
    }
}

/// Where the `index`th face the borrow `id` lends lies now, in the
/// borrowing body's frame, when it is flat: what a sketch placed on it
/// follows.
pub(crate) fn lent_face(
    document: &Document,
    id: FeatureId,
    index: usize,
) -> Option<([f32; 3], [f32; 3])> {
    let borrow = borrow_of(document, id)?;
    let (faces, _) = seen(document, &borrow);
    flat(faces.get(index)?)
}

/// What the borrow draws in its body's frame: the sketch's curves, or the
/// outlines of its faces and edges.
pub(crate) fn lines(document: &Document, borrow: &Borrow) -> TriMesh {
    if borrow.lends_sketch() && !borrow.options.fill {
        return match borrowed_sketch(document, borrow) {
            Ok(feature) => wb_sketch::render::sketch_to_lines(&feature.sketch, &feature.plane),
            Err(_) => TriMesh::default(),
        };
    }
    let (faces, edges) = seen(document, borrow);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for face in &faces {
        positions.extend_from_slice(&face.outline);
        normals.extend(std::iter::repeat_n(face.pick.normal, face.outline.len()));
    }
    for edge in &edges {
        positions.extend_from_slice(&edge.outline);
        normals.extend(std::iter::repeat_n([0.0, 0.0, 1.0], edge.outline.len()));
    }
    let edges = (0..positions.len() as u32 / 2 * 2).collect();
    TriMesh {
        positions,
        normals,
        edges,
        ..TriMesh::default()
    }
}

/// What the scene draws for the borrow changes when this does.
pub(crate) fn lines_revision(document: &Document, id: FeatureId, borrow: &Borrow) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    document
        .feature_values(id)
        .map(core_document::data_revision)
        .hash(&mut hasher);
    if borrow.frozen.is_none() {
        inputs(document, borrow.body, &borrow.source).hash(&mut hasher);
    }
    hasher.finish()
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// The point of the segment `a`–`b` nearest `p`.
fn nearest_on_segment(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    if len2 <= f32::EPSILON {
        return a;
    }
    let t = (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0);
    [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]
}

/// How far `p` is from the triangle `a`, `b`, `c`.
fn triangle_distance(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let n = cross(sub(b, a), sub(c, a));
    let n_len = length(n);
    if n_len > f32::EPSILON {
        let n = n.map(|v| v / n_len);
        let d = dot(sub(p, a), n);
        let q = [p[0] - n[0] * d, p[1] - n[1] * d, p[2] - n[2] * d];
        let inside = [(a, b), (b, c), (c, a)]
            .iter()
            .all(|(u, v)| dot(cross(sub(*v, *u), sub(q, *u)), n) >= -1e-6);
        if inside {
            return d.abs();
        }
    }
    [(a, b), (b, c), (c, a)]
        .iter()
        .map(|(u, v)| length(sub(p, nearest_on_segment(p, *u, *v))))
        .fold(f32::INFINITY, f32::min)
}

/// A point rounded to a thousandth of a millimetre, for matching.
type Key = [i64; 3];

/// How many triangles share a side, and its two ends.
type Side = (usize, [f32; 3], [f32; 3]);

/// How far a pick may lie from the geometry it names: a pick lands on
/// drawn triangles, a chord off a curved surface.
const PICK_REACH_MM: f32 = 0.5;

/// The face of `mesh` a pick names: its outline, its surface when the mesh
/// records it, and the pick's normal.
/// The name of the face of `body`'s solid at `point` (its own frame), so
/// a borrow of it finds it again wherever it goes; 0 when the body has no
/// solid yet or no face there.
pub(crate) fn face_name_at(
    document: &Document,
    body: BodyId,
    point: [f32; 3],
) -> kernel_api::TopoName {
    let Some((mesh, _)) = document.local_geometry(body) else {
        return 0;
    };
    let count = mesh.indices.len() / 3;
    let at = |t: usize, k: usize| mesh.positions[mesh.indices[t * 3 + k] as usize];
    (0..count)
        .map(|t| (t, triangle_distance(point, at(t, 0), at(t, 1), at(t, 2))))
        .filter(|(_, d)| *d <= PICK_REACH_MM)
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .and_then(|(t, _)| mesh.faces.get(t))
        .and_then(|face| mesh.face_names.get(*face as usize))
        .copied()
        .unwrap_or(0)
}

fn mesh_face(mesh: &TriMesh, pick: &FacePick) -> Option<SeenFace> {
    let triangle = |t: usize| {
        let at = |k: usize| mesh.positions[mesh.indices[t * 3 + k] as usize];
        (at(0), at(1), at(2))
    };
    let count = mesh.indices.len() / 3;
    // The face bearing the pick's name, wherever it went; else the face
    // under the point it was picked at.
    let named: Option<u32> = (pick.name != 0)
        .then(|| mesh.face_names.iter().position(|n| *n == pick.name))
        .flatten()
        .map(|face| face as u32);
    let candidates: Vec<usize> = match named {
        Some(face) => (0..count)
            .filter(|t| mesh.faces.get(*t) == Some(&face))
            .collect(),
        None => (0..count).collect(),
    };
    let (nearest, distance) = candidates
        .iter()
        .map(|&t| {
            let (a, b, c) = triangle(t);
            (t, triangle_distance(pick.point, a, b, c))
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))?;
    if named.is_none() && distance > PICK_REACH_MM {
        return None;
    }
    // Where the face is now: its point nearest the pick.
    let (a, b, c) = triangle(nearest);
    let point = nearest_on_triangle(pick.point, a, b, c);
    let members: Vec<usize> = match mesh.faces.get(nearest) {
        Some(face) => (0..count)
            .filter(|t| mesh.faces.get(*t) == Some(face))
            .collect(),
        None => vec![nearest],
    };
    let surface = mesh
        .faces
        .get(nearest)
        .and_then(|face| mesh.face_surfaces.get(*face as usize))
        .copied()
        .filter(|s| *s != FaceSurface::Other);
    // The outline: the sides of the face's triangles no other of them
    // shares, points matched to a thousandth of a millimetre.
    let key = |p: [f32; 3]| -> Key { p.map(|v| (v * 1000.0).round() as i64) };
    let mut sides: HashMap<(Key, Key), Side> = HashMap::new();
    for t in &members {
        let (a, b, c) = triangle(*t);
        for (u, v) in [(a, b), (b, c), (c, a)] {
            let (ku, kv) = (key(u), key(v));
            let k = if ku <= kv { (ku, kv) } else { (kv, ku) };
            sides.entry(k).or_insert((0, u, v)).0 += 1;
        }
    }
    let mut outline = Vec::new();
    for (count, u, v) in sides.into_values() {
        if count == 1 {
            outline.push(u);
            outline.push(v);
        }
    }
    let normal = match surface {
        Some(FaceSurface::Plane { normal, .. }) => normal,
        _ => pick.normal,
    };
    Some(SeenFace {
        pick: FacePick {
            point,
            normal,
            name: pick.name,
        },
        surface,
        outline,
    })
}

/// The point of triangle `abc` nearest `p`.
fn nearest_on_triangle(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let n = cross(sub(b, a), sub(c, a));
    let len = length(n);
    if len > 1e-12 {
        let n = n.map(|v| v / len);
        let d = dot(sub(p, a), n);
        let q = [p[0] - n[0] * d, p[1] - n[1] * d, p[2] - n[2] * d];
        // Inside the triangle when on the inner side of all three sides.
        let inside = [(a, b), (b, c), (c, a)]
            .iter()
            .all(|(u, v)| dot(cross(sub(*v, *u), sub(q, *u)), n) >= 0.0);
        if inside {
            return q;
        }
    }
    [(a, b), (b, c), (c, a)]
        .into_iter()
        .map(|(u, v)| nearest_on_segment(p, u, v))
        .min_by(|x, y| length(sub(*x, p)).total_cmp(&length(sub(*y, p))))
        .unwrap_or(a)
}

/// The edge of `mesh`'s outline a pick names: a point of it nearest the
/// pick and its direction there (turned the way the pick ran), with the
/// whole edge's outline.
fn mesh_edge(mesh: &TriMesh, pick: &EdgePick) -> Option<FrozenEdge> {
    let segment = |s: usize| {
        (
            mesh.positions[mesh.edges[s * 2] as usize],
            mesh.positions[mesh.edges[s * 2 + 1] as usize],
        )
    };
    let count = mesh.edges.len() / 2;
    let (nearest, distance) = (0..count)
        .map(|s| {
            let (a, b) = segment(s);
            (
                s,
                length(sub(pick.point, nearest_on_segment(pick.point, a, b))),
            )
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))?;
    if distance > PICK_REACH_MM {
        return None;
    }
    let (a, b) = segment(nearest);
    let mut direction = sub(b, a);
    let len = length(direction);
    if len <= f32::EPSILON {
        return None;
    }
    direction = direction.map(|v| v / len);
    if dot(direction, pick.direction) < 0.0 {
        direction = direction.map(|v| -v);
    }
    let members: Vec<usize> = match mesh.edge_ids.get(nearest) {
        Some(edge) => (0..count)
            .filter(|s| mesh.edge_ids.get(*s) == Some(edge))
            .collect(),
        None => vec![nearest],
    };
    let mut outline = Vec::with_capacity(members.len() * 2);
    for s in members {
        let (u, v) = segment(s);
        outline.push(u);
        outline.push(v);
    }
    Some(FrozenEdge {
        pick: EdgePick {
            faces: [0, 0],
            point: nearest_on_segment(pick.point, a, b),
            direction,
        },
        outline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube's top face as two triangles, a side as two more, with
    /// their kernel faces and the top's outline edges named.
    fn cube_top() -> TriMesh {
        TriMesh {
            positions: vec![
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 6],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 1, 4, 1, 0],
            edges: vec![0, 1, 1, 2, 2, 3, 3, 0],
            faces: vec![0, 0, 1, 1],
            edge_ids: vec![0, 1, 2, 3],
            face_surfaces: vec![
                FaceSurface::Plane {
                    origin: [0.0, 0.0, 1.0],
                    normal: [0.0, 0.0, 1.0],
                },
                FaceSurface::Plane {
                    origin: [0.0, 0.0, 0.0],
                    normal: [0.0, -1.0, 0.0],
                },
            ],
            ..TriMesh::default()
        }
    }

    #[test]
    fn a_whole_solid_lends_every_edge_once() {
        let edges = all_edges(&cube_top());
        assert_eq!(edges.len(), 4);
        assert!(edges.iter().all(|e| e.outline.len() == 2));
    }

    #[test]
    fn closed_edges_fill_to_a_face_with_its_round_parts_round() {
        // A 10 × 10 square at z = 3, and a circle of radius 2 inside it,
        // as a mesh gives them: the circle in 32 pieces.
        let side = |a: [f32; 3], b: [f32; 3]| FrozenEdge {
            pick: EdgePick {
                faces: [0, 0],
                point: a,
                direction: sub(b, a),
            },
            outline: vec![a, b],
        };
        let corners = [
            [0.0, 0.0, 3.0],
            [10.0, 0.0, 3.0],
            [10.0, 10.0, 3.0],
            [0.0, 10.0, 3.0],
        ];
        let mut edges: Vec<FrozenEdge> = (0..4)
            .map(|i| side(corners[i], corners[(i + 1) % 4]))
            .collect();
        let on = |k: usize| {
            let t = k as f32 / 32.0 * std::f32::consts::TAU;
            [5.0 + 2.0 * t.cos(), 5.0 + 2.0 * t.sin(), 3.0]
        };
        edges.push(FrozenEdge {
            pick: EdgePick {
                faces: [0, 0],
                point: on(0),
                direction: [0.0, 1.0, 0.0],
            },
            outline: (0..32).flat_map(|k| [on(k), on(k + 1)]).collect(),
        });
        let feature = filled(&edges).unwrap();
        let circles = feature
            .sketch
            .geometry
            .iter()
            .filter(|g| matches!(g, wb_sketch::sketch::GeometryElement::Circle(_)))
            .count();
        assert_eq!(circles, 1, "the round edge comes back a circle");
        let wires = wb_sketch::profile::extract_wires(&feature.sketch).unwrap();
        assert_eq!(wires.len(), 2, "the square with the circle a hole in it");
        assert!((feature.plane.origin[2] - 3.0).abs() < 1e-5);
        // Edges out of one plane, or open, fill nothing.
        let mut bent = edges[..4].to_vec();
        bent[2].outline[1][2] = 5.0;
        assert!(filled(&bent).is_err());
        assert!(filled(&edges[..3]).is_err());
    }

    #[test]
    fn a_picked_face_brings_its_whole_outline_and_its_plane() {
        let mesh = cube_top();
        let face = mesh_face(
            &mesh,
            &FacePick {
                name: 0,
                point: [0.6, 0.3, 1.0],
                normal: [0.0, 0.0, 1.0],
            },
        )
        .unwrap();
        // Four sides, the diagonal the two triangles share left out.
        assert_eq!(face.outline.len(), 8);
        assert!(matches!(face.surface, Some(FaceSurface::Plane { .. })));
        let far = FacePick {
            name: 0,
            point: [5.0, 5.0, 5.0],
            normal: [0.0, 0.0, 1.0],
        };
        assert!(mesh_face(&mesh, &far).is_none(), "too far to name a face");
    }

    #[test]
    fn a_picked_edge_runs_the_way_it_was_picked() {
        let mesh = cube_top();
        let edge = mesh_edge(
            &mesh,
            &EdgePick {
                faces: [0, 0],
                point: [0.5, 0.0, 1.0],
                direction: [-1.0, 0.0, 0.0],
            },
        )
        .unwrap();
        assert_eq!(edge.pick.direction, [-1.0, 0.0, 0.0]);
        assert_eq!(edge.pick.point, [0.5, 0.0, 1.0]);
        assert_eq!(edge.outline.len(), 2);
    }

    #[test]
    fn a_placed_sketch_moves_its_plane_and_drops_its_support() {
        let mut feature = SketchFeature::new(
            wb_sketch::sketch::Sketch::new("s"),
            wb_sketch::sketch::SketchPlane::xy(),
        );
        feature.support = Some(wb_sketch::DatumSupport {
            datum: FeatureId::new(),
            plane: None,
            offset: 0.0,
            shift: [0.0, 0.0],
            turn: 0.0,
        });
        // A quarter turn about x, then 10 mm along it.
        let half = std::f32::consts::FRAC_PI_4;
        let placement = BodyPlacement {
            translation: [10.0, 0.0, 0.0],
            rotation: [half.sin(), 0.0, 0.0, half.cos()],
        };
        let placed = placed_sketch(feature, &placement);
        assert!(placed.support.is_none());
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5);
        assert!(close(placed.plane.origin, [10.0, 0.0, 0.0]));
        // A quarter turn about x takes +z to -y.
        assert!(close(placed.plane.normal, [0.0, -1.0, 0.0]));
        assert_eq!(placed.plane, placed.sketch.plane);
    }

    use crate::DesignWorkbench;
    use crate::feature::{ExtrudeDirection, ExtrudeMode};
    use core_document::{CommandResult, Workbench, WorkbenchRuntimeContext};
    use serde_json::{Value, json};

    fn call(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        let mut bench = DesignWorkbench::default();
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        bench.run_command(id, args.as_object().unwrap(), &mut ctx)
    }

    fn id_of(value: &Value) -> FeatureId {
        FeatureId(uuid::Uuid::parse_str(value.as_str().unwrap()).unwrap())
    }

    /// Two bodies, the first with a sketch in it.
    fn two_bodies(doc: &mut Document) -> (BodyId, FeatureId, BodyId) {
        let a = doc.create_body(Some("A".into()));
        let sketch = doc
            .add_feature_in_body(
                SketchFeature::new(
                    wb_sketch::sketch::Sketch::new("s"),
                    wb_sketch::sketch::SketchPlane::xy(),
                ),
                "Sketch".into(),
                Some(a),
            )
            .unwrap();
        let b = doc.create_body(Some("B".into()));
        (a, sketch, b)
    }

    fn data(doc: &Document, id: FeatureId) -> DesignFeature {
        DesignFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap()
    }

    /// Live, a borrowed sketch depends on its source, so an edit of the
    /// sketch reaches what the borrowing body builds from it; frozen, it
    /// keeps a copy placed in its own body's frame and depends on nothing.
    #[test]
    fn freezing_a_borrowed_sketch_keeps_a_placed_copy_and_lets_go_of_the_source() {
        let mut doc = Document::new("t");
        let (_, sketch, b) = two_bodies(&mut doc);
        doc.set_body_placement(
            b,
            BodyPlacement {
                translation: [0.0, 0.0, 10.0],
                ..BodyPlacement::IDENTITY
            },
        );
        let borrowed = id_of(
            &call(
                &mut doc,
                "design.borrow",
                json!({"body": b.0.to_string(), "sketch": sketch.0.to_string()}),
            )
            .unwrap(),
        );
        assert_eq!(doc.feature_tree().dependencies(borrowed), vec![sketch]);
        assert!(lends_sketch(&doc, borrowed));
        let seen = super::sketch(&doc, borrowed).unwrap().unwrap();
        assert_eq!(seen.plane.origin, [0.0, 0.0, -10.0], "A's plane, from B");

        call(
            &mut doc,
            "design.freeze",
            json!({"feature": borrowed.0.to_string()}),
        )
        .unwrap();
        let DesignFeature::Borrow {
            frozen: Some(frozen),
            ..
        } = data(&doc, borrowed)
        else {
            panic!("frozen");
        };
        let kept = SketchFeature::from_json(frozen.sketch.as_ref().unwrap()).unwrap();
        assert_eq!(kept.plane.origin, [0.0, 0.0, -10.0]);
        assert!(doc.feature_tree().dependencies(borrowed).is_empty());
        doc.clear_feature_dirty(borrowed);
        doc.mark_feature_dirty(sketch);
        assert!(
            !doc.get_feature_meta(borrowed).unwrap().dirty,
            "an edit of the source does not reach a frozen borrow"
        );

        call(
            &mut doc,
            "design.freeze",
            json!({"feature": borrowed.0.to_string(), "frozen": false}),
        )
        .unwrap();
        assert!(matches!(
            data(&doc, borrowed),
            DesignFeature::Borrow { frozen: None, .. }
        ));
        assert_eq!(doc.feature_tree().dependencies(borrowed), vec![sketch]);
    }

    #[test]
    fn a_body_borrows_from_another_body_only() {
        let mut doc = Document::new("t");
        let (a, sketch, b) = two_bodies(&mut doc);
        let own = call(
            &mut doc,
            "design.borrow",
            json!({"body": a.0.to_string(), "sketch": sketch.0.to_string()}),
        );
        assert!(own.is_err(), "its own sketch needs no borrowing");
        let itself = call(
            &mut doc,
            "design.borrow",
            json!({"body": b.0.to_string(), "from": b.0.to_string(), "faces": []}),
        );
        assert!(itself.is_err());
        let both = call(
            &mut doc,
            "design.borrow",
            json!({"body": b.0.to_string(), "sketch": sketch.0.to_string(), "from": a.0.to_string()}),
        );
        assert!(both.is_err());
        // Faces are taken out of a solid by the kernel; A has none yet.
        let frozen_faces = call(
            &mut doc,
            "design.borrow",
            json!({"body": b.0.to_string(), "from": a.0.to_string(), "frozen": true,
                "faces": [{"point": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0]}]}),
        );
        assert!(frozen_faces.is_err());
        assert_eq!(
            doc.feature_tree().all_nodes().count(),
            1,
            "a refused call adds nothing"
        );
        let made = call(
            &mut doc,
            "design.borrow",
            json!({"body": b.0.to_string(), "from": a.0.to_string(),
                "faces": [{"point": [1.0, 2.0, 3.0], "normal": [0.0, 0.0, 1.0]}],
                "edges": [{"point": {"x": 1.0, "y": 2.0, "z": 0.0}, "direction": [1.0, 0.0, 0.0]}]}),
        )
        .unwrap();
        let DesignFeature::Borrow {
            source: BorrowSource::Solid { body, faces, edges },
            frozen: None,
            ..
        } = data(&doc, id_of(&made))
        else {
            panic!("a live borrow of faces and edges");
        };
        assert_eq!(body, a);
        assert_eq!(faces[0].point, [1.0, 2.0, 3.0]);
        assert_eq!(edges[0].point, [1.0, 2.0, 0.0]);
    }

    /// A feature that stops on a borrowed face or runs along a borrowed
    /// edge depends on the borrow, so a change of what it lends rebuilds it.
    #[test]
    fn what_takes_a_borrowed_face_or_edge_depends_on_the_borrow() {
        let face = BorrowedRef {
            borrow: FeatureId::new(),
            index: 0,
        };
        let edge = BorrowedRef {
            borrow: FeatureId::new(),
            index: 1,
        };
        let sketch = FeatureId::new();
        let pad = DesignFeature::from_json(&json!({"Pad": {
            "sketch": sketch.0.to_string(), "length": 10.0, "reversed": false,
            "mode2": {"UpToBorrowed": face},
            "direction": {"Borrowed": edge},
        }}))
        .unwrap();
        assert!(matches!(
            pad,
            DesignFeature::Pad {
                mode: ExtrudeMode::Dimension,
                direction: ExtrudeDirection::Borrowed(_),
                ..
            }
        ));
        assert_eq!(pad.dependencies(), vec![sketch, face.borrow, edge.borrow]);
    }

    /// The tool makes a borrow of the latest sketch of another body, and
    /// its task records as the call that makes the same borrow.
    #[cfg(feature = "egui")]
    #[test]
    fn the_tool_borrows_another_bodys_sketch_and_records_the_call() {
        let mut doc = Document::new("t");
        let (_, sketch, b) = two_bodies(&mut doc);
        let before = doc.clone();
        let mut bench = DesignWorkbench::default();
        let made = {
            let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            ctx.selected_body_id = Some(b.0);
            bench.on_input(
                &core_document::WorkbenchInputEvent::KeyPress {
                    key: core_document::KeyCode::A,
                },
                Some("design.borrow"),
                &mut ctx,
            );
            ctx.active_document_object.unwrap()
        };
        assert_eq!(doc.get_feature_meta(made).unwrap().body, Some(b));
        assert!(matches!(
            data(&doc, made),
            DesignFeature::Borrow { source: BorrowSource::Sketch(s), frozen: None, .. } if s == sketch
        ));
        let egui_ctx = egui::Context::default();
        ui_kit::apply_theme(&egui_ctx);
        let mut recorded = Vec::new();
        for accept in [false, true] {
            let mut output = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut ctx =
                    WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
                ctx.active_document_object = Some(made);
                bench.ui_task_panel(
                    ui,
                    &mut ctx,
                    core_document::TaskRequest {
                        accept,
                        cancel: false,
                    },
                );
                recorded = core_document::HookOutcome::take(&mut ctx).recorded;
            });
            output.textures_delta.clear();
        }
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].id, "design.borrow");
        let mut replay = before;
        let again = call(
            &mut replay,
            "design.borrow",
            Value::Object(recorded[0].args.clone()),
        )
        .unwrap();
        assert_eq!(
            replay.get_feature_data(id_of(&again)),
            doc.get_feature_data(made)
        );
    }
}
