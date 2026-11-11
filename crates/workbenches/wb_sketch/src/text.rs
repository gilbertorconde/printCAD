//! Text in a sketch: one object, a string in a font standing on a point
//! (the start of its first line on the baseline, an ordinary point
//! constraints can hold). The block keeps the letters it uses as the font
//! draws them ([`Glyphs`]), so its outlines, closed lines and splines that
//! pad and pocket as any profile does, are made from it on any machine:
//! whenever the sketch is read or solved ([`refresh`]), with ids drawn
//! from the block's, so what is built from them keeps its names. They
//! stand in the sketch's geometry while it is in memory and are never
//! stored; the solver leaves them be. [`explode`] turns a block into
//! plain curves, the one way its letters become geometry of their own.

use std::borrow::Cow;
use std::collections::BTreeMap;

use glam::DVec2;
use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::sketch::{BSpline, GeometryElement, Line, Point, Sketch, TextBlock, Vec2D};

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
    std::fs::read(std::path::Path::new(font))
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

/// The family names of the installed fonts, sorted.
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
    // A name that looks like a file is read as one; any other is a file
    // when one can be read by it, else an installed family.
    let read = match std::fs::read(path) {
        Err(e) if is_file(&spec.font) => {
            return Err(format!("cannot read the font {}: {e}", spec.font));
        }
        read => read.ok(),
    };
    let found = match read {
        Some(bytes) => InstalledFont {
            family: String::new(),
            bytes,
            index: 0,
            file: path.file_name().map(|n| n.to_string_lossy().to_string()),
        },
        None => installed(&spec.font).ok_or_else(|| {
            format!(
                "no font named {} is installed here or kept in the document",
                spec.font
            )
        })?,
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

/// The letters a text block uses, as its font draws them, in the font's
/// own units: what its outlines are made from without the font.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Glyphs {
    /// The font's units in an em.
    pub em: f64,
    /// From one baseline to the next.
    pub line: f64,
    /// How far capitals and the tallest letters reach above the baseline.
    #[serde(default)]
    pub ascender: f64,
    /// How far letters reach below it, a negative number.
    #[serde(default)]
    pub descender: f64,
    /// Each character the font draws, by the character.
    pub chars: BTreeMap<String, Glyph>,
}

impl Glyphs {
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The letter `ch` draws as, when the font has one.
    pub fn get(&self, ch: char) -> Option<&Glyph> {
        let mut buffer = [0u8; 4];
        self.chars.get(&*ch.encode_utf8(&mut buffer))
    }
}

/// One character as its font draws it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    /// How far the pen moves after it.
    pub advance: f64,
    /// Each closed contour: its start, then each piece to its end as two
    /// numbers (a line to there), four (a quadratic's control and end) or
    /// six (a cubic's two controls and end). A space has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contours: Vec<Vec<Vec<f64>>>,
}

/// Records a glyph's contours in font units.
#[derive(Default)]
struct Recorder {
    start: [f64; 2],
    pen: [f64; 2],
    current: Vec<Vec<f64>>,
    done: Vec<Vec<Vec<f64>>>,
}

impl Recorder {
    fn to(&mut self, piece: Vec<f64>) {
        let n = piece.len();
        let end = [piece[n - 2], piece[n - 1]];
        // A piece that goes nowhere adds nothing to the outline.
        if (end[0] - self.pen[0]).hypot(end[1] - self.pen[1]) > 1e-6 {
            self.current.push(piece);
        }
        self.pen = end;
    }
}

impl ttf_parser::OutlineBuilder for Recorder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = [x.into(), y.into()];
        self.pen = self.start;
        self.current = vec![self.start.to_vec()];
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.to(vec![x.into(), y.into()]);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.to(vec![x1.into(), y1.into(), x.into(), y.into()]);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.to(vec![
            x1.into(),
            y1.into(),
            x2.into(),
            y2.into(),
            x.into(),
            y.into(),
        ]);
    }

    fn close(&mut self) {
        if self.current.len() > 1 {
            if (self.pen[0] - self.start[0]).hypot(self.pen[1] - self.start[1]) > 1e-6 {
                self.current.push(self.start.to_vec());
            }
            self.done.push(std::mem::take(&mut self.current));
        }
        self.current.clear();
        self.pen = self.start;
    }
}

