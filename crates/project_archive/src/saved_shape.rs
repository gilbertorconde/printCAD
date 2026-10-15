//! A saved shape's topology, read from its `.brp` text: its edges and
//! faces numbered as features name them (`Edge1`, `Face1`, … from one),
//! each with where it lies, and the box its edges reach.
//!
//! Numbering follows the text's own structure: a depth-first walk from the
//! root, children in the order written, each edge or face counted the
//! first time it is met. Two meetings are one element when they share a
//! record and land in the same place. Geometry is what the records state
//! (vertices, lines, circles, ellipses and planes); an edge on another
//! kind of curve stands at its vertices' middle, a face on another kind of
//! surface has no mark.

use glam::{DAffine3, DMat3, DVec3};

use crate::archive::FcstdError;

/// A point on an edge and its direction there, or a point on a flat face
/// and its outward normal.
pub type Mark = ([f64; 3], [f64; 3]);

#[derive(Debug, Clone, Default)]
pub struct SavedShape {
    pub edges: Vec<Option<Mark>>,
    pub faces: Vec<Option<Mark>>,
    /// The box the vertices and edges reach, `(low, high)`.
    pub bounds: Option<([f64; 3], [f64; 3])>,
}

fn bad(reason: impl Into<String>) -> FcstdError {
    FcstdError::Format {
        object: "saved shape".into(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, Copy)]
enum Curve {
    Line {
        p: DVec3,
        d: DVec3,
    },
    Conic {
        c: DVec3,
        x: DVec3,
        y: DVec3,
        rx: f64,
        ry: f64,
    },
    Other,
}

impl Curve {
    fn at(&self, t: f64) -> Option<(DVec3, DVec3)> {
        match *self {
            Curve::Line { p, d } => Some((p + d * t, d)),
            Curve::Conic { c, x, y, rx, ry } => Some((
                c + x * (rx * t.cos()) + y * (ry * t.sin()),
                x * (-rx * t.sin()) + y * (ry * t.cos()),
            )),
            Curve::Other => None,
        }
    }
}

/// Numbers in a section, read record by record.
struct Tokens<'a> {
    words: std::vec::IntoIter<&'a str>,
}

impl Tokens<'_> {
    fn number(&mut self) -> Result<f64, FcstdError> {
        let word = self
            .words
            .next()
            .ok_or_else(|| bad("a record ends early"))?;
        word.parse()
            .map_err(|_| bad(format!("{word:?} is not a number")))
    }

    fn count(&mut self) -> Result<usize, FcstdError> {
        let value = self.number()?;
        if value < 0.0 || value.fract() != 0.0 || value > 1e7 {
            return Err(bad(format!("{value} is not a count")));
        }
        Ok(value as usize)
    }

    fn vector(&mut self) -> Result<DVec3, FcstdError> {
        Ok(DVec3::new(self.number()?, self.number()?, self.number()?))
    }

    fn skip(&mut self, n: usize) -> Result<(), FcstdError> {
        for _ in 0..n {
            self.number()?;
        }
        Ok(())
    }

    fn curve(&mut self, depth: usize) -> Result<Curve, FcstdError> {
        if depth > 8 {
            return Err(bad("curves nest too deep"));
        }
        Ok(match self.count()? {
            1 => Curve::Line {
                p: self.vector()?,
                d: self.vector()?,
            },
            kind @ 2..=5 => {
                let c = self.vector()?;
                let _normal = self.vector()?;
                let x = self.vector()?;
                let y = self.vector()?;
                let rx = self.number()?;
                let ry = if kind == 2 || kind == 4 {
                    rx
                } else {
                    self.number()?
                };
                if kind <= 3 {
                    Curve::Conic { c, x, y, rx, ry }
                } else {
                    Curve::Other
                }
            }
            6 => {
                let rational = self.count()?;
                let degree = self.count()?;
                self.skip((degree + 1) * (3 + rational))?;
                Curve::Other
            }
            7 => {
                let rational = self.count()?;
                let _periodic = self.count()?;
                let _degree = self.count()?;
                let poles = self.count()?;
                let knots = self.count()?;
                self.skip(poles * (3 + rational) + knots * 2)?;
                Curve::Other
            }
            8 => {
                self.skip(2)?;
                self.curve(depth + 1)?
            }
            9 => {
                self.skip(4)?;
                self.curve(depth + 1)?;
                Curve::Other
            }
            other => return Err(bad(format!("curve kind {other}"))),
        })
    }

    /// A surface: a plane's point and normal, `None` for any other kind.
    fn surface(&mut self, depth: usize) -> Result<Option<(DVec3, DVec3)>, FcstdError> {
        if depth > 8 {
            return Err(bad("surfaces nest too deep"));
        }
        Ok(match self.count()? {
            1 => {
                let plane = (self.vector()?, self.vector()?);
                self.skip(6)?;
                Some(plane)
            }
            kind @ 2..=5 => {
                self.skip(12 + if kind == 3 || kind == 5 { 2 } else { 1 })?;
                None
            }
            6 => {
                self.skip(3)?;
                self.curve(depth + 1)?;
                None
            }
            7 => {
                self.skip(6)?;
                self.curve(depth + 1)?;
                None
            }
            8 => {
                let rational = self.count()?.max(self.count()?);
                let u = self.count()?;
                let v = self.count()?;
                self.skip((u + 1) * (v + 1) * (3 + rational))?;
                None
            }
            9 => {
                let rational = self.count()?.max(self.count()?);
                self.skip(4)?;
                let (u, v, uk, vk) = (self.count()?, self.count()?, self.count()?, self.count()?);
                self.skip(u * v * (3 + rational) + (uk + vk) * 2)?;
                None
            }
            10 => {
                self.skip(4)?;
                self.surface(depth + 1)?
            }
            11 => {
                self.skip(1)?;
                self.surface(depth + 1)?;
                None
            }
            other => return Err(bad(format!("surface kind {other}"))),
        })
    }
}

