//! A saved shape's `.brp` text restated in the format's plainest form, for
//! a reader that takes nothing else:
//!
//! - the content-type line before the version line;
//! - no placements: every shape that stands somewhere other than where its
//!   record puts it is written again where it stands, its curves and
//!   surfaces with it. Placements compose as `SavedShape` reads them, a
//!   composed one applying its factors in the order listed;
//! - every frame right-handed, its normal the cross product of its axes. A
//!   curve or a plane keeps its points and parameters. A cylinder, cone,
//!   sphere or torus on a left-handed frame turns its normal over, which
//!   mirrors its chart in v: the pcurves on it are mirrored to match, and
//!   the faces on it are turned over so they face where they did;
//! - an extrusion's chart starting at its basis curve: the curve is moved
//!   back along the extrusion so every v the source uses is past it, and
//!   the pcurves on it moved up in v as far;
//! - a wire's edges in the order they run, and an edge's starting vertex
//!   before its ending one;
//! - Bezier curves and surfaces as the B-splines they are;
//! - a seam's continuity a word of its own, never glued to the pcurve
//!   number before it (`32CN`);
//! - no cached polygons or triangulations, no vertex representations on
//!   curves and surfaces, and no regularity records, all of which a reader
//!   can work out.
//!
//! A shape met twice at one place is written once, so edges and faces
//! number as `SavedShape` numbers them: one for each record and place.

use std::collections::HashMap;
use std::fmt::Write as _;

use glam::{DAffine3, DMat3, DQuat, DVec3};

use crate::archive::FcstdError;
use crate::document_xml::Placement;

fn bad(reason: impl Into<String>) -> FcstdError {
    FcstdError::Format {
        object: "saved shape".into(),
        reason: reason.into(),
    }
}

/// The line a reader of the format looks for first.
const CONTENT_TYPE: &str = "DBRep_DrawableShape";

/// One topology record as written.
struct Record<'a> {
    kind: &'a str,
    data: Data<'a>,
    flags: &'a str,
    /// Orientation, record number and placement of each child.
    children: Vec<(&'a str, usize, usize)>,
}

/// An edge's curve (`1`), pcurve (`2`) or seam (`3`): its pcurves, the
/// curve or surface it names and that one's placement, and its range.
struct Use<'a> {
    kind: usize,
    pcurves: Vec<usize>,
    continuity: Option<&'a str>,
    geometry: usize,
    at: usize,
    range: [&'a str; 2],
}

enum Data<'a> {
    Vertex {
        tolerance: &'a str,
        point: DVec3,
    },
    Edge {
        flags: [&'a str; 4],
        uses: Vec<Use<'a>>,
    },
    Face {
        natural: &'a str,
        tolerance: &'a str,
        surface: usize,
        at: usize,
    },
    Container,
}

/// A placement as a key: its numbers rounded well below any model's size.
type PlaceKey = [i64; 12];

fn key(place: &DAffine3) -> PlaceKey {
    let mut out = [0; 12];
    for (slot, value) in out.iter_mut().zip(place.to_cols_array()) {
        *slot = (value * 1e9).round() as i64;
    }
    out
}