/// The letters of `chars` as `spec`'s font draws them; characters the font
/// has no letter for are left out.
pub fn read_glyphs(spec: &TextSpec, chars: &str) -> Result<Glyphs, String> {
    let (bytes, index) = font_bytes(spec)?;
    let face = ttf_parser::Face::parse(&bytes, index)
        .map_err(|e| format!("{} is not a font this can read: {e}", spec.font))?;
    let mut glyphs = Glyphs {
        em: f64::from(face.units_per_em()),
        line: f64::from(face.ascender() - face.descender() + face.line_gap()),
        ascender: f64::from(face.ascender()),
        descender: f64::from(face.descender()),
        chars: BTreeMap::new(),
    };
    for ch in chars.chars().filter(|c| !c.is_control()) {
        if glyphs.get(ch).is_some() {
            continue;
        }
        let Some(id) = face.glyph_index(ch) else {
            continue;
        };
        let mut recorder = Recorder::default();
        face.outline_glyph(id, &mut recorder);
        ttf_parser::OutlineBuilder::close(&mut recorder);
        glyphs.chars.insert(
            ch.to_string(),
            Glyph {
                advance: f64::from(face.glyph_hor_advance(id).unwrap_or(0)),
                contours: recorder.done,
            },
        );
    }
    Ok(glyphs)
}

/// One piece of a laid-out outline, in sketch coordinates: where it
/// starts and its controls; it ends where the next piece of its contour
/// starts.
#[derive(Debug, Clone, Copy)]
enum Seg {
    Line(DVec2),
    Quad(DVec2, DVec2),
    Cubic(DVec2, DVec2, DVec2),
}

impl Seg {
    fn start(&self) -> DVec2 {
        match *self {
            Seg::Line(a) | Seg::Quad(a, _) | Seg::Cubic(a, _, _) => a,
        }
    }
}

/// `glyph`'s contours, each point in font units taken by `place`.
fn placed(glyph: &Glyph, place: impl Fn(DVec2) -> DVec2) -> Vec<Vec<Seg>> {
    glyph
        .contours
        .iter()
        .filter_map(|contour| {
            let (first, pieces) = contour.split_first()?;
            let mut pen = place(DVec2::new(*first.first()?, *first.get(1)?));
            let mut segs = Vec::new();
            for piece in pieces {
                let p = |i: usize| place(DVec2::new(piece[i], piece[i + 1]));
                let seg = match piece.len() {
                    2 => Seg::Line(pen),
                    4 => Seg::Quad(pen, p(0)),
                    6 => Seg::Cubic(pen, p(0), p(2)),
                    _ => continue,
                };
                pen = p(piece.len() - 2);
                segs.push(seg);
            }
            (!segs.is_empty()).then_some(segs)
        })
        .collect()
}

/// Each drawn character of `block` by its place in the string, with its
/// contours where the block lays it out, or `None` when the block has
/// nothing to stand on.
fn layout(block: &TextBlock, sketch: &Sketch) -> Option<Vec<(usize, Vec<Vec<Seg>>)>> {
    let glyphs = &block.glyphs;
    let at = sketch.point_position(block.anchor)?.to_glam();
    if !(glyphs.em > 0.0 && block.size.is_finite() && block.size > 0.0) {
        return None;
    }
    if !(at.is_finite() && block.spacing.is_finite() && block.angle.is_finite()) {
        return None;
    }
    let scale = block.size / glyphs.em;
    let turn = DVec2::from_angle(block.angle.to_radians());
    let mut pen = DVec2::ZERO;
    let mut out = Vec::new();
    for (i, ch) in block.text.chars().enumerate() {
        if ch == '\n' {
            pen = DVec2::new(0.0, pen.y - glyphs.line * scale);
            continue;
        }
        let Some(glyph) = glyphs.get(ch) else {
            continue;
        };
        let origin = pen;
        let contours = placed(glyph, |p| at + turn.rotate(origin + p * scale));
        if !contours.is_empty() {
            out.push((i, contours));
        }
        pen.x += glyph.advance * scale + block.spacing;
    }
    Some(out)
}

