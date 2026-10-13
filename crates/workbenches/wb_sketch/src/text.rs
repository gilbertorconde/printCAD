//! Text in a sketch: a string in a font as closed outlines of lines and
//! splines, which pad and pocket as any profile does. A text block keeps
//! what it was made from (the string, the font, its size, the spacing
//! between letters and its turn) and the point it stands on: the start of
//! its first line on the baseline, an ordinary point constraints can hold.
//! The outlines are the block's own: the solver leaves them be, they
//! follow the point wherever it goes, and a change to the text makes them
//! again.

use std::borrow::Cow;

use glam::DVec2;
use uuid::Uuid;

use crate::sketch::{BSpline, GeometryElement, Line, Point, Sketch, TextBlock, TextShape, Vec2D};

/// The fonts that come with the application, by name. Any other font is
/// one installed on the machine, by its family name, or a file's path.
pub const FONTS: &[(&str, &[u8])] = &[
    (
        "IBM Plex Sans",
        include_bytes!("../../../ui_kit/fonts/IBMPlexSans-Regular.ttf"),
    ),
    (
        "IBM Plex Sans SemiBold",
        include_bytes!("../../../ui_kit/fonts/IBMPlexSans-SemiBold.ttf"),
    ),
    (
        "IBM Plex Mono",
        include_bytes!("../../../ui_kit/fonts/IBMPlexMono-Regular.ttf"),
    ),
];

/// The font new text takes.
pub const DEFAULT_FONT: &str = "IBM Plex Sans";

/// What a text block is made of, apart from where it stands.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpec {
    pub text: String,
    pub font: String,
    /// The document asset holding a font file the application does not
    /// carry.
    pub font_asset: Option<Uuid>,
    /// That asset's bytes, read for the outlines.
    pub font_data: Option<std::sync::Arc<Vec<u8>>>,
    /// Which font of that file is meant, for a file holding several.
    pub font_index: u32,
    /// The font's em in millimetres.
    pub size: f64,
    /// Millimetres added between letters.
    pub spacing: f64,
    /// Degrees the text turns counter-clockwise about its point.
    pub angle: f64,
}

impl Default for TextSpec {
    fn default() -> Self {
        Self {
            text: "Text".to_string(),
            font: DEFAULT_FONT.to_string(),
            font_asset: None,
            font_data: None,
            font_index: 0,
            size: 10.0,
            spacing: 0.0,
            angle: 0.0,
        }
    }
}

impl TextSpec {
    pub fn of(block: &TextBlock) -> Self {
        Self {
            text: block.text.clone(),
            font: block.font.clone(),
            font_asset: block.font_asset,
            font_data: None,
            font_index: 0,
            size: block.size,
            spacing: block.spacing,
            angle: block.angle,
        }
    }
}

fn font_bytes(spec: &TextSpec) -> Result<(Cow<'_, [u8]>, u32), String> {
    let font = spec.font.as_str();
    if let Some((_, bytes)) = FONTS.iter().find(|(name, _)| *name == font) {
        return Ok((Cow::Borrowed(bytes), 0));
    }
    if let Some(data) = &spec.font_data {
        return Ok((Cow::Borrowed(data.as_slice()), spec.font_index));
    }
    std::fs::read(font)
        .map(|bytes| (Cow::Owned(bytes), 0))
        .map_err(|e| format!("the font {font} is not in the document and cannot be read: {e}"))
}

/// A font found installed on the machine.
#[derive(Debug, Clone)]
pub struct InstalledFont {
    /// Its family name as the font gives it.
    pub family: String,
    /// Its file's bytes.
    pub bytes: Vec<u8>,
    /// Which font of the file it is, for a file holding several.
    pub index: u32,
    /// The file's name, which the kept asset records.
    pub file: Option<String>,
}

/// The installed fonts, looked through once, the first time one is asked
/// for.
#[cfg(not(target_arch = "wasm32"))]
fn system_fonts() -> &'static fontdb::Database {
    static FONTS: std::sync::OnceLock<fontdb::Database> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// The regular font of the installed family `family`, its letters' case
