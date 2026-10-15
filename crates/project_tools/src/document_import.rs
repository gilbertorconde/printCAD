//! A source object's solid is one native body, linked to its predecessor by a clone.
//! This keeps intermediate results addressable for parity and cross-object booleans.

use crate::error::{ErrorCategory, Failure, ImportError};
use crate::numeric::{design_length, frame, sketch_plane};
use crate::report::{Finding, note};
use crate::sketch_import::{ImportedSketch, import_sketch};
use core_document::{BodyId, Document, FeatureId, ImportedGeometry};
use kernel_api::{Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use project_archive::archive::FcstdError;
use project_archive::document_xml::{FcDocument, FcObject, Placement};
use project_archive::sketch_decode::FcSketch;
use project_archive::traversal::{ParentMap, TraversalPath};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use wb_design::DesignFeature;
use wb_sketch::SketchFeature;

pub struct ImportedProject {
    pub document: Document,
    pub sketches: BTreeMap<String, ImportedSketch>,
    pub objects: BTreeMap<String, ImportedObject>,
    pub findings: Vec<Finding>,
    pub build_failures: BTreeMap<String, Failure>,
}

/// What a conversion added to a document, by source object name.
#[derive(Default)]
pub struct Conversion {
    pub sketches: BTreeMap<String, ImportedSketch>,
    pub objects: BTreeMap<String, ImportedObject>,
    pub findings: Vec<Finding>,
    /// Source objects whose solid is a body of its own, in dependency
    /// order: each one's base and tool come before it.
    pub solids: Vec<(String, BodyId)>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ImportedObject {
    Sketch(FeatureId),
    Solid(BodyId),
}

fn same_placement(a: Placement, b: Placement) -> bool {
    a.same_as(b)
}

pub fn parent_placement(source: &FcDocument, name: &str) -> Result<Placement, FcstdError> {
    project_archive::traversal::parent_placement(source, name)
}

// A suspended conversion asks the traversal for an unresolved dependency. A
// Dependency result must precede output mutation so resuming cannot duplicate it.
enum ResolveError {
    Dependency(String),
    Source(FcstdError),
}

impl From<FcstdError> for ResolveError {
    fn from(error: FcstdError) -> Self {
        Self::Source(error)
    }
}

/// Add the source's sketches, features and bodies to `document` through
/// its own mutators, one op each. Objects the conversion cannot represent,
/// and those depending on them, are left out and named in the findings;
/// the rest still come in. An error in the source's own data stops the
/// conversion part way, so a caller that must not keep a partial result
/// converts into a scratch document first.
pub fn convert_into(
    source: &FcDocument,
    document: &mut Document,
) -> Result<Conversion, FcstdError> {
    let mut importer = Importer {
        source,
        document,
        out: Conversion::default(),
        visited: BTreeSet::new(),
        rejected: BTreeMap::new(),
        order: Vec::new(),
        parents: ParentMap::build(source)?,
    };
    for object in &source.objects {
        importer.resolve(&object.name)?;
    }
    let mut out = importer.out;
    for name in importer.order {
        // A body's entry is its tip's body, listed under the tip.
        if source.object(&name).expect("resolved source").type_name == "PartDesign::Body" {
            continue;
        }
        if let ImportedObject::Solid(body) = out.objects[&name] {
            out.solids.push((name, body));
        }
    }
    Ok(out)
}

pub fn import_document(source: &FcDocument, name: &str) -> Result<ImportedProject, ImportError> {
    import_with_initializer(source, name, |kernel| kernel.initialize())
}

fn import_with_initializer(
    source: &FcDocument,
    name: &str,
    initialize: impl FnOnce(&mut OgeomKernel) -> Result<(), kernel_api::KernelError>,
) -> Result<ImportedProject, ImportError> {
    let mut document = Document::new(name);
    let conversion = convert_into(source, &mut document)?;
    let mut project = ImportedProject {
        document,
        sketches: conversion.sketches,
        objects: conversion.objects,
        findings: conversion.findings,
        build_failures: BTreeMap::new(),
    };
    let mut kernel = OgeomKernel::new();
    initialize(&mut kernel).map_err(ImportError::KernelInitialization)?;
    for (name, body) in conversion.solids {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let plan = wb_design::body_build_ops(&project.document, body)
                .map_err(|e| Failure::new(ErrorCategory::Rebuild, format!("planning: {e}")))?;
            kernel
                .execute_solid_chain(&plan.ops, &TessellationSettings::default())
                .map_err(|e| Failure::new(ErrorCategory::Rebuild, format!("kernel: {e}")))
        }));
        match result {
            Ok(Ok(built)) => {
                project.document.set_imported_geometry(
                    body,
                    ImportedGeometry {
                        mesh: Arc::new(built.mesh),
                        mesh_path: None,
                        source_asset: None,
                        revision: 0,
                        bounds_mm: built.bounds_mm,
                        brep_blob_path: None,
                        face_colors_path: None,
                        health: None,
                    },
                );
                project
                    .document
                    .set_imported_brep_data(body, built.brep_blob, Vec::new());
            }
            Ok(Err(reason)) => {
                project.build_failures.insert(name, reason);
            }
            Err(panic) => {
                project.build_failures.insert(
                    name,
                    Failure::new(
                        ErrorCategory::Rebuild,
                        format!("kernel panic: {}", panic_message(&panic)),
                    ),
                );
            }
        }
    }
    Ok(project)
}