/// One topology record: its kind, the lines before its flags, and its
/// children as (reversed, record number, location number).
struct Record<'a> {
    kind: &'a str,
    data: Vec<&'a str>,
    children: Vec<(bool, usize, usize)>,
}

impl SavedShape {
    pub fn parse(text: &str) -> Result<Self, FcstdError> {
        let lines: Vec<&str> = text.lines().collect();
        let header = |name: &str| {
            lines.iter().position(|l| {
                let mut words = l.split_whitespace();
                words.next() == Some(name)
                    && words.next().is_some_and(|n| n.parse::<usize>().is_ok())
            })
        };
        let count_of = |at: usize| -> usize {
            lines[at]
                .split_whitespace()
                .nth(1)
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };
        let (Some(loc_at), Some(curves_at), Some(surfaces_at), Some(shapes_at)) = (
            header("Locations"),
            header("Curves"),
            header("Surfaces"),
            header("TShapes"),
        ) else {
            return Err(bad("a section is missing"));
        };
        let section = |from: usize| Tokens {
            words: lines[from + 1..]
                .iter()
                .flat_map(|l| l.split_whitespace())
                .collect::<Vec<_>>()
                .into_iter(),
        };

        let mut locations: Vec<DAffine3> = Vec::new();
        let mut tokens = section(loc_at);
        for _ in 0..count_of(loc_at) {
            match tokens.count()? {
                1 => {
                    let mut rows = [[0.0; 4]; 3];
                    for row in &mut rows {
                        for cell in row.iter_mut() {
                            *cell = tokens.number()?;
                        }
                    }
                    let matrix = DMat3::from_cols_array_2d(&[0, 1, 2].map(|c| rows.map(|r| r[c])));
                    let translation = DVec3::from_array(rows.map(|r| r[3]));
                    locations.push(DAffine3::from_mat3_translation(matrix, translation));
                }
                2 => {
                    // Each factor applies after those listed before it.
                    let mut composed = DAffine3::IDENTITY;
                    loop {
                        let index = tokens.count()?;
                        if index == 0 {
                            break;
                        }
                        let power = tokens.number()?;
                        let base = *locations
                            .get(index - 1)
                            .ok_or_else(|| bad(format!("location {index} is not above it")))?;
                        let step = if power < 0.0 { base.inverse() } else { base };
                        for _ in 0..(power.abs() as usize).min(64) {
                            composed = step * composed;
                        }
                    }
                    locations.push(composed);
                }
                other => return Err(bad(format!("location kind {other}"))),
            }
        }
        let location = |index: usize| -> Result<DAffine3, FcstdError> {
            if index == 0 {
                return Ok(DAffine3::IDENTITY);
            }
            locations
                .get(index - 1)
                .copied()
                .ok_or_else(|| bad(format!("no location {index}")))
        };

        let mut tokens = section(curves_at);
        let curves = (0..count_of(curves_at))
            .map(|_| tokens.curve(0))
            .collect::<Result<Vec<_>, _>>()?;
        let mut tokens = section(surfaces_at);
        let surfaces = (0..count_of(surfaces_at))
            .map(|_| tokens.surface(0))
            .collect::<Result<Vec<_>, _>>()?;

        let mut records: Vec<Record> = Vec::new();
        let mut rest = lines[shapes_at + 1..].iter().copied();
        for _ in 0..count_of(shapes_at) {
            let kind = rest
                .by_ref()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .ok_or_else(|| bad("a shape record is missing"))?;
            let mut data = Vec::new();
            for line in rest.by_ref() {
                let line = line.trim();
                if line.len() == 7 && line.bytes().all(|b| b == b'0' || b == b'1') {
                    break;
                }
                data.push(line);
            }
            let mut words = Vec::new();
            for line in rest.by_ref() {
                words.extend(line.split_whitespace());
                if words.last() == Some(&"*") {
                    break;
                }
            }
            words.pop();
            let children = words
                .chunks(2)
                .map(|pair| {
                    let (sign, number) = pair[0].split_at(1);
                    let number = number.parse().map_err(|_| bad("a child is not a number"))?;
                    let location = pair
                        .get(1)
                        .and_then(|l| l.parse().ok())
                        .ok_or_else(|| bad("a child has no location"))?;
                    Ok((sign == "-", number, location))
                })
                .collect::<Result<Vec<_>, FcstdError>>()?;
            records.push(Record {
                kind,
                data,
                children,
            });
        }
        // Records count backwards: the last one written is number one.
        let total = records.len();
        let record = |number: usize| -> Result<&Record, FcstdError> {
            number
                .checked_sub(1)
                .filter(|&n| n < total)
                .map(|n| &records[total - 1 - n])
                .ok_or_else(|| bad(format!("no shape record {number}")))
        };
        let root = lines
            .iter()
            .rev()
            .map(|l| l.split_whitespace().collect::<Vec<_>>())
            .find(|w| !w.is_empty())
            .ok_or_else(|| bad("no root"))?;
        let root_number = root[0]
            .get(1..)
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| bad("the root is not a shape"))?;
        let root_location = root.get(1).and_then(|l| l.parse().ok()).unwrap_or(0);

