//! A source body is one native body: its features in their history order,
//! its tip the source's. Objects outside bodies chain the same way, a
//! boolean continuing the body of its base when nothing else uses that
//! base. An object that cannot be rebuilt as features comes in as the
//! shape the file saved for it, and every body built from features keeps
//! its tip's saved shape to stand in while its history does not build.

use crate::error::{ErrorCategory, Failure, ImportError};
use crate::numeric::{design_length, frame, sketch_plane};
use crate::report::{Finding, note};
use crate::sketch_import::{ImportedSketch, import_sketch};
use core_document::{BodyId, BodyPlacement, Document, FeatureId, ImportedGeometry};
use kernel_api::{Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use project_archive::archive::FcstdError;
use project_archive::document_xml::{FcDocument, FcObject, Placement};
use project_archive::saved_shape::{Mark, SavedShape};
use project_archive::sketch_decode::FcSketch;
use project_archive::traversal::{ParentMap, TraversalPath};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use wb_design::{
    ChamferMode, DesignFeature, EdgePick, EdgeSel, FacePick, LoftSection, MirrorPlane, PatternAxis,
    SketchAxis, TransformStep,
};
use wb_sketch::SketchFeature;

pub struct ImportedProject {
    pub document: Document,
    pub sketches: BTreeMap<String, ImportedSketch>,
    pub objects: BTreeMap<String, ImportedObject>,
    pub bodies: Vec<ConvertedBody>,
    pub findings: Vec<Finding>,
    /// Bodies that did not build, by the object whose solid each is
    /// (`ConvertedBody::shape_of`), with why.
    pub build_failures: BTreeMap<String, Failure>,
    /// Bodies that show their stand-in, by the object whose solid each is.
    pub stood_in: BTreeSet<String>,
}

/// What a conversion added to a document, by source object name.
#[derive(Default)]
pub struct Conversion {
    pub sketches: BTreeMap<String, ImportedSketch>,
    pub objects: BTreeMap<String, ImportedObject>,
    pub findings: Vec<Finding>,
    /// Every body made, in the order made.
    pub bodies: Vec<ConvertedBody>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ImportedObject {
    Sketch(FeatureId),
    /// A solid: the body it stands in, and the feature it is there when it
    /// came in as one.
    Solid {
        body: BodyId,
        feature: Option<FeatureId>,
    },
}

impl ImportedObject {
    pub fn body(&self) -> Option<BodyId> {
        match self {
            Self::Solid { body, .. } => Some(*body),
            Self::Sketch(_) => None,
        }
    }
}

/// A body a conversion made.
#[derive(Debug, Clone, Serialize)]
pub struct ConvertedBody {
    pub body: BodyId,
    pub name: String,
    pub kind: BodyKind,
    /// The source object whose saved shape the body's solid is: its tip's.
    pub shape_of: String,
    /// The source objects that came in as its features, in history order.
    pub features: Vec<String>,
    /// The source body it holds the history of, when it came from one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_body: Option<String>,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    /// Built from its features.
    Features,
    /// The shape the file saved, with no history.
    SavedShape,
    /// A linked copy of another body, placed on its own.
    LinkedCopy,
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
/// its own mutators, one op each. An object the conversion cannot rebuild
/// comes in as the shape the file saved for it; one with no saved shape is
/// left out, with what depends on it only where that has none either, and
/// each is named in the findings. An error in the source's own data stops
/// the conversion part way, so a caller that must not keep a partial
/// result converts into a scratch document first.
pub fn convert_into(
    source: &FcDocument,
    shapes: &Shapes,
    document: &mut Document,
) -> Result<Conversion, FcstdError> {
    let mut importer = Importer {
        source,
        shapes,
        saved: BTreeMap::new(),
        restated: BTreeMap::new(),
        document,
        out: Conversion::default(),
        visited: BTreeSet::new(),
        rejected: BTreeMap::new(),
        parents: ParentMap::build(source)?,
        consumers: consumers(source),
        steps: BTreeSet::new(),
        segments: BTreeMap::new(),
        last_solid: BTreeMap::new(),
        saved_bodies: BTreeMap::new(),
    };
    for object in &source.objects {
        if object.type_name == "PartDesign::MultiTransform" && object.has("Transformations") {
            importer.steps.extend(
                object
                    .link_list("Transformations")?
                    .into_iter()
                    .map(str::to_owned),
            );
        }
    }
    for object in &source.objects {
        importer.resolve(&object.name)?;
    }
    // Texts no extrusion took come in as sketches of their own.
    for object in &source.objects {
        if is_text(object)
            && !importer.out.objects.contains_key(&object.name)
            && !importer.rejected.contains_key(&object.name)
        {
            importer.visited.remove(&object.name);
            importer.resolve(&object.name)?;
        }
    }
    importer.finish()?;
    Ok(importer.out)
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
        bodies: conversion.bodies,
        findings: conversion.findings,
        build_failures: BTreeMap::new(),
        stood_in: BTreeSet::new(),
    };
    let mut kernel = OgeomKernel::new();
    initialize(&mut kernel).map_err(ImportError::KernelInitialization)?;
    let detail = TessellationSettings::default();
    // Shapes read from the file first: bodies built from features may
    // start from them.
    for converted in &project.bodies {
        let Some(asset) = project
            .document
            .bodies()
            .iter()
            .find(|b| b.id == converted.body)
            .and_then(|b| b.shape_asset)
        else {
            continue;
        };
        match read_asset(&mut kernel, &project.document, asset, &detail) {
            Ok(geometry) => store(&mut project.document, converted.body, geometry),
            Err(reason) => {
                project
                    .build_failures
                    .insert(converted.shape_of.clone(), reason);
            }
        }
    }
    // Then the bodies built from features, each once what it takes from
    // other bodies is built.
    let mut pending: Vec<&ConvertedBody> = project
        .bodies
        .iter()
        .filter(|b| b.kind == BodyKind::Features && !b.features.is_empty())
        .collect();
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .position(|b| {
                taken_from(&project.document, b.body)
                    .iter()
                    .all(|source| !pending.iter().any(|p| p.body == *source))
            })
            .unwrap_or(0);
        let converted = pending.remove(ready);
        let body = converted.body;
        let document = &project.document;
        let named = |feature: Option<FeatureId>| {
            feature
                .and_then(|f| document.get_feature_meta(f))
                .map_or(String::new(), |n| format!(" {}", n.name))
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let plan = wb_design::body_build_ops(document, body).map_err(|e| {
                let at = named(e.feature);
                Failure::new(ErrorCategory::Rebuild, format!("planning{at}: {e}"))
            })?;
            kernel.execute_solid_chain(&plan.ops, &detail).map_err(|e| {
                let at = named(plan.op_features.get(e.op_index).copied());
                Failure::new(ErrorCategory::Rebuild, format!("kernel{at}: {e}"))
            })
        }));
        let failure = match result {
            Ok(Ok(built)) => {
                store(
                    &mut project.document,
                    body,
                    (
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
                        built.brep_blob,
                    ),
                );
                continue;
            }
            Ok(Err(reason)) => reason,
            Err(panic) => Failure::new(
                ErrorCategory::Rebuild,
                format!("kernel panic: {}", panic_message(&panic)),
            ),
        };
        // What the file saved stands in for a history that does not build.
        let stand_in = project
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .and_then(|b| b.stand_in);
        if let Some(stand_in) = stand_in
            && let Ok(geometry) =
                read_asset(&mut kernel, &project.document, stand_in.asset, &detail)
        {
            store(&mut project.document, body, geometry);
            project.stood_in.insert(converted.shape_of.clone());
        }
        project
            .build_failures
            .insert(converted.shape_of.clone(), failure);
    }
    Ok(project)
}