/// aside.
#[cfg(not(target_arch = "wasm32"))]
pub fn installed_font(family: &str) -> Option<InstalledFont> {
    let db = system_fonts();
    let wanted = family.trim();
    let name = db.faces().find_map(|face| {
        face.families
            .iter()
            .map(|(name, _)| name)
            .find(|name| name.eq_ignore_ascii_case(wanted))
            .cloned()
    })?;
    let id = db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(&name)],
        ..Default::default()
    })?;
    let file = match &db.face(id)?.source {
        fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
            path.file_name().map(|n| n.to_string_lossy().to_string())
        }
        fontdb::Source::Binary(_) => None,
    };
    db.with_face_data(id, |bytes, index| InstalledFont {
        family: name.clone(),
        bytes: bytes.to_vec(),
        index,
        file,
    })
}

/// A browser page has no installed fonts to look through.
#[cfg(target_arch = "wasm32")]
pub fn installed_font(_family: &str) -> Option<InstalledFont> {
    None
}

/// The family names of the installed fonts, sorted.
#[cfg(not(target_arch = "wasm32"))]
pub fn installed_families() -> &'static [String] {
    static FAMILIES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FAMILIES.get_or_init(|| {
        let mut names: Vec<String> = system_fonts()
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    })
}

/// None on a browser page.
#[cfg(target_arch = "wasm32")]
pub fn installed_families() -> &'static [String] {
    &[]
}

/// Whether `font` is one the application carries.
pub fn is_bundled(font: &str) -> bool {
    FONTS.iter().any(|(name, _)| *name == font)
}

/// Whether `font` names a font file rather than an installed family.
pub fn is_file(font: &str) -> bool {
    let lower = font.to_ascii_lowercase();
    font.contains(['/', '\\'])
        || [".ttf", ".otf", ".ttc", ".otc"]
            .iter()
            .any(|e| lower.ends_with(e))
}