/// Restate `text`, a saved shape, as the module says, standing where
/// `placement` puts it (the placement of the groups its object is in).
pub fn restate(text: &str, placement: Placement) -> Result<String, FcstdError> {
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    let version = lines
        .next()
        .ok_or_else(|| bad("the shape is empty"))?
        .trim();
    if version.starts_with(CONTENT_TYPE) {
        return Err(bad("the shape is restated already"));
    }
    let words: Vec<&str> = lines.flat_map(str::split_whitespace).collect();
    let mut input = Words::new(&words);

    input.expect("Locations")?;
    let mut locations: Vec<DAffine3> = Vec::new();
    for _ in 0..input.count()? {
        let location = match input.count()? {
            1 => {
                let mut rows = [[0.0; 4]; 3];
                for row in &mut rows {
                    for cell in row.iter_mut() {
                        *cell = input.number()?;
                    }
                }
                let matrix = DMat3::from_cols_array_2d(&[0, 1, 2].map(|c| rows.map(|r| r[c])));
                DAffine3::from_mat3_translation(matrix, DVec3::from_array(rows.map(|r| r[3])))
            }
            2 => {
                let mut composed = DAffine3::IDENTITY;
                loop {
                    let index = input.count()?;
                    if index == 0 {
                        break;
                    }
                    let power = input.integer()?;
                    let base = *locations
                        .get(index - 1)
                        .ok_or_else(|| bad(format!("location {index} is not above it")))?;
                    let step = if power < 0 { base.inverse() } else { base };
                    for _ in 0..power.unsigned_abs().min(64) {
                        composed = step * composed;
                    }
                }
                composed
            }
            other => return Err(bad(format!("location kind {other}"))),
        };
        let det = location.matrix3.determinant();
        if (det - 1.0).abs() > 1e-6 {
            return Err(bad(format!(
                "a placement that scales or mirrors (determinant {det})"
            )));
        }
        locations.push(location);
    }

    input.expect("Curve2ds")?;
    let pcurves = input.records(|w, out| w.curve2(out, Chart::SAME, 0))?;
    input.expect("Curves")?;
    let curves = input.records(|w, out| w.curve3(out, &DAffine3::IDENTITY, 0))?;
    input.expect("Polygon3D")?;
    input.skip_section("PolygonOnTriangulations");
    input.expect("PolygonOnTriangulations")?;
    input.skip_section("Surfaces");
    input.expect("Surfaces")?;
    let surfaces = input.records(|w, out| w.surface(out, &DAffine3::IDENTITY, 0).map(|_| ()))?;
    input.expect("Triangulations")?;
    input.skip_section("TShapes");
    input.expect("TShapes")?;

    let total = input.count()?;
    let mut records = Vec::with_capacity(total.min(words.len()));
    for _ in 0..total {
        records.push(input.record()?);
    }
    let (sign, root) = input.reference()?;
    let root_at = input.count()?;

    let mut out = Restated {
        words: &words,
        locations: &locations,
        records: &records,
        tables: [&pcurves, &curves, &surfaces],
        written: Vec::new(),
        shapes: HashMap::new(),
        copies: Default::default(),
    };
    let root_place = DAffine3::from_rotation_translation(
        DQuat::from_array(placement.rotation.to_array()),
        DVec3::from_array(placement.position.to_array()),
    ) * out.place(root_at)?;
    let root = out.shape(root, root_place, 0)?;

    let count = out.written.len();
    let number = |index: usize| count - index;
    let mut text = format!("{CONTENT_TYPE}\n\n{version}\nLocations 0\n");
    for (name, table) in ["Curve2ds", "Curves", "Surfaces"].iter().zip(&out.copies) {
        if *name == "Surfaces" {
            text.push_str("Polygon3D 0\nPolygonOnTriangulations 0\n");
        }
        let _ = writeln!(text, "{name} {}", table.texts.len());
        for record in &table.texts {
            text.push_str(record);
        }
    }
    let _ = writeln!(text, "Triangulations 0\n\nTShapes {count}");
    for shape in &out.written {
        text.push_str(&shape.text);
        for &(sign, child) in &shape.children {
            let sign = turned(sign, out.written[child].turned);
            let _ = write!(text, "{sign}{} 0 ", number(child));
        }
        text.push_str("*\n");
    }
    let sign = turned(sign, out.written[root].turned);
    let _ = writeln!(text, "\n{sign}{} 0", number(root));
    Ok(text)
}

/// An orientation, the other way when `turn`.
fn turned(sign: &str, turn: bool) -> &str {
    match (sign, turn) {
        ("+", true) => "-",
        ("-", true) => "+",
        (sign, _) => sign,
    }
}

/// A record written: its text up to its children, its children by their
/// place among the records written, and whether it is a face turned over.
struct Written<'a> {
    text: String,
    children: Vec<(&'a str, usize)>,
    turned: bool,
}

