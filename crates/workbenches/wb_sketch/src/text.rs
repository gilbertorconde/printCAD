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
    /// From its point, in a frame or along curves.
    pub layout: TextLayout,
    pub align: Align,
    pub valign: VAlign,
    /// From one baseline to the next, as a share of the font's own.
    pub line_spacing: f64,
    /// What each `{formula}` in the string comes to, worked out against
    /// the document ([`values_of`]).
    pub values: Vec<TextValue>,
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
            layout: TextLayout::Point,
            align: Align::Start,
            valign: VAlign::Baseline,
            line_spacing: 1.0,
            values: Vec::new(),
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
            layout: block.layout.clone(),
            align: block.align,
            valign: block.valign,
            line_spacing: block.line_spacing,
            values: block.values.clone(),
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

/// How a text block lays its lines out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TextLayout {
    /// Lines from its point, turned by its angle.
    #[default]
    Point,
    /// Inside a frame: four corner points, counter-clockwise from its
    /// point, the bottom-left one. The frame's bottom edge is the lines'
    /// way; its size is the room they have.
    Frame {
        corners: [Uuid; 4],
        /// Lines too long for the frame break between words.
        #[serde(default)]
        wrap: bool,
        /// Text larger than the frame is made smaller to fit it.
        #[serde(default)]
        fit: bool,
    },
    /// Along curves, end to end in the order given; each letter stands
    /// upright on the curve where its middle falls.
    Path {
        curves: Vec<Uuid>,
        /// Which side of the curve the letters stand on.
        #[serde(default)]
        side: PathSide,
        /// Millimetres along the curve the text stands from its point,
        /// which lies on the curve: the text starts, is centred or ends
        /// there as its alignment says.
        #[serde(default)]
        offset: f64,
        /// The letters spread to fill the whole curve.
        #[serde(default)]
        fit: bool,
        /// The curve walked from its other end.
        #[serde(default)]
        reversed: bool,
    },
}

impl TextLayout {
    pub fn is_point(&self) -> bool {
        matches!(self, TextLayout::Point)
    }
}

/// Where lines stand across their room: along the line, from its start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Start,
    Middle,
    End,
}

/// Where the block of lines stands up and down from its point, or in its
/// frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VAlign {
    /// The first line's baseline on the point (in a frame, at its top).
    #[default]
    Baseline,
    Top,
    Middle,
    Bottom,
}

/// The side of its curve text along one stands on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathSide {
    /// Standing on the curve, to its left as it runs.
    #[default]
    Above,
    /// Hanging from it, the letters' tops on the curve.
    Below,
    /// Its middle on the curve.
    Centred,
}

/// What one `{formula}` of a text comes to: a number formulas set, in
/// millimetres or degrees, and the kind of quantity it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TextValue {
    pub value: f64,
    /// Its power of length: 1 for a length, 0 for a plain number.
    #[serde(default, skip_serializing_if = "is_default")]
    pub length: i8,
    /// Its power of angle.
    #[serde(default, skip_serializing_if = "is_default")]
    pub angle: i8,
}

impl TextValue {
    pub fn dim(&self) -> core_document::expr::Dim {
        core_document::expr::Dim {
            length: self.length,
            angle: self.angle,
        }
    }
}

/// A piece of a text's string: words as they are, or a formula with the
/// decimals it is written to (as many as it needs, up to three, when not
/// given).
#[derive(Debug, Clone, PartialEq)]
pub enum Part<'a> {
    Words(Cow<'a, str>),
    Formula {
        text: &'a str,
        decimals: Option<usize>,
    },
}

