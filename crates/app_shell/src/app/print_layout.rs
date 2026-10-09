//! The print layout: every part to print laid on the face it rests on
//! best, as many copies as the parts list prints, packed on the bed. The
//! assembly stays where it is: a layout is placements for the copies,
//! which export and the slicer write and the Print layout task draws.
//!
//! The resting face is the largest flat face the whole part stands on (no
//! point of the part below that face's plane), among faces of nearly the
//! same area the one leaving the part lowest; a part with no such face
//! keeps its own frame's up. Each part is then turned about the vertical
//! to the smallest rectangle around its footprint, its longer side along
//! X, and the copies go on in rows (shelves), tallest first, a gap
//! between them; what does not fit on the bed starts another plate,
//! drawn beside the first.

use std::collections::HashMap;
use std::sync::Arc;

use core_document::{BodyId, BodyPlacement, Document, DocumentService, PrintPart};
use glam::{DVec2, DVec3, Quat, Vec3};
use kernel_api::TriMesh;

/// The bed and the gap a layout keeps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayoutSettings {
    /// Width, depth and build height, mm.
    pub bed_mm: [f32; 3],
    /// The bed's origin is at its centre rather than a corner.
    pub origin_center: bool,
    /// Between copies, and half of it from the bed's edges, mm.
    pub gap_mm: f32,
}

impl LayoutSettings {
    pub(crate) fn of(printing: &settings::PrintingSettings) -> Self {
        Self {
            bed_mm: printing.bed_mm,
            origin_center: printing.origin_center,
            gap_mm: printing.layout_gap_mm,
        }
    }

    /// The bed's corner nearest the origin, mm.
    fn corner(&self) -> [f64; 2] {
        if self.origin_center {
            [
                -f64::from(self.bed_mm[0]) / 2.0,
                -f64::from(self.bed_mm[1]) / 2.0,
            ]
        } else {
            [0.0, 0.0]
        }
    }

    /// How far each plate after the first sits to the right of the one
    /// before, mm.
    pub(crate) fn plate_step(&self) -> f64 {
        let width = f64::from(self.bed_mm[0]);
        width + (width * 0.1).max(20.0)
    }
}

/// One copy on the bed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece {
    /// The body whose shape it is.
    pub body: BodyId,
    /// The part's name, numbered when there are several copies.
    pub name: String,
    /// The plate it is on, 0 the bed itself.
    pub plate: usize,
    /// From the body's own frame to the bed.
    pub placement: BodyPlacement,
}

/// A layout of the parts to print.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Layout {
    pub pieces: Vec<Piece>,
    /// Plates used, at least one when there are pieces.
    pub plates: usize,
    /// Parts wider or deeper than the bed, each on a plate of its own.
    pub too_big: Vec<String>,
    /// Parts taller than the build height.
    pub too_tall: Vec<String>,
    /// Parts with no geometry yet, left out.
    pub unbuilt: Vec<String>,
}

/// The parts to print and how many of each: the first bench's parts list
/// that answers, a part left out when none of its bodies is visible;
/// without one, every visible body that is made, once.
pub(crate) fn parts_to_print(document: &Document, registry: &DocumentService) -> Vec<PrintPart> {
    let visible = |b: &BodyId| document.imported_body_effective_visible(*b);
    match registry.print_parts(document) {
        Some(parts) => parts
            .into_iter()
            .filter(|p| p.count > 0 && p.bodies.iter().any(visible))
            .map(|mut p| {
                // The visible body's shape is printed, its copies alike.
                if let Some(at) = p.bodies.iter().position(visible) {
                    p.bodies.swap(0, at);
                }
                p
            })
            .collect(),
        None => {
            let not_made = registry.not_printed(document);
            document
                .bodies()
                .iter()
                .filter(|b| visible(&b.id) && !not_made.contains(&b.id))
                .map(|b| PrintPart {
                    name: b.name.clone(),
                    bodies: vec![b.id],
                    count: 1,
                })
                .collect()
        }
    }
}

