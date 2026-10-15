//! A text object of the file as a sketch holding a text block: the string
//! in the file's font where this machine has it, laid out at the size and
//! justification the file gives, standing where the file places it.

use core_document::Document;
use project_archive::archive::FcstdError;
use project_archive::document_xml::{FcObject, Placement};
use wb_sketch::sketch::{GeometryElement, Sketch, Vec2D};
use wb_sketch::text::{self, TextSpec};

use crate::numeric::sketch_plane;
use crate::report::{Finding, note};

/// The justifications the file numbers, in its order.
const JUSTIFICATIONS: [&str; 9] = [
    "Top-Left",
    "Top-Center",
    "Top-Right",
    "Middle-Left",
    "Middle-Center",
    "Middle-Right",
    "Bottom-Left",
    "Bottom-Center",
    "Bottom-Right",
];

/// The sketch `object` (a text) becomes, placed by `placement` (its
/// groups' and its own), its font kept in `document` when it is read from
/// a file or an installed family. `Err` says why it cannot come in.
pub(crate) fn text_sketch(
    object: &FcObject,
    placement: Placement,
    document: &mut Document,
    findings: &mut Vec<Finding>,
) -> Result<Result<wb_sketch::SketchFeature, String>, FcstdError> {
    let name = object.name.as_str();
    let string = object.string("String")?;
    if string.trim().is_empty() {
        return Ok(Err("an empty text".into()));
    }
    if object.has("MakeFace") && !object.bool("MakeFace")? {
        return Ok(Err("a text of outlines that make no faces".into()));
    }
    let file = if object.has("FontFile") {
        object.string("FontFile")?
    } else {
        ""
    };
    let mut spec = TextSpec {
        text: string.to_owned(),
        ..TextSpec::default()
    };
    font(document, &mut spec, file, name, findings);

    // The file's size is the height of its capitals unless it says not.
    let size = object.float("Size")?;
    if !(size.is_finite() && size > 0.0) {
        return Ok(Err(format!("text size {size}")));
    }
    let cap = cap_height(&spec);
    let scale_to_size = !object.has("ScaleToSize") || object.bool("ScaleToSize")?;
    let em = match (scale_to_size, cap) {
        (true, Some(cap)) => size / cap,
        _ => size,
    };
    let tracking = if object.has("Tracking") {
        object.float("Tracking")?
    } else {
        0.0
    };
    if tracking != 0.0 {
        note(
            findings,
            name,
            "object setting",
            "letter spacing scaled with the text, as near as printCAD's spacing comes",
        );
    }
    if object.has("ObliqueAngle") && object.float("ObliqueAngle")? != 0.0 {
        note(
            findings,
            name,
            "object setting",
            "slanted text kept upright",
        );
    }
    spec.size = em;
    spec.spacing = tracking * em / size;

    let mut sketch = Sketch::new(name);
    let block = match text::add(&mut sketch, Vec2D::new(0.0, 0.0), &spec) {
        Ok(block) => block,
        Err(why) => return Ok(Err(why)),
    };
    // Where the file's justification puts the text's point.
    let Some((low, high)) = outline_bounds(&sketch, block) else {
        return Ok(Err("the text has nothing to draw".into()));
    };
    let justification = justification(object)?;
    let by_shape =
        object.has("JustificationReference") && object.integer("JustificationReference")? == 1;
    let keep_margin = object.has("KeepLeftMargin") && object.bool("KeepLeftMargin")?;
    let mut shift = Vec2D::new(0.0, 0.0);
    if !(keep_margin && justification.ends_with("Left")) {
        shift.x = -low.x;
    }
    let height = if by_shape {
        shift.y = -low.y;
        high.y - low.y
    } else {
        cap.map_or(size, |cap| cap * em)
    };
    if justification.starts_with("Top") {
        shift.y -= height;
    } else if justification.starts_with("Middle") {
        shift.y -= height / 2.0;
    }
    if justification.ends_with("Right") {
        shift.x -= high.x - low.x;
    } else if justification.ends_with("Center") {
        shift.x -= (high.x - low.x) / 2.0;
    }
    let anchor = sketch
        .texts
        .iter()
        .find(|b| b.id == block)
        .map(|b| b.anchor)
        .expect("the block just made");
    if let Some(GeometryElement::Point(point)) = sketch.get_geometry_mut(anchor) {
        point.position = shift;
    }
    text::follow(&mut sketch);
    let plane = sketch_plane(name, placement)?;
    sketch.plane = plane;
    Ok(Ok(wb_sketch::SketchFeature::new(sketch, plane)))
}