/// `text` cut into words and `{formula}`s; `{formula:2}` writes two
/// decimals, `{{` and `}}` are braces as they are, and a brace with no
/// partner is a brace too.
pub fn parts(text: &str) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    let mut words = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(['{', '}']) {
        words.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            words.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(after) = tail.strip_prefix('}') {
            words.push('}');
            rest = after;
            continue;
        }
        let Some(end) = tail.find('}') else {
            words.push_str(tail);
            rest = "";
            break;
        };
        let inner = &tail[1..end];
        let (formula, decimals) = match inner.rsplit_once(':') {
            Some((f, d)) if !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()) => {
                (f, d.parse::<usize>().ok().map(|d| d.min(9)))
            }
            _ => (inner, None),
        };
        if formula.trim().is_empty() {
            words.push_str(&tail[..=end]);
        } else {
            if !words.is_empty() {
                out.push(Part::Words(Cow::Owned(std::mem::take(&mut words))));
            }
            out.push(Part::Formula {
                text: formula.trim(),
                decimals,
            });
        }
        rest = &tail[end + 1..];
    }
    words.push_str(rest);
    if !words.is_empty() {
        out.push(Part::Words(Cow::Owned(words)));
    }
    out
}

/// The formulas of `text`, in order.
pub fn formulas(text: &str) -> Vec<&str> {
    parts(text)
        .into_iter()
        .filter_map(|p| match p {
            Part::Formula { text, .. } => Some(text),
            Part::Words(_) => None,
        })
        .collect()
}

/// `text` with its formulas written as `values`; one without a value yet
/// shows as itself.
pub fn fill<'a>(text: &'a str, values: &[TextValue]) -> Cow<'a, str> {
    if !text.contains(['{', '}']) {
        return Cow::Borrowed(text);
    }
    let mut out = String::new();
    let mut k = 0;
    for part in parts(text) {
        match part {
            Part::Words(words) => out.push_str(&words),
            Part::Formula { text, decimals } => {
                match values.get(k) {
                    Some(v) => out.push_str(&number(v.value, decimals)),
                    None => out.push_str(text),
                }
                k += 1;
            }
        }
    }
    Cow::Owned(out)
}

/// `value` written to `decimals`, or with as many as it needs up to three.
fn number(value: f64, decimals: Option<usize>) -> String {
    let written = match decimals {
        Some(d) => format!("{value:.d$}"),
        None => {
            let s = format!("{value:.3}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            s.to_string()
        }
    };
    if written
        .trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        written.trim_start_matches('-').to_string()
    } else {
        written
    }
}

/// The characters a text can show: its own, and with a formula the ones
/// numbers are written in.
fn letters_of(text: &str) -> String {
    if formulas(text).is_empty() {
        text.to_string()
    } else {
        format!("{text}0123456789.-")
    }
}

/// What each formula of `text` comes to in `document` now, or why one
/// cannot be worked out.
pub fn values_of(document: &core_document::Document, text: &str) -> Result<Vec<TextValue>, String> {
    formulas(text)
        .into_iter()
        .map(|formula| {
            let q = document
                .evaluate_formula(formula, None)
                .map_err(|why| format!("{{{formula}}}: {why}"))?;
            Ok(TextValue {
                value: q.value,
                length: q.dim.length,
                angle: q.dim.angle,
            })
        })
        .collect()
}

/// The key the `k`-th formula of block `block` is kept under.
pub fn formula_key(block: Uuid, k: usize) -> String {
    format!("{block}/value/{k}")
}

