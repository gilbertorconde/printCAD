//! The annotations and layers a STEP or IGES file carries, read off the
//! reader's document into `kernel_api` terms.
//!
//! A file's annotations come in two halves: the semantic ones (a
//! dimension's value, a tolerance's zone, a datum's letter) and the drawn
//! ones (callouts: the polylines a viewer puts on screen), with a link from
//! a callout to the annotation it draws where the file gives one. Each
//! callout becomes one [`ImportedAnnotation`] per body it describes, placed
//! where that body is placed; a semantic annotation no callout draws is
//! still listed, drawing nothing. Datum targets and the document's notes
//! follow.

use std::collections::{HashMap, HashSet};

use kernel_api::{AnnotationKind, ImportedAnnotation};
use ogeom::doc::{
    Annotated, Callout, Dimension, Document, GeometricTolerance, MeasureKind, Pmi, ProductId,
    ProductKind,
};
use ogeom::math::{Point, Transform};
use ogeom::topo::{Filter, Shape, TShapeId, explore};

/// One imported body as the annotations see it: its placed shape and the
/// part it places.
pub(crate) struct PlacedBody<'a> {
    pub shape: &'a Shape,
    pub part: Option<ProductId>,
}

/// The names of the layers a body's shape, or the part it places, sits on.
pub(crate) fn layers_of(document: &Document, body: &PlacedBody<'_>) -> Vec<String> {
    let mut ids = document.layers_of(body.shape).to_vec();
    if let Some(ProductKind::Part { shape }) =
        body.part.and_then(|p| document.get(p)).map(|p| &p.kind)
    {
        for id in document.layers_of(shape) {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
    }
    ids.iter()
        .filter_map(|id| document.layer(*id))
        .map(|layer| layer.name.clone())
        .collect()
}

/// Every annotation the document carries, in file order: the drawn ones,
/// then the semantic ones nothing draws, the datum targets and the notes.
///
/// `text_is_drawn` says the reader names each callout by the text it
/// shows, as the IGES reader does; otherwise a callout's name is a label
/// (`Linear Size.3`) and its text comes from what it annotates.
pub(crate) fn annotations(
    document: &Document,
    bodies: &[PlacedBody<'_>],
    text_is_drawn: bool,
) -> Vec<ImportedAnnotation> {
    let pmi = document.pmi();
    let mut out = Vec::new();
    if pmi.is_empty() && document.notes().is_empty() {
        return out;
    }
    let owners = Owners::new(document, bodies);
    let mut drawn: HashSet<Annotated> = HashSet::new();

    for callout in &pmi.callouts {
        if let Some(what) = callout.annotates {
            drawn.insert(what);
        }
        let kind = callout_kind(callout, text_is_drawn);
        let text = callout_text(pmi, callout, text_is_drawn);
        let name = if callout.name.trim().is_empty() {
            kind.label().to_string()
        } else {
            callout.name.clone()
        };
        let items = callout
            .annotates
            .map(|what| annotated_items(pmi, what))
            .unwrap_or_default();
        for (body_index, placement) in owners.of(&items) {
            out.push(ImportedAnnotation {
                name: name.clone(),
                kind,
                text: text.clone(),
                polylines: callout
                    .polylines
                    .iter()
                    .map(|line| line.iter().map(|p| placed(&placement, *p)).collect())
                    .collect(),
                anchor: anchor(callout).map(|p| placed(&placement, p)),
                body_index,
            });
        }
    }

    // What the file states and nothing draws: listed, so the model's
    // annotations are all there to read, with nothing on screen.
    let undrawn = |what: Annotated, name: String, kind, text, items: Vec<TShapeId>| {
        (!drawn.contains(&what)).then(|| ImportedAnnotation {
            name,
            kind,
            text,
            polylines: Vec::new(),
            anchor: None,
            body_index: owners.of(&items).first().and_then(|(index, _)| *index),
        })
    };
    for (i, dimension) in pmi.dimensions.iter().enumerate() {
        out.extend(undrawn(
            Annotated::Dimension(i),
            capitalised(&dimension.name),
            AnnotationKind::Dimension,
            dimension_text(dimension),
            dimension.items().collect(),
        ));
    }
    for (i, tolerance) in pmi.tolerances.iter().enumerate() {
        let name = if tolerance.name.trim().is_empty() {
            capitalised(&tolerance.kind.replace('_', " "))
        } else {
            tolerance.name.clone()
        };
        out.extend(undrawn(
            Annotated::Tolerance(i),
            name,
            AnnotationKind::Tolerance,
            tolerance_text(tolerance),
            tolerance.items.clone(),
        ));
    }
    for (i, datum) in pmi.datums.iter().enumerate() {
        out.extend(undrawn(
            Annotated::Datum(i),
            format!("Datum {}", datum.label),
            AnnotationKind::Datum,
            datum.label.clone(),
            datum.items.clone(),
        ));
    }

    // A datum target is a point on the part: its identifier shows there.
    for target in &pmi.targets {
        for (body_index, placement) in owners.of(&target.items) {
            out.push(ImportedAnnotation {
                name: format!("Datum target {}", target.identifier()),
                kind: AnnotationKind::Datum,
                text: target.identifier(),
                polylines: Vec::new(),
                anchor: Some(placed(&placement, target.at)),
                body_index,
            });
        }
    }

    for note in document.notes() {
        let author = note.author.trim();
        out.push(ImportedAnnotation {
            name: if author.is_empty() {
                "Note".to_string()
            } else {
                format!("Note by {author}")
            },
            kind: AnnotationKind::Note,
            text: note.text.clone(),
            polylines: Vec::new(),
            anchor: None,
            body_index: None,
        });
    }
    out
}

/// Which bodies an annotation describes, from the topology it names.
struct Owners {
    /// Each body's part and its placement relative to that part's own
    /// coordinates, which is where the file draws its annotations.
    bodies: Vec<(Option<ProductId>, Transform)>,
    /// Which part each topology node belongs to; built only when the file
    /// has more than one part, since with one the answer is always it.
    part_of: HashMap<TShapeId, ProductId>,
}

impl Owners {
    fn new(document: &Document, bodies: &[PlacedBody<'_>]) -> Self {
        let datums = document.model().datums();
        let placements = bodies
            .iter()
            .map(|body| {
                let part_shape =
                    body.part
                        .and_then(|p| document.get(p))
                        .and_then(|p| match &p.kind {
                            ProductKind::Part { shape } => Some(shape),
                            ProductKind::Assembly { .. } => None,
                        });
                let placement = part_shape
                    .and_then(|own| {
                        let to_part = own.transform(datums).ok()?.inverse().ok()?;
                        Some(body.shape.transform(datums).ok()? * to_part)
                    })
                    .unwrap_or(Transform::IDENTITY);
                (body.part, placement)
            })
            .collect::<Vec<_>>();
        let parts: HashSet<ProductId> = bodies.iter().filter_map(|b| b.part).collect();
        let mut part_of = HashMap::new();
        if parts.len() > 1 {
            for part in parts {
                let Some(ProductKind::Part { shape }) = document.get(part).map(|p| &p.kind) else {
                    continue;
                };
                if let Ok(all) = explore(document.model(), shape, Filter::All) {
                    for sub in all {
                        part_of.entry(sub.node()).or_insert(part);
                    }
                }
            }
        }
        Self {
            bodies: placements,
            part_of,
        }
    }

    /// The bodies an annotation naming `items` belongs to, each with the
    /// placement its drawing takes; one entry with no body, unplaced, when
    /// the file does not say.
    fn of(&self, items: &[TShapeId]) -> Vec<(Option<usize>, Transform)> {
        let every = || {
            self.bodies
                .iter()
                .enumerate()
                .map(|(i, (_, placement))| (Some(i), *placement))
                .collect::<Vec<_>>()
        };
        let single_part = {
            let mut parts = self.bodies.iter().map(|(part, _)| *part);
            let first = parts.next();
            first.is_some() && parts.all(|p| Some(p) == first)
        };
        let found = if single_part {
            every()
        } else {
            let parts: HashSet<ProductId> = items
                .iter()
                .filter_map(|item| self.part_of.get(item).copied())
                .collect();
            self.bodies
                .iter()
                .enumerate()
                .filter(|(_, (part, _))| part.is_some_and(|p| parts.contains(&p)))
                .map(|(i, (_, placement))| (Some(i), *placement))
                .collect()
        };
        if found.is_empty() {
            vec![(None, Transform::IDENTITY)]
        } else {
            found
        }
    }
}

/// The topology a semantic annotation names.
fn annotated_items(pmi: &Pmi, what: Annotated) -> Vec<TShapeId> {
    match what {
        Annotated::Dimension(i) => pmi
            .dimensions
            .get(i)
            .map(|d| d.items().collect())
            .unwrap_or_default(),
        Annotated::Tolerance(i) => pmi
            .tolerances
            .get(i)
            .map(|t| t.items.clone())
            .unwrap_or_default(),
        Annotated::Datum(i) => pmi
            .datums
            .get(i)
            .map(|d| d.items.clone())
            .unwrap_or_default(),
    }
}

fn callout_kind(callout: &Callout, text_is_drawn: bool) -> AnnotationKind {
    match callout.annotates {
        Some(Annotated::Dimension(_)) => AnnotationKind::Dimension,
        Some(Annotated::Tolerance(_)) => AnnotationKind::Tolerance,
        Some(Annotated::Datum(_)) => AnnotationKind::Datum,
        None if text_is_drawn => {
            if callout.name.trim().is_empty() {
                AnnotationKind::Other
            } else {
                AnnotationKind::Note
            }
        }
        None => {
            // The names drawing tools give their callouts say what they are.
            let name = callout.name.to_ascii_lowercase();
            if name.contains("datum") {
                AnnotationKind::Datum
            } else if name.starts_with("text") || name.starts_with("note") {
                AnnotationKind::Note
            } else if name.contains("size")
                || name.contains("dimension")
                || name.contains("distance")
            {
                AnnotationKind::Dimension
            } else if [
                "flatness",
                "position",
                "profile",
                "perpendicularity",
                "parallelism",
                "angularity",
                "cylindricity",
                "roundness",
                "circularity",
                "straightness",
                "concentricity",
                "symmetry",
                "runout",
            ]
            .iter()
            .any(|word| name.contains(word))
            {
                AnnotationKind::Tolerance
            } else {
                AnnotationKind::Other
            }
        }
    }
}

fn callout_text(pmi: &Pmi, callout: &Callout, text_is_drawn: bool) -> String {
    if text_is_drawn && !callout.name.trim().is_empty() {
        return callout.name.clone();
    }
    let semantic = callout.annotates.and_then(|what| match what {
        Annotated::Dimension(i) => pmi.dimensions.get(i).map(dimension_text),
        Annotated::Tolerance(i) => pmi.tolerances.get(i).map(tolerance_text),
        Annotated::Datum(i) => pmi.datums.get(i).map(|d| d.label.clone()),
    });
    semantic.unwrap_or_else(|| callout.name.clone())
}

/// Where a callout's label goes: over the top of what it draws, in the
/// plane it is drawn in (its middle where it names no plane), or at the
/// plane's origin where it draws nothing.
fn anchor(callout: &Callout) -> Option<Point> {
    let points: Vec<Point> = callout.polylines.iter().flatten().copied().collect();
    if points.is_empty() {
        return callout.plane.map(|plane| plane.origin());
    }
    match callout.plane {
        Some(plane) => {
            let local: Vec<Point> = points.iter().map(|p| plane.to_local(*p)).collect();
            let (lo, hi) = bounds(&local);
            Some(plane.to_world(Point::new((lo.x + hi.x) / 2.0, hi.y, (lo.z + hi.z) / 2.0)))
        }
        None => {
            let (lo, hi) = bounds(&points);
            Some(Point::new(
                (lo.x + hi.x) / 2.0,
                (lo.y + hi.y) / 2.0,
                (lo.z + hi.z) / 2.0,
            ))
        }
    }
}

fn bounds(points: &[Point]) -> (Point, Point) {
    let mut lo = Point::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        lo = Point::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

fn placed(placement: &Transform, p: Point) -> [f32; 3] {
    let p = placement.apply(p);
    [p.x as f32, p.y as f32, p.z as f32]
}

/// A number as a drawing writes it: up to three decimals, no trailing
/// zeros.
fn number(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// An allowance with its sign: `+0.2`, `-0.05`, `0`.
fn signed(value: f64) -> String {
    let text = number(value);
    if text == "0" || text.starts_with('-') {
        text
    } else {
        format!("+{text}")
    }
}

/// What a dimension shows: its symbol, its value and its bounds, as
/// `Ø 35 ±0.2`, `R5 +0.1/-0.05` or `60° ±0.5°`.
pub(crate) fn dimension_text(dimension: &Dimension) -> String {
    let (symbol, scale, unit) = match dimension.kind {
        MeasureKind::Angle => ("", 180.0 / std::f64::consts::PI, "°"),
        MeasureKind::Length => {
            let name = dimension.name.to_ascii_lowercase();
            let symbol = if name.contains("spherical") && name.contains("diameter") {
                "SØ "
            } else if name.contains("spherical") && name.contains("radius") {
                "SR"
            } else if name.contains("diameter") {
                "Ø "
            } else if name.contains("radius") {
                "R"
            } else {
                ""
            };
            (symbol, 1.0, "")
        }
    };
    let show = |v: f64| format!("{}{unit}", number(v * scale));
    let Some(&nominal) = dimension.values.first() else {
        return capitalised(&dimension.name);
    };
    // Two stated values are the limits themselves; three are the nominal
    // with its upper and lower limits.
    if let [a, b] = dimension.values.as_slice() {
        return format!("{symbol}{} / {}", show(a.min(*b)), show(a.max(*b)));
    }
    let (plus, minus) = match (dimension.plus, dimension.minus) {
        (Some(plus), Some(minus)) => (Some(plus), Some(minus)),
        (Some(plus), None) => (Some(plus), None),
        (None, Some(minus)) => (None, Some(minus)),
        (None, None) if dimension.values.len() > 2 => {
            let rest = &dimension.values[1..];
            let hi = rest.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let lo = rest.iter().copied().fold(f64::INFINITY, f64::min);
            (Some(hi - nominal), Some(lo - nominal))
        }
        (None, None) => (None, None),
    };
    let mut text = format!("{symbol}{}", show(nominal));
    match (plus, minus) {
        (Some(p), Some(m)) if number(p * scale) == number(-m * scale) && number(p) != "0" => {
            text.push_str(&format!(" ±{}", show(p)));
        }
        (Some(p), Some(m)) => {
            text.push_str(&format!(
                " {}{unit}/{}{unit}",
                signed(p * scale),
                signed(m * scale)
            ));
        }
        (Some(p), None) => text.push_str(&format!(" {}{unit}", signed(p * scale))),
        (None, Some(m)) => text.push_str(&format!(" {}{unit}", signed(m * scale))),
        (None, None) => {}
    }
    text
}

/// What a tolerance shows: its kind, its zone with any modifiers, and the
/// datums it references, as `Position 0.75 (M) | A | B`.
pub(crate) fn tolerance_text(tolerance: &GeometricTolerance) -> String {
    let mut text = format!(
        "{} {}",
        capitalised(&tolerance.kind.replace('_', " ")),
        number(tolerance.magnitude)
    );
    for modifier in &tolerance.modifiers {
        let mark = match modifier.as_str() {
            "maximum_material_requirement" => "(M)",
            "least_material_requirement" => "(L)",
            "projected_tolerance_zone" => "(P)",
            "free_state" => "(F)",
            "tangent_plane" => "(T)",
            "unequally_disposed" => "(U)",
            "statistical_tolerance" => "(ST)",
            _ => continue,
        };
        text.push(' ');
        text.push_str(mark);
    }
    for datum in &tolerance.datums {
        text.push_str(" | ");
        text.push_str(datum);
    }
    text
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimension(name: &str, values: &[f64], plus: Option<f64>, minus: Option<f64>) -> Dimension {
        Dimension {
            name: name.to_string(),
            values: values.to_vec(),
            kind: MeasureKind::Length,
            plus,
            minus,
            features: Vec::new(),
            location: false,
        }
    }

    #[test]
    fn a_dimension_shows_its_symbol_value_and_bounds() {
        assert_eq!(
            dimension_text(&dimension("diameter", &[25.0], Some(0.15), Some(-0.15))),
            "Ø 25 ±0.15"
        );
        assert_eq!(
            dimension_text(&dimension("diameter", &[35.0], Some(0.0), Some(-0.2))),
            "Ø 35 0/-0.2"
        );
        assert_eq!(
            dimension_text(&dimension("diameter", &[35.0, 35.2, 34.8], None, None)),
            "Ø 35 ±0.2"
        );
        assert_eq!(
            dimension_text(&dimension("radius", &[5.5], None, None)),
            "R5.5"
        );
        assert_eq!(
            dimension_text(&dimension("linear distance", &[40.0], None, None)),
            "40"
        );
        let mut angle = dimension("angle", &[60_f64.to_radians()], None, None);
        angle.kind = MeasureKind::Angle;
        angle.plus = Some(0.5_f64.to_radians());
        angle.minus = Some(-0.5_f64.to_radians());
        assert_eq!(dimension_text(&angle), "60° ±0.5°");
    }

    #[test]
    fn a_tolerance_shows_its_kind_zone_modifiers_and_datums() {
        let tolerance = GeometricTolerance {
            kind: "surface_profile".to_string(),
            name: "Profile.1".to_string(),
            magnitude: 0.5,
            modifiers: vec!["maximum_material_requirement".to_string()],
            datums: vec!["A".to_string(), "B-C".to_string()],
            items: Vec::new(),
        };
        assert_eq!(
            tolerance_text(&tolerance),
            "Surface profile 0.5 (M) | A | B-C"
        );
    }
}