/// What a layout is made from, to tell when it is out of date.
pub(crate) fn layout_key(
    document: &Document,
    parts: &[PrintPart],
    settings: &LayoutSettings,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.name.hash(&mut hasher);
        part.count.hash(&mut hasher);
        if let Some(body) = part.bodies.first() {
            body.hash(&mut hasher);
            document
                .imported_geometry(*body)
                .map(|g| g.revision)
                .hash(&mut hasher);
        }
    }
    settings.bed_mm.map(f32::to_bits).hash(&mut hasher);
    settings.origin_center.hash(&mut hasher);
    settings.gap_mm.to_bits().hash(&mut hasher);
    hasher.finish()
}

/// How one part lies on the bed: turned onto its resting face and about
/// the vertical, and the box of it so turned.
#[derive(Debug, Clone, Copy)]
struct Lying {
    turn: Quat,
    /// The box's lower and upper corners after the turn, mm.
    low: DVec3,
    high: DVec3,
}

impl Lying {
    fn size(&self) -> DVec3 {
        self.high - self.low
    }
}

/// Lay out `parts` of `document` on the bed `settings` describes.
pub(crate) fn lay_out(
    document: &Document,
    parts: &[PrintPart],
    settings: &LayoutSettings,
) -> Layout {
    let mut layout = Layout::default();
    // Each copy to place: the part's index, its copy number, its lying.
    let mut copies: Vec<(usize, u32, Lying)> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let Some(body) = part.bodies.first() else {
            continue;
        };
        let Some(lying) = document
            .local_geometry(*body)
            .filter(|(mesh, _)| !mesh.indices.is_empty())
            .map(|(mesh, _)| lying_of(&mesh))
        else {
            layout.unbuilt.push(part.name.clone());
            continue;
        };
        if lying.size().z > f64::from(settings.bed_mm[2]) {
            layout.too_tall.push(part.name.clone());
        }
        for copy in 0..part.count {
            copies.push((index, copy, lying));
        }
    }
    // Tallest rows first, the deepest footprints leading each.
    copies.sort_by(|a, b| {
        let (sa, sb) = (a.2.size(), b.2.size());
        sb.y.total_cmp(&sa.y)
            .then(sb.x.total_cmp(&sa.x))
            .then(a.0.cmp(&b.0))
            .then(a.1.cmp(&b.1))
    });
    let gap = f64::from(settings.gap_mm.max(0.0));
    let bed = [f64::from(settings.bed_mm[0]), f64::from(settings.bed_mm[1])];
    let mut shelves = Shelves::new(bed);
    let corner = settings.corner();
    for (index, copy, lying) in copies {
        let part = &parts[index];
        let size = lying.size();
        let (spot, quarter) = match shelves.place(size.x + gap, size.y + gap) {
            Some(spot) => spot,
            None => {
                if !layout.too_big.contains(&part.name) {
                    layout.too_big.push(part.name.clone());
                }
                (shelves.alone(), false)
            }
        };
        let (plate, at) = spot;
        // A quarter turn makes the footprint's depth its width.
        let (turn, low) = if quarter {
            let q = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
            let low = DVec3::new(-lying.high.y, lying.low.x, lying.low.z);
            (q * lying.turn, low)
        } else {
            (lying.turn, lying.low)
        };
        let target = DVec3::new(
            corner[0] + plate as f64 * settings.plate_step() + at[0] + gap / 2.0,
            corner[1] + at[1] + gap / 2.0,
            0.0,
        );
        let offset = (target - low).as_vec3();
        let name = if part.count > 1 {
            format!("{} {}", part.name, copy + 1)
        } else {
            part.name.clone()
        };
        layout.pieces.push(Piece {
            body: part.bodies[0],
            name,
            plate,
            placement: BodyPlacement::new(turn, offset),
        });
    }
    layout.plates = shelves
        .plates
        .len()
        .max(usize::from(!layout.pieces.is_empty()));
    layout
}

/// Rows of rectangles on plates of one size, each row as tall as the
/// first rectangle put in it.
struct Shelves {
    bed: [f64; 2],
    /// Each plate's rows: where the row starts, how tall it is, how much
    /// of its width is taken.
    plates: Vec<Vec<(f64, f64, f64)>>,
}