/// One table written: each record's text, and the record a source record
/// became at a place (a pcurve: on a chart), with how a surface's chart
/// stands to the source's.
#[derive(Default)]
struct Copies {
    texts: Vec<String>,
    seen: HashMap<(usize, PlaceKey), (usize, Chart)>,
}

/// How a surface's chart restated stands to the source's: the same u, and
/// v times `scale` plus `offset`.
#[derive(Debug, Clone, Copy)]
struct Chart {
    scale: f64,
    offset: f64,
}

impl Chart {
    const SAME: Self = Self {
        scale: 1.0,
        offset: 0.0,
    };

    fn v(self, v: f64) -> f64 {
        self.scale.mul_add(v, self.offset)
    }

    /// The chart as a plane's transform, a key for the pcurves on it.
    fn affine(self) -> DAffine3 {
        DAffine3::from_translation(DVec3::new(0.0, self.offset, 0.0))
            * DAffine3::from_scale(DVec3::new(1.0, self.scale, 1.0))
    }
}

/// How far back an extrusion's basis curve is moved: past any v a part
/// uses.
const EXTRUSION_REACH: f64 = 1e5;

/// Records written so far, and the pcurves, curves and surfaces they name.
struct Restated<'a> {
    words: &'a [&'a str],
    locations: &'a [DAffine3],
    records: &'a [Record<'a>],
    /// The pcurve, curve and surface records as spans of words.
    tables: [&'a [(usize, usize)]; 3],
    written: Vec<Written<'a>>,
    shapes: HashMap<(usize, PlaceKey), usize>,
    copies: [Copies; 3],
}

const PCURVES: usize = 0;
const CURVES: usize = 1;
const SURFACES: usize = 2;

impl<'a> Restated<'a> {
    fn place(&self, index: usize) -> Result<DAffine3, FcstdError> {
        match index {
            0 => Ok(DAffine3::IDENTITY),
            n => self
                .locations
                .get(n - 1)
                .copied()
                .ok_or_else(|| bad(format!("no location {n}"))),
        }
    }

    /// Write record `number` where `at` puts it, once for each place:
    /// its index among the records written.
    fn shape(&mut self, number: usize, at: DAffine3, depth: usize) -> Result<usize, FcstdError> {
        if depth > 64 {
            return Err(bad("shapes nest too deep"));
        }
        if let Some(&index) = self.shapes.get(&(number, key(&at))) {
            return Ok(index);
        }
        let records = self.records;
        let record = number
            .checked_sub(1)
            .and_then(|n| records.len().checked_sub(n + 1))
            .and_then(|i| records.get(i))
            .ok_or_else(|| bad(format!("no shape record {number}")))?;
        let mut children = Vec::with_capacity(record.children.len());
        for &(sign, child, child_at) in &record.children {
            let place = at * self.place(child_at)?;
            children.push((sign, self.shape(child, place, depth + 1)?));
        }
        match record.kind {
            "Wi" => self.chain(&mut children),
            // The vertex an edge starts at first, then the one it ends at.
            "Ed" => children.sort_by_key(|&(sign, _)| sign != "+"),
            _ => {}
        }
        let mut text = format!("{}\n", record.kind);
        let mut turned = false;
        match &record.data {
            Data::Vertex { tolerance, point } => {
                let p = at.transform_point3(*point);
                let _ = writeln!(text, "{tolerance}\n{} {} {}\n0 0", p.x, p.y, p.z);
            }
            Data::Edge { flags, uses } => {
                let _ = writeln!(text, "{}", flags.join(" "));
                for using in uses {
                    let placed = at * self.place(using.at)?;
                    let (geometry, chart) = if using.kind == 1 {
                        self.copy(CURVES, using.geometry, placed)?
                    } else {
                        self.copy(SURFACES, using.geometry, placed)?
                    };
                    let _ = write!(text, "{}", using.kind);
                    for &pcurve in &using.pcurves {
                        let _ = write!(text, " {}", self.copy(PCURVES, pcurve, chart.affine())?.0);
                    }
                    if let Some(continuity) = using.continuity {
                        let _ = write!(text, " {continuity}");
                    }
                    let _ = writeln!(text, " {geometry} 0 {} {}", using.range[0], using.range[1]);
                }
                text.push_str("0\n");
            }
            Data::Face {
                natural,
                tolerance,
                surface,
                at: surface_at,
            } => {
                let placed = at * self.place(*surface_at)?;
                let (surface, chart) = self.copy(SURFACES, *surface, placed)?;
                turned = chart.scale < 0.0;
                let _ = writeln!(text, "{natural} {tolerance} {surface} 0");
            }
            Data::Container => {}
        }
        let _ = writeln!(text, "{}", record.flags);
        self.written.push(Written {
            text,
            children,
            turned,
        });
        let index = self.written.len() - 1;
        self.shapes.insert((number, key(&at)), index);
        Ok(index)
    }