/// A new point at `p` named `id`.
fn new_point(sketch: &mut Sketch, made: &mut Vec<Uuid>, p: DVec2, id: Uuid) -> Uuid {
    let mut point = Point::new(Vec2D::new(p.x, p.y));
    point.id = id;
    made.push(sketch.add_geometry(GeometryElement::Point(point)));
    id
}

/// Put `contours` into the sketch: a line for each straight piece, one
/// spline for each run of curved pieces of one degree (joined exactly, a
/// knot standing as often as the degree at each join), consecutive pieces
/// sharing their end points, each element named by `name`. Every element
/// made goes into `made`.
fn build(
    sketch: &mut Sketch,
    contours: Vec<Vec<Seg>>,
    made: &mut Vec<Uuid>,
    name: &mut dyn FnMut() -> Uuid,
) {
    for contour in contours {
        let n = contour.len();
        if n == 0 {
            continue;
        }
        let first = new_point(sketch, made, contour[0].start(), name());
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
                new_point(sketch, made, contour[j].start(), name())
            };
            let element = match contour[i] {
                Seg::Line(..) => {
                    let mut line = Line::new(from, to);
                    line.id = name();
                    GeometryElement::Line(line)
                }
                _ => {
                    let degree = if matches!(contour[i], Seg::Quad(..)) {
                        2
                    } else {
                        3
                    };
                    let mut control = vec![from];
                    for (k, seg) in contour[i..j].iter().enumerate() {
                        let inner: Vec<DVec2> = match *seg {
                            Seg::Quad(_, b) => vec![b],
                            Seg::Cubic(_, b, c) => vec![b, c],
                            Seg::Line(..) => Vec::new(),
                        };
                        for p in inner {
                            control.push(new_point(sketch, made, p, name()));
                        }
                        // The join with the next piece, or the run's far end.
                        control.push(match contour.get(i + k + 1) {
                            Some(next) if i + k + 1 < j => {
                                new_point(sketch, made, next.start(), name())
                            }
                            _ => to,
                        });
                    }
                    let pieces = j - i;
                    let mut knots = vec![0.0; degree + 1];
                    for p in 1..pieces {
                        let k = p as f64 / pieces as f64;
                        knots.extend(std::iter::repeat_n(k, degree));
                    }
                    knots.extend(std::iter::repeat_n(1.0, degree + 1));
                    GeometryElement::BSpline(BSpline {
                        id: name(),
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
}

/// Take the outlines made from text out of the sketch, as it is stored.
pub fn strip(sketch: &mut Sketch) {
    if sketch.derived.is_empty() {
        return;
    }
    let derived = std::mem::take(&mut sketch.derived);
    let ids: Vec<Uuid> = derived.into_iter().collect();
    remove_elements(sketch, &ids);
    for block in sketch.texts.iter_mut().filter(|b| !b.glyphs.is_empty()) {
        block.elements.clear();
    }
}

/// Make every block's outlines again where it stands.
pub fn refresh(sketch: &mut Sketch) {
    strip(sketch);
    follow_stored(sketch);
    for i in 0..sketch.texts.len() {
        if sketch.texts[i].glyphs.is_empty() {
            continue;
        }
        let Some(outlines) = layout(&sketch.texts[i], sketch) else {
            continue;
        };
        let block = sketch.texts[i].id;
        let mut made = Vec::new();
        for (place, contours) in outlines {
            let mut k = 0u32;
            let mut name = || {
                k += 1;
                Uuid::new_v5(&block, format!("{place}.{k}").as_bytes())
            };
            build(sketch, contours, &mut made, &mut name);
        }
        sketch.derived.extend(made.iter().copied());
        sketch.texts[i].elements = made;
    }
}

/// Bring a sketch just read up to date: a block an older file stored as
/// outlines in a font the application carries reads its letters from that
/// font and drops the stored curves (and any constraint on them), then
/// every block's outlines are made.
pub fn loaded(sketch: &mut Sketch) {
    let mut stored = Vec::new();
    for block in sketch.texts.iter_mut().filter(|b| b.glyphs.is_empty()) {
        if !is_bundled(&block.font) {
            continue;
        }
        if let Ok(glyphs) = read_glyphs(&TextSpec::of(block), &block.text) {
            block.glyphs = glyphs;
            block.placed = None;
            stored.append(&mut block.elements);
        }
    }
    remove_elements(sketch, &stored);
    refresh(sketch);
}

/// [`loaded`] for blocks whose font the document keeps: their letters read
/// from it, their stored curves dropped. Whether any changed.
pub fn upgrade_stored(sketch: &mut Sketch, document: &core_document::Document) -> bool {
    let mut stored = Vec::new();
    for block in sketch.texts.iter_mut().filter(|b| b.glyphs.is_empty()) {
        let mut spec = TextSpec::of(block);
        if !read_kept_font(document, &mut spec) {
            continue;
        }
        if let Ok(glyphs) = read_glyphs(&spec, &block.text) {
            block.glyphs = glyphs;
            block.placed = None;
            stored.append(&mut block.elements);
        }
    }
    if stored.is_empty() {
        return false;
    }
    remove_elements(sketch, &stored);
    refresh(sketch);
    true
}

/// A new text block of `spec`, its font ready, standing on a new point at
/// `at`. Returns the block's id.
pub fn add(sketch: &mut Sketch, at: Vec2D, spec: &TextSpec) -> Result<Uuid, String> {
    let block = new_block(spec, Uuid::nil())?;
    let anchor = sketch.add_geometry(GeometryElement::Point(Point::new(at)));
    let id = block.id;
    sketch.texts.push(TextBlock { anchor, ..block });
    refresh(sketch);
    if block_of(sketch, id).is_none_or(|b| b.elements.is_empty()) {
        sketch.texts.retain(|b| b.id != id);
        sketch.remove_geometry_cascade(&[anchor]);
        return Err("the text has nothing to draw".to_string());
    }
    Ok(id)
}

/// A block of `spec` on `anchor`, its letters read from the font.
fn new_block(spec: &TextSpec, anchor: Uuid) -> Result<TextBlock, String> {
    check(spec)?;
    Ok(TextBlock {
        id: Uuid::new_v4(),
        text: spec.text.clone(),
        font: spec.font.clone(),
        font_asset: spec.font_asset,
        size: spec.size,
        spacing: spec.spacing,
        angle: spec.angle,
        anchor,
        glyphs: read_glyphs(spec, &spec.text)?,
        elements: Vec::new(),
        placed: None,
    })
}

/// Why `spec` makes no text, if it does not.
fn check(spec: &TextSpec) -> Result<(), String> {
    if !(spec.size.is_finite() && spec.size > 0.0) {
        return Err("the text's size must be finite and more than 0".to_string());
    }
    if !(spec.spacing.is_finite() && spec.angle.is_finite()) {
        return Err("the text's spacing and angle must be finite".to_string());
    }
    if spec.text.chars().all(char::is_whitespace) {
        return Err("the text has nothing to draw".to_string());
    }
    Ok(())
}

/// Make block `id` as `spec` says, where its point stands now; its letters
/// are read again from the font when its string or font changed, so the
/// font must be ready then.
pub fn change(sketch: &mut Sketch, id: Uuid, spec: &TextSpec) -> Result<(), String> {
    check(spec)?;
    let Some(index) = sketch.texts.iter().position(|b| b.id == id) else {
        return Err("no such text".to_string());
    };
    let block = &sketch.texts[index];
    let reread = block.glyphs.is_empty()
        || block.text != spec.text
        || block.font != spec.font
        || block.font_asset != spec.font_asset;
    let glyphs = if reread {
        let glyphs = read_glyphs(spec, &spec.text)?;
        if spec
            .text
            .chars()
            .all(|c| c.is_whitespace() || glyphs.get(c).is_none())
        {
            return Err("the font has no letter for any of the text".to_string());
        }
        Some(glyphs)
    } else {
        None
    };
    let stored = if block.is_stored_outlines() {
        block.elements.clone()
    } else {
        Vec::new()
    };
    let block = &mut sketch.texts[index];
    block.text = spec.text.clone();
    block.font = spec.font.clone();
    block.font_asset = spec.font_asset;
    block.size = spec.size;
    block.spacing = spec.spacing;
    block.angle = spec.angle;
    if let Some(glyphs) = glyphs {
        block.glyphs = glyphs;
        block.placed = None;
        block.elements.clear();
    }
    remove_elements(sketch, &stored);
    refresh(sketch);
    Ok(())
}

/// Turn block `id` into plain curves: its outlines stay as ordinary
/// geometry, named as they were, its point stays, and the text is gone.
/// Returns the curves and points it left.
pub fn explode(sketch: &mut Sketch, id: Uuid) -> Result<Vec<Uuid>, String> {
    let Some(index) = sketch.texts.iter().position(|b| b.id == id) else {
        return Err("no such text".to_string());
    };
    let block = sketch.texts.remove(index);
    for element in &block.elements {
        sketch.derived.remove(element);
    }
    Ok(block.elements)
}

/// Take elements away without the text-block bookkeeping that removing
/// through the sketch does.
fn remove_elements(sketch: &mut Sketch, ids: &[Uuid]) {
    if ids.is_empty() {
        return;
    }
    let doomed: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
    sketch.geometry.retain(|g| !doomed.contains(&g.id()));
    sketch.constraints.retain(|c| {
        !crate::sketch::constraint_refs(&c.kind)
            .iter()
            .any(|id| doomed.contains(id))
    });
    sketch.construction.retain(|id| !doomed.contains(id));
}

/// Carry the stored outlines of an older file's blocks to where their
/// points now stand.
fn follow_stored(sketch: &mut Sketch) {
    for i in 0..sketch.texts.len() {
        let block = &sketch.texts[i];
        if !block.is_stored_outlines() {
            continue;
        }
        let Some(at) = sketch.point_position(block.anchor) else {
            continue;
        };
        let delta = at - block.placed.unwrap_or(at);
        sketch.texts[i].placed = Some(at);
        if delta.to_glam().length() < 1e-7 {
            continue;
        }
        let points: Vec<Uuid> = sketch.texts[i].elements.clone();
        for id in points {
            if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(id) {
                p.position = p.position + delta;
            }
        }
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
        .find(|b| b.id == id || b.anchor == id || b.elements.contains(&id))
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

    fn lowest(sketch: &Sketch) -> f64 {
        outline_points(sketch)
            .iter()
            .filter_map(|p| sketch.point_position(*p))
            .fold(f64::MAX, |m, p| m.min(p.y))
    }

    #[test]
    fn letters_make_closed_profiles_with_their_holes() {
        let mut sketch = Sketch::new("t");
        add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("oi")).unwrap();
        // An o is a ring, an i a stem and a dot: three regions.
        let wires = crate::profile::extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 4, "the o's two loops, the i's stem and dot");
    }

    /// A file holds the block and its point, never its outlines; read
    /// back, the outlines are made again with the same names, and stand
    /// where the point does after a solve and a change.
    #[test]
    fn text_is_stored_as_one_object_and_follows_its_point() {
        let mut sketch = Sketch::new("t");
        let id = add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("L")).unwrap();
        let outlines = sketch.texts[0].elements.clone();
        assert!(outlines.len() > 4);
        let json = serde_json::to_value(&sketch).unwrap();
        assert_eq!(
            json["geometry"].as_array().unwrap().len(),
            1,
            "the point alone"
        );
        assert!(json["texts"][0].get("elements").is_none());
        let mut read: Sketch = serde_json::from_value(json).unwrap();
        assert_eq!(read.texts[0].elements, outlines);
        assert!(lowest(&read).abs() < 1e-3, "the L stands on the baseline");

        let anchor = read.texts[0].anchor;
        read.add_constraint(crate::sketch::ConstraintKind::FixedPoint {
            point: anchor,
            position: Vec2D::new(5.0, 7.0),
        });
        let outcome = crate::solver::solve(&mut read);
        assert!(
            matches!(outcome, crate::operations::SolveOutcome::Converged { .. }),
            "{outcome:?}"
        );
        assert!((lowest(&read) - 7.0).abs() < 1e-3);
        change(
            &mut read,
            id,
            &TextSpec {
                size: 20.0,
                ..spec("LL")
            },
        )
        .unwrap();
        assert!((lowest(&read) - 7.0).abs() < 1e-3);
        let widest = outline_points(&read)
            .iter()
            .filter_map(|p| read.point_position(*p))
            .fold(f64::MIN, |m, p| m.max(p.x));
        // Two letters about half an em wide each, at a 20 mm em.
        assert!(widest > 5.0 + 18.0, "two letters at 20 mm: {widest}");
        assert!(
            read.texts[0].elements.starts_with(&outlines[..1]),
            "the first letter keeps its names"
        );
    }

    /// Exploded, the letters are plain curves of their own, stored as any
    /// and named as they were.
    #[test]
    fn exploding_leaves_plain_curves() {
        let mut sketch = Sketch::new("t");
        let id = add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("I")).unwrap();
        let outlines = explode(&mut sketch, id).unwrap();
        assert!(sketch.texts.is_empty() && sketch.derived.is_empty());
        let read: Sketch = serde_json::from_value(serde_json::to_value(&sketch).unwrap()).unwrap();
        for element in &outlines {
            assert!(read.get_geometry(*element).is_some());
        }
        assert_eq!(read.geometry.len(), outlines.len() + 1, "and the point");
    }

    /// An older file stored its text's outlines as geometry: read, the
    /// block takes its letters from the font, and the stored curves, with
    /// what was constrained to them, are gone.
    #[test]
    fn an_older_files_stored_outlines_are_made_again() {
        let mut sketch = Sketch::new("t");
        let anchor = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(2.0, 0.0))));
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(2.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(3.0, 7.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_constraint(crate::sketch::ConstraintKind::Vertical { element: line });
        sketch.texts.push(TextBlock {
            id: Uuid::new_v4(),
            text: "I".into(),
            font: DEFAULT_FONT.into(),
            font_asset: None,
            size: 10.0,
            spacing: 0.0,
            angle: 0.0,
            anchor,
            glyphs: Glyphs::default(),
            elements: vec![a, b, line],
            placed: Some(Vec2D::new(2.0, 0.0)),
        });
        let read: Sketch = serde_json::from_value(serde_json::to_value(&sketch).unwrap()).unwrap();
        assert!(read.constraints.is_empty());
        assert!(read.get_geometry(line).is_none());
        let block = &read.texts[0];
        assert!(!block.glyphs.is_empty() && read.derived.len() == block.elements.len());
        assert!(lowest(&read).abs() < 1e-3);
    }

    /// Copied and pasted, a text stays text: a second block on its own
    /// point and outlines.
    #[test]
    fn a_pasted_text_is_text_of_its_own() {
        let mut sketch = Sketch::new("t");
        add(&mut sketch, Vec2D::new(0.0, 0.0), &spec("L")).unwrap();
        let first = sketch.texts[0].clone();
        let mut clip = Sketch::new("clipboard");
        crate::tools::copy_from(
            &sketch,
            &mut clip,
            &first.elements[1..2].iter().copied().collect(),
            &crate::tools::Similarity::translation(glam::DVec2::ZERO),
        );
        assert_eq!(clip.texts.len(), 1, "the clipboard holds the text");
        assert_eq!(clip.geometry.len(), 1 + first.elements.len());
        crate::commands::paste(&mut sketch, &clip, Vec2D::new(30.0, 0.0));

        assert_eq!(sketch.texts.len(), 2);
        let pasted = sketch.texts[1].clone();
        assert_ne!(pasted.anchor, first.anchor);
        assert!(pasted.elements.iter().all(|e| !first.elements.contains(e)));
        let at = sketch.point_position(pasted.anchor).unwrap();
        assert!((at.x - 30.0).abs() < 1e-4 && at.y.abs() < 1e-4, "{at:?}");
    }

    /// A family the machine lists is found by its name in any case and
    /// reads letters. A machine with no fonts installed has none to try.
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
        assert!(read_glyphs(&spec, "A").is_ok());
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