        let mut shape = SavedShape::default();
        let mut seen_edges: Vec<(usize, DAffine3)> = Vec::new();
        let mut seen_faces: Vec<(usize, DAffine3)> = Vec::new();
        let mut points: Vec<DVec3> = Vec::new();
        let mut stack = vec![(
            root_number,
            location(root_location)?,
            root[0].starts_with('-'),
            0,
        )];
        while let Some((number, place, reversed, depth)) = stack.pop() {
            if depth > 64 {
                return Err(bad("shapes nest too deep"));
            }
            let here = record(number)?;
            let same = |seen: &[(usize, DAffine3)]| {
                seen.iter()
                    .any(|(n, p)| *n == number && p.abs_diff_eq(place, 1e-9))
            };
            match here.kind {
                "Ed" if !same(&seen_edges) => {
                    seen_edges.push((number, place));
                    let ends = vertices(here, &record, &location, place)?;
                    let mut mark = None;
                    // `1 curve location first last`: the edge's own curve.
                    if let Some(words) = here
                        .data
                        .iter()
                        .map(|l| l.split_whitespace().collect::<Vec<_>>())
                        .find(|w| w.len() == 5 && w[0] == "1")
                    {
                        let number =
                            |i: usize| words[i].parse::<f64>().map_err(|_| bad("an edge's curve"));
                        let curve = curves
                            .get((number(1)? as usize).wrapping_sub(1))
                            .copied()
                            .unwrap_or(Curve::Other);
                        let on = place * location(number(2)? as usize)?;
                        let (first, last) = (number(3)?, number(4)?);
                        for step in 0..=16 {
                            let t = first + (last - first) * f64::from(step) / 16.0;
                            if let Some((p, d)) = curve.at(t) {
                                points.push(on.transform_point3(p));
                                if step == 8 {
                                    mark = Some((on.transform_point3(p), on.transform_vector3(d)));
                                }
                            }
                        }
                    }
                    if mark.is_none() && ends.len() == 2 && ends[0].distance(ends[1]) > 1e-9 {
                        mark = Some(((ends[0] + ends[1]) / 2.0, ends[1] - ends[0]));
                    }
                    points.extend(&ends);
                    shape
                        .edges
                        .push(mark.map(|(p, d)| (p.to_array(), d.normalize_or_zero().to_array())));
                }
                "Fa" if !same(&seen_faces) => {
                    seen_faces.push((number, place));
                    let header: Vec<&str> = here
                        .data
                        .first()
                        .map_or(Vec::new(), |l| l.split_whitespace().collect());
                    let surface = header
                        .get(2)
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|s| surfaces.get(s.wrapping_sub(1)).copied().flatten());
                    let on =
                        place * location(header.get(3).and_then(|l| l.parse().ok()).unwrap_or(0))?;
                    let corners = face_vertices(here, &record, &location, place)?;
                    shape
                        .faces
                        .push(surface.filter(|_| !corners.is_empty()).map(|(_, normal)| {
                            let centre = corners.iter().sum::<DVec3>() / corners.len() as f64;
                            let normal = on.transform_vector3(normal).normalize_or_zero();
                            let outward = if reversed { -normal } else { normal };
                            (centre.to_array(), outward.to_array())
                        }));
                    for &(child_reversed, child, at) in here.children.iter().rev() {
                        stack.push((
                            child,
                            place * location(at)?,
                            reversed ^ child_reversed,
                            depth + 1,
                        ));
                    }
                }
                "Ed" | "Fa" | "Ve" => {}
                _ => {
                    for &(child_reversed, child, at) in here.children.iter().rev() {
                        stack.push((
                            child,
                            place * location(at)?,
                            reversed ^ child_reversed,
                            depth + 1,
                        ));
                    }
                }
            }
        }
        shape.bounds = points
            .iter()
            .copied()
            .reduce(DVec3::min)
            .zip(points.iter().copied().reduce(DVec3::max))
            .map(|(lo, hi)| (lo.to_array(), hi.to_array()));
        Ok(shape)
    }
}