/// The bodies `body`'s features take solids from: a clone's source and
/// the tools of its booleans.
fn taken_from(document: &Document, body: BodyId) -> Vec<BodyId> {
    wb_design::design_features_of_body(document, body)
        .into_iter()
        .flat_map(|(_, feature)| match feature {
            DesignFeature::Clone { source } => vec![source],
            DesignFeature::BodyBoolean {
                tool_body,
                more_tools,
                ..
            } => std::iter::once(tool_body).chain(more_tools).collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// The shape asset `asset` of `document` holds, read by the kernel.
fn read_asset(
    kernel: &mut OgeomKernel,
    document: &Document,
    asset: uuid::Uuid,
    detail: &TessellationSettings,
) -> Result<(ImportedGeometry, Vec<u8>), Failure> {
    let failed = |e: String| Failure::new(ErrorCategory::Rebuild, format!("saved shape: {e}"));
    let (_, bytes) = document
        .asset_with_bytes(asset)
        .ok_or_else(|| failed("the shape is not kept".into()))?;
    let read = kernel
        .read_brep(&String::from_utf8_lossy(&bytes), detail)
        .map_err(|e| failed(e.to_string()))?;
    Ok((
        ImportedGeometry {
            bounds_mm: read.bounds_mm.or_else(|| read.mesh.bounds()),
            mesh: Arc::new(read.mesh),
            mesh_path: None,
            source_asset: Some(asset),
            revision: 0,
            brep_blob_path: None,
            face_colors_path: None,
            health: Some(read.health),
        },
        read.brep_blob,
    ))
}

fn store(document: &mut Document, body: BodyId, (geometry, blob): (ImportedGeometry, Vec<u8>)) {
    document.set_imported_geometry(body, geometry);
    document.set_imported_brep_data(body, blob, Vec::new());
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

/// The objects that take each object's solid (as their base, a tool, the
/// object they link to) or extrude it.
fn consumers(source: &FcDocument) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for object in &source.objects {
        let mut taken = BTreeSet::new();
        for property in ["BaseFeature", "Base", "Tool", "Shapes", "LinkedObject"] {
            taken.extend(linked(object, property));
        }
        for target in taken {
            out.entry(target).or_default().push(object.name.clone());
        }
    }
    out
}

/// The objects a link property of any kind names.
fn linked(object: &FcObject, property: &str) -> Vec<String> {
    let Ok(prop) = object.property(property) else {
        return Vec::new();
    };
    prop.value
        .iter()
        .flat_map(|element| match element.tag.as_str() {
            "Link" | "LinkSub" | "XLink" => element
                .attr("value")
                .or_else(|| element.attr("name"))
                .into_iter()
                .collect::<Vec<_>>(),
            "LinkList" | "LinkSubList" => element
                .children_named("Link")
                .filter_map(|l| l.attr("value").or_else(|| l.attr("obj")))
                .collect(),
            _ => Vec::new(),
        })
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_text(object: &FcObject) -> bool {
    object.has("String") && object.has("FontFile") && object.has("Size")
}

/// Kinds that are reference geometry: they come in as nothing.
fn is_reference(type_name: &str) -> bool {
    matches!(
        type_name,
        "App::Origin" | "App::Plane" | "App::Line" | "App::Point"
    )
}

struct Importer<'a> {
    source: &'a FcDocument,
    shapes: &'a Shapes,
    saved: BTreeMap<String, Option<Rc<SavedShape>>>,
    /// Saved shapes restated for the kernel, by entry name.
    restated: BTreeMap<String, Option<Rc<String>>>,
    document: &'a mut Document,
    out: Conversion,
    visited: BTreeSet<String>,
    // Keep the originating reason without nesting consumer messages. A walk
    // starting at resolved leaves can be long while using little pending depth.
    rejected: BTreeMap<String, String>,
    parents: ParentMap,
    consumers: BTreeMap<String, Vec<String>>,
    /// The transforms a multi-transform holds, which come in as its steps.
    steps: BTreeSet<String>,
    /// The body each source body's history goes on in now.
    segments: BTreeMap<String, BodyId>,
    /// The last solid object each body was built up to.
    last_solid: BTreeMap<BodyId, String>,
    /// Bodies of saved shapes, by the object whose shape they are.
    saved_bodies: BTreeMap<String, BodyId>,
}

/// Why an object does not come in as features: a setting with no
/// counterpart here, or an input that did not come in.
enum Lost {
    Setting(String),
    Dependency(String),
}

type Made = Result<DesignFeature, Lost>;

/// Edges and faces picked on a saved shape.
type Picks = (Vec<EdgePick>, Vec<FacePick>);

impl<'a> Importer<'a> {
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
            // A missing object is rejected; what waits on it goes on,
            // finding it skipped.
            let Some(object) = self.source.object(current) else {
                let reason = path.missing_target(current).to_string();
                self.visited.insert(current.into());
                self.rejected.insert(current.into(), reason);
                path.leave();
                continue;
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
                        self.reject_cycle(&mut path, &dependency, &error.to_string());
                    }
                }
                Err(ResolveError::Source(error)) => return Err(error),
            }
        }
        Ok(self.out.objects.get(name).copied())
    }

    // Every object of a cycle is rejected once, with the cycle's chain;
    // what waits on the cycle goes on, finding them skipped. Cached
    // rejection keeps later roots from retrying them or repeating findings.
    fn reject_cycle(&mut self, path: &mut TraversalPath, start: &str, reason: &str) {
        while let Some(name) = path.current() {
            let last = name == start;
            note(&mut self.out.findings, name, "dependency", reason);
            self.visited.insert(name.into());
            self.rejected.insert(name.into(), reason.into());
            path.leave();
            if last {
                break;
            }
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
        // A text an extrusion takes comes in with the extrusion's body.
        if is_text(object) && !self.out.objects.contains_key(name) && self.extruded(name) {
            return Ok(None);
        }
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
                let body = self.sketch_body(name)?;
                let id = self.add_sketch(
                    object,
                    SketchFeature::new(sketch.sketch.clone(), plane),
                    body,
                )?;
                self.out.sketches.insert(name.into(), sketch);
                Some(ImportedObject::Sketch(id))
            }
            _ if is_text(object) => {
                let placement = self
                    .parents
                    .placement(self.source, name)?
                    .compose(object.placement("Placement")?, name)?;
                match crate::text_import::text_sketch(
                    object,
                    placement,
                    self.document,
                    &mut self.out.findings,
                )? {
                    Ok(sketch) => Some(ImportedObject::Sketch(
                        self.add_sketch(object, sketch, None)?,
                    )),
                    Err(reason) => self.skip(object, reason),
                }
            }
            "PartDesign::Body" => match object.link("Tip")? {
                Some(tip) => match self.resolved(tip)? {
                    Some(imported) => Some(imported),
                    None => return Ok(self.skip_dependency(object, tip)),
                },
                None => None,
            },
            "App::Part" => {
                note(
                    &mut self.out.findings,
                    name,
                    "group",
                    "group hierarchy flattened; evaluated placement retained",
                );
                None
            }
            "App::Link" => self.link(object)?,
            type_name if is_reference(type_name) => None,
            _ if self.steps.contains(name) => None,
            type_name => match self.solid(object)? {
                Ok(imported) => Some(imported),
                Err(Lost::Setting(reason)) => {
                    let category = if reason == format!("unsupported {type_name}") {
                        "object"
                    } else {
                        "object setting"
                    };
                    self.fall_back(object, category, reason)
                }
                Err(Lost::Dependency(target)) => {
                    let cause = self.cause(&target);
                    let reason = format!("depends on skipped {target}: {cause}");
                    match self.saved_body(object, &reason)? {
                        Some(imported) => Some(imported),
                        None => {
                            note(&mut self.out.findings, name, "dependency", reason);
                            self.rejected.insert(name.into(), cause);
                            None
                        }
                    }
                }
            },
        };
        if let Some(value) = imported {
            self.out.objects.insert(name.into(), value);
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

    /// Add the sketch `object` became, shown as the file shows it.
    fn add_sketch(
        &mut self,
        object: &FcObject,
        sketch: SketchFeature,
        body: Option<BodyId>,
    ) -> Result<FeatureId, FcstdError> {
        let id = self
            .document
            .add_feature_in_body(sketch, label(object)?, body)
            .map_err(|e| object.format_error(e.to_string()))?;
        if !object.visible()? {
            self.document.set_feature_visible(id, false);
        }
        Ok(id)
    }

    /// Whether an extrusion takes the text `name` as its profile.
    fn extruded(&self, name: &str) -> bool {
        self.consumers.get(name).is_some_and(|users| {
            users.iter().any(|user| {
                self.source
                    .object(user)
                    .is_some_and(|u| u.type_name == "Part::Extrusion")
                    && !self.rejected.contains_key(user)
                    && !self.visited.contains(user)
            })
        })
    }

    fn cause(&self, target: &str) -> String {
        self.rejected
            .get(target)
            .cloned()
            .unwrap_or_else(|| format!("{target} has no compatible imported object"))
    }

    /// An object the conversion cannot rebuild: its saved shape, else left
    /// out with `category` and `reason`.
    fn fall_back(
        &mut self,
        object: &FcObject,
        category: &str,
        reason: String,
    ) -> Option<ImportedObject> {
        // Reference geometry's shape is no solid to keep.
        let datum = object.type_name.contains("Datum")
            || matches!(
                object.type_name.as_str(),
                "PartDesign::Plane"
                    | "PartDesign::Line"
                    | "PartDesign::Point"
                    | "PartDesign::CoordinateSystem"
            );
        match (!datum).then(|| self.saved_body(object, &reason)) {
            Some(Ok(Some(imported))) => Some(imported),
            _ => {
                note(&mut self.out.findings, &object.name, category, &reason);
                self.rejected.insert(object.name.clone(), reason);
                None
            }
        }
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
        let cause = self.cause(target);
        let reason = format!("depends on skipped {target}: {cause}");
        note(&mut self.out.findings, &object.name, "dependency", reason);
        self.rejected.insert(object.name.clone(), cause);
        // The container branch returns directly from import_ready.
        self.visited.insert(object.name.clone());
        None
    }

    /// The source body `name` belongs to, when it is in one.
    fn source_body(&self, name: &str) -> Option<&'a FcObject> {
        let source: &'a FcDocument = self.source;
        self.parents
            .parent(name)
            .and_then(|parent| source.object(parent))
            .filter(|parent| parent.type_name == "PartDesign::Body")
    }

    /// The body a sketch goes in: its source body's, made now if needed.
    fn sketch_body(&mut self, name: &str) -> Result<Option<BodyId>, FcstdError> {
        let Some(owner) = self.source_body(name) else {
            return Ok(None);
        };
        Ok(Some(self.segment(owner)?))
    }

    /// The body `owner`'s history goes on in, made empty if there is none.
    fn segment(&mut self, owner: &FcObject) -> Result<BodyId, FcstdError> {
        if let Some(&body) = self.segments.get(&owner.name) {
            return Ok(body);
        }
        let body = self.new_body(label(owner)?, BodyKind::Features, Some(&owner.name));
        self.segments.insert(owner.name.clone(), body);
        Ok(body)
    }

    fn new_body(&mut self, name: String, kind: BodyKind, owner: Option<&str>) -> BodyId {
        let body = self.document.create_body(Some(name.clone()));
        self.out.bodies.push(ConvertedBody {
            body,
            name,
            kind,
            shape_of: String::new(),
            features: Vec::new(),
            source_body: owner.map(str::to_owned),
            visible: true,
        });
        body
    }

    fn converted(&mut self, body: BodyId) -> &mut ConvertedBody {
        self.out
            .bodies
            .iter_mut()
            .find(|b| b.body == body)
            .expect("a body the conversion made")
    }

    /// The body `name`'s solid is, for another object to take: its body
    /// when nothing comes after it there, else its saved shape. `None`
    /// when neither can be had.
    fn solid_of(&mut self, name: &str, user: &str) -> Result<Result<BodyId, Lost>, ResolveError> {
        let Some(ImportedObject::Solid { body, feature }) = self.resolved(name)? else {
            return Ok(Err(Lost::Dependency(name.into())));
        };
        let object = self.source.object(name).expect("resolved source");
        // A source body's solid is its tip's; a feature of one is that
        // solid only when it is the tip.
        let at_tip = match self.source_body(name) {
            Some(owner) => owner.link("Tip")? == Some(name),
            None => true,
        };
        if feature.is_none() || object.type_name == "PartDesign::Body" || at_tip {
            return Ok(Ok(body));
        }
        let reason = format!("{user} takes it from the middle of its body's history");
        Ok(match self.saved_body(object, &reason)? {
            Some(imported) => Ok(imported.body().expect("a solid")),
            None => Err(Lost::Dependency(name.into())),
        })
    }

    /// A body of `object`'s saved shape, made once, the finding saying why.
    /// `None` when the file saved no solid or sheet for it.
    fn saved_body(
        &mut self,
        object: &FcObject,
        reason: &str,
    ) -> Result<Option<ImportedObject>, FcstdError> {
        let name = object.name.as_str();
        if let Some(&body) = self.saved_bodies.get(name) {
            return Ok(Some(ImportedObject::Solid {
                body,
                feature: None,
            }));
        }
        let Some(text) = self.restated(name) else {
            return Ok(None);
        };
        let body = self.new_body(label(object)?, BodyKind::SavedShape, None);
        self.document
            .shape_body_from_file(body, &format!("{name}.brep"), text.as_bytes().to_vec());
        self.converted(body).shape_of = name.into();
        self.saved_bodies.insert(name.into(), body);
        note(
            &mut self.out.findings,
            name,
            "saved shape",
            format!("kept as its saved shape: {reason}"),
        );
        Ok(Some(ImportedObject::Solid {
            body,
            feature: None,
        }))
    }

    /// The shape the file saved for `name`, restated for the kernel, when
    /// it holds faces.
    fn restated(&mut self, name: &str) -> Option<Rc<String>> {
        let file = self
            .source
            .object(name)?
            .shape_file("Shape")
            .ok()?
            .to_owned();
        if let Some(text) = self.restated.get(&file) {
            return text.clone();
        }
        let placement = self.parents.placement(self.source, name).ok()?;
        let text = self
            .saved_shape(name)
            .filter(|shape| !shape.faces.is_empty())
            .and_then(|_| {
                project_archive::restate::restate(self.shapes.get(&file)?, placement).ok()
            })
            .map(Rc::new);
        self.restated.insert(file, text.clone());
        text
    }

    /// An `App::Link`: a linked copy of the body of what it links to,
    /// placed where the link puts it.
    fn link(&mut self, object: &FcObject) -> Result<Option<ImportedObject>, ResolveError> {
        let property = object.property("LinkedObject")?;
        let Some(target) = property.value.iter().find(|e| e.tag == "XLink") else {
            return Ok(self.skip(object, "a link to nothing"));
        };
        if target.attr("file").is_some_and(|f| !f.is_empty()) {
            return Ok(self.skip(object, "a link to another file"));
        }
        let Some(linked) = target.attr("name").filter(|n| !n.is_empty()) else {
            return Ok(self.skip(object, "a link to nothing"));
        };
        let source = match self.solid_of(linked, &object.name)? {
            Ok(body) => body,
            Err(Lost::Dependency(target) | Lost::Setting(target)) => {
                return Ok(self.skip_dependency(object, &target));
            }
        };
        let name = label(object)?;
        let Some(copy) = self.document.create_linked_copy(source, Some(name.clone())) else {
            return Ok(self.skip(object, format!("{linked} cannot be copied")));
        };
        // The link stands where its own placement puts it, in place of
        // the linked object's, unless it carries that one along.
        let own = if object.has("LinkPlacement") {
            object.placement("LinkPlacement")?
        } else {
            object.placement("Placement")?
        };
        let link = self
            .parents
            .placement(self.source, &object.name)?
            .compose(own, &object.name)?;
        let linked_object = self.source.object(linked).expect("resolved source");
        let at = self
            .parents
            .placement(self.source, linked)?
            .compose(linked_object.placement("Placement")?, linked)?;
        let carried = object.has("LinkTransform") && object.bool("LinkTransform")?;
        let placement = if carried {
            to_glam(link)
        } else {
            to_glam(link) * to_glam(at).inverse()
        };
        let (_, rotation, translation) = placement.to_scale_rotation_translation();
        self.document.set_body_placement(
            copy,
            BodyPlacement::new(rotation.as_quat(), translation.as_vec3()),
        );
        self.out.bodies.push(ConvertedBody {
            body: copy,
            name,
            kind: BodyKind::LinkedCopy,
            shape_of: linked.into(),
            features: Vec::new(),
            source_body: None,
            visible: true,
        });
        Ok(Some(ImportedObject::Solid {
            body: copy,
            feature: None,
        }))
    }

    /// Put `feature`, which `object` became, into the body whose history it
    /// continues: its source body's, after its base; outside source bodies,
    /// the body of a base nothing else takes. Any other base starts a new
    /// body with a clone of it.
    fn place(
        &mut self,
        object: &FcObject,
        feature: DesignFeature,
        base: Option<&str>,
    ) -> Result<Result<ImportedObject, Lost>, ResolveError> {
        let name = object.name.as_str();
        let owner = self.source_body(name).map(|o| o.name.clone());
        let continued = match (base, &owner) {
            (None, Some(owner)) => self
                .segments
                .get(owner)
                .copied()
                .filter(|body| !self.last_solid.contains_key(body)),
            (Some(base), Some(owner)) => self
                .segments
                .get(owner)
                .copied()
                .filter(|body| self.last_solid.get(body).is_some_and(|last| last == base)),
            (Some(base), None) => self
                .out
                .objects
                .get(base)
                .and_then(ImportedObject::body)
                .filter(|body| {
                    self.last_solid.get(body).is_some_and(|last| last == base)
                        && self.converted_ref(*body).source_body.is_none()
                        && self
                            .consumers
                            .get(base)
                            .is_some_and(|users| users == &[name])
                }),
            (None, None) => None,
        };
        let body = match continued {
            Some(body) => body,
            None => {
                let clone = match base {
                    Some(base) => match self.solid_of(base, name)? {
                        Ok(source) => Some(source),
                        Err(lost) => return Ok(Err(lost)),
                    },
                    None => None,
                };
                let body = match &owner {
                    // A source body's history goes on in its body while that
                    // holds no solid yet, else in a new one.
                    Some(owner) => {
                        let owner = self.source.object(owner).expect("a parent");
                        let current = self.segment(owner)?;
                        if self.last_solid.contains_key(&current) {
                            let body =
                                self.new_body(label(owner)?, BodyKind::Features, Some(&owner.name));
                            self.segments.insert(owner.name.clone(), body);
                            body
                        } else {
                            current
                        }
                    }
                    None => self.new_body(label(object)?, BodyKind::Features, None),
                };
                if let Some(source) = clone {
                    self.document
                        .add_feature_in_body(
                            DesignFeature::Clone { source },
                            format!("{} base", label(object)?),
                            Some(body),
                        )
                        .map_err(|e| object.format_error(e.to_string()))?;
                }
                body
            }
        };
        let id = self
            .document
            .add_feature_in_body(feature, label(object)?, Some(body))
            .map_err(|e| object.format_error(e.to_string()))?;
        // Marked, the feature builds wherever the document is rebuilt.
        self.document.mark_feature_dirty(id);
        self.last_solid.insert(body, name.into());
        let converted = self.converted(body);
        converted.features.push(name.into());
        converted.shape_of = name.into();
        Ok(Ok(ImportedObject::Solid {
            body,
            feature: Some(id),
        }))
    }

    fn converted_ref(&self, body: BodyId) -> &ConvertedBody {
        self.out
            .bodies
            .iter()
            .find(|b| b.body == body)
            .expect("a body the conversion made")
    }

    /// A solid object as features, or why it cannot be.
    fn solid(&mut self, object: &FcObject) -> Result<Result<ImportedObject, Lost>, ResolveError> {
        let type_name = object.type_name.as_str();
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        let cut =
            type_name == "PartDesign::Pocket" || type_name.starts_with("PartDesign::Subtractive");
        let is_boolean = matches!(type_name, "Part::Fuse" | "Part::Cut" | "Part::Common");
        let supported = type_name == "PartDesign::Pad"
            || type_name == "PartDesign::Pocket"
            || type_name.starts_with("PartDesign::Additive")
            || type_name.starts_with("PartDesign::Subtractive")
            || is_boolean
            || matches!(
                type_name,
                "PartDesign::Fillet"
                    | "PartDesign::Chamfer"
                    | "PartDesign::Hole"
                    | "PartDesign::Mirrored"
                    | "PartDesign::LinearPattern"
                    | "PartDesign::PolarPattern"
                    | "PartDesign::MultiTransform"
                    | "PartDesign::FeatureBase"
                    | "PartDesign::Boolean"
                    | "Part::Box"
                    | "Part::Cylinder"
                    | "Part::Cone"
                    | "Part::Sphere"
                    | "Part::Torus"
                    | "Part::Extrusion"
            );
        if !supported {
            return lost(format!("unsupported {type_name}"));
        }
        if type_name == "Part::Extrusion" {
            return self.extrusion_body(object);
        }
        if is_boolean && !same_placement(object.placement("Placement")?, Placement::IDENTITY) {
            return lost("placement of the boolean result".into());
        }
        let base = if is_boolean {
            object.link("Base")?
        } else if object.has("BaseFeature") {
            object.link("BaseFeature")?
        } else {
            None
        };
        // Each input is resolved before anything is made.
        if let Some(base) = base {
            self.resolved(base)?;
        }
        if base.is_none()
            && (cut
                || is_boolean
                || matches!(
                    type_name,
                    "PartDesign::Fillet"
                        | "PartDesign::Chamfer"
                        | "PartDesign::Hole"
                        | "PartDesign::Mirrored"
                        | "PartDesign::LinearPattern"
                        | "PartDesign::PolarPattern"
                        | "PartDesign::MultiTransform"
                        | "PartDesign::FeatureBase"
                        | "PartDesign::Boolean"
                ))
        {
            return lost("missing base solid".into());
        }
        let feature = if is_boolean {
            let Some(tool) = object.link("Tool")? else {
                return lost("missing boolean tool".into());
            };
            let tool = match self.solid_of(tool, &object.name)? {
                Ok(body) => body,
                Err(lost) => return Ok(Err(lost)),
            };
            DesignFeature::BodyBoolean {
                tool_body: tool,
                kind: match type_name {
                    "Part::Fuse" => kernel_api::BoolKind::Fuse,
                    "Part::Cut" => kernel_api::BoolKind::Cut,
                    _ => kernel_api::BoolKind::Common,
                },
                more_tools: Vec::new(),
                refine: optional_bool(object, "Refine", false)?,
                boolean_tolerance: None,
            }
        } else if type_name == "PartDesign::FeatureBase" {
            // A body started from another solid takes a clone of it, which
            // `place` makes when the base is not this body's own.
            return self.based_on(object, base.expect("checked above"));
        } else if let Some(made) = self.design_feature(object, cut)? {
            match made {
                Ok(feature) => feature,
                Err(lost) => return Ok(Err(lost)),
            }
        } else {
            match self.primitive(object, cut)? {
                Ok(feature) => feature,
                Err(lost) => return Ok(Err(lost)),
            }
        };
        self.place(object, feature, base)
    }

    /// A source body started from another solid: its body begins with a
    /// clone of that solid's body.
    fn based_on(
        &mut self,
        object: &FcObject,
        base: &str,
    ) -> Result<Result<ImportedObject, Lost>, ResolveError> {
        let source = match self.solid_of(base, &object.name)? {
            Ok(source) => source,
            Err(lost) => return Ok(Err(lost)),
        };
        let Some(owner) = self.source_body(&object.name).cloned() else {
            return Ok(Err(Lost::Setting("a base outside a body".into())));
        };
        let body = self.segment(&owner)?;
        if self.last_solid.contains_key(&body) {
            return Ok(Err(Lost::Setting("a base after the body's start".into())));
        }
        let id = self
            .document
            .add_feature_in_body(DesignFeature::Clone { source }, label(object)?, Some(body))
            .map_err(|e| object.format_error(e.to_string()))?;
        self.document.mark_feature_dirty(id);
        self.last_solid.insert(body, object.name.clone());
        let converted = self.converted(body);
        converted.features.push(object.name.clone());
        converted.shape_of = object.name.clone();
        Ok(Ok(ImportedObject::Solid {
            body,
            feature: Some(id),
        }))
    }

    fn primitive(&mut self, object: &FcObject, cut: bool) -> Result<Made, ResolveError> {
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        for skew in ["FirstAngle", "SecondAngle"] {
            if object.has(skew) && object.float(skew)? != 0.0 {
                return lost(format!("unsupported {skew}"));
            }
        }
        let kind = object
            .type_name
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
            _ => return lost(format!("unsupported primitive {kind}")),
        };
        let pose = self
            .parents
            .placement(self.source, &object.name)?
            .compose(object.placement("Placement")?, &object.name)?;
        let [x_axis, _, z_axis] = frame(&object.name, pose)?;
        Ok(Ok(DesignFeature::Primitive {
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
        }))
    }

    /// An extrusion of a sketch or a text: a body of its own holding the
    /// sketch (a text's made here) and a pad of it.
    fn extrusion_body(
        &mut self,
        object: &FcObject,
    ) -> Result<Result<ImportedObject, Lost>, ResolveError> {
        let lost = |reason: &str| Ok(Err(Lost::Setting(reason.into())));
        let Some(profile) = object.link("Base")? else {
            return lost("an extrusion of nothing");
        };
        let Some(shape) = self.source.object(profile) else {
            return Ok(Err(Lost::Dependency(profile.into())));
        };
        let text = is_text(shape);
        if !text && shape.type_name != "Sketcher::SketchObject" {
            return lost("an extrusion of other than a sketch or a text");
        }
        if !same_placement(object.placement("Placement")?, Placement::IDENTITY) {
            return lost("placement of the extrusion's result");
        }
        for taper in ["TaperAngle", "TaperAngleRev"] {
            if object.has(taper) && object.float(taper)? != 0.0 {
                return lost("a tapered extrusion");
            }
        }
        if !text && !optional_bool(object, "Solid", false)? {
            return lost("an extrusion of outlines into sheets");
        }
        // Along the profile's normal, or a direction square to it.
        let placement = self
            .parents
            .placement(self.source, profile)?
            .compose(shape.placement("Placement")?, profile)?;
        let normal = placement.rotation.transform_vector([0.0, 0.0, 1.0]);
        let mode = if object.has("DirMode") {
            object.integer("DirMode")?
        } else {
            0
        };
        let direction = vector(object, "Dir")?;
        let along = glam::DVec3::from_array(direction).dot(glam::DVec3::from_array(normal));
        let flip = match mode {
            2 => false,
            0 if (along.abs() - glam::DVec3::from_array(direction).length()).abs() < 1e-9
                && along != 0.0 =>
            {
                along < 0.0
            }
            _ => return lost("an extrusion along other than its profile's normal"),
        } ^ optional_bool(object, "Reversed", false)?;
        let forward = object.float("LengthFwd")?;
        let back = if object.has("LengthRev") {
            object.float("LengthRev")?
        } else {
            0.0
        };
        let symmetric = optional_bool(object, "Symmetric", false)?;
        let (forward, back) = match (forward, back) {
            (f, b) if f == 0.0 && b == 0.0 && mode == 0 => (along.abs(), 0.0),
            (f, b) if f >= 0.0 && b >= 0.0 && f + b > 0.0 => (f, b),
            _ => return lost("an extrusion with no length"),
        };
        let name = object.name.as_str();
        let mut data = serde_json::json!({ "symmetric": symmetric, "refine": false });
        if forward > 0.0 && (back == 0.0 || symmetric) {
            data["mode"] = "Dimension".into();
            data["length"] = design_length(name, "LengthFwd", forward)?.into();
            data["reversed"] = flip.into();
        } else if forward == 0.0 {
            data["mode"] = "Dimension".into();
            data["length"] = design_length(name, "LengthRev", back)?.into();
            data["reversed"] = (!flip).into();
        } else {
            data["mode"] = "TwoLengths".into();
            data["length"] = design_length(name, "LengthFwd", forward)?.into();
            data["length2"] = design_length(name, "LengthRev", back)?.into();
            data["reversed"] = flip.into();
        }
        let made_text = if text && !self.out.objects.contains_key(profile) {
            match crate::text_import::text_sketch(
                shape,
                placement,
                self.document,
                &mut self.out.findings,
            )? {
                Ok(sketch) => Some(sketch),
                Err(reason) => return Ok(Err(Lost::Setting(reason))),
            }
        } else {
            None
        };
        if made_text.is_none() && !self.out.objects.contains_key(profile) {
            self.resolved(profile)?;
        }
        let body = self.new_body(label(object)?, BodyKind::Features, None);
        let sketch = match made_text {
            Some(sketch) => {
                let id = self.add_sketch(shape, sketch, Some(body))?;
                self.out
                    .objects
                    .insert(profile.into(), ImportedObject::Sketch(id));
                self.visited.insert(profile.into());
                id
            }
            None => match self.out.objects.get(profile) {
                Some(ImportedObject::Sketch(id)) => *id,
                _ => return Ok(Err(Lost::Dependency(profile.into()))),
            },
        };
        data["sketch"] = serde_json::to_value(sketch).expect("an id serializes");
        let feature: DesignFeature = serde_json::from_value(serde_json::json!({ "Pad": data }))
            .map_err(|e| object.format_error(e.to_string()))?;
        let id = self
            .document
            .add_feature_in_body(feature, label(object)?, Some(body))
            .map_err(|e| object.format_error(e.to_string()))?;
        self.document.mark_feature_dirty(id);
        self.last_solid.insert(body, name.into());
        let converted = self.converted(body);
        converted.features.push(name.into());
        converted.shape_of = name.into();
        Ok(Ok(ImportedObject::Solid {
            body,
            feature: Some(id),
        }))
    }

    /// Pads, pockets, dress-ups, holes, transforms, lofts and body booleans;
    /// `None` for any other kind.
    fn design_feature(
        &mut self,
        object: &FcObject,
        cut: bool,
    ) -> Result<Option<Made>, ResolveError> {
        Ok(Some(match object.type_name.as_str() {
            "PartDesign::Pad" | "PartDesign::Pocket" => self.extrusion(object, cut)?,
            "PartDesign::Fillet" | "PartDesign::Chamfer" => self.dress_up(object)?,
            "PartDesign::Hole" => self.hole(object)?,
            "PartDesign::Mirrored"
            | "PartDesign::LinearPattern"
            | "PartDesign::PolarPattern"
            | "PartDesign::MultiTransform" => self.transformed(object)?,
            "PartDesign::AdditiveLoft" | "PartDesign::SubtractiveLoft" => self.loft(object, cut)?,
            "PartDesign::Boolean" => self.body_boolean(object)?,
            _ => return Ok(None),
        }))
    }

    /// A boolean of the body's solid with whole other bodies.
    fn body_boolean(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let mut tools = Vec::new();
        for tool in object.link_list("Group")? {
            match self.solid_of(tool, &object.name)? {
                Ok(body) => tools.push(body),
                Err(lost) => return Ok(Err(lost)),
            }
        }
        let Some((&tool_body, more_tools)) = tools.split_first() else {
            return Ok(Err(Lost::Setting("a boolean with no tool body".into())));
        };
        let kind = match object.integer("Type")? {
            0 => kernel_api::BoolKind::Fuse,
            1 => kernel_api::BoolKind::Cut,
            2 => kernel_api::BoolKind::Common,
            other => return Ok(Err(Lost::Setting(format!("unsupported Type={other}")))),
        };
        Ok(Ok(DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools: more_tools.to_vec(),
            refine: optional_bool(object, "Refine", false)?,
            boolean_tolerance: None,
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

    /// A mirror or a pattern of chosen features, which sit earlier in the
    /// same body, or of the whole solid so far.
    fn transformed(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let whole = object.has("TransformMode") && object.integer("TransformMode")? == 1;
        let mut originals = Vec::new();
        if !whole {
            let body = self.source_body(&object.name).map(|o| o.name.clone());
            for original in object.link_list("Originals")? {
                let in_body = body
                    .as_ref()
                    .and_then(|owner| self.segments.get(owner))
                    .copied();
                match self.resolved(original)? {
                    Some(ImportedObject::Solid {
                        body,
                        feature: Some(feature),
                    }) if Some(body) == in_body => originals.push(feature),
                    Some(_) => {
                        return Ok(Err(Lost::Setting(format!(
                            "{original} is not a feature of the same body"
                        ))));
                    }
                    None => return Ok(Err(Lost::Dependency(original.into()))),
                }
            }
        }
        let refine = optional_bool(object, "Refine", false)?;
        if object.type_name == "PartDesign::MultiTransform" {
            let mut steps = Vec::new();
            for step in object.link_list("Transformations")? {
                let Some(step) = self.source.object(step) else {
                    return Ok(Err(Lost::Dependency(step.into())));
                };
                match self.step(object, step)? {
                    Ok(step) => steps.push(step),
                    Err(lost) => return Ok(Err(lost)),
                }
            }
            return Ok(Ok(DesignFeature::MultiTransform {
                originals,
                steps,
                refine,
                boolean_tolerance: None,
                skipped: Vec::new(),
            }));
        }
        Ok(match self.step(object, object)? {
            Err(lost) => Err(lost),
            Ok(TransformStep::Mirror { plane }) => Ok(DesignFeature::Mirrored {
                originals,
                plane,
                refine,
                boolean_tolerance: None,
                skipped: Vec::new(),
            }),
            Ok(TransformStep::Linear {
                axis,
                length,
                occurrences,
            }) => Ok(DesignFeature::LinearPattern {
                originals,
                axis,
                length: length.abs(),
                occurrences,
                spacing_mode: false,
                reversed: length < 0.0,
                refine,
                boolean_tolerance: None,
                spacings: Vec::new(),
                skipped: Vec::new(),
            }),
            Ok(TransformStep::Polar {
                axis,
                angle_deg,
                occurrences,
            }) => Ok(DesignFeature::PolarPattern {
                originals,
                axis,
                angle_deg: angle_deg.abs(),
                occurrences,
                reversed: angle_deg < 0.0,
                refine,
                boolean_tolerance: None,
                step_mode: false,
                angles: Vec::new(),
                skipped: Vec::new(),
            }),
            Ok(_) => Err(Lost::Setting(format!("unsupported {}", object.type_name))),
        })
    }

    /// One transform (`step`, `owner` itself or one of its steps) as a
    /// step: a mirror, or a linear or polar pattern its length or angle
    /// spanning the occurrences, a reversed one running the other way.
    fn step(
        &mut self,
        owner: &FcObject,
        step: &FcObject,
    ) -> Result<Result<TransformStep, Lost>, ResolveError> {
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        let occurrences = |o: &FcObject| -> Result<Result<u32, String>, FcstdError> {
            let n = o.integer("Occurrences")?;
            Ok(u32::try_from(n)
                .ok()
                .filter(|&n| n >= 1)
                .ok_or(format!("{n} occurrences")))
        };
        // A newer file's spacing between occurrences, in place of the span.
        let by_offset = step.has("Mode") && step.integer("Mode")? == 1;
        let reversed = optional_bool(step, "Reversed", false)?;
        Ok(Ok(match step.type_name.as_str() {
            "PartDesign::Mirrored" => match self.mirror_plane(owner, step)? {
                Ok(plane) => TransformStep::Mirror { plane },
                Err(lost) => return Ok(Err(lost)),
            },
            "PartDesign::LinearPattern" => {
                let axis = match self.pattern_axis(owner, step, "Direction")? {
                    Ok(axis) => axis,
                    Err(lost) => return Ok(Err(lost)),
                };
                let occurrences = match occurrences(step)? {
                    Ok(n) => n,
                    Err(why) => return lost(why),
                };
                let span = if by_offset {
                    step.float("Offset")? * f64::from(occurrences.saturating_sub(1))
                } else {
                    step.float("Length")?
                };
                let sense = if reversed { -1.0 } else { 1.0 };
                TransformStep::Linear {
                    axis,
                    length: design_length(&step.name, "Length", sense * span)?,
                    occurrences,
                }
            }
            "PartDesign::PolarPattern" => {
                let axis = match self.pattern_axis(owner, step, "Axis")? {
                    Ok(axis) => axis,
                    Err(lost) => return Ok(Err(lost)),
                };
                let occurrences = match occurrences(step)? {
                    Ok(n) => n,
                    Err(why) => return lost(why),
                };
                let span = if by_offset {
                    step.float("Offset")? * f64::from(occurrences.saturating_sub(1))
                } else {
                    step.float("Angle")?
                };
                let sense = if reversed { -1.0 } else { 1.0 };
                TransformStep::Polar {
                    axis,
                    angle_deg: design_length(&step.name, "Angle", sense * span)?,
                    occurrences,
                }
            }
            other => return lost(format!("unsupported transform {other}")),
        }))
    }

    /// A mirror's plane: one of the body's own planes, a sketch's plane or
    /// one of its axes, or a flat face of the saved shape.
    fn mirror_plane(
        &mut self,
        owner: &FcObject,
        object: &FcObject,
    ) -> Result<Result<MirrorPlane, Lost>, ResolveError> {
        let Some((target, subs)) = object.link_sub("MirrorPlane")? else {
            return Ok(Err(Lost::Setting("missing MirrorPlane".into())));
        };
        let Some(plane) = self.source.object(target) else {
            return Ok(Err(Lost::Dependency(target.into())));
        };
        let plane = match (plane.type_name.as_str(), subs.as_slice()) {
            ("App::Plane", []) => match self.base_axis(owner, plane, target)? {
                Some(0) => Some(MirrorPlane::YZ),
                Some(1) => Some(MirrorPlane::XZ),
                Some(2) => Some(MirrorPlane::XY),
                _ => None,
            },
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
        Ok(plane.ok_or_else(|| Lost::Setting(format!("mirror plane {target}"))))
    }

    /// A pattern's direction or axis: one of the body's own axes, a
    /// sketch's axis, or an edge of the saved shape.
    fn pattern_axis(
        &mut self,
        owner: &FcObject,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<PatternAxis, Lost>, ResolveError> {
        let Some((target, subs)) = object.link_sub(property)? else {
            return Ok(Err(Lost::Setting(format!("missing {property}"))));
        };
        let Some(reference) = self.source.object(target) else {
            return Ok(Err(Lost::Dependency(target.into())));
        };
        let axis = match (reference.type_name.as_str(), subs.as_slice()) {
            ("App::Line", []) => match self.base_axis(owner, reference, target)? {
                Some(0) => Some(PatternAxis::X),
                Some(1) => Some(PatternAxis::Y),
                Some(2) => Some(PatternAxis::Z),
                _ => None,
            },
            ("Sketcher::SketchObject", [axis @ ("H_Axis" | "V_Axis" | "N_Axis")]) => {
                match self.sketch(target)? {
                    Ok(sketch) => Some(PatternAxis::Sketch {
                        sketch,
                        axis: match *axis {
                            "H_Axis" => SketchAxis::Horizontal,
                            "V_Axis" => SketchAxis::Vertical,
                            _ => SketchAxis::Normal,
                        },
                    }),
                    Err(lost) => return Ok(Err(lost)),
                }
            }
            _ => match self.picks(object, property)? {
                Ok((edges, faces)) if edges.len() == 1 && faces.is_empty() => {
                    Some(PatternAxis::Edge(edges[0]))
                }
                Ok(_) => None,
                Err(lost) => return Ok(Err(lost)),
            },
        };
        Ok(axis.ok_or_else(|| Lost::Setting(format!("{property} {target}"))))
    }

    /// Which of the world's axes (0 x, 1 y, 2 z) one of a body's own planes
    /// or axes stands for: a plane by its normal, an axis by its direction.
    /// `None` when the body has been moved, so they no longer stand where
    /// the world's do.
    fn base_axis(
        &self,
        owner: &FcObject,
        reference: &FcObject,
        target: &str,
    ) -> Result<Option<usize>, FcstdError> {
        if !same_placement(
            self.parents.placement(self.source, &owner.name)?,
            Placement::IDENTITY,
        ) {
            return Ok(None);
        }
        let role = if reference.has("Role") {
            reference.string("Role")?
        } else {
            target
        };
        Ok(match role {
            role if role.starts_with("X_") || role.starts_with("YZ") => Some(0),
            role if role.starts_with("Y_") || role.starts_with("XZ") => Some(1),
            role if role.starts_with("Z_") || role.starts_with("XY") => Some(2),
            _ => None,
        })
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
    fn saved_shape(&mut self, name: &str) -> Option<Rc<SavedShape>> {
        let file = self.source.object(name)?.shape_file("Shape").ok()?;
        if let Some(read) = self.saved.get(file) {
            return read.clone();
        }
        let read = self
            .shapes
            .get(file)
            .and_then(|text| SavedShape::parse(text).ok())
            .map(Rc::new);
        self.saved.insert(file.to_owned(), read.clone());
        read
    }

    /// Name, show and tip every body as its source has it, and keep each
    /// history's saved shape to stand in while it does not build.
    fn finish(&mut self) -> Result<(), FcstdError> {
        for index in 0..self.out.bodies.len() {
            let converted = self.out.bodies[index].clone();
            let (name, visible, tip) = self.presented(&converted)?;
            self.document.rename_body(converted.body, &name);
            self.document.set_body_visible(converted.body, visible);
            let body = &mut self.out.bodies[index];
            body.name = name;
            body.visible = visible;
            if converted.kind != BodyKind::Features {
                continue;
            }
            let Some(tip) = tip.or_else(|| converted.features.last().cloned()) else {
                continue;
            };
            body.shape_of = tip.clone();
            let Some(ImportedObject::Solid {
                feature: Some(feature),
                ..
            }) = self.out.objects.get(&tip).copied()
            else {
                continue;
            };
            self.document.set_body_tip(converted.body, Some(feature));
            if let Some(text) = self.restated(&tip) {
                self.document.set_body_stand_in(
                    converted.body,
                    feature,
                    &format!("{tip}.brep"),
                    text.as_bytes().to_vec(),
                );
            }
        }
        Ok(())
    }

    /// A body's name, whether it shows, and the object its tip is, as its
    /// source has them. A source body shows on its body holding its tip;
    /// the rest of its history, ahead of a shape it could not rebuild, is
    /// named for where it stops and hidden, unless nothing holds the tip. The saved shape a source
    /// body's tip fell back to takes the body's name and shows as it does;
    /// any other body takes the name of the object its solid is.
    fn presented(
        &self,
        converted: &ConvertedBody,
    ) -> Result<(String, bool, Option<String>), FcstdError> {
        if let Some(owner) = converted.source_body.as_deref() {
            let owner = self.source.object(owner).expect("a parent");
            let tip = owner.link("Tip")?.map(str::to_owned);
            if tip
                .as_ref()
                .is_some_and(|tip| converted.features.contains(tip))
            {
                return Ok((label(owner)?, owner.visible()?, tip));
            }
            let last = converted.features.last().cloned();
            // With its tip left out, the source body is what came in last.
            let tip_kept = tip.as_ref().is_some_and(|tip| {
                self.saved_bodies.contains_key(tip)
                    || self.out.bodies.iter().any(|b| b.features.contains(tip))
            });
            if !tip_kept && self.segments.get(&owner.name) == Some(&converted.body) {
                return Ok((label(owner)?, owner.visible()?, last));
            }
            let name = match &last {
                Some(last) => format!(
                    "{} up to {}",
                    label(owner)?,
                    label(self.source.object(last).expect("a feature"))?
                ),
                None => format!("{} sketches", label(owner)?),
            };
            return Ok((name, false, last));
        }
        if converted.kind == BodyKind::LinkedCopy {
            let link = self
                .out
                .objects
                .iter()
                .find(|(_, o)| o.body() == Some(converted.body))
                .and_then(|(name, _)| self.source.object(name));
            let visible = match link {
                Some(link) => link.visible()?,
                None => true,
            };
            return Ok((converted.name.clone(), visible, None));
        }
        let Some(object) = self.source.object(&converted.shape_of) else {
            return Ok((converted.name.clone(), converted.visible, None));
        };
        if converted.kind == BodyKind::SavedShape
            && let Some(owner) = self.source_body(&object.name)
            && owner.link("Tip")? == Some(object.name.as_str())
        {
            return Ok((label(owner)?, owner.visible()?, None));
        }
        // A chain of objects outside bodies is named for where it ends.
        Ok((label(object)?, object.visible()?, None))
    }
}

/// A vector property, `[0; 3]` when the object has none.
fn vector(object: &FcObject, name: &str) -> Result<[f64; 3], FcstdError> {
    if !object.has(name) {
        return Ok([0.0; 3]);
    }
    let property = object.property(name)?;
    let Some(element) = property.value.iter().find(|e| e.tag == "PropertyVector") else {
        return Err(object.format_error(format!("property {name}: not a vector")));
    };
    let mut out = [0.0; 3];
    for (slot, attr) in out.iter_mut().zip(["valueX", "valueY", "valueZ"]) {
        *slot = object.attr_f64(name, element, attr)?;
    }
    Ok(out)
}

fn to_glam(placement: Placement) -> glam::DAffine3 {
    glam::DAffine3::from_rotation_translation(
        glam::DQuat::from_array(placement.rotation.to_array()),
        glam::DVec3::from_array(placement.position.to_array()),
    )
}

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