impl Shelves {
    fn new(bed: [f64; 2]) -> Self {
        Self {
            bed,
            plates: Vec::new(),
        }
    }

    /// Where a `width` × `depth` rectangle goes: its plate and corner, and
    /// whether it goes a quarter turned. `None` when it fits on no bed.
    fn place(&mut self, width: f64, depth: f64) -> Option<((usize, [f64; 2]), bool)> {
        let fits = |w: f64, d: f64| w <= self.bed[0] + 1e-9 && d <= self.bed[1] + 1e-9;
        let quarter = match (fits(width, depth), fits(depth, width)) {
            (true, _) => false,
            (false, true) => true,
            (false, false) => return None,
        };
        let (w, d) = if quarter {
            (depth, width)
        } else {
            (width, depth)
        };
        for (plate, rows) in self.plates.iter_mut().enumerate() {
            for row in rows.iter_mut() {
                if row.2 + w <= self.bed[0] + 1e-9 && d <= row.1 + 1e-9 {
                    let at = [row.2, row.0];
                    row.2 += w;
                    return Some(((plate, at), quarter));
                }
            }
            let top = rows.last().map_or(0.0, |r| r.0 + r.1);
            if top + d <= self.bed[1] + 1e-9 {
                rows.push((top, d, w));
                return Some(((plate, [0.0, top]), quarter));
            }
        }
        self.plates.push(vec![(0.0, d, w)]);
        Some(((self.plates.len() - 1, [0.0, 0.0]), quarter))
    }

    /// A plate of its own for a rectangle no bed holds.
    fn alone(&mut self) -> (usize, [f64; 2]) {
        self.plates.push(vec![(0.0, self.bed[1], self.bed[0])]);
        (self.plates.len() - 1, [0.0, 0.0])
    }
}

/// How a part with this mesh, in its own frame, lies on the bed.
fn lying_of(mesh: &TriMesh) -> Lying {
    let down = resting_normal(mesh).unwrap_or(DVec3::NEG_Z);
    let onto = Quat::from_rotation_arc(down.as_vec3(), Vec3::NEG_Z);
    let points: Vec<Vec3> = mesh
        .positions
        .iter()
        .map(|p| onto * Vec3::from_array(*p))
        .collect();
    let about = smallest_footprint_turn(&points);
    let turn = Quat::from_rotation_z(about) * onto;
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = DVec3::splat(f64::NEG_INFINITY);
    for p in &mesh.positions {
        let q = (turn * Vec3::from_array(*p)).as_dvec3();
        low = low.min(q);
        high = high.max(q);
    }
    Lying { turn, low, high }
}

/// The outward normal of the face the part rests on best: the largest
/// flat face whose plane has the whole part on its inner side, among
/// those within a hundredth of its area the one leaving the part lowest.
/// `None` when no flat face can carry it.
fn resting_normal(mesh: &TriMesh) -> Option<DVec3> {
    let points: Vec<DVec3> = mesh
        .positions
        .iter()
        .map(|p| DVec3::from_array(p.map(f64::from)))
        .collect();
    let (low, high) = points.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(l, h), p| (l.min(*p), h.max(*p)),
    );
    let reach = (high - low).length();
    if !reach.is_finite() || reach <= 0.0 {
        return None;
    }
    // Triangles grouped by the plane they lie in: its normal to a
    // thousandth and its distance from the origin to a hundredth of a mm.
    let mut planes: HashMap<[i64; 4], (f64, DVec3, f64)> = HashMap::new();
    for t in mesh.indices.as_chunks::<3>().0 {
        let [Some(a), Some(b), Some(c)] = t.map(|i| points.get(i as usize)) else {
            continue;
        };
        let cross = (*b - *a).cross(*c - *a);
        let twice = cross.length();
        if twice <= 1e-12 {
            continue;
        }
        let n = cross / twice;
        let d = n.dot(*a);
        let key = [
            (n.x * 1e3).round() as i64,
            (n.y * 1e3).round() as i64,
            (n.z * 1e3).round() as i64,
            (d * 1e2).round() as i64,
        ];
        let area = twice / 2.0;
        let entry = planes.entry(key).or_insert((0.0, DVec3::ZERO, 0.0));
        entry.0 += area;
        entry.1 += n * area;
        entry.2 += d * area;
    }
    let mut candidates: Vec<(f64, DVec3, f64)> = planes
        .into_values()
        .filter_map(|(area, n, d)| {
            let n = n.try_normalize()?;
            Some((area, n, d / area))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let tolerance = 1e-4 * reach + 1e-3;
    let carrying: Vec<(f64, DVec3, f64)> = candidates
        .into_iter()
        .take(64)
        .filter(|(_, n, d)| points.iter().all(|p| n.dot(*p) <= d + tolerance))
        .map(|(area, n, d)| {
            let height = d - points
                .iter()
                .map(|p| n.dot(*p))
                .fold(f64::INFINITY, f64::min);
            (area, n, height)
        })
        .collect();
    let largest = carrying.first()?.0;
    carrying
        .into_iter()
        .filter(|(area, ..)| *area >= largest * 0.99)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(_, n, _)| n)
}