/// Where an edge's vertices stand.
fn vertices<'a>(
    edge: &Record<'a>,
    record: &impl Fn(usize) -> Result<&'a Record<'a>, FcstdError>,
    location: &impl Fn(usize) -> Result<DAffine3, FcstdError>,
    place: DAffine3,
) -> Result<Vec<DVec3>, FcstdError> {
    let mut out = Vec::new();
    for &(_, number, at) in &edge.children {
        let vertex = record(number)?;
        let point = vertex
            .data
            .get(1)
            .map(|l| {
                l.split_whitespace()
                    .filter_map(|w| w.parse().ok())
                    .collect::<Vec<f64>>()
            })
            .filter(|p| p.len() == 3)
            .ok_or_else(|| bad("a vertex has no point"))?;
        out.push((place * location(at)?).transform_point3(DVec3::from_slice(&point)));
    }
    Ok(out)
}

/// Where the vertices around a face stand.
fn face_vertices<'a>(
    face: &Record<'a>,
    record: &impl Fn(usize) -> Result<&'a Record<'a>, FcstdError>,
    location: &impl Fn(usize) -> Result<DAffine3, FcstdError>,
    place: DAffine3,
) -> Result<Vec<DVec3>, FcstdError> {
    let mut out = Vec::new();
    for &(_, wire, at) in &face.children {
        let wire_place = place * location(at)?;
        for &(_, edge, at) in &record(wire)?.children {
            out.extend(vertices(
                record(edge)?,
                record,
                location,
                wire_place * location(at)?,
            )?);
        }
    }
    Ok(out)
}