/// The font file `document` keeps as asset `id`: its bytes and which font
/// of the file is meant.
fn kept_font(document: &core_document::Document, id: Uuid) -> Option<(Vec<u8>, u32)> {
    let bytes = document.asset_bytes(id)?;
    let index = document
        .get_asset(id)
        .and_then(|a| a.metadata.get("index"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|i| u32::try_from(i).ok())
        .unwrap_or(0);
    Some((bytes.to_vec(), index))
}

/// Ready `spec`'s font from what `document` keeps, changing nothing: a
/// font the application carries needs nothing, one kept in the document
/// is read from its asset. Whether the font is ready.
pub fn read_kept_font(document: &core_document::Document, spec: &mut TextSpec) -> bool {
    if is_bundled(&spec.font) {
        return true;
    }
    match spec.font_asset.and_then(|id| kept_font(document, id)) {
        Some((bytes, index)) => {
            spec.font_data = Some(std::sync::Arc::new(bytes));
            spec.font_index = index;
            true
        }
        None => false,
    }
}

/// Ready `spec`'s font from `document`: a font the application carries
/// needs nothing; one kept in the document is read from its asset; a font
/// file named by its path, or a font installed on the machine named by its
/// family, is read and kept in the document (once for the same font), so
/// the text can be made again on any machine.
pub fn load_font(
    document: &mut core_document::Document,
    spec: &mut TextSpec,
) -> Result<(), String> {
    load_font_from(document, spec, &installed_font)
}

/// [`load_font`], with `installed` finding a font installed by family.
pub fn load_font_from(
    document: &mut core_document::Document,
    spec: &mut TextSpec,
    installed: &dyn Fn(&str) -> Option<InstalledFont>,
) -> Result<(), String> {
    if is_bundled(&spec.font) {
        spec.font_asset = None;
        spec.font_data = None;
        spec.font_index = 0;
        return Ok(());
    }
    if read_kept_font(document, spec) {
        return Ok(());
    }
    let path = std::path::Path::new(&spec.font);
    let found = if is_file(&spec.font) || path.is_file() {
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read the font {}: {e}", spec.font))?;
        InstalledFont {
            family: String::new(),
            bytes,
            index: 0,
            file: path.file_name().map(|n| n.to_string_lossy().to_string()),
        }
    } else {
        installed(&spec.font).ok_or_else(|| {
            format!(
                "no font named {} is installed here or kept in the document",
                spec.font
            )
        })?
    };
    ttf_parser::Face::parse(&found.bytes, found.index)
        .map_err(|e| format!("{} is not a font this can read: {e}", spec.font))?;
    let kept = document
        .assets()
        .filter(|asset| asset.metadata.get("kind").and_then(|k| k.as_str()) == Some("font"))
        .map(|asset| asset.id)
        .find(|id| {
            kept_font(document, *id)
                .is_some_and(|(bytes, index)| index == found.index && bytes == found.bytes)
        });
    let id = kept.unwrap_or_else(|| {
        let extension = found
            .file
            .as_deref()
            .and_then(|f| std::path::Path::new(f).extension())
            .and_then(|e| e.to_str())
            .unwrap_or("ttf")
            .to_ascii_lowercase();
        let mut about = serde_json::json!({"kind": "font", "file": found.file});
        if !found.family.is_empty() {
            about["family"] = serde_json::json!(found.family);
        }
        if found.index != 0 {
            about["index"] = serde_json::json!(found.index);
        }
        document.add_asset_with_data(
            core_document::AssetReference::new(
                format!("assets/font-{}.{extension}", Uuid::new_v4()),
                core_document::AssetType::Other,
                about,
            ),
            found.bytes.clone(),
        )
    });
    if !found.family.is_empty() {
        spec.font = found.family;
    }
    spec.font_asset = Some(id);
    spec.font_data = Some(std::sync::Arc::new(found.bytes));
    spec.font_index = found.index;
    Ok(())
}

/// One piece of a glyph's outline.
#[derive(Debug, Clone, Copy)]
enum Seg {
    Line(DVec2, DVec2),
    Quad(DVec2, DVec2, DVec2),
    Cubic(DVec2, DVec2, DVec2, DVec2),
}

impl Seg {
    fn start(&self) -> DVec2 {
        match *self {
            Seg::Line(a, _) | Seg::Quad(a, _, _) | Seg::Cubic(a, _, _, _) => a,
        }
    }

    fn map(self, f: impl Fn(DVec2) -> DVec2) -> Self {
        match self {
            Seg::Line(a, b) => Seg::Line(f(a), f(b)),
            Seg::Quad(a, b, c) => Seg::Quad(f(a), f(b), f(c)),
            Seg::Cubic(a, b, c, d) => Seg::Cubic(f(a), f(b), f(c), f(d)),
        }
    }
}

/// Gathers a glyph's contours, placed where the pen stands.
struct Contours {
    at: DVec2,
    scale: f64,
    start: DVec2,
    pen: DVec2,
    current: Vec<Seg>,
    done: Vec<Vec<Seg>>,
}

impl Contours {
    fn place(&self, x: f64, y: f64) -> DVec2 {
        self.at + DVec2::new(x, y) * self.scale
    }

    fn push(&mut self, seg: Seg, to: DVec2) {
        // A piece that goes nowhere adds nothing to the outline.
        if (to - self.pen).length() > 1e-9 {
            self.current.push(seg);
        }
        self.pen = to;
    }
}

impl ttf_parser::OutlineBuilder for Contours {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = self.place(x.into(), y.into());
        self.pen = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.place(x.into(), y.into());
        self.push(Seg::Line(self.pen, to), to);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let to = self.place(x.into(), y.into());
        self.push(
            Seg::Quad(self.pen, self.place(x1.into(), y1.into()), to),
            to,
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let to = self.place(x.into(), y.into());
        let seg = Seg::Cubic(
            self.pen,
            self.place(x1.into(), y1.into()),
            self.place(x2.into(), y2.into()),
            to,
        );
        self.push(seg, to);
    }

    fn close(&mut self) {
        if self.current.is_empty() {
            return;
        }
        if (self.pen - self.start).length() > 1e-9 {
            self.current.push(Seg::Line(self.pen, self.start));
        }
        self.pen = self.start;
        self.done.push(std::mem::take(&mut self.current));
    }
}

/// The closed contours `spec` lays out with its point at `at`.
fn contours(spec: &TextSpec, at: DVec2) -> Result<Vec<Vec<Seg>>, String> {
    let (bytes, index) = font_bytes(spec)?;
    let face = ttf_parser::Face::parse(&bytes, index)
        .map_err(|e| format!("{} is not a font this can read: {e}", spec.font))?;
    if spec.size.is_nan() || spec.size <= 0.0 {
        return Err("the text's size must be more than 0".to_string());
    }
    let scale = spec.size / f64::from(face.units_per_em());
    let line = f64::from(face.ascender() - face.descender() + face.line_gap()) * scale;
    let mut out = Contours {
        at: DVec2::ZERO,
        scale,
        start: DVec2::ZERO,
        pen: DVec2::ZERO,
        current: Vec::new(),
        done: Vec::new(),
    };
    let mut pen = DVec2::ZERO;
    for ch in spec.text.chars() {
        if ch == '\n' {
            pen = DVec2::new(0.0, pen.y - line);
            continue;
        }
        let Some(glyph) = face.glyph_index(ch) else {
            continue;
        };
        out.at = pen;
        face.outline_glyph(glyph, &mut out);
        ttf_parser::OutlineBuilder::close(&mut out);
        let advance = f64::from(face.glyph_hor_advance(glyph).unwrap_or(0)) * scale;
        pen.x += advance + spec.spacing;
    }
    let turn = DVec2::from_angle(spec.angle.to_radians());
    Ok(out
        .done
        .into_iter()
        .map(|contour| {
            contour
                .into_iter()
                .map(|seg| seg.map(|p| at + turn.rotate(p)))
                .collect()
        })
        .collect())
}

fn new_point(sketch: &mut Sketch, made: &mut Vec<Uuid>, p: DVec2) -> Uuid {
    let id = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(p.x, p.y))));
    made.push(id);
    id
}