    /// Put a wire's edges in the order they run, each starting at the
    /// vertex the one before it ends at, as the format leaves free and a
    /// reader may not. A wire whose edges do not chain is left as written.
    fn chain(&self, edges: &mut [(&'a str, usize)]) {
        let ends = |&(sign, edge): &(&str, usize)| {
            let vertices = &self.written[edge].children;
            let at = |s: &str| vertices.iter().find(|(v, _)| *v == s).map(|(_, i)| *i);
            let (start, end) = (at("+")?, at("-")?);
            Some(if sign == "-" {
                (end, start)
            } else {
                (start, end)
            })
        };
        let Some(runs) = edges.iter().map(ends).collect::<Option<Vec<_>>>() else {
            return;
        };
        // A depth-first search, closed edges tried first at each vertex (a
        // seam met at a circle's vertex must wait for the circle), within a
        // budget a wire of any sense meets.
        let mut order = vec![0];
        let mut used = vec![false; edges.len()];
        let mut tried = vec![0];
        if let Some(first) = used.first_mut() {
            *first = true;
        }
        let candidates = |end: usize, used: &[bool]| -> Vec<usize> {
            let mut next: Vec<usize> = (0..runs.len())
                .filter(|&i| !used[i] && runs[i].0 == end)
                .collect();
            next.sort_by_key(|&i| runs[i].0 != runs[i].1);
            next
        };
        let mut budget = 10_000;
        while order.len() < edges.len() && !order.is_empty() && budget > 0 {
            budget -= 1;
            let last = order[order.len() - 1];
            let options = candidates(runs[last].1, &used);
            let at = tried[tried.len() - 1];
            if let Some(&next) = options.get(at) {
                *tried.last_mut().expect("a step") += 1;
                used[next] = true;
                order.push(next);
                tried.push(0);
            } else {
                used[last] = order.len() == 1;
                order.pop();
                tried.pop();
            }
        }
        if order.len() == edges.len() && runs[order[order.len() - 1]].1 == runs[0].0 {
            let chained: Vec<_> = order.iter().map(|&i| edges[i]).collect();
            edges.copy_from_slice(&chained);
        }
    }

    /// The number of curve or surface `index` written where `at` puts it
    /// (a pcurve: its v as the chart `at` stands for has it), and for a
    /// surface how its chart's v stands to the source's.
    fn copy(
        &mut self,
        table: usize,
        index: usize,
        at: DAffine3,
    ) -> Result<(usize, Chart), FcstdError> {
        let copies = &mut self.copies[table];
        if let Some(&found) = copies.seen.get(&(index, key(&at))) {
            return Ok(found);
        }
        let &(from, to) = self.tables[table]
            .get(index.wrapping_sub(1))
            .ok_or_else(|| bad(format!("no geometry {index}")))?;
        let mut text = String::new();
        let mut record = Words::new(&self.words[from..to]);
        let chart = match table {
            PCURVES => {
                let chart = Chart {
                    scale: at.matrix3.y_axis.y,
                    offset: at.translation.y,
                };
                record.curve2(&mut text, chart, 0)?;
                Chart::SAME
            }
            CURVES => {
                record.curve3(&mut text, &at, 0)?;
                Chart::SAME
            }
            _ => record.surface(&mut text, &at, 0)?,
        };
        copies.texts.push(text);
        let found = (copies.texts.len(), chart);
        copies.seen.insert((index, key(&at)), found);
        Ok(found)
    }
}

/// The words of the text, read in order. A pcurve number glued to the
/// continuity after it reads as two words.
struct Words<'a> {
    words: &'a [&'a str],
    at: usize,
    /// The continuity left over from a glued word, read next.
    pending: Option<&'a str>,
}