/// The turn about the vertical, radians, that puts `points` in the
/// smallest rectangle along X and Y, its longer side along X.
fn smallest_footprint_turn(points: &[Vec3]) -> f32 {
    let hull = convex_hull(
        &points
            .iter()
            .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y)))
            .collect::<Vec<_>>(),
    );
    if hull.len() < 3 {
        return 0.0;
    }
    let box_at = |angle: f64| {
        let (s, c) = angle.sin_cos();
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for p in &hull {
            let q = DVec2::new(c * p.x - s * p.y, s * p.x + c * p.y);
            lo = lo.min(q);
            hi = hi.max(q);
        }
        hi - lo
    };
    // The smallest rectangle has a side along an edge of the hull.
    let mut best: (f64, f64) = (f64::INFINITY, 0.0);
    for (i, a) in hull.iter().enumerate() {
        let b = hull[(i + 1) % hull.len()];
        let edge = b - *a;
        if edge.length_squared() <= 1e-18 {
            continue;
        }
        let angle = -edge.y.atan2(edge.x);
        let size = box_at(angle);
        // Ties go to the turn nearest none, so a part already square to
        // the axes stays so.
        let area = size.x * size.y;
        let nearer = angle.abs() < best.1.abs();
        if area < best.0 * (1.0 - 1e-9) || (area <= best.0 * (1.0 + 1e-9) && nearer) {
            best = (area, angle);
        }
    }
    let mut angle = best.1;
    let size = box_at(angle);
    if size.y > size.x + 1e-9 {
        angle += std::f64::consts::FRAC_PI_2;
    }
    angle as f32
}