/// Put `contours` into the sketch: a line for each straight piece, one
/// spline for each run of curved pieces of one degree (joined exactly, a
/// knot standing as often as the degree at each join), consecutive pieces
/// sharing their end points. Returns every element made.
fn build(sketch: &mut Sketch, contours: Vec<Vec<Seg>>) -> Vec<Uuid> {
    let mut made = Vec::new();
    for contour in contours {
        let n = contour.len();
        if n == 0 {
            continue;
        }
        let first = new_point(sketch, &mut made, contour[0].start());
        let mut i = 0;
        let mut from = first;
        while i < n {
            // The run of pieces of one kind from here.
            let kind = std::mem::discriminant(&contour[i]);
            let mut j = i + 1;
            while j < n
                && std::mem::discriminant(&contour[j]) == kind
                && !matches!(contour[i], Seg::Line(..))
            {
                j += 1;
            }
            let to = if j == n {
                first
            } else {
                new_point(sketch, &mut made, contour[j].start())
            };
            let element = match contour[i] {
                Seg::Line(..) => GeometryElement::Line(Line::new(from, to)),
                _ => {
                    let degree = if matches!(contour[i], Seg::Quad(..)) {
                        2
                    } else {
                        3
                    };
                    let mut control = vec![from];
                    for seg in &contour[i..j] {
                        let inner: Vec<DVec2> = match *seg {
                            Seg::Quad(_, b, _) => vec![b],
                            Seg::Cubic(_, b, c, _) => vec![b, c],
                            Seg::Line(..) => Vec::new(),
                        };
                        for p in inner {
                            control.push(new_point(sketch, &mut made, p));
                        }
                        control.push(Uuid::nil());
                    }
                    // The joins between pieces, then the run's far end.
                    let joins: Vec<DVec2> = contour[i + 1..j].iter().map(Seg::start).collect();
                    for (k, slot) in control.iter_mut().filter(|c| c.is_nil()).enumerate() {
                        *slot = match joins.get(k) {
                            Some(p) => {
                                let id = sketch.add_geometry(GeometryElement::Point(Point::new(
                                    Vec2D::new(p.x, p.y),
                                )));
                                made.push(id);
                                id
                            }
                            None => to,
                        };
                    }
                    let pieces = j - i;
                    let mut knots = vec![0.0; degree + 1];
                    for p in 1..pieces {
                        let k = p as f64 / pieces as f64;
                        knots.extend(std::iter::repeat_n(k, degree));
                    }
                    knots.extend(std::iter::repeat_n(1.0, degree + 1));
                    GeometryElement::BSpline(BSpline {
                        degree: degree as u32,
                        knots,
                        ..BSpline::new(control, false)
                    })
                }
            };
            made.push(sketch.add_geometry(element));
            from = to;
            i = j;
        }
    }
    made
}