impl<'a> Words<'a> {
    fn new(words: &'a [&'a str]) -> Self {
        Self {
            words,
            at: 0,
            pending: None,
        }
    }

    fn word(&mut self) -> Result<&'a str, FcstdError> {
        if let Some(word) = self.pending.take() {
            return Ok(word);
        }
        let word = *self
            .words
            .get(self.at)
            .ok_or_else(|| bad("the shape ends in the middle of a record"))?;
        self.at += 1;
        let digits = word.bytes().take_while(u8::is_ascii_digit).count();
        let rest = &word[digits..];
        if digits > 0 && ["C0", "C1", "C2", "C3", "CN", "G1", "G2"].contains(&rest) {
            self.pending = Some(rest);
            return Ok(&word[..digits]);
        }
        Ok(word)
    }

    fn peek(&self) -> Option<&'a str> {
        self.pending.or_else(|| self.words.get(self.at).copied())
    }

    /// A child or the root: its orientation and record number.
    fn reference(&mut self) -> Result<(&'a str, usize), FcstdError> {
        let word = self.word()?;
        let number = word
            .get(1..)
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| bad(format!("{word:?} names no shape")))?;
        Ok((&word[..1], number))
    }

    /// One topology record.
    fn record(&mut self) -> Result<Record<'a>, FcstdError> {
        let kind = self.word()?;
        let data = match kind {
            "Ve" => {
                let tolerance = self.word()?;
                let point = DVec3::new(self.number()?, self.number()?, self.number()?);
                loop {
                    let (parameter, which) = (self.word()?, self.word()?);
                    if parameter == "0" && which == "0" {
                        break;
                    }
                    match which {
                        "1" => self.skip(2)?,
                        "2" | "3" => self.skip(3)?,
                        other => return Err(bad(format!("vertex representation {other}"))),
                    }
                }
                Data::Vertex { tolerance, point }
            }
            "Ed" => {
                let flags = [self.word()?, self.word()?, self.word()?, self.word()?];
                let mut uses = Vec::new();
                loop {
                    let kind = self.count()?;
                    let (pcurves, continuity) = match kind {
                        0 => break,
                        1 => (Vec::new(), None),
                        2 => (vec![self.count()?], None),
                        3 => (vec![self.count()?, self.count()?], Some(self.word()?)),
                        4 => {
                            self.skip(5)?;
                            continue;
                        }
                        5..=7 => {
                            self.skip(kind - 3)?;
                            continue;
                        }
                        other => return Err(bad(format!("edge representation {other}"))),
                    };
                    uses.push(Use {
                        kind,
                        pcurves,
                        continuity,
                        geometry: self.count()?,
                        at: self.count()?,
                        range: [self.word()?, self.word()?],
                    });
                }
                Data::Edge { flags, uses }
            }
            "Fa" => {
                let (natural, tolerance) = (self.word()?, self.word()?);
                let (surface, at) = (self.count()?, self.count()?);
                if self.peek() == Some("2") {
                    self.skip(2)?;
                }
                Data::Face {
                    natural,
                    tolerance,
                    surface,
                    at,
                }
            }
            "Wi" | "Sh" | "So" | "CS" | "Co" => Data::Container,
            other => return Err(bad(format!("shape record {other}"))),
        };
        let flags = self.word()?;
        let mut children = Vec::new();
        while self.peek() != Some("*") {
            let (sign, number) = self.reference()?;
            children.push((sign, number, self.count()?));
        }
        self.word()?;
        Ok(Record {
            kind,
            data,
            flags,
            children,
        })
    }

    fn expect(&mut self, name: &str) -> Result<(), FcstdError> {
        match self.word()? {
            word if word == name => Ok(()),
            word => Err(bad(format!("expected the {name} table, found {word:?}"))),
        }
    }

    fn number(&mut self) -> Result<f64, FcstdError> {
        let word = self.word()?;
        word.parse()
            .map_err(|_| bad(format!("{word:?} is not a number")))
    }

    fn integer(&mut self) -> Result<i64, FcstdError> {
        let word = self.word()?;
        word.parse()
            .map_err(|_| bad(format!("{word:?} is not a whole number")))
    }

    fn count(&mut self) -> Result<usize, FcstdError> {
        let value = self.integer()?;
        usize::try_from(value)
            .ok()
            .filter(|&n| n <= 10_000_000)
            .ok_or_else(|| bad(format!("{value} is not a count")))
    }

    fn skip(&mut self, n: usize) -> Result<(), FcstdError> {
        for _ in 0..n {
            self.word()?;
        }
        Ok(())
    }

    /// Step over everything up to the table `name`.
    fn skip_section(&mut self, name: &str) {
        while self.words.get(self.at).is_some_and(|w| *w != name) {
            self.at += 1;
        }
    }

    /// A table's records, each as the span of words it takes, checked by
    /// reading it with `read`.
    fn records(
        &mut self,
        read: impl Fn(&mut Words<'a>, &mut String) -> Result<(), FcstdError>,
    ) -> Result<Vec<(usize, usize)>, FcstdError> {
        let count = self.count()?;
        let mut spans = Vec::with_capacity(count.min(self.words.len()));
        let mut scratch = String::new();
        for _ in 0..count {
            let from = self.at;
            read(self, &mut scratch)?;
            scratch.clear();
            spans.push((from, self.at));
        }
        Ok(spans)
    }

    fn numbers(&mut self, out: &mut String, n: usize) -> Result<(), FcstdError> {
        for _ in 0..n {
            let _ = write!(out, " {}", self.number()?);
        }
        Ok(())
    }

    fn vector(&mut self) -> Result<DVec3, FcstdError> {
        Ok(DVec3::new(self.number()?, self.number()?, self.number()?))
    }

    fn point(&mut self, out: &mut String, t: &DAffine3) -> Result<(), FcstdError> {
        write_vector(out, t.transform_point3(self.vector()?));
        Ok(())
    }

    fn direction(&mut self, out: &mut String, t: &DAffine3) -> Result<(), FcstdError> {
        write_vector(out, t.transform_vector3(self.vector()?));
        Ok(())
    }

    /// A seat (origin, normal and two axes) where `t` puts it, and whether
    /// its axes were left-handed: then the normal written is their cross
    /// product, the seat right-handed.
    fn seat(&mut self, out: &mut String, t: &DAffine3) -> Result<bool, FcstdError> {
        let origin = t.transform_point3(self.vector()?);
        let normal = t.transform_vector3(self.vector()?);
        let x = t.transform_vector3(self.vector()?);
        let y = t.transform_vector3(self.vector()?);
        let left = x.cross(y).dot(normal) < 0.0;
        write_vector(out, origin);
        write_vector(out, if left { x.cross(y).normalize() } else { normal });
        write_vector(out, x);
        write_vector(out, y);
        Ok(left)
    }

    /// Poles, each a point and its weight when rational.
    fn poles(
        &mut self,
        out: &mut String,
        t: &DAffine3,
        n: usize,
        rational: bool,
    ) -> Result<(), FcstdError> {
        for _ in 0..n {
            self.point(out, t)?;
            if rational {
                self.numbers(out, 1)?;
            }
        }
        Ok(())
    }

    fn curve3(&mut self, out: &mut String, t: &DAffine3, depth: usize) -> Result<(), FcstdError> {
        if depth > 8 {
            return Err(bad("curves nest too deep"));
        }
        let kind = self.count()?;
        match kind {
            1 => {
                out.push('1');
                self.point(out, t)?;
                self.direction(out, t)?;
            }
            2..=5 => {
                let _ = write!(out, "{kind}");
                self.seat(out, t)?;
                self.numbers(out, if kind == 3 || kind == 5 { 2 } else { 1 })?;
            }
            6 => {
                let rational = self.count()?;
                let degree = self.count()?;
                let _ = write!(out, "7 {rational} 0 {degree} {} 2", degree + 1);
                self.poles(out, t, degree + 1, rational != 0)?;
                let _ = write!(out, " 0 {} 1 {}", degree + 1, degree + 1);
            }
            7 => {
                let rational = self.count()?;
                let head = [rational, self.count()?, self.count()?];
                let (poles, knots) = (self.count()?, self.count()?);
                let _ = write!(out, "7 {} {} {} {poles} {knots}", head[0], head[1], head[2]);
                self.poles(out, t, poles, rational != 0)?;
                self.numbers(out, knots * 2)?;
            }
            8 => {
                out.push('8');
                self.numbers(out, 2)?;
                out.push('\n');
                return self.curve3(out, t, depth + 1);
            }
            9 => {
                out.push('9');
                self.numbers(out, 1)?;
                self.direction(out, t)?;
                out.push('\n');
                return self.curve3(out, t, depth + 1);
            }
            other => return Err(bad(format!("curve kind {other}"))),
        }
        out.push('\n');
        Ok(())
    }

    /// A pcurve on `chart`.
    fn curve2(&mut self, out: &mut String, chart: Chart, depth: usize) -> Result<(), FcstdError> {
        if depth > 8 {
            return Err(bad("curves nest too deep"));
        }
        let even = (chart.scale.abs() - 1.0).abs() < 1e-12;
        let point = |w: &mut Self| -> Result<(f64, f64), FcstdError> {
            Ok((w.number()?, chart.v(w.number()?)))
        };
        let direction = |w: &mut Self| -> Result<(f64, f64), FcstdError> {
            Ok((w.number()?, chart.scale * w.number()?))
        };
        let kind = self.count()?;
        match kind {
            1 => {
                let ((u, v), (du, dv)) = (point(self)?, direction(self)?);
                if even {
                    let _ = write!(out, "1 {u} {v} {du} {dv}");
                } else {
                    // A line its direction stretched is no longer at unit
                    // speed: a straight B-spline over a reach no edge
                    // leaves keeps its parameter.
                    let reach = 1e6;
                    let _ = write!(
                        out,
                        "7 0 0 1 2 2 {} {} {} {} {} 2 {reach} 2",
                        u - reach * du,
                        v - reach * dv,
                        u + reach * du,
                        v + reach * dv,
                        -reach
                    );
                }
            }
            2..=5 if even => {
                let (u, v) = point(self)?;
                let _ = write!(out, "{kind} {u} {v}");
                for _ in 0..2 {
                    let (du, dv) = direction(self)?;
                    let _ = write!(out, " {du} {dv}");
                }
                self.numbers(out, if kind == 3 || kind == 5 { 2 } else { 1 })?;
            }
            6 | 7 => {
                let rational = self.count()?;
                let (head, poles, knots) = if kind == 6 {
                    let degree = self.count()?;
                    (format!("{rational} 0 {degree}"), degree + 1, None)
                } else {
                    let head = format!("{rational} {} {}", self.count()?, self.count()?);
                    (head, self.count()?, Some(self.count()?))
                };
                let _ = write!(out, "7 {head} {poles} {}", knots.unwrap_or(2));
                for _ in 0..poles {
                    let (u, v) = point(self)?;
                    let _ = write!(out, " {u} {v}");
                    if rational != 0 {
                        self.numbers(out, 1)?;
                    }
                }
                match knots {
                    Some(knots) => self.numbers(out, knots * 2)?,
                    None => {
                        let _ = write!(out, " 0 {poles} 1 {poles}");
                    }
                }
            }
            8 => {
                out.push('8');
                self.numbers(out, 2)?;
                out.push('\n');
                return self.curve2(out, chart, depth + 1);
            }
            9 if even => {
                let _ = writeln!(out, "9 {}", chart.scale * self.number()?);
                return self.curve2(out, chart, depth + 1);
            }
            2..=5 | 9 => return Err(bad("a conic or offset pcurve on a cone")),
            other => return Err(bad(format!("pcurve kind {other}"))),
        }
        out.push('\n');
        Ok(())
    }

    /// A surface where `t` puts it, and how its chart stands to the
    /// source's.
    fn surface(
        &mut self,
        out: &mut String,
        t: &DAffine3,
        depth: usize,
    ) -> Result<Chart, FcstdError> {
        if depth > 8 {
            return Err(bad("surfaces nest too deep"));
        }
        let kind = self.count()?;
        let scale = match kind {
            1 => {
                out.push('1');
                self.seat(out, t)?;
                1.0
            }
            2..=5 => {
                let _ = write!(out, "{kind}");
                let sense = if self.seat(out, t)? { -1.0 } else { 1.0 };
                self.numbers(out, 1)?;
                match kind {
                    // A cone's half angle opens the other way about a turned
                    // normal. Its chart stays measured along its slant, as
                    // the reader takes it.
                    3 => {
                        let angle = self.number()?;
                        let _ = write!(out, " {}", sense * angle);
                        sense
                    }
                    5 => {
                        self.numbers(out, 1)?;
                        sense
                    }
                    _ => sense,
                }
            }
            6 => {
                let along = t.transform_vector3(self.vector()?);
                out.push('6');
                write_vector(out, along);
                out.push('\n');
                let back = DAffine3::from_translation(-EXTRUSION_REACH * along.normalize_or_zero());
                self.curve3(out, &(back * *t), depth + 1)?;
                return Ok(Chart {
                    scale: 1.0,
                    offset: EXTRUSION_REACH,
                });
            }
            7 => {
                out.push('7');
                self.point(out, t)?;
                self.direction(out, t)?;
                out.push('\n');
                self.curve3(out, t, depth + 1)?;
                return Ok(Chart::SAME);
            }
            8 => {
                let rational = [self.count()?, self.count()?];
                let (u, v) = (self.count()?, self.count()?);
                let _ = write!(
                    out,
                    "9 {} {} 0 0 {u} {v} {} {} 2 2",
                    rational[0],
                    rational[1],
                    u + 1,
                    v + 1
                );
                self.poles(out, t, (u + 1) * (v + 1), rational != [0, 0])?;
                let _ = write!(out, " 0 {0} 1 {0} 0 {1} 1 {1}", u + 1, v + 1);
                1.0
            }
            9 => {
                let mut head = [0; 10];
                for slot in &mut head {
                    *slot = self.count()?;
                }
                out.push('9');
                for value in head {
                    let _ = write!(out, " {value}");
                }
                let cells = head[6]
                    .checked_mul(head[7])
                    .ok_or_else(|| bad("a pole grid too large"))?;
                self.poles(out, t, cells, head[0] != 0 || head[1] != 0)?;
                self.numbers(out, (head[8] + head[9]) * 2)?;
                1.0
            }
            10 => {
                let bounds = [
                    self.number()?,
                    self.number()?,
                    self.number()?,
                    self.number()?,
                ];
                let mut basis = String::new();
                let chart = self.surface(&mut basis, t, depth + 1)?;
                let (low, high) = (chart.v(bounds[2]), chart.v(bounds[3]));
                let _ = writeln!(
                    out,
                    "10 {} {} {} {}",
                    bounds[0],
                    bounds[1],
                    low.min(high),
                    low.max(high)
                );
                out.push_str(&basis);
                return Ok(chart);
            }
            11 => {
                let distance = self.number()?;
                let mut basis = String::new();
                let chart = self.surface(&mut basis, t, depth + 1)?;
                // An offset along a turned normal runs the other way.
                let _ = writeln!(out, "11 {}", chart.scale.signum() * distance);
                out.push_str(&basis);
                return Ok(chart);
            }
            other => return Err(bad(format!("surface kind {other}"))),
        };
        out.push('\n');
        Ok(Chart { scale, offset: 0.0 })
    }
}

fn write_vector(out: &mut String, v: DVec3) {
    let _ = write!(out, " {} {} {}", v.x, v.y, v.z);
}