/// The convex hull of `points`, anticlockwise.
fn convex_hull(points: &[DVec2]) -> Vec<DVec2> {
    let mut sorted: Vec<DVec2> = points.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    sorted.dedup_by(|a, b| (*a - *b).length_squared() <= 1e-18);
    if sorted.len() < 3 {
        return sorted;
    }
    let turn = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<DVec2> = Vec::with_capacity(sorted.len() * 2);
    for pass in [false, true] {
        let start = hull.len();
        let mut walk: Box<dyn Iterator<Item = &DVec2>> = if pass {
            Box::new(sorted.iter().rev())
        } else {
            Box::new(sorted.iter())
        };
        for p in &mut walk {
            while hull.len() >= start + 2
                && turn(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0
            {
                hull.pop();
            }
            hull.push(*p);
        }
        hull.pop();
    }
    hull
}

/// A piece's mesh where the layout puts it, from its body's own mesh.
pub(crate) fn piece_mesh(document: &Document, piece: &Piece) -> Option<Arc<TriMesh>> {
    let (local, _) = document.local_geometry(piece.body)?;
    Some(Arc::new(piece.placement.mesh(&local)))
}

/// The layout the Print layout task shows: what it was made from, the
/// copies' meshes on the bed, and the plates' outlines.
#[derive(Debug, Clone)]
pub(crate) struct ShownLayout {
    pub key: u64,
    pub layout: Layout,
    /// Each copy's id in the scene and its mesh where it lies.
    pub preview: Vec<(uuid::Uuid, Arc<TriMesh>)>,
    /// Every plate's outline as lines, one mesh.
    pub beds: Arc<TriMesh>,
}

/// The build volume's twelve edges on each of `plates` plates.
fn plates_mesh(printing: &settings::PrintingSettings, plates: usize, step: f64) -> TriMesh {
    let one = crate::app::frame::print_bed_mesh(printing);
    let mut out = TriMesh::default();
    for plate in 0..plates.max(1) {
        let base = out.positions.len() as u32;
        let dx = (plate as f64 * step) as f32;
        out.positions
            .extend(one.positions.iter().map(|p| [p[0] + dx, p[1], p[2]]));
        out.normals.extend_from_slice(&one.normals);
        out.edges.extend(one.edges.iter().map(|i| i + base));
    }
    out
}

impl crate::PrintCadApp {
    /// Keep the shown layout up to the parts, their shapes and the bed
    /// while the Print layout task is open, and drop it once it closes. A
    /// layout shown for the first time frames the view on the plates.
    pub(crate) fn drive_print_layout(&mut self) {
        if !self.session.print_layout_shown {
            self.session.print_layout = None;
            return;
        }
        let document = &self.session.document;
        let parts = parts_to_print(document, &self.registry);
        let settings = LayoutSettings::of(&self.user_settings.printing);
        let key = layout_key(document, &parts, &settings);
        if self
            .session
            .print_layout
            .as_ref()
            .is_some_and(|shown| shown.key == key)
        {
            return;
        }
        let first = self.session.print_layout.is_none();
        let layout = lay_out(document, &parts, &settings);
        let preview = layout
            .pieces
            .iter()
            .filter_map(|piece| Some((uuid::Uuid::new_v4(), piece_mesh(document, piece)?)))
            .collect();
        let beds = Arc::new(plates_mesh(
            &self.user_settings.printing,
            layout.plates,
            settings.plate_step(),
        ));
        self.session.print_layout = Some(ShownLayout {
            key,
            layout,
            preview,
            beds,
        });
        if first {
            self.frame_print_layout();
        }
    }

    /// Frame the view on the shown layout's plates.
    fn frame_print_layout(&mut self) {
        let Some((lo, hi)) = self
            .session
            .print_layout
            .as_ref()
            .and_then(|shown| shown.beds.bounds())
        else {
            return;
        };
        let aabb = (Vec3::from_array(lo), Vec3::from_array(hi));
        let (center, radius) = crate::app::frame::aabb_fit_center_radius(aabb.0, aabb.1);
        self.session
            .camera
            .reset_to_fit(center, radius, Some(aabb), &self.user_settings.camera);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box `size` mm with a corner at `at`, its triangles outward.
    fn block(size: [f32; 3], at: [f32; 3]) -> TriMesh {
        let corner =
            |i: usize| [0, 1, 2].map(|k| at[k] + if (i >> k) & 1 == 1 { size[k] } else { 0.0 });
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        TriMesh {
            positions: (0..8).map(corner).collect(),
            indices: quads
                .iter()
                .flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]])
                .collect(),
            ..Default::default()
        }
    }

    /// Two meshes as one.
    fn joined(a: TriMesh, b: TriMesh) -> TriMesh {
        let base = a.positions.len() as u32;
        let mut out = a;
        out.positions.extend(b.positions);
        out.indices.extend(b.indices.into_iter().map(|i| i + base));
        out
    }

    fn document_with(meshes: &[(&str, TriMesh)]) -> (Document, Vec<BodyId>) {
        let mut document = Document::new("t");
        let mut bodies = Vec::new();
        for (name, mesh) in meshes {
            let body = document.create_body(Some((*name).into()));
            document.set_imported_geometry(
                body,
                core_document::ImportedGeometry {
                    bounds_mm: mesh.bounds(),
                    mesh: Arc::new(mesh.clone()),
                    source_asset: None,
                    revision: 1,
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
            bodies.push(body);
        }
        (document, bodies)
    }

    fn bounds(mesh: &TriMesh) -> ([f32; 3], [f32; 3]) {
        mesh.bounds().unwrap()
    }

    const BED: LayoutSettings = LayoutSettings {
        bed_mm: [220.0, 220.0, 250.0],
        origin_center: false,
        gap_mm: 5.0,
    };

    /// A tall post stands up as modelled no more: it lies on a long side,
    /// the largest face it rests on, its longer side along X.
    #[test]
    fn a_part_lies_on_its_largest_face() {
        let (document, bodies) =
            document_with(&[("Post", block([10.0, 20.0, 60.0], [5.0, -3.0, 7.0]))]);
        let parts = [PrintPart {
            name: "Post".into(),
            bodies: bodies.clone(),
            count: 1,
        }];
        let layout = lay_out(&document, &parts, &BED);
        assert_eq!(layout.pieces.len(), 1);
        let mesh = piece_mesh(&document, &layout.pieces[0]).unwrap();
        let (lo, hi) = bounds(&mesh);
        let size = [0, 1, 2].map(|k| hi[k] - lo[k]);
        assert!(
            (size[2] - 10.0).abs() < 1e-3,
            "on its 20 × 60 side: {size:?}"
        );
        assert!((size[0] - 60.0).abs() < 1e-3 && (size[1] - 20.0).abs() < 1e-3);
        assert!(lo[2].abs() < 1e-3, "on the bed");
        assert!(
            (lo[0] - 2.5).abs() < 1e-3 && (lo[1] - 2.5).abs() < 1e-3,
            "half the gap in"
        );
    }

    /// A flange on a boss: the boss's end is small, the flange's face is
    /// large but the boss stands out past it; the part rests on the
    /// flange's other face, the one with nothing beyond it.
    #[test]
    fn a_part_rests_only_on_a_face_nothing_stands_past() {
        let flange = block([40.0, 40.0, 4.0], [0.0, 0.0, 0.0]);
        let boss = block([10.0, 10.0, 20.0], [15.0, 15.0, 4.0]);
        let (document, bodies) = document_with(&[("Flange", joined(flange, boss))]);
        let parts = [PrintPart {
            name: "Flange".into(),
            bodies,
            count: 1,
        }];
        let layout = lay_out(&document, &parts, &BED);
        let mesh = piece_mesh(&document, &layout.pieces[0]).unwrap();
        let (lo, hi) = bounds(&mesh);
        assert!(
            (hi[2] - lo[2] - 24.0).abs() < 1e-3,
            "standing on the flange"
        );
        // The flange's full face lies on the bed.
        let on_bed = mesh
            .positions
            .iter()
            .filter(|p| (p[2] - lo[2]).abs() < 1e-3)
            .count();
        assert_eq!(on_bed, 4, "the flange's four corners");
        let (x0, x1) = mesh
            .positions
            .iter()
            .filter(|p| (p[2] - lo[2]).abs() < 1e-3)
            .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
        assert!((x1 - x0 - 40.0).abs() < 1e-3);
    }

    /// Copies go on in rows, a gap apart, none overlapping, all on the
    /// bed; what the bed cannot hold goes on a second plate.
    #[test]
    fn copies_pack_on_the_bed_without_touching() {
        let (document, bodies) = document_with(&[("Tile", block([50.0, 40.0, 3.0], [0.0; 3]))]);
        let parts = [PrintPart {
            name: "Tile".into(),
            bodies,
            count: 20,
        }];
        let layout = lay_out(&document, &parts, &BED);
        assert_eq!(layout.pieces.len(), 20);
        assert_eq!(layout.pieces[3].name, "Tile 4");
        let boxes: Vec<(usize, [f32; 3], [f32; 3])> = layout
            .pieces
            .iter()
            .map(|p| {
                let (lo, hi) = bounds(&piece_mesh(&document, p).unwrap());
                (p.plate, lo, hi)
            })
            .collect();
        // Four across (4 × 55 = 220) and five deep (5 × 45 = 225 > 220,
        // so four): sixteen on the bed, four on a second plate.
        assert_eq!(layout.plates, 2);
        assert_eq!(boxes.iter().filter(|b| b.0 == 0).count(), 16);
        let step = BED.plate_step() as f32;
        for (i, (plate, lo, hi)) in boxes.iter().enumerate() {
            let x0 = *plate as f32 * step;
            assert!(lo[0] >= x0 + 2.5 - 1e-3 && hi[0] <= x0 + 220.0 - 2.5 + 1e-3);
            assert!(lo[1] >= 2.5 - 1e-3 && hi[1] <= 220.0 - 2.5 + 1e-3);
            assert!(lo[2].abs() < 1e-3);
            for (other_plate, olo, ohi) in &boxes[i + 1..] {
                if other_plate != plate {
                    continue;
                }
                let apart =
                    (0..2).any(|k| lo[k] >= ohi[k] + 5.0 - 1e-3 || olo[k] >= hi[k] + 5.0 - 1e-3);
                assert!(apart, "{lo:?}..{hi:?} and {olo:?}..{ohi:?}");
            }
        }
    }

    /// A part wider than the bed one way fits turned; one too big either
    /// way gets a plate of its own and is named, as is one too tall.
    #[test]
    fn what_the_bed_cannot_hold_is_named() {
        let (document, bodies) = document_with(&[
            ("Rail", block([230.0, 30.0, 10.0], [0.0; 3])),
            ("Slab", block([300.0, 300.0, 5.0], [0.0; 3])),
        ]);
        let small = LayoutSettings {
            bed_mm: [100.0, 240.0, 4.0],
            ..BED
        };
        let parts = [
            PrintPart {
                name: "Rail".into(),
                bodies: vec![bodies[0]],
                count: 1,
            },
            PrintPart {
                name: "Slab".into(),
                bodies: vec![bodies[1]],
                count: 1,
            },
        ];
        let layout = lay_out(&document, &parts, &small);
        assert_eq!(layout.too_big, ["Slab"]);
        assert_eq!(layout.too_tall, ["Rail", "Slab"]);
        let rail = layout.pieces.iter().find(|p| p.name == "Rail").unwrap();
        let (lo, hi) = bounds(&piece_mesh(&document, rail).unwrap());
        assert!(
            (hi[1] - lo[1] - 230.0).abs() < 1e-3,
            "turned to lie along the bed's depth"
        );
        let slab = layout.pieces.iter().find(|p| p.name == "Slab").unwrap();
        assert_ne!(slab.plate, rail.plate);
    }

    /// A centred bed puts the layout about the origin.
    #[test]
    fn a_centred_bed_starts_at_its_corner() {
        let (document, bodies) = document_with(&[("Cube", block([10.0; 3], [100.0; 3]))]);
        let centred = LayoutSettings {
            origin_center: true,
            ..BED
        };
        let parts = [PrintPart {
            name: "Cube".into(),
            bodies,
            count: 1,
        }];
        let layout = lay_out(&document, &parts, &centred);
        let (lo, _) = bounds(&piece_mesh(&document, &layout.pieces[0]).unwrap());
        assert!(
            (lo[0] + 107.5).abs() < 1e-3 && (lo[1] + 107.5).abs() < 1e-3,
            "{lo:?}"
        );
    }

    #[test]
    fn a_footprint_turns_square_to_the_axes() {
        // A 40 × 10 rectangle at 30°.
        let (s, c) = 30f32.to_radians().sin_cos();
        let points: Vec<Vec3> = [[0.0, 0.0], [40.0, 0.0], [40.0, 10.0], [0.0, 10.0]]
            .iter()
            .map(|[x, y]| Vec3::new(c * x - s * y, s * x + c * y, 0.0))
            .collect();
        let turn = smallest_footprint_turn(&points);
        let q = Quat::from_rotation_z(turn);
        let turned: Vec<Vec3> = points.iter().map(|p| q * *p).collect();
        let (lo, hi) = turned.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(l, h), p| (l.min(*p), h.max(*p)),
        );
        assert!((hi.x - lo.x - 40.0).abs() < 1e-3 && (hi.y - lo.y - 10.0).abs() < 1e-3);
    }
}