/// A new text block of `spec` standing on a new point at `at`. Returns the
/// block's id.
pub fn add(sketch: &mut Sketch, at: Vec2D, spec: &TextSpec) -> Result<Uuid, String> {
    let where_ = DVec2::new(at.x, at.y);
    let outlines = contours(spec, where_)?;
    if outlines.is_empty() {
        return Err("the text has nothing to draw".to_string());
    }
    let anchor = sketch.add_geometry(GeometryElement::Point(Point::new(at)));
    let elements = build(sketch, outlines);
    let id = Uuid::new_v4();
    sketch.texts.push(TextBlock {
        id,
        text: spec.text.clone(),
        font: spec.font.clone(),
        font_asset: spec.font_asset,
        size: spec.size,
        spacing: spec.spacing,
        angle: spec.angle,
        anchor,
        elements,
        placed: at,
        outlined: Some(TextShape {
            size: spec.size,
            spacing: spec.spacing,
            angle: spec.angle,
        }),
    });
    Ok(id)
}

/// Make block `id` again as `spec` says, where its point stands now. When
/// the new outlines are made of the same pieces as the old (the size,
/// spacing or angle changed, not the letters), the old elements take the
/// new shape and keep their ids, and with them whatever refers to them.
pub fn change(sketch: &mut Sketch, id: Uuid, spec: &TextSpec) -> Result<(), String> {
    let Some(index) = sketch.texts.iter().position(|b| b.id == id) else {
        return Err("no such text".to_string());
    };
    let anchor = sketch.texts[index].anchor;
    let at = sketch
        .point_position(anchor)
        .ok_or("the text's point is gone")?;
    let outlines = contours(spec, DVec2::new(at.x, at.y))?;
    let mut scratch = Sketch::new("text");
    let made = build(&mut scratch, outlines.clone());
    let old = sketch.texts[index].elements.clone();
    let alike = made.len() == old.len()
        && made.iter().zip(&old).all(|(new, old)| {
            match (scratch.get_geometry(*new), sketch.get_geometry(*old)) {
                (Some(a), Some(b)) => std::mem::discriminant(a) == std::mem::discriminant(b),
                _ => false,
            }
        });
    let elements = if alike {
        for (new, old) in made.iter().zip(&old) {
            if let (Some(GeometryElement::Point(p)), Some(GeometryElement::Point(q))) =
                (scratch.get_geometry(*new), sketch.get_geometry_mut(*old))
            {
                q.position = p.position;
            }
        }
        old
    } else {
        remove_elements(sketch, &old);
        build(sketch, outlines)
    };
    let block = &mut sketch.texts[index];
    block.text = spec.text.clone();
    block.font = spec.font.clone();
    block.font_asset = spec.font_asset;
    block.size = spec.size;
    block.spacing = spec.spacing;
    block.angle = spec.angle;
    block.elements = elements;
    block.placed = at;
    block.outlined = Some(block.shape());
    Ok(())
}

