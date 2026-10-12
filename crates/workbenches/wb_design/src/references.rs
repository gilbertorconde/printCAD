//! The inputs of a Design feature the property panel can swap: the
//! profile a pad, pocket, revolution, groove, pipe or helix was made from.
//! The task panel keeps the settings of the operation itself.

use core_document::{
    FeatureId, FeatureNode, FeatureReference, ReferenceChoice, WorkbenchFeature,
    WorkbenchRuntimeContext,
};
use serde_json::{Map, Value, json};

use crate::feature::{DesignFeature, FacePick};

/// The key of a feature's profile.
pub(crate) const PROFILE: &str = "profile";

/// Where a feature's profile is kept: the data fields that hold it (the
/// regions of the sketch it takes among them), and whether a flat face of
/// the solid can stand in for its sketch.
fn profile_fields(feature: &DesignFeature) -> Option<(&'static [&'static str], bool)> {
    match feature {
        DesignFeature::Pad { .. } | DesignFeature::Pocket { .. } => {
            Some((&["sketch", "profile_face", "regions"], true))
        }
        DesignFeature::Revolution { .. } | DesignFeature::Groove { .. } => {
            Some((&["sketch", "regions"], false))
        }
        DesignFeature::Helix { .. } => Some((&["sketch"], false)),
        DesignFeature::Pipe { .. } => Some((&["profile"], false)),
        _ => None,
    }
}

/// The profile of `feature`: the sketch it reads, or the face standing in.
fn profile_of(feature: &DesignFeature) -> (Option<FeatureId>, Option<FacePick>) {
    match feature {
        DesignFeature::Pad {
            sketch,
            profile_face,
            ..
        }
        | DesignFeature::Pocket {
            sketch,
            profile_face,
            ..
        } => (*sketch, *profile_face),
        other => (other.sketch(), None),
    }
}

/// Point `feature`'s profile at `sketch`, or at `face` in its place. The
/// regions it took of the sketch before are let go: a new profile is
/// taken whole.
fn set_profile(feature: &mut DesignFeature, to: Result<FeatureId, FacePick>) -> Result<(), String> {
    match (feature, to) {
        (
            DesignFeature::Pad {
                sketch,
                profile_face,
                regions,
                ..
            }
            | DesignFeature::Pocket {
                sketch,
                profile_face,
                regions,
                ..
            },
            to,
        ) => {
            (*sketch, *profile_face) = match to {
                Ok(id) => (Some(id), None),
                Err(face) => (None, Some(face)),
            };
            regions.clear();
        }
        (
            DesignFeature::Revolution {
                sketch, regions, ..
            }
            | DesignFeature::Groove {
                sketch, regions, ..
            },
            Ok(id),
        ) => {
            *sketch = id;
            regions.clear();
        }
        (DesignFeature::Helix { sketch, .. }, Ok(id)) => *sketch = id,
        (DesignFeature::Pipe { profile, .. }, Ok(id)) => *profile = id,
        (_, Err(_)) => return Err("only a pad or a pocket takes a face as its profile".into()),
        _ => return Err("this feature has no profile to change".into()),
    }
    Ok(())
}

/// The inputs `references` offers for feature `id`.
pub(crate) fn references(
    document: &core_document::Document,
    id: FeatureId,
    node: &FeatureNode,
) -> Vec<FeatureReference> {
    let (Ok(feature), Some(body)) = (DesignFeature::from_json(&node.data), node.body) else {
        return Vec::new();
    };
    let Some((fields, takes_face)) = profile_fields(&feature) else {
        return Vec::new();
    };
    let (sketch, face) = profile_of(&feature);
    let name_of = |id: FeatureId| {
        document
            .get_feature_meta(id)
            .map_or_else(|| "(missing)".to_string(), |n| n.name.clone())
    };
    let current = match (sketch, face) {
        (Some(id), _) => name_of(id),
        (None, Some(face)) => format!(
            "Face at ({:.1}, {:.1}, {:.1})",
            face.point[0], face.point[1], face.point[2]
        ),
        (None, None) => "(none)".to_string(),
    };
    vec![FeatureReference {
        key: PROFILE.to_string(),
        label: "Profile".to_string(),
        current,
        selected: sketch,
        choices: crate::build::sketch_choices(document, body, id, sketch),
        takes_face,
        fields: fields.iter().map(|f| f.to_string()).collect(),
    }]
}