/// Keep `text`'s formulas as sketch `sketch`'s formulas for block
/// `block`'s values, taking away any it no longer has.
pub fn keep_formulas(
    document: &mut core_document::Document,
    sketch: core_document::FeatureId,
    block: Uuid,
    text: &str,
) -> Result<(), String> {
    let wanted = formulas(text);
    let prefix = format!("{block}/value/");
    let stale: Vec<String> = document
        .get_feature_meta(sketch)
        .map(|node| {
            node.formulas
                .keys()
                .filter(|key| {
                    key.strip_prefix(&prefix)
                        .and_then(|k| k.parse::<usize>().ok())
                        .is_some_and(|k| k >= wanted.len())
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for key in stale {
        document
            .set_feature_formula(sketch, key, None)
            .map_err(|e| e.to_string())?;
    }
    for (k, formula) in wanted.into_iter().enumerate() {
        document
            .set_feature_formula(sketch, formula_key(block, k), Some(formula.to_string()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Whether `value` is its kind's default, which a file leaves out.
pub fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

pub(crate) fn one() -> f64 {
    1.0
}

pub(crate) fn is_one(value: &f64) -> bool {
    *value == 1.0
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

/// One character of a laid-out line: its place in the string, its letter,
/// where its pen stands along the line and how far it advances, mm.
struct Laid<'a> {
    place: usize,
    glyph: &'a Glyph,
    x: f64,
    advance: f64,
    space: bool,
}

/// The lines `text` makes in `glyphs` at `scale` with `spacing` between
/// letters, broken between words where a line would pass `room`.
fn lines<'a>(
    text: &str,
    glyphs: &'a Glyphs,
    scale: f64,
    spacing: f64,
    room: Option<f64>,
) -> Vec<Vec<Laid<'a>>> {
    let mut out = Vec::new();
    let mut line: Vec<Laid> = Vec::new();
    let place_x = |line: &mut Vec<Laid>| {
        let mut x = 0.0;
        for laid in line.iter_mut() {
            laid.x = x;
            x += laid.advance + spacing;
        }
    };
    for (place, ch) in text.chars().enumerate() {
        if ch == '\n' {
            out.push(std::mem::take(&mut line));
            continue;
        }
        let Some(glyph) = glyphs.get(ch) else {
            continue;
        };
        line.push(Laid {
            place,
            glyph,
            x: 0.0,
            advance: glyph.advance * scale,
            space: ch.is_whitespace(),
        });
        place_x(&mut line);
        if let (Some(room), Some(last)) = (room, line.last())
            && last.x + last.advance > room
            && let Some(cut) = line.iter().rposition(|l| l.space).filter(|c| *c > 0)
        {
            let rest = line.split_off(cut + 1);
            line.pop();
            out.push(std::mem::replace(&mut line, rest));
            place_x(&mut line);
        }
    }
    out.push(line);
    for line in &mut out {
        place_x(line);
    }
    out
}

/// How long a line reads, its trailing spaces left out.
fn width(line: &[Laid]) -> f64 {
    line.iter()
        .rev()
        .find(|l| !l.space)
        .map_or(0.0, |l| l.x + l.advance)
}

/// The curves `curves` run end to end as one polyline, each turned to
/// follow on from the one before, and whether it closes. A closed one runs
/// clockwise from its top, so text above it stands outside it and reads
/// across its top; `reversed` walks it the other way.
fn path_points(sketch: &Sketch, curves: &[Uuid], reversed: bool) -> Option<(Vec<DVec2>, bool)> {
    let mut pieces: Vec<(Vec<DVec2>, bool)> = Vec::new();
    for id in curves {
        let geom = sketch.get_geometry(*id)?;
        let curve = crate::curves::Curve::of(sketch, geom)?;
        let n = if matches!(geom, GeometryElement::Line(_)) {
            1
        } else {
            256
        };
        let (t0, t1) = curve.span;
        let points: Vec<DVec2> = (0..=n)
            .map(|i| curve.at(t0 + (t1 - t0) * i as f64 / n as f64))
            .collect();
        pieces.push((points, curve.closed));
    }
    let near = |a: DVec2, b: DVec2| a.distance(b) < 1e-4;
    if pieces.len() > 1 {
        let (first_end, first_start) = (*pieces[0].0.last()?, pieces[0].0[0]);
        let (a, b) = (pieces[1].0[0], *pieces[1].0.last()?);
        let to_second = |p: DVec2| p.distance(a).min(p.distance(b));
        if to_second(first_start) < to_second(first_end) {
            pieces[0].0.reverse();
        }
    }
    let mut points: Vec<DVec2> = Vec::new();
    for (mut piece, _) in pieces.iter().cloned() {
        if let Some(last) = points.last().copied() {
            if piece.last().is_some_and(|p| near(*p, last))
                || piece[0].distance(last) > piece.last()?.distance(last)
            {
                piece.reverse();
            }
            if near(piece[0], last) {
                piece.remove(0);
            }
        }
        points.extend(piece);
    }
    let closed = if pieces.len() == 1 {
        pieces[0].1
    } else {
        points.len() > 2 && near(points[0], *points.last()?)
    };
    if closed {
        if near(points[0], *points.last()?) {
            points.pop();
        }
        // From the top, clockwise.
        let area: f64 = (0..points.len())
            .map(|i| points[i].perp_dot(points[(i + 1) % points.len()]))
            .sum();
        if area > 0.0 {
            points.reverse();
        }
        let top = (0..points.len()).max_by(|a, b| points[*a].y.total_cmp(&points[*b].y))?;
        points.rotate_left(top);
        points.push(points[0]);
    }
    if reversed {
        points.reverse();
    }
    (points.len() > 1).then_some((points, closed))
}

/// A polyline walked by distance along it.
struct Walk {
    points: Vec<DVec2>,
    along: Vec<f64>,
    closed: bool,
}

impl Walk {
    fn new(points: Vec<DVec2>, closed: bool) -> Option<Self> {
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along.last()? + w[0].distance(w[1]));
        }
        (*along.last()? > 1e-9).then_some(Self {
            points,
            along,
            closed,
        })
    }

    fn length(&self) -> f64 {
        *self.along.last().unwrap_or(&0.0)
    }

    /// How far along the walk the spot nearest `p` is.
    fn nearest(&self, p: DVec2) -> f64 {
        let mut best = (f64::MAX, 0.0);
        for (i, w) in self.points.windows(2).enumerate() {
            let d = w[1] - w[0];
            let t = ((p - w[0]).dot(d) / d.length_squared().max(1e-18)).clamp(0.0, 1.0);
            let gap = p.distance(w[0] + d * t);
            if gap < best.0 {
                best = (gap, self.along[i] + d.length() * t);
            }
        }
        best.1
    }

    /// Where the walk is `s` along it and which way it runs there; past an
    /// open walk's ends it carries straight on.
    fn at(&self, s: f64) -> (DVec2, DVec2) {
        let length = self.length();
        let s = if self.closed { s.rem_euclid(length) } else { s };
        let i = match self.along.binary_search_by(|a| a.total_cmp(&s)) {
            Ok(i) | Err(i) => i.clamp(1, self.points.len() - 1),
        };
        let (a, b) = (self.points[i - 1], self.points[i]);
        let way = (b - a).normalize_or_zero();
        (a + way * (s - self.along[i - 1]), way)
    }
}

/// Each drawn character of `block` by its place in the string, with its
/// contours where the block lays it out, or `None` when the block has
/// nothing to stand on.
fn layout(block: &TextBlock, sketch: &Sketch) -> Option<Vec<(usize, Vec<Vec<Seg>>)>> {
    let glyphs = &block.glyphs;
    sketch.point_position(block.anchor)?;
    let finite = [block.size, block.spacing, block.angle, block.line_spacing];
    if !(glyphs.em > 0.0 && block.size > 0.0 && finite.iter().all(|v| v.is_finite())) {
        return None;
    }
    let mut scale = block.size / glyphs.em;
    let mut spacing = block.spacing;
    let mut out = Vec::new();
    // A letter placed by `place`, which takes it from font units.
    let mut put = |laid: &Laid, place: &dyn Fn(DVec2) -> DVec2| {
        let contours = placed(laid.glyph, place);
        if !contours.is_empty() {
            out.push((laid.place, contours));
        }
    };
    match &block.layout {
        TextLayout::Path {
            curves,
            side,
            offset,
            fit,
            reversed,
        } => {
            let offset = *offset;
            let (points, closed) = path_points(sketch, curves, *reversed)?;
            let walk = Walk::new(points, closed)?;
            let text = block.shown().replace('\n', " ");
            let mut line = lines(&text, glyphs, scale, spacing, None).swap_remove(0);
            let length = walk.length();
            let n = line.len();
            if *fit && n > 1 {
                let letters: f64 = line.iter().map(|l| l.advance).sum();
                let gaps = if closed { n } else { n - 1 } as f64;
                let gap = (length - letters) / gaps;
                let mut x = if closed { gap / 2.0 } else { 0.0 };
                for laid in &mut line {
                    laid.x = x;
                    x += laid.advance + gap;
                }
            }
            let w = width(&line);
            // The text stands where its point lies along the curves.
            let from = walk.nearest(sketch.point_position(block.anchor)?.to_glam()) + offset;
            let start = match block.align {
                _ if *fit && !closed => 0.0,
                _ if *fit => from,
                Align::Start => from,
                Align::Middle => from - w / 2.0,
                Align::End => from - w,
            };
            let (asc, desc) = (glyphs.ascender * scale, glyphs.descender * scale);
            let shift = match side {
                PathSide::Above => 0.0,
                PathSide::Below => -asc,
                PathSide::Centred => -(asc + desc) / 2.0,
            };
            for laid in &line {
                let middle = start + laid.x + laid.advance / 2.0;
                let (at, way) = walk.at(middle);
                let up = way.perp();
                let half = laid.advance / 2.0;
                put(laid, &|p| {
                    at + way * (p.x * scale - half) + up * (p.y * scale + shift)
                });
            }
        }
        layout => {
            // Lines in a frame of their own: an origin, the way they read
            // and the way up, and the room they have.
            let (origin, way, room) = match layout {
                TextLayout::Frame { corners, .. } => {
                    let c = corners.map(|id| sketch.point_position(id).map(Vec2D::to_glam));
                    let [Some(c0), Some(c1), _, Some(c3)] = c else {
                        return None;
                    };
                    let way = (c1 - c0).normalize_or_zero();
                    let height = (c3 - c0).dot(way.perp());
                    if way == DVec2::ZERO || height.abs() < 1e-9 {
                        return None;
                    }
                    (c0, way, Some((c0.distance(c1), height)))
                }
                _ => (
                    sketch.point_position(block.anchor)?.to_glam(),
                    DVec2::from_angle(block.angle.to_radians()),
                    None,
                ),
            };
            let (wrap, fit) = match layout {
                TextLayout::Frame { wrap, fit, .. } => (*wrap, *fit),
                _ => (false, false),
            };
            let shown = block.shown();
            let text = &*shown;
            let mut rows = lines(
                text,
                glyphs,
                scale,
                spacing,
                room.filter(|_| wrap).map(|r| r.0),
            );
            let tall = |rows: &[Vec<Laid>], scale: f64| {
                let line = glyphs.line * scale * block.line_spacing;
                (glyphs.ascender - glyphs.descender) * scale + line * (rows.len() - 1) as f64
            };
            if let (true, Some((w, h))) = (fit, room) {
                let widest = rows.iter().map(|r| width(r)).fold(0.0, f64::max);
                let shrink = (w / widest).min(h.abs() / tall(&rows, scale)).min(1.0);
                if shrink < 1.0 && shrink > 0.0 {
                    scale *= shrink;
                    spacing *= shrink;
                    rows = lines(
                        text,
                        glyphs,
                        scale,
                        spacing,
                        room.filter(|_| wrap).map(|r| r.0),
                    );
                }
            }
            let up = way.perp() * room.map_or(1.0, |r| r.1.signum());
            let (asc, desc) = (glyphs.ascender * scale, glyphs.descender * scale);
            let line = glyphs.line * scale * block.line_spacing;
            let last = line * (rows.len() - 1) as f64;
            let top_room = room.map_or(0.0, |r| r.1.abs());
            let first = match (block.valign, room.is_some()) {
                (VAlign::Baseline, false) => 0.0,
                (VAlign::Top, _) | (VAlign::Baseline, true) => top_room - asc,
                (VAlign::Bottom, _) => last - desc,
                (VAlign::Middle, _) => (top_room - asc + last - desc) / 2.0,
            };
            let room_w = room.map_or(0.0, |r| r.0);
            for (j, row) in rows.iter().enumerate() {
                let w = width(row);
                let dx = match block.align {
                    Align::Start => 0.0,
                    Align::Middle => (room_w - w) / 2.0,
                    Align::End => room_w - w,
                };
                let base = first - line * j as f64;
                for laid in row {
                    let x = dx + laid.x;
                    put(laid, &|p| {
                        origin + way * (x + p.x * scale) + up * (base + p.y * scale)
                    });
                }
            }
        }
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
    let anchor = sketch.add_geometry(GeometryElement::Point(Point::new(at)));
    let spec = TextSpec {
        layout: TextLayout::Point,
        ..spec.clone()
    };
    place(sketch, anchor, &spec, &[anchor])
}

/// A new text block of `spec` in a frame with corners `a` and `b` across
/// it: four construction lines kept square to the sketch's axes, whose
/// bottom-left corner is the block's point. `spec`'s frame settings are
/// kept. Returns the block's id.
pub fn add_in_frame(
    sketch: &mut Sketch,
    a: Vec2D,
    b: Vec2D,
    spec: &TextSpec,
) -> Result<Uuid, String> {
    use crate::sketch::ConstraintKind;
    let (lo, hi) = (
        Vec2D::new(a.x.min(b.x), a.y.min(b.y)),
        Vec2D::new(a.x.max(b.x), a.y.max(b.y)),
    );
    if hi.x - lo.x < 1e-6 || hi.y - lo.y < 1e-6 {
        return Err("a text frame needs a width and a height".to_string());
    }
    let corners = [lo, Vec2D::new(hi.x, lo.y), hi, Vec2D::new(lo.x, hi.y)]
        .map(|p| sketch.add_geometry(GeometryElement::Point(Point::new(p))));
    let mut made = corners.to_vec();
    for i in 0..4 {
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )));
        sketch.set_construction(line, true);
        sketch.add_constraint(if i % 2 == 0 {
            ConstraintKind::Horizontal { element: line }
        } else {
            ConstraintKind::Vertical { element: line }
        });
        made.push(line);
    }
    let (wrap, fit) = match spec.layout {
        TextLayout::Frame { wrap, fit, .. } => (wrap, fit),
        _ => (false, false),
    };
    let spec = TextSpec {
        layout: TextLayout::Frame { corners, wrap, fit },
        ..spec.clone()
    };
    place(sketch, corners[0], &spec, &made)
}

/// A new text block of `spec` along `curves`, end to end, standing on a new
/// point held on the curve nearest `at` (the curves' start when not given),
/// which carries it along them. `spec`'s path settings are kept. Returns
/// the block's id.
pub fn add_on_path(
    sketch: &mut Sketch,
    curves: &[Uuid],
    at: Option<Vec2D>,
    spec: &TextSpec,
) -> Result<Uuid, String> {
    let (side, offset, fit, reversed) = match spec.layout {
        TextLayout::Path {
            side,
            offset,
            fit,
            reversed,
            ..
        } => (side, offset, fit, reversed),
        _ => (PathSide::Above, 0.0, false, false),
    };
    let (points, closed) =
        path_points(sketch, curves, reversed).ok_or("text runs along curves of the sketch")?;
    let walk = Walk::new(points, closed).ok_or("the curves have no length")?;
    let spot = match at {
        Some(at) => walk.at(walk.nearest(at.to_glam())).0,
        None => walk.at(0.0).0,
    };
    let host = curves
        .iter()
        .copied()
        .min_by(|a, b| {
            let d = |id: &Uuid| {
                sketch
                    .get_geometry(*id)
                    .and_then(|g| {
                        crate::snap::distance_to_element(sketch, g, Vec2D::from_glam(spot))
                    })
                    .unwrap_or(f64::MAX)
            };
            d(a).total_cmp(&d(b))
        })
        .ok_or("text runs along at least one curve")?;
    let anchor = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::from_glam(spot))));
    if let Some(kind) = on_curve(sketch, anchor, host) {
        sketch.add_constraint(kind);
    }
    let spec = TextSpec {
        layout: TextLayout::Path {
            curves: curves.to_vec(),
            side,
            offset,
            fit,
            reversed,
        },
        ..spec.clone()
    };
    place(sketch, anchor, &spec, &[anchor])
}