/// Make again the outlines of every block whose size, spacing or angle
/// formulas have set to other than the outlines were made at, its font
/// read from what `document` keeps. Whether any changed. A block whose
/// font is neither carried nor kept, or whose values make no text, keeps
/// the outlines it has.
pub fn follow_values(sketch: &mut Sketch, document: &core_document::Document) -> bool {
    let stale: Vec<Uuid> = sketch
        .texts
        .iter()
        .filter(|b| b.outlines_stale())
        .map(|b| b.id)
        .collect();
    let mut moved = false;
    for id in stale {
        let Some(block) = sketch.texts.iter().find(|b| b.id == id) else {
            continue;
        };
        let mut spec = TextSpec::of(block);
        if read_kept_font(document, &mut spec) || is_file(&spec.font) {
            moved |= change(sketch, id, &spec).is_ok();
        }
    }
    moved
}

/// Take elements away without the text-block bookkeeping that removing
/// through the sketch does.
fn remove_elements(sketch: &mut Sketch, ids: &[Uuid]) {
    let doomed: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
    sketch.geometry.retain(|g| !doomed.contains(&g.id()));
    sketch.constraints.retain(|c| {
        !crate::sketch::constraint_refs(&c.kind)
            .iter()
            .any(|id| doomed.contains(id))
    });
    sketch.construction.retain(|id| !doomed.contains(id));
}

/// Carry every block's outlines to where its point now stands.
pub fn follow(sketch: &mut Sketch) {
    for i in 0..sketch.texts.len() {
        let block = &sketch.texts[i];
        let Some(at) = sketch.point_position(block.anchor) else {
            continue;
        };
        let delta = at - block.placed;
        if delta.to_glam().length() < 1e-7 {
            continue;
        }
        let points: Vec<Uuid> = block.elements.clone();
        for id in points {
            if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(id) {
                p.position = p.position + delta;
            }
        }
        sketch.texts[i].placed = at;
    }
}

/// The points of every block's outlines: what the solver leaves be.
pub fn outline_points(sketch: &Sketch) -> Vec<Uuid> {
    sketch
        .texts
        .iter()
        .flat_map(|b| b.elements.iter().copied())
        .filter(|id| matches!(sketch.get_geometry(*id), Some(GeometryElement::Point(_))))
        .collect()
}