pub fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).into()))
        .unwrap_or_else(|| "kernel panic (non-string payload)".into())
}

struct Importer<'a> {
    source: &'a FcDocument,
    document: &'a mut Document,
    out: Conversion,
    visited: BTreeSet<String>,
    // Keep the originating reason without nesting consumer messages. A walk
    // starting at resolved leaves can be long while using little pending depth.
    rejected: BTreeMap<String, String>,
    order: Vec<String>,
    parents: ParentMap,
}

impl Importer<'_> {
    fn resolve(&mut self, name: &str) -> Result<Option<ImportedObject>, FcstdError> {
        if self.visited.contains(name) {
            return Ok(self.out.objects.get(name).copied());
        }
        let mut path = TraversalPath::new("dependency");
        path.enter(name)?;
        while let Some(current) = path.current() {
            if self.visited.contains(current) {
                path.leave();
                continue;
            }
            let Some(object) = self.source.object(current) else {
                let reason = path.missing_target(current).to_string();
                self.reject_path(&mut path, &reason);
                break;
            };
            match self.import_ready(object) {
                Ok(_) => {
                    path.leave();
                }
                Err(ResolveError::Dependency(dependency)) => {
                    let cycle = path.is_pending(&dependency);
                    if let Err(error) = path.enter(&dependency) {
                        if !cycle {
                            return Err(error);
                        }
                        self.reject_path(&mut path, &error.to_string());
                        break;
                    }
                }
                Err(ResolveError::Source(error)) => return Err(error),
            }
        }
        Ok(self.out.objects.get(name).copied())
    }

    // A failed dependency rejects every suspended consumer once. Cached rejection
    // keeps later roots from retrying the broken chain or duplicating findings.
    fn reject_path(&mut self, path: &mut TraversalPath, reason: &str) {
        while let Some(name) = path.current() {
            if self.source.object(name).is_some() {
                note(&mut self.out.findings, name, "dependency", reason);
            }
            self.visited.insert(name.into());
            self.rejected.insert(name.into(), reason.into());
            path.leave();
        }
    }

    fn resolved(&self, name: &str) -> Result<Option<ImportedObject>, ResolveError> {
        if self.visited.contains(name) {
            Ok(self.out.objects.get(name).copied())
        } else {
            Err(ResolveError::Dependency(name.into()))
        }
    }

    fn import_ready(&mut self, object: &FcObject) -> Result<Option<ImportedObject>, ResolveError> {
        let name = object.name.as_str();
        let imported = match object.type_name.as_str() {
            "Sketcher::SketchObject" => {
                let mut sketch = import_sketch(&FcSketch::decode(object)?, name)?;
                let placement = self
                    .parents
                    .placement(self.source, name)?
                    .compose(object.placement("Placement")?, name)?;
                let plane = sketch_plane(name, placement)?;
                sketch.sketch.plane = plane;
                self.out.findings.extend(sketch.findings.clone());
                let id = self
                    .document
                    .add_feature_in_body(
                        SketchFeature::new(sketch.sketch.clone(), plane),
                        label(object)?,
                        None,
                    )
                    .map_err(|e| object.format_error(e.to_string()))?;
                self.out.sketches.insert(name.into(), sketch);
                Some(ImportedObject::Sketch(id))
            }
            "PartDesign::Body" => {
                let tip = match object.link("Tip")? {
                    Some(tip) => match self.resolved(tip)? {
                        Some(imported) => Some(imported),
                        None => return Ok(self.skip_dependency(object, tip)),
                    },
                    None => None,
                };
                note(
                    &mut self.out.findings,
                    name,
                    "group",
                    "body history represented by linked solid objects",
                );
                tip
            }
            "App::Part" => {
                note(
                    &mut self.out.findings,
                    name,
                    "group",
                    "group hierarchy flattened; evaluated placement retained",
                );
                None
            }
            "App::Origin" | "App::Plane" | "App::Line" | "App::Point" => None,
            type_name
                if type_name == "PartDesign::Pad"
                    || type_name == "PartDesign::Pocket"
                    || type_name.starts_with("PartDesign::Additive")
                    || type_name.starts_with("PartDesign::Subtractive")
                    || matches!(
                        type_name,
                        "Part::Box"
                            | "Part::Cylinder"
                            | "Part::Cone"
                            | "Part::Sphere"
                            | "Part::Torus"
                            | "Part::Fuse"
                            | "Part::Cut"
                            | "Part::Common"
                    ) =>
            {
                self.solid(object)?
            }
            _ => {
                let reason = format!("unsupported {}", object.type_name);
                note(&mut self.out.findings, name, "object", &reason);
                self.rejected.insert(name.into(), reason);
                None
            }
        };
        if let Some(value) = imported {
            self.out.objects.insert(name.into(), value);
            self.order.push(name.into());
        }
        if !object.expressions()?.is_empty() {
            note(
                &mut self.out.findings,
                name,
                "expression",
                "expressions frozen to evaluated values",
            );
        }
        for prop in ["AttachmentSupport", "Support"] {
            if object.has(prop)
                && object.has("MapMode")
                && object.integer("MapMode")? != 0
                && !object.link_sub_list(prop)?.is_empty()
            {
                note(
                    &mut self.out.findings,
                    name,
                    "attachment",
                    "attachment frozen to evaluated placement",
                );
                break;
            }
        }
        self.visited.insert(name.into());
        Ok(imported)
    }

    fn skip(&mut self, object: &FcObject, reason: impl Into<String>) -> Option<ImportedObject> {
        let reason = reason.into();
        note(
            &mut self.out.findings,
            &object.name,
            "object setting",
            &reason,
        );
        self.rejected.insert(object.name.clone(), reason);
        None
    }

    fn skip_dependency(&mut self, object: &FcObject, target: &str) -> Option<ImportedObject> {
        let cause = self
            .rejected
            .get(target)
            .cloned()
            .unwrap_or_else(|| format!("{target} has no compatible imported object"));
        let reason = format!("depends on skipped {target}: {cause}");
        note(&mut self.out.findings, &object.name, "dependency", reason);
        self.rejected.insert(object.name.clone(), cause);
        // The container branch returns directly from import_ready.
        self.visited.insert(object.name.clone());
        None
    }

    fn solid(&mut self, object: &FcObject) -> Result<Option<ImportedObject>, ResolveError> {
        let type_name = object.type_name.as_str();
        let cut =
            type_name == "PartDesign::Pocket" || type_name.starts_with("PartDesign::Subtractive");
        let is_boolean = matches!(type_name, "Part::Fuse" | "Part::Cut" | "Part::Common");
        if is_boolean && !same_placement(object.placement("Placement")?, Placement::IDENTITY) {
            return Ok(self.skip(object, "placement of the boolean result"));
        }
        let base_name = if is_boolean {
            object.link("Base")?
        } else if object.has("BaseFeature") {
            object.link("BaseFeature")?
        } else {
            None
        };
        let base = match base_name {
            Some(name) => match self.resolved(name)? {
                Some(ImportedObject::Solid(body)) => Some(body),
                _ => {
                    return Ok(self.skip_dependency(object, name));
                }
            },
            None if cut || is_boolean => return Ok(self.skip(object, "missing base solid")),
            None => None,
        };
        let mut tool_body = None;
        let feature = if matches!(type_name, "PartDesign::Pad" | "PartDesign::Pocket") {
            for setting in ["TaperAngle", "TaperAngle2", "Offset"] {
                if object.has(setting) && object.float(setting)? != 0.0 {
                    return Ok(self.skip(object, format!("unsupported {setting}")));
                }
            }
            if object.has("AlongSketchNormal") && !object.bool("AlongSketchNormal")? {
                return Ok(self.skip(object, "custom direction"));
            }
            if optional_bool(object, "UseCustomVector", false)? {
                return Ok(self.skip(object, "custom direction"));
            }
            if object.has("StartType") && object.integer("StartType")? != 0 {
                return Ok(self.skip(object, "start offset"));
            }
            let symmetric = match if object.has("SideType") {
                object.integer("SideType")?
            } else {
                0
            } {
                0 => optional_bool(object, "Midplane", false)?,
                2 => true,
                _ => return Ok(self.skip(object, "two-sided extrusion")),
            };
            let mode = object.integer("Type")?;
            if mode != 0 && !(cut && mode == 1) {
                return Ok(self.skip(object, format!("unsupported extrusion Type={mode}")));
            }
            let Some((profile, subs)) = object.link_sub("Profile")? else {
                return Ok(self.skip(object, "missing profile"));
            };
            if !subs.is_empty() {
                return Ok(self.skip(object, "profile sub-elements"));
            }
            if !same_placement(
                self.parents.placement(self.source, &object.name)?,
                self.parents.placement(self.source, profile)?,
            ) {
                return Ok(self.skip(object, "profile in another coordinate system"));
            }
            let Some(ImportedObject::Sketch(sketch)) = self.resolved(profile)? else {
                return Ok(self.skip_dependency(object, profile));
            };
            let length = if mode == 0 {
                object.float("Length")?
            } else {
                1.0
            };
            if length == 0.0 {
                return Ok(self.skip(object, "zero extrusion length"));
            }
            let reversed = object.bool("Reversed")? ^ (length < 0.0);
            let mut data = serde_json::json!({ "sketch": sketch, "reversed": reversed, "symmetric": symmetric, "refine": optional_bool(object, "Refine", false)? });
            if cut {
                data["depth"] = design_length(&object.name, "Length", length.abs())?.into();
                data["through_all"] = (mode == 1).into();
            } else {
                data["length"] = design_length(&object.name, "Length", length.abs())?.into();
            }
            serde_json::from_value(serde_json::json!({ if cut { "Pocket" } else { "Pad" }: data }))
                .map_err(|e| object.format_error(e.to_string()))?
        } else if is_boolean {
            let Some(tool) = object.link("Tool")? else {
                return Ok(self.skip(object, "missing boolean tool"));
            };
            let Some(ImportedObject::Solid(body)) = self.resolved(tool)? else {
                return Ok(self.skip_dependency(object, tool));
            };
            tool_body = Some(body);
            DesignFeature::BodyBoolean {
                tool_body: body,
                kind: match type_name {
                    "Part::Fuse" => kernel_api::BoolKind::Fuse,
                    "Part::Cut" => kernel_api::BoolKind::Cut,
                    _ => kernel_api::BoolKind::Common,
                },
                more_tools: Vec::new(),
                refine: optional_bool(object, "Refine", false)?,
                boolean_tolerance: None,
            }
        } else {
            for skew in ["FirstAngle", "SecondAngle"] {
                if object.has(skew) && object.float(skew)? != 0.0 {
                    return Ok(self.skip(object, format!("unsupported {skew}")));
                }
            }
            let kind = type_name
                .rsplit(':')
                .next()
                .expect("type has a name")
                .trim_start_matches("Additive")
                .trim_start_matches("Subtractive");
            let f = |p| object.float(p);
            let primitive = match kind {
                "Box" => kernel_api::PrimitiveKind::Box {
                    length: f("Length")?,
                    width: f("Width")?,
                    height: f("Height")?,
                },
                "Cylinder" => kernel_api::PrimitiveKind::Cylinder {
                    radius: f("Radius")?,
                    height: f("Height")?,
                    angle_deg: f("Angle")?,
                },
                "Cone" => kernel_api::PrimitiveKind::Cone {
                    radius1: f("Radius1")?,
                    radius2: f("Radius2")?,
                    height: f("Height")?,
                    angle_deg: f("Angle")?,
                },
                "Sphere" => kernel_api::PrimitiveKind::Sphere {
                    radius: f("Radius")?,
                    angle1_deg: f("Angle1")?,
                    angle2_deg: f("Angle2")?,
                    angle3_deg: f("Angle3")?,
                },
                "Torus" => kernel_api::PrimitiveKind::Torus {
                    radius1: f("Radius1")?,
                    radius2: f("Radius2")?,
                    angle1_deg: f("Angle1")?,
                    angle2_deg: f("Angle2")?,
                    angle3_deg: f("Angle3")?,
                },
                _ => return Ok(self.skip(object, format!("unsupported primitive {kind}"))),
            };
            let pose = self
                .parents
                .placement(self.source, &object.name)?
                .compose(object.placement("Placement")?, &object.name)?;
            let [x_axis, _, z_axis] = frame(&object.name, pose)?;
            DesignFeature::Primitive {
                kind: primitive,
                placement: kernel_api::Placement {
                    origin: pose.position.to_array(),
                    x_axis,
                    z_axis,
                },
                subtractive: cut,
                keep_inside: false,
                attached: None,
                refine: optional_bool(object, "Refine", false)?,
                boolean_tolerance: None,
            }
        };
        let body = self.document.create_body(Some(label(object)?));
        if let Some(base) = base {
            self.document
                .add_feature_in_body(
                    DesignFeature::Clone { source: base },
                    format!("{} base", object.name),
                    Some(body),
                )
                .map_err(|e| object.format_error(e.to_string()))?;
            self.document.set_body_visible(base, false);
        }
        if let Some(tool) = tool_body {
            self.document.set_body_visible(tool, false);
        }
        let id = self
            .document
            .add_feature_in_body(feature, label(object)?, Some(body))
            .map_err(|e| object.format_error(e.to_string()))?;
        self.document.set_body_tip(body, Some(id));
        self.document.set_body_visible(body, object.visible()?);
        // Marked, the feature builds wherever the document is rebuilt.
        self.document.mark_feature_dirty(id);
        Ok(Some(ImportedObject::Solid(body)))
    }
}

fn label(object: &FcObject) -> Result<String, FcstdError> {
    Ok(if object.has("Label") {
        object.string("Label")?.into()
    } else {
        object.name.clone()
    })
}
fn optional_bool(object: &FcObject, name: &str, absent: bool) -> Result<bool, FcstdError> {
    if object.has(name) {
        object.bool(name)
    } else {
        Ok(absent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn initialization_failure_preserves_the_kernel_error_and_its_stage() {
        let source = FcDocument::parse("<Document><Objects/><ObjectData/></Document>").unwrap();
        let error = import_with_initializer(&source, "empty", |_| {
            Err(kernel_api::KernelError::Initialization(
                "test backend unavailable".into(),
            ))
        })
        .err()
        .expect("initialization must fail");
        assert_eq!(error.category(), ErrorCategory::KernelInitialization);
        assert!(
            matches!(error.source().unwrap().downcast_ref::<kernel_api::KernelError>(),
            Some(kernel_api::KernelError::Initialization(reason)) if reason == "test backend unavailable")
        );
        let failure = Failure::from(error);
        assert!(failure.reason.contains("test backend unavailable"));
        assert_eq!(
            serde_json::to_value(failure).unwrap()["category"],
            "kernel_initialization"
        );
    }
}