/// Point feature `id`'s input `key` at `to`: its data, the dependencies it
/// reads, the sketches it hides, recorded as the `design.set` that does it.
pub(crate) fn set_reference(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    key: &str,
    to: ReferenceChoice,
) -> Result<(), String> {
    if key != PROFILE {
        return Err(format!("no input called {key}"));
    }
    let node = ctx
        .document
        .get_feature_meta(id)
        .cloned()
        .ok_or("the feature is gone")?;
    let body = node.body.ok_or("the feature belongs to no body")?;
    let mut feature = DesignFeature::from_json(&node.data).map_err(|e| e.to_string())?;
    let sketch_before = feature.sketch();
    let deps_before = feature.dependencies();
    let target = match to {
        ReferenceChoice::Feature(sketch) => {
            let allowed = crate::build::sketch_choices(ctx.document, body, id, sketch_before);
            if !allowed.iter().any(|(s, _)| *s == sketch) {
                return Err("that sketch comes after the feature, or is not the body's".into());
            }
            Ok(sketch)
        }
        ReferenceChoice::SelectedFace => {
            let face = ctx
                .selected_face_in(body)
                .ok_or("select a flat face of the solid in the view first")?;
            Err(FacePick::of(face))
        }
    };
    set_profile(&mut feature, target)?;
    let data = feature.to_json();
    ctx.document
        .update_feature_data(id, data.clone())
        .map_err(|e| e.to_string())?;
    let deps_after = feature.dependencies();
    if deps_before != deps_after {
        ctx.document.set_feature_dependencies(id, deps_after);
    }
    // The new profile is consumed like the first: it hides, and the one
    // it replaced shows again.
    if sketch_before != feature.sketch() {
        crate::build::swap_consumed_sketch(ctx.document, id, sketch_before, feature.sketch());
    }
    ctx.document.mark_feature_dirty(id);

    let fields = profile_fields(&feature).map_or(&[][..], |(f, _)| f);
    let changed = data
        .as_object()
        .and_then(|m| m.values().next())
        .and_then(Value::as_object)
        .map(|inner| {
            fields
                .iter()
                .map(|f| (f.to_string(), inner.get(*f).cloned().unwrap_or(Value::Null)))
                .collect::<Map<String, Value>>()
        })
        .unwrap_or_default();
    let mut args = changed;
    args.insert("feature".into(), json!(id.0.to_string()));
    ctx.record("design.set", args, Value::Null);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::Document;

    fn pad(sketch: FeatureId) -> DesignFeature {
        DesignFeature::Pad {
            regions: Vec::new(),
            profile_borrowed: None,
            extras: Default::default(),
            refine: false,
            boolean_tolerance: None,
            sketch: Some(sketch),
            length: 5.0,
            reversed: false,
            symmetric: false,
            mode: crate::feature::ExtrudeMode::Dimension,
            length2: 0.0,
            taper_deg: 0.0,
            up_to_face: None,
            up_to_offset: 0.0,
            profile_face: None,
            direction: Default::default(),
            up_to_shape: Vec::new(),
            mode2: None,
            up_to_face2: None,
            up_to_offset2: 0.0,
            up_to_shape2: Vec::new(),
        }
    }

    #[test]
    fn a_pad_offers_its_profile_and_swaps_it_for_an_earlier_sketch() {
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Body".into()));
        let a = doc
            .add_feature_in_body(Stub("a".into()), "a".into(), Some(body))
            .unwrap();
        let b = doc
            .add_feature_in_body(Stub("b".into()), "b".into(), Some(body))
            .unwrap();
        let pad_id = doc
            .add_feature_in_body(pad(a), "Pad".into(), Some(body))
            .unwrap();
        let node = doc.get_feature_meta(pad_id).unwrap().clone();
        let refs = references(&doc, pad_id, &node);
        assert_eq!(refs.len(), 1);
        let profile = &refs[0];
        assert_eq!(profile.current, "a");
        assert!(profile.takes_face);
        assert_eq!(profile.fields, ["sketch", "profile_face", "regions"]);
        assert!(profile.choices.iter().any(|(id, _)| *id == b));

        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        set_reference(&mut ctx, pad_id, PROFILE, ReferenceChoice::Feature(b)).unwrap();
        let recorded = core_document::HookOutcome::take(&mut ctx).recorded;
        let data = DesignFeature::from_json(&doc.get_feature_meta(pad_id).unwrap().data).unwrap();
        assert_eq!(data.sketch(), Some(b));
        assert!(doc.feature_tree().dependencies(pad_id).contains(&b));
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id, "design.set");

        // No face selected: refused.
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        assert!(set_reference(&mut ctx, pad_id, PROFILE, ReferenceChoice::SelectedFace).is_err());
    }

    /// A sketch as the sketcher stores it, as far as a profile needs: its
    /// kind and its name.
    struct Stub(String);

    impl WorkbenchFeature for Stub {
        fn workbench_id() -> core_document::WorkbenchId {
            core_document::WorkbenchId::from("wb.sketch")
        }
        fn to_json(&self) -> Value {
            json!({ "name": self.0 })
        }
        fn from_json(value: &Value) -> core_document::DocumentResult<Self> {
            Ok(Stub(value["name"].as_str().unwrap_or_default().to_string()))
        }
        fn dependencies(&self) -> Vec<FeatureId> {
            Vec::new()
        }
        fn name(&self) -> &str {
            &self.0
        }
    }
}
