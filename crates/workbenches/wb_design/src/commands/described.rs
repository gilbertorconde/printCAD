//! The fields each feature command's feature has, as `describe` and
//! `help` list them: the feature's own description (`Describe`, derived
//! from its type), its defaults what the command makes alone.
//!
//! What a command makes alone is found by making it in a scratch
//! document: a body with a padded sketch, a second sketch and a second
//! body beside it, the sketch selected and a face picked. It is made
//! twice, from two such documents of other sizes and ids, and a field
//! that differs between them is one the call gives (a sketch, a picked
//! face), shown as such rather than as a default.

use std::collections::HashMap;
use std::sync::OnceLock;

use core_document::describe::{self, Describe, Shape};
use core_document::{Document, FaceRef, FeatureId, WorkbenchRuntimeContext};
use serde_json::Value;
use sketch_model::SketchFeature;
use sketch_model::sketch::{GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

use super::features::FEATURES;
use crate::DesignWorkbench;
use crate::feature::DesignFeature;

/// Feature command `id`'s fields, described; `None` for one that makes
/// nothing in the scratch document.
pub(super) fn fields_of(id: &str) -> Option<String> {
    static ALL: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    ALL.get_or_init(|| {
        FEATURES
            .iter()
            .filter_map(|(id, _)| Some((*id, describe_made(id)?)))
            .collect()
    })
    .get(id)
    .cloned()
}

fn describe_made(id: &str) -> Option<String> {
    let (first, second) = (made_alone(id, 20.0)?, made_alone(id, 36.0)?);
    let (kind, given) = first.as_object()?.iter().next()?;
    let other = second.get(kind)?;
    let Shape::Variants(variants) = DesignFeature::shape() else {
        return None;
    };
    let Some(Shape::Fields(fields)) = variants.into_iter().find(|v| v.name == *kind)?.shape else {
        return None;
    };
    let from_call: Vec<String> = given
        .as_object()?
        .iter()
        .filter(|(name, value)| other.get(name.as_str()) != Some(value))
        .map(|(name, _)| name.clone())
        .collect();
    let lines = describe::render(&fields, Some(given), &from_call)
        .lines()
        .map(|line| match line.strip_prefix("refine: bool = ") {
            Some(_) => REFINE_LINE,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("{}\n{lines}", describe::READING))
}

/// How `refine` reads: a command takes it from the Design preference
/// (`DesignOptions::refine_result`), not from the scratch feature.
const REFINE_LINE: &str = "refine: bool = your Refine result preference (on unless turned off)";

/// The feature command `id` makes alone in a scratch document of
/// `size`, as JSON.
fn made_alone(id: &str, size: f64) -> Option<Value> {
    let mut doc = Document::new("scratch");
    let body = doc.create_body(None);
    let sketch = doc
        .add_feature_in_body(rectangle(size), "profile".into(), Some(body))
        .ok()?;
    doc.add_feature_in_body(rectangle(size / 2.0), "path".into(), Some(body))
        .ok()?;
    let pad = feature(&mut doc, "design.pad", sketch, size)?;
    doc.add_feature_in_body(pad, "pad".into(), Some(body))
        .ok()?;
    doc.create_body(None);
    let made = feature(&mut doc, id, sketch, size)?;
    serde_json::to_value(made).ok()
}

/// The feature `tool` makes from the document's first body with `sketch`
/// selected and a face on top of its pad picked.
fn feature(doc: &mut Document, tool: &str, sketch: FeatureId, size: f64) -> Option<DesignFeature> {
    let body = doc.bodies().first()?.id;
    let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
    ctx.active_document_object = Some(sketch);
    ctx.selected_face = Some(FaceRef {
        name: 0,
        point: [size as f32 / 2.0, size as f32 / 4.0, 10.0],
        normal: [0.0, 0.0, 1.0],
        surface: None,
    });
    let given = crate::Given {
        scripted: true,
        spine: None,
    };
    DesignWorkbench::feature_for_tool(tool, None, &ctx, body, given)
        .ok()
        .map(|(feature, _)| feature)
}

/// A `size` by `size / 2` rectangle on the XY plane.
fn rectangle(size: f64) -> SketchFeature {
    let mut sketch = Sketch::new("s");
    let corners = [
        (0.0, 0.0),
        (size, 0.0),
        (size, size / 2.0),
        (0.0, size / 2.0),
    ]
    .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    for i in 0..4 {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )));
    }
    SketchFeature::new(sketch, SketchPlane::default())
}

#[cfg(test)]
mod tests {
    /// Every feature command makes its feature in the scratch document,
    /// so none goes without its fields listed.
    #[test]
    fn every_feature_command_lists_its_fields() {
        for (id, _) in super::FEATURES {
            assert!(super::fields_of(id).is_some(), "{id} makes nothing");
        }
        let pad = super::fields_of("design.pad").unwrap();
        assert!(
            pad.contains("\nrefine: bool = your Refine result preference"),
            "{pad}"
        );
    }
}
