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
use project_archive::saved_shape::{Mark, SavedShape};
use project_archive::sketch_decode::FcSketch;
use project_archive::traversal::{ParentMap, TraversalPath};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use wb_design::{
    ChamferMode, DesignFeature, EdgePick, EdgeSel, FacePick, LoftSection, MirrorPlane, SketchAxis,
};
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
    shapes: &Shapes,
    document: &mut Document,
) -> Result<Conversion, FcstdError> {
    let mut importer = Importer {
        source,
        shapes,
        saved: BTreeMap::new(),
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

/// Convert `source`, finding the edges and faces its features name on
/// the saved shapes in `shapes` (`FcstdArchive::shapes`), and build it.
pub fn import_document(
    source: &FcDocument,
    shapes: &Shapes,
    name: &str,
) -> Result<ImportedProject, ImportError> {
    import_with_initializer(source, shapes, name, |kernel| kernel.initialize())
}

fn import_with_initializer(
    source: &FcDocument,
    shapes: &Shapes,
    name: &str,
    initialize: impl FnOnce(&mut OgeomKernel) -> Result<(), kernel_api::KernelError>,
) -> Result<ImportedProject, ImportError> {
    let mut document = Document::new(name);
    let conversion = convert_into(source, shapes, &mut document)?;
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

/// The archive's saved shapes by entry name, as `FcstdArchive::shapes`.
pub type Shapes = BTreeMap<String, String>;

struct Importer<'a> {
    source: &'a FcDocument,
    shapes: &'a Shapes,
    saved: BTreeMap<String, Option<std::rc::Rc<SavedShape>>>,
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
                        "PartDesign::Fillet"
                            | "PartDesign::Chamfer"
                            | "PartDesign::Hole"
                            | "PartDesign::Mirrored"
                            | "Part::Box"
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
            None if cut
                || is_boolean
                || matches!(
                    type_name,
                    "PartDesign::Fillet"
                        | "PartDesign::Chamfer"
                        | "PartDesign::Hole"
                        | "PartDesign::Mirrored"
                ) =>
            {
                return Ok(self.skip(object, "missing base solid"));
            }
            None => None,
        };
        let mut tool_body = None;
        let feature = if is_boolean {
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
        } else if let Some(made) = self.design_feature(object, cut)? {
            match made {
                Ok(feature) => feature,
                Err(Lost::Setting(reason)) => return Ok(self.skip(object, reason)),
                Err(Lost::Dependency(target)) => return Ok(self.skip_dependency(object, &target)),
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

    /// Fillets, chamfers, holes and lofts; `None` for any other kind.
    fn design_feature(
        &mut self,
        object: &FcObject,
        cut: bool,
    ) -> Result<Option<Made>, ResolveError> {
        Ok(Some(match object.type_name.as_str() {
            "PartDesign::Pad" | "PartDesign::Pocket" => self.extrusion(object, cut)?,
            "PartDesign::Fillet" | "PartDesign::Chamfer" => self.dress_up(object)?,
            "PartDesign::Hole" => self.hole(object)?,
            "PartDesign::Mirrored" => self.mirrored(object)?,
            "PartDesign::AdditiveLoft" | "PartDesign::SubtractiveLoft" => self.loft(object, cut)?,
            _ => return Ok(None),
        }))
    }

    fn extrusion(&mut self, object: &FcObject, cut: bool) -> Result<Made, ResolveError> {
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        let sides = if object.has("SideType") {
            object.integer("SideType")?
        } else {
            0
        };
        let first = object.integer("Type")?;
        // Files that set the sides apart end each side on its own and have
        // no two-lengths end.
        let two_lengths = !object.has("SideType") && first == 4;
        let second = if sides == 1 {
            Some(object.integer("Type2")?)
        } else {
            None
        };
        let symmetric = match sides {
            0 => optional_bool(object, "Midplane", false)?,
            1 => false,
            2 => true,
            other => return lost(format!("unsupported SideType={other}")),
        };
        let two_sided = two_lengths || second.is_some();
        for (setting, unused) in [
            ("TaperAngle", true),
            ("TaperAngle2", two_sided),
            ("Offset", first != 3),
            ("Offset2", second.is_some_and(|end| end != 3)),
        ] {
            if unused && object.has(setting) && object.float(setting)? != 0.0 {
                return lost(format!("unsupported {setting}"));
            }
        }
        if object.has("AlongSketchNormal") && !object.bool("AlongSketchNormal")? {
            return lost("custom direction".into());
        }
        if optional_bool(object, "UseCustomVector", false)? {
            return lost("custom direction".into());
        }
        if object.has("StartType") && object.integer("StartType")? != 0 {
            return lost("start offset".into());
        }
        let (mode, up_to_face) = if two_lengths {
            ("TwoLengths", None)
        } else {
            match self.extrusion_end(object, cut, first, "UpToFace")? {
                Ok(end) => end,
                Err(lost) => return Ok(Err(lost)),
            }
        };
        let second_end = match second {
            Some(end) => match self.extrusion_end(object, cut, end, "UpToFace2")? {
                Ok(end) => Some(end),
                Err(lost) => return Ok(Err(lost)),
            },
            None => None,
        };
        if symmetric && !matches!(mode, "Dimension" | "ThroughAll") {
            return lost("symmetric extrusion to a face".into());
        }
        let Some((profile, subs)) = object.link_sub("Profile")? else {
            return lost("missing profile".into());
        };
        if !subs.is_empty() {
            return lost("profile sub-elements".into());
        }
        if !same_placement(
            self.parents.placement(self.source, &object.name)?,
            self.parents.placement(self.source, profile)?,
        ) {
            return lost("profile in another coordinate system".into());
        }
        let sketch = match self.sketch(profile)? {
            Ok(sketch) => sketch,
            Err(lost) => return Ok(Err(lost)),
        };
        let measured = matches!(mode, "Dimension" | "TwoLengths");
        // Only a measured end reads its length; the others keep one for show.
        let length = if measured {
            object.float("Length")?
        } else {
            1.0
        };
        if mode == "Dimension" && length == 0.0 {
            return lost("zero extrusion length".into());
        }
        let reversed = object.bool("Reversed")? ^ (measured && length < 0.0);
        let mut data = serde_json::json!({ "sketch": sketch, "reversed": reversed, "symmetric": symmetric, "mode": mode, "refine": optional_bool(object, "Refine", false)? });
        let (first_length, second_length) = if cut {
            ("depth", "depth2")
        } else {
            ("length", "length2")
        };
        let length_of = |name| design_length(&object.name, name, object.float(name)?.abs());
        data[first_length] = design_length(&object.name, "Length", length.abs())?.into();
        if two_lengths || second_end.is_some_and(|(end, _)| end == "Dimension") {
            data[second_length] = length_of("Length2")?.into();
        }
        // A pocket's offset moves its end along the target face's outward
        // normal, as here; a pad's moves it the other way.
        let sense = if cut { 1.0 } else { -1.0 };
        let offset_of = |name| design_length(&object.name, name, sense * object.float(name)?);
        if let Some(pick) = up_to_face {
            data["up_to_face"] = serde_json::to_value(pick).expect("a pick serializes");
            data["up_to_offset"] = offset_of("Offset")?.into();
        }
        if let Some((end, pick)) = second_end {
            data["mode2"] = end.into();
            if let Some(pick) = pick {
                data["up_to_face2"] = serde_json::to_value(pick).expect("a pick serializes");
                data["up_to_offset2"] = offset_of("Offset2")?.into();
            }
        }
        if cut {
            data["through_all"] = (mode == "ThroughAll").into();
        }
        Ok(Ok(serde_json::from_value(
            serde_json::json!({ if cut { "Pocket" } else { "Pad" }: data }),
        )
        .map_err(|e| object.format_error(e.to_string()))?))
    }

    /// How one side of a pad or pocket ends, by the file's number for it.
    /// The two kinds number their ends alike but for the second: a pad's
    /// runs to the last face, a pocket's through all.
    fn extrusion_end(
        &mut self,
        object: &FcObject,
        cut: bool,
        end: i64,
        face: &str,
    ) -> Result<Result<(&'static str, Option<FacePick>), Lost>, ResolveError> {
        Ok(Ok(match (end, cut) {
            (0, _) => ("Dimension", None),
            (1, true) => ("ThroughAll", None),
            (1, false) => ("ToLast", None),
            (2, _) => ("ToFirst", None),
            (3, _) => match self.face_pick(object, face)? {
                Ok(pick) => ("UpToFace", Some(pick)),
                Err(reason) => return Ok(Err(Lost::Setting(reason))),
            },
            _ => {
                return Ok(Err(Lost::Setting(format!(
                    "unsupported extrusion Type={end}"
                ))));
            }
        }))
    }

    fn dress_up(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let edges = if optional_bool(object, "UseAllEdges", false)? {
            EdgeSel::All
        } else {
            let (edges, faces) = match self.picks(object, "Base")? {
                Ok(picks) => picks,
                Err(lost) => return Ok(Err(lost)),
            };
            match (edges.is_empty(), faces.is_empty()) {
                (false, true) => EdgeSel::Edges(edges),
                (true, false) => EdgeSel::Faces(faces),
                (true, true) => return Ok(Err(Lost::Setting("no edges picked".into()))),
                (false, false) => {
                    return Ok(Err(Lost::Setting("edges and faces picked together".into())));
                }
            }
        };
        let size = |name| design_length(&object.name, name, object.float(name)?);
        if object.type_name == "PartDesign::Fillet" {
            return Ok(Ok(DesignFeature::Fillet {
                radius: size("Radius")?,
                edges,
                follow_tangent: false,
            }));
        }
        let mode = match object.integer("ChamferType")? {
            0 => ChamferMode::EqualDistance,
            1 => ChamferMode::TwoDistances,
            2 => ChamferMode::DistanceAngle,
            other => {
                return Ok(Err(Lost::Setting(format!(
                    "unsupported ChamferType={other}"
                ))));
            }
        };
        Ok(Ok(DesignFeature::Chamfer {
            size: size("Size")?,
            mode,
            size2: if object.has("Size2") {
                size("Size2")?
            } else {
                0.0
            },
            angle_deg: if object.has("Angle") {
                size("Angle")?
            } else {
                0.0
            },
            flip: optional_bool(object, "FlipDirection", false)?,
            edges,
            follow_tangent: false,
        }))
    }

    fn hole(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let sketch = match self.profile_sketch(object, "Profile")? {
            Ok(sketch) => sketch,
            Err(lost) => return Ok(Err(lost)),
        };
        let through_all = match object.integer("DepthType")? {
            0 => false,
            1 => true,
            other => return Ok(Err(Lost::Setting(format!("unsupported DepthType={other}")))),
        };
        if optional_bool(object, "Midplane", false)? {
            return Ok(Err(Lost::Setting("hole drilled both ways".into())));
        }
        let length = |name| design_length(&object.name, name, object.float(name)?);
        let cut = match enum_name(object, "HoleCutType")?.as_deref() {
            None | Some("None") => serde_json::json!("None"),
            Some(name) => {
                let diameter = length("HoleCutDiameter")?;
                let depth = length("HoleCutDepth")?;
                let angle_deg = length("HoleCutCountersinkAngle")?;
                // A standard's seat is a counterbore unless the standard
                // sinks the head.
                let sunk = ["Countersink", "2009", "7046", "10642", "14584"]
                    .iter()
                    .any(|mark| name.contains(mark));
                if name == "Counterdrill" {
                    serde_json::json!({ "Counterdrill": { "diameter": diameter, "depth": depth, "angle_deg": angle_deg } })
                } else if sunk {
                    serde_json::json!({ "Countersink": { "diameter": diameter, "angle_deg": angle_deg } })
                } else {
                    serde_json::json!({ "Counterbore": { "diameter": diameter, "depth": depth } })
                }
            }
        };
        let drill_point = if !through_all && object.integer("DrillPoint")? == 1 {
            serde_json::json!({ "Angled": { "angle_deg": length("DrillPointAngle")? } })
        } else {
            serde_json::json!("Flat")
        };
        let taper_deg = if optional_bool(object, "Tapered", false)? {
            90.0 - length("TaperedAngle")?
        } else {
            0.0
        };
        if optional_bool(object, "Threaded", false)? {
            note(
                &mut self.out.findings,
                &object.name,
                "object setting",
                "thread kept as the bore it is drilled to",
            );
        }
        let data = serde_json::json!({ "Hole": {
            "sketch": sketch,
            "diameter": length("Diameter")?,
            "depth": length("Depth")?,
            "through_all": through_all,
            "cut": cut,
            "drill_point": drill_point,
            "point_in_depth": optional_bool(object, "DrillForDepth", false)?,
            "taper_deg": taper_deg,
            "reversed": optional_bool(object, "Reversed", false)?,
            "refine": optional_bool(object, "Refine", false)?,
        }});
        Ok(Ok(
            serde_json::from_value(data).map_err(|e| object.format_error(e.to_string()))?
        ))
    }

    /// A mirror of the whole solid so far. Each feature here is a body of
    /// its own, so a mirror of chosen features has nothing to repeat.
    fn mirrored(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        if !object.link_list("Originals")?.is_empty() {
            return Ok(Err(Lost::Setting("mirror of chosen features".into())));
        }
        let Some((target, subs)) = object.link_sub("MirrorPlane")? else {
            return Ok(Err(Lost::Setting("missing MirrorPlane".into())));
        };
        let Some(plane) = self.source.object(target) else {
            return Ok(Err(Lost::Dependency(target.into())));
        };
        let plane = match (plane.type_name.as_str(), subs.as_slice()) {
            ("App::Plane", []) => {
                // The body's own planes, which stand where the world's do
                // only when the body has not been moved.
                let role = if plane.has("Role") {
                    plane.string("Role")?
                } else {
                    target
                };
                let moved = !same_placement(
                    self.parents.placement(self.source, &object.name)?,
                    Placement::IDENTITY,
                );
                match role {
                    _ if moved => None,
                    role if role.starts_with("XY") => Some(MirrorPlane::XY),
                    role if role.starts_with("XZ") => Some(MirrorPlane::XZ),
                    role if role.starts_with("YZ") => Some(MirrorPlane::YZ),
                    _ => None,
                }
            }
            ("Sketcher::SketchObject", [] | ["N_Axis"]) => match self.sketch(target)? {
                Ok(sketch) => Some(MirrorPlane::Sketch { sketch, axis: None }),
                Err(lost) => return Ok(Err(lost)),
            },
            ("Sketcher::SketchObject", [axis @ ("H_Axis" | "V_Axis")]) => {
                match self.sketch(target)? {
                    Ok(sketch) => Some(MirrorPlane::Sketch {
                        sketch,
                        axis: Some(if *axis == "H_Axis" {
                            SketchAxis::Horizontal
                        } else {
                            SketchAxis::Vertical
                        }),
                    }),
                    Err(lost) => return Ok(Err(lost)),
                }
            }
            _ => match self.face_pick(object, "MirrorPlane")? {
                Ok(pick) => Some(MirrorPlane::Face(pick)),
                Err(reason) => return Ok(Err(Lost::Setting(reason))),
            },
        };
        let Some(plane) = plane else {
            return Ok(Err(Lost::Setting(format!("mirror plane {target}"))));
        };
        Ok(Ok(DesignFeature::Mirrored {
            originals: Vec::new(),
            plane,
            refine: optional_bool(object, "Refine", false)?,
            boolean_tolerance: None,
            skipped: Vec::new(),
        }))
    }

    fn loft(&mut self, object: &FcObject, cut: bool) -> Result<Made, ResolveError> {
        let mut sections = Vec::new();
        match self.profile_sketch(object, "Profile")? {
            Ok(sketch) => sections.push(LoftSection::Feature(sketch)),
            Err(lost) => return Ok(Err(lost)),
        }
        for (name, sub) in object.link_sub_list("Sections")? {
            if !sub.is_empty() {
                return Ok(Err(Lost::Setting("section sub-elements".into())));
            }
            match self.sketch(name)? {
                Ok(sketch) => sections.push(LoftSection::Feature(sketch)),
                Err(lost) => return Ok(Err(lost)),
            }
        }
        Ok(Ok(DesignFeature::Loft {
            keep_inside: false,
            sections,
            ruled: optional_bool(object, "Ruled", false)?,
            closed: optional_bool(object, "Closed", false)?,
            subtractive: cut,
            refine: optional_bool(object, "Refine", false)?,
            boolean_tolerance: None,
        }))
    }

    /// The sketch a link names as a whole.
    fn profile_sketch(
        &mut self,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<FeatureId, Lost>, ResolveError> {
        match object.link_sub(property)? {
            None => Ok(Err(Lost::Setting(format!("missing {property}")))),
            Some((_, subs)) if !subs.is_empty() => {
                Ok(Err(Lost::Setting(format!("{property} sub-elements"))))
            }
            Some((name, _)) => self.sketch(name),
        }
    }

    fn sketch(&mut self, name: &str) -> Result<Result<FeatureId, Lost>, ResolveError> {
        Ok(match self.resolved(name)? {
            Some(ImportedObject::Sketch(id)) => Ok(id),
            _ => Err(Lost::Dependency(name.into())),
        })
    }

    /// The one face a link names, as `face_pick` for an extrusion's end.
    fn face_pick(
        &mut self,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<FacePick, String>, ResolveError> {
        Ok(match self.picks(object, property)? {
            Ok((edges, faces)) if edges.is_empty() && faces.len() == 1 => Ok(faces[0]),
            Ok(_) => Err(format!("{property} names other than one face")),
            Err(Lost::Setting(reason) | Lost::Dependency(reason)) => Err(reason),
        })
    }

    /// The edges and faces a link names (`Edge3`, `Face1`), found on the
    /// shape the file saved for the linked object, placed where the
    /// document places that object.
    fn picks(
        &mut self,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<Picks, Lost>, ResolveError> {
        let Some((target, subs)) = object.link_sub(property)? else {
            return Ok(Err(Lost::Setting(format!("{property} names nothing"))));
        };
        let Some(saved) = self.saved_shape(target) else {
            return Ok(Err(Lost::Setting(format!(
                "{property}: {target} has no saved shape to find {} on",
                subs.join(", ")
            ))));
        };
        let frame = self.parents.placement(self.source, target)?;
        let place = |(point, direction): Mark| {
            let point = frame.rotation.transform_vector(point);
            let origin = frame.position.to_array();
            (
                [0, 1, 2].map(|i| (point[i] + origin[i]) as f32),
                frame.rotation.transform_vector(direction).map(|v| v as f32),
            )
        };
        let (mut edges, mut faces) = (Vec::new(), Vec::new());
        for sub in subs {
            let numbered = |prefix: &str, marks: &[Option<Mark>]| {
                let index = sub.strip_prefix(prefix)?.parse::<usize>().ok()?;
                marks.get(index.checked_sub(1)?).copied().flatten()
            };
            if let Some(mark) = numbered("Edge", &saved.edges) {
                let (point, direction) = place(mark);
                edges.push(EdgePick {
                    point,
                    direction,
                    faces: Default::default(),
                });
            } else if let Some(mark) = numbered("Face", &saved.faces) {
                let (point, normal) = place(mark);
                faces.push(FacePick {
                    point,
                    normal,
                    name: Default::default(),
                });
            } else {
                return Ok(Err(Lost::Setting(format!(
                    "{property}: {sub} is not found on {target}'s saved shape"
                ))));
            }
        }
        Ok(Ok((edges, faces)))
    }

    /// The edges and faces of the shape the file saved for `name`, read once.
    fn saved_shape(&mut self, name: &str) -> Option<std::rc::Rc<SavedShape>> {
        let file = self.source.object(name)?.shape_file("Shape").ok()?;
        if let Some(read) = self.saved.get(file) {
            return read.clone();
        }
        let read = self
            .shapes
            .get(file)
            .and_then(|text| SavedShape::parse(text).ok())
            .map(std::rc::Rc::new);
        self.saved.insert(file.to_owned(), read.clone());
        read
    }
}

/// Why a feature stays out: a setting with no counterpart here, or an
/// input that did not come in.
enum Lost {
    Setting(String),
    Dependency(String),
}

type Made = Result<DesignFeature, Lost>;

/// Edges and faces picked on a saved shape.
type Picks = (Vec<EdgePick>, Vec<FacePick>);

/// An enumeration's chosen value by the name its list gives it, where the
/// property carries its list.
fn enum_name(object: &FcObject, name: &str) -> Result<Option<String>, FcstdError> {
    let index = object.integer(name)?;
    Ok(object
        .property(name)?
        .value
        .iter()
        .find(|element| element.tag == "CustomEnumList")
        .and_then(|list| {
            list.children_named("Enum")
                .nth(usize::try_from(index).ok()?)
        })
        .and_then(|value| value.attr("value"))
        .map(str::to_owned))
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
        let error = import_with_initializer(&source, &Shapes::new(), "empty", |_| {
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