/// The block `id` belongs to: its point or one of its outlines' elements.
pub fn block_of(sketch: &Sketch, id: Uuid) -> Option<&TextBlock> {
    sketch
        .texts
        .iter()
        .find(|b| b.anchor == id || b.elements.contains(&id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(text: &str) -> TextSpec {
        TextSpec {
            text: text.to_string(),
            ..TextSpec::default()
        }
    }

    #[test]
    fn letters_make_closed_profiles_with_their_holes() {
        let mut sketch = Sketch::new("t");
        add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("oi")).unwrap();
        // An o is a ring, an i a stem and a dot: three regions.
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 4, "the o's two loops, the i's stem and dot");
    }

    #[test]
    fn the_text_follows_its_point_and_the_solver_leaves_it_be() {
        let mut sketch = Sketch::new("t");
        let id = add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("L")).unwrap();
        let anchor = sketch.texts[0].anchor;
        let lowest = |sketch: &Sketch| {
            outline_points(sketch)
                .iter()
                .filter_map(|p| sketch.point_position(*p))
                .fold(f64::MAX, |m, p| m.min(p.y))
        };
        assert!(lowest(&sketch).abs() < 1e-3, "the L stands on the baseline");
        sketch.add_constraint(crate::sketch::ConstraintKind::FixedPoint {
            point: anchor,
            position: Vec2D::new(5.0, 7.0),
        });
        let outcome = crate::solver::solve(&mut sketch);
        assert!(
            matches!(outcome, crate::solver::SolveOutcome::Converged { .. }),
            "{outcome:?}"
        );
        assert!((lowest(&sketch) - 7.0).abs() < 1e-3);
        // Changed, it is made again where its point is.
        change(
            &mut sketch,
            id,
            &TextSpec {
                size: 20.0,
                ..spec("LL")
            },
        )
        .unwrap();
        assert!((lowest(&sketch) - 7.0).abs() < 1e-3);
        let widest = outline_points(&sketch)
            .iter()
            .filter_map(|p| sketch.point_position(*p))
            .fold(f64::MIN, |m, p| m.max(p.x));
        // Two letters about half an em wide each, at a 20 mm em.
        assert!(widest > 5.0 + 18.0, "two letters at 20 mm: {widest}");
    }

    /// Copied and pasted, a text stays text: a second block on its own
    /// point and outlines, which follow that point as the first's do.
    #[test]
    fn a_pasted_text_is_text_of_its_own() {
        let mut sketch = Sketch::new("t");
        add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("L")).unwrap();
        let first = sketch.texts[0].clone();
        let outline: std::collections::HashSet<Uuid> = first
            .elements
            .iter()
            .copied()
            .filter(|e| !matches!(sketch.get_geometry(*e), Some(GeometryElement::Point(_))))
            .collect();
        let mut clip = Sketch::new("clipboard");
        crate::tools::copy_from(
            &sketch,
            &mut clip,
            &outline,
            &crate::tools::Similarity::translation(glam::DVec2::ZERO),
        );
        assert_eq!(clip.texts.len(), 1, "the clipboard holds the text");
        crate::commands::paste(&mut sketch, &clip, Vec2D::new(30.0, 0.0));

        assert_eq!(sketch.texts.len(), 2);
        let pasted = sketch.texts[1].clone();
        assert_ne!(pasted.anchor, first.anchor);
        assert!(pasted.elements.iter().all(|e| !first.elements.contains(e)));
        let at = sketch.point_position(pasted.anchor).unwrap();
        assert!((at.x - 30.0).abs() < 1e-4 && at.y.abs() < 1e-4, "{at:?}");

        // Its point moved, its outlines go with it, the first's stay.
        let left = |sketch: &Sketch, elements: &[Uuid]| {
            elements
                .iter()
                .filter_map(|p| sketch.point_position(*p))
                .fold(f64::MAX, |m, p| m.min(p.x))
        };
        let (before, other) = (
            left(&sketch, &pasted.elements),
            left(&sketch, &first.elements),
        );
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(pasted.anchor) {
            p.position = Vec2D::new(40.0, 0.0);
        }
        follow(&mut sketch);
        assert!((left(&sketch, &pasted.elements) - before - 10.0).abs() < 1e-3);
        assert!((left(&sketch, &first.elements) - other).abs() < 1e-6);
    }

    /// A family the machine lists is found by its name in any case and
    /// lays out text. A machine with no fonts installed has none to try.
    #[test]
    fn an_installed_family_is_found_by_name() {
        let Some(family) = installed_families().first() else {
            return;
        };
        let found = installed_font(&family.to_ascii_uppercase()).expect("listed, so found");
        assert_eq!(&found.family, family);
        let spec = TextSpec {
            font: found.family,
            font_data: Some(std::sync::Arc::new(found.bytes)),
            font_index: found.index,
            ..spec("A")
        };
        assert!(contours(&spec, DVec2::ZERO).is_ok());
    }

    #[test]
    fn deleting_the_point_takes_the_text() {
        let mut sketch = Sketch::new("t");
        add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("A")).unwrap();
        let anchor = sketch.texts[0].anchor;
        sketch.remove_geometry_cascade(&[anchor]);
        assert!(sketch.texts.is_empty());
        assert!(sketch.geometry.is_empty());
    }
}