/// The constraint keeping `point` on `curve`.
fn on_curve(sketch: &Sketch, point: Uuid, curve: Uuid) -> Option<crate::sketch::ConstraintKind> {
    use crate::sketch::ConstraintKind;
    Some(match sketch.get_geometry(curve)? {
        GeometryElement::Point(_) => return None,
        GeometryElement::Line(_) => ConstraintKind::PointOnLine { point, line: curve },
        GeometryElement::Circle(_) | GeometryElement::Arc(_) => ConstraintKind::PointOnCircle {
            point,
            circle: curve,
        },
        GeometryElement::Ellipse(_) => ConstraintKind::PointOnEllipse {
            point,
            ellipse: curve,
        },
        _ => ConstraintKind::PointOnCurve { point, curve },
    })
}

/// Stand a block of `spec` on `anchor` and make its outlines; when it draws
/// nothing, take it and `made` (what was made for it) away again.
fn place(
    sketch: &mut Sketch,
    anchor: Uuid,
    spec: &TextSpec,
    made: &[Uuid],
) -> Result<Uuid, String> {
    let block = match new_block(spec, anchor) {
        Ok(block) => block,
        Err(why) => {
            sketch.remove_geometry_cascade(made);
            return Err(why);
        }
    };
    let id = block.id;
    sketch.texts.push(block);
    refresh(sketch);
    if block_of(sketch, id).is_none_or(|b| b.elements.is_empty()) {
        sketch.texts.retain(|b| b.id != id);
        sketch.remove_geometry_cascade(made);
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
        layout: spec.layout.clone(),
        align: spec.align,
        valign: spec.valign,
        line_spacing: spec.line_spacing,
        values: spec.values.clone(),
        glyphs: read_glyphs(spec, &letters_of(&spec.text))?,
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
    if !(spec.line_spacing.is_finite() && spec.line_spacing > 0.0) {
        return Err("the text's line spacing must be more than 0".to_string());
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
        let glyphs = read_glyphs(spec, &letters_of(&spec.text))?;
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
    block.layout = spec.layout.clone();
    block.align = spec.align;
    block.valign = spec.valign;
    block.line_spacing = spec.line_spacing;
    block.values = spec.values.clone();
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
            layout: TextLayout::Point,
            align: Align::Start,
            valign: VAlign::Baseline,
            line_spacing: 1.0,
            values: Vec::new(),
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

    /// Every outline point of block `id`.
    fn outline_of(sketch: &Sketch, id: Uuid) -> Vec<DVec2> {
        block_of(sketch, id)
            .unwrap()
            .elements
            .iter()
            .filter(|e| matches!(sketch.get_geometry(**e), Some(GeometryElement::Point(_))))
            .filter_map(|e| sketch.point_position(*e).map(Vec2D::to_glam))
            .collect()
    }

    fn bounds(points: &[DVec2]) -> (DVec2, DVec2) {
        points.iter().fold(
            (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        )
    }

    /// In a frame the lines stand where its alignment says, break between
    /// words to keep inside it, shrink to fit it, and follow its corners.
    #[test]
    fn text_in_a_frame_keeps_to_it() {
        let mut sketch = Sketch::new("t");
        let centred = TextSpec {
            align: Align::Middle,
            valign: VAlign::Middle,
            ..spec("HI")
        };
        let id = add_in_frame(
            &mut sketch,
            Vec2D::new(0.0, 0.0),
            Vec2D::new(60.0, 20.0),
            &centred,
        )
        .unwrap();
        let (lo, hi) = bounds(&outline_of(&sketch, id));
        let middle = (lo + hi) / 2.0;
        assert!((middle.x - 30.0).abs() < 0.5, "centred across: {middle:?}");
        assert!(
            (middle.y - 10.0).abs() < 1.5,
            "centred up and down: {middle:?}"
        );

        let wrapped = TextSpec {
            layout: TextLayout::Frame {
                corners: [Uuid::nil(); 4],
                wrap: true,
                fit: false,
            },
            ..spec("HHHH HHHH")
        };
        let id = add_in_frame(
            &mut sketch,
            Vec2D::new(0.0, 50.0),
            Vec2D::new(40.0, 80.0),
            &wrapped,
        )
        .unwrap();
        let (lo, hi) = bounds(&outline_of(&sketch, id));
        assert!(hi.x <= 40.0 + 1e-6, "kept inside the frame: {hi:?}");
        assert!(hi.y - lo.y > 15.0, "two lines: {lo:?} {hi:?}");

        let fitted = TextSpec {
            layout: TextLayout::Frame {
                corners: [Uuid::nil(); 4],
                wrap: false,
                fit: true,
            },
            ..spec("HHHHHHHHHH")
        };
        let id = add_in_frame(
            &mut sketch,
            Vec2D::new(0.0, 100.0),
            Vec2D::new(20.0, 120.0),
            &fitted,
        )
        .unwrap();
        let (lo, hi) = bounds(&outline_of(&sketch, id));
        assert!(
            hi.x <= 20.0 + 1e-6 && lo.x >= -1e-6,
            "shrunk to the frame: {lo:?} {hi:?}"
        );

        // Its corner dragged, the frame's square kept by its constraints,
        // the text follows.
        let corner = match &block_of(&sketch, id).unwrap().layout {
            TextLayout::Frame { corners, .. } => corners[2],
            _ => unreachable!(),
        };
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(corner) {
            p.position = Vec2D::new(40.0, 120.0);
        }
        crate::solver::solve_holding(&mut sketch, &[corner]);
        let (_, wider) = bounds(&outline_of(&sketch, id));
        assert!(wider.x > hi.x + 5.0, "the text grows with its frame");
    }

    /// Along a circle the letters stand outside it, reading clockwise from
    /// their point; reversed they stand inside; dragged, the point slides
    /// round and carries them.
    #[test]
    fn text_along_a_circle_stands_on_it() {
        let mut sketch = Sketch::new("t");
        let centre = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let circle = sketch.add_geometry(GeometryElement::Circle(crate::sketch::Circle::new(
            centre, 30.0,
        )));
        let reach = |sketch: &Sketch, id| {
            outline_of(sketch, id)
                .iter()
                .map(|p| p.length())
                .fold((f64::MAX, f64::MIN), |(lo, hi), r| (lo.min(r), hi.max(r)))
        };
        let around = TextSpec {
            align: Align::Middle,
            ..spec("ROUND")
        };
        let id = add_on_path(&mut sketch, &[circle], Some(Vec2D::new(0.0, 40.0)), &around).unwrap();
        let (inner, outer) = reach(&sketch, id);
        assert!(
            inner > 29.0 && outer < 30.0 + 10.0,
            "outside the circle: {inner} {outer}"
        );
        let (lo, hi) = bounds(&outline_of(&sketch, id));
        assert!(
            lo.y > 20.0 && (lo.x + hi.x).abs() < 1.0,
            "centred on its point at the top"
        );

        let inside = TextSpec {
            layout: TextLayout::Path {
                curves: Vec::new(),
                side: PathSide::Above,
                offset: 0.0,
                fit: false,
                reversed: true,
            },
            ..around.clone()
        };
        let id = add_on_path(
            &mut sketch,
            &[circle],
            Some(Vec2D::new(0.0, -40.0)),
            &inside,
        )
        .unwrap();
        let (_, outer) = reach(&sketch, id);
        // Each letter stands straight on the curve where its middle falls:
        // its baseline's ends lean a hair past it.
        assert!(outer < 30.5, "inside the circle: {outer}");

        let anchor = block_of(&sketch, id).unwrap().anchor;
        if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(anchor) {
            p.position = Vec2D::new(30.0, 0.0);
        }
        crate::solver::solve_holding(&mut sketch, &[anchor]);
        let (lo, _) = bounds(&outline_of(&sketch, id));
        assert!(lo.x > 10.0, "slid round to the right: {lo:?}");
        sketch.remove_geometry_cascade(&[anchor]);
        assert!(sketch.get_geometry(circle).is_some(), "the curve stays");
    }

    /// A string's formulas are found between braces, written to the
    /// decimals asked, and braces doubled are braces.
    #[test]
    fn formulas_in_a_string_show_their_values() {
        let text = "REV {Part.rev} {{x}} {Width * 2:1} mm {";
        assert_eq!(formulas(text), vec!["Part.rev", "Width * 2"]);
        let values = [
            TextValue {
                value: 12.0,
                ..TextValue::default()
            },
            TextValue {
                value: 25.04,
                length: 1,
                angle: 0,
            },
        ];
        assert_eq!(fill(text, &values), "REV 12 {x} 25.0 mm {");
        assert_eq!(fill("{a} {b}", &values[..1]), "12 b", "a value yet to come");
        assert_eq!(number(-0.0001, None), "0");
        assert_eq!(number(2.5, None), "2.5");
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