/// The file's justification by its name.
fn justification(object: &FcObject) -> Result<&'static str, FcstdError> {
    if !object.has("Justification") {
        return Ok("Bottom-Left");
    }
    let index = object.integer("Justification")?;
    Ok(usize::try_from(index)
        .ok()
        .and_then(|i| JUSTIFICATIONS.get(i))
        .copied()
        .unwrap_or("Bottom-Left"))
}

/// Ready `spec`'s font: the file's own when this machine has it, else an
/// installed font of its family, else the font printCAD carries nearest
/// it, a finding saying which stands in for which.
fn font(
    document: &mut Document,
    spec: &mut TextSpec,
    file: &str,
    name: &str,
    findings: &mut Vec<Finding>,
) {
    let path = std::path::Path::new(file);
    if !file.is_empty() && path.is_file() {
        spec.font = file.to_owned();
        if text::load_font(document, spec).is_ok() {
            return;
        }
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    for family in families(&stem) {
        if text::installed_font(&family).is_none() {
            continue;
        }
        spec.font = family.clone();
        if text::load_font(document, spec).is_ok() {
            note(
                findings,
                name,
                "font",
                format!("{file} is not on this machine; the installed {family} stands in for it"),
            );
            return;
        }
    }
    let lower = stem.to_ascii_lowercase();
    spec.font = if lower.contains("mono") {
        "IBM Plex Mono"
    } else if ["bold", "black", "heavy", "semi"]
        .iter()
        .any(|w| lower.contains(w))
    {
        "IBM Plex Sans SemiBold"
    } else {
        text::DEFAULT_FONT
    }
    .to_owned();
    spec.font_asset = None;
    spec.font_data = None;
    note(
        findings,
        name,
        "font",
        format!(
            "{} is not on this machine, nor a font of its family; printCAD's {} stands in \
             for it, so the letters' shapes and widths differ from the file's",
            if file.is_empty() {
                "the text's font"
            } else {
                file
            },
            spec.font
        ),
    );
}

/// The family names a font file's name may stand for, likeliest first:
/// `UbuntuNerdFont-Bold` gives `Ubuntu Nerd Font`, `UbuntuNerdFont` and
/// `Ubuntu`.
fn families(stem: &str) -> Vec<String> {
    let base = stem.split(['-', '_']).next().unwrap_or(stem);
    let mut spaced = String::new();
    let mut before: Option<char> = None;
    for c in base.chars() {
        if c.is_uppercase() && before.is_some_and(char::is_lowercase) {
            spaced.push(' ');
        }
        spaced.push(c);
        before = Some(c);
    }
    let mut out = vec![spaced.clone(), base.to_owned()];
    if let Some(first) = spaced.split(' ').next() {
        out.push(first.to_owned());
    }
    out.retain(|f| !f.is_empty());
    out.dedup();
    out
}

/// The height of the font's capitals for an em of one millimetre: the top
/// of its `M`.
fn cap_height(spec: &TextSpec) -> Option<f64> {
    let probe = TextSpec {
        text: "M".into(),
        size: 1.0,
        spacing: 0.0,
        angle: 0.0,
        ..spec.clone()
    };
    let mut sketch = Sketch::new("cap");
    let block = text::add(&mut sketch, Vec2D::new(0.0, 0.0), &probe).ok()?;
    let (_, high) = outline_bounds(&sketch, block)?;
    (high.y > 0.0).then_some(high.y)
}

/// The box a text block's outline points reach.
fn outline_bounds(sketch: &Sketch, block: uuid::Uuid) -> Option<(Vec2D, Vec2D)> {
    let block = sketch.texts.iter().find(|b| b.id == block)?;
    let points: Vec<Vec2D> = block
        .elements
        .iter()
        .filter_map(|id| match sketch.get_geometry(*id) {
            Some(GeometryElement::Point(p)) => Some(p.position),
            _ => None,
        })
        .collect();
    let low = points
        .iter()
        .copied()
        .reduce(|a, b| Vec2D::new(a.x.min(b.x), a.y.min(b.y)))?;
    let high = points
        .iter()
        .copied()
        .reduce(|a, b| Vec2D::new(a.x.max(b.x), a.y.max(b.y)))?;
    Some((low, high))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_font_file_name_gives_the_families_it_may_stand_for() {
        assert_eq!(
            super::families("UbuntuNerdFont-Bold"),
            ["Ubuntu Nerd Font", "UbuntuNerdFont", "Ubuntu"]
        );
    }
}
