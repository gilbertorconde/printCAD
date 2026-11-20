//! A regression check over a folder of real project archives: each one
//! imported the way File › Import brings it in, every body rebuilt, and
//! each held to the solid the archive saved for it. Geometry results,
//! features the conversion could not carry and parametric data it lost
//! are kept apart, so a partial import never reads as a faithful one.

pub mod measure;
pub mod report;

use crate::document_import::{BodyKind, ConvertedBody, ImportedObject, ImportedProject};
use crate::error::{ErrorCategory, Failure};
use crate::report::Finding;
use build_driver::Chain;
use core_document::FeatureId;
use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;
use measure::{Difference, Measures, Tolerances, compare};
use project_archive::archive::FcstdArchive as Archive;
use project_archive::document_xml::{FcDocument, Placement};
use project_archive::traversal::ParentMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// What became of one rebuilt body against the archive's saved solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Rebuilt from its features and within every tolerance.
    Matches,
    /// Rebuilt, but some measure strays past its tolerance.
    Differs,
    /// Rebuilt, but the archive saved no solid to hold it to.
    NoReference,
    /// Its history did not build (the saved solid may stand in for it in
    /// the app, which this check never counts as a match).
    Failed,
    /// Kept as the solid the archive saved, without a history: nothing
    /// was rebuilt, so nothing is proved.
    SavedShape,
    /// A linked copy, which shows another body's solid and is measured
    /// there.
    LinkedCopy,
}

/// Where along a body's history its solid first strays from the archive's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Divergence {
    /// The source feature's name in the archive.
    pub feature: String,
    /// Whether the history up to it builds but differs from the solid the
    /// archive saved for that step, or does not build at all.
    pub kind: DivergenceKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub differences: Vec<Difference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Found by holding every step to its own saved solid, or only named
    /// by the build's refusal when the steps carry no saved solids.
    pub located_by: Located,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceKind {
    Differs,
    Fails,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Located {
    IntermediateShapes,
    BuildFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BodyResult {
    pub name: String,
    pub kind: BodyKind,
    /// The source object whose saved solid the body is held to.
    pub shape_of: String,
    pub features: usize,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub differences: Vec<Difference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<Measures>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built: Option<Measures>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_divergence: Option<Divergence>,
}

/// How a conversion finding bears on the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// A feature, setting or curve the conversion skipped, kept as a
    /// saved solid, or approximated: the geometry may not be the source's.
    Unsupported,
    /// The geometry came through but something that drives it did not:
    /// constraints, expressions, attachments, links to other geometry,
    /// group structure.
    LostParametric,
    /// Presentation only (where a dimension's label sits).
    Cosmetic,
}

/// The kind of a conversion finding, by its category. A category this
/// does not know counts as unsupported, so a new kind of loss is never
/// hidden.
pub fn finding_kind(category: &str) -> FindingKind {
    match category {
        "constraint" | "expression" | "attachment" | "external geometry" | "group" => {
            FindingKind::LostParametric
        }
        "dimension label" => FindingKind::Cosmetic,
        _ => FindingKind::Unsupported,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveStatus {
    /// Every body rebuilt and matched, with nothing unsupported or lost.
    Complete,
    /// It imported, but some body does not match or something was lost.
    Partial,
    /// The archive could not be read or converted, or the check crashed.
    Failed,
    /// The check ran past the time limit and was stopped.
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveReport {
    pub file: String,
    pub status: ArchiveStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub writer_version: String,
    #[serde(default)]
    pub bodies: Vec<BodyResult>,
    #[serde(default)]
    pub unsupported: Vec<Finding>,
    #[serde(default)]
    pub lost_parametric: Vec<Finding>,
    #[serde(default)]
    pub cosmetic: Vec<Finding>,
    #[serde(default)]
    pub seconds: f64,
}

impl ArchiveReport {
    /// An archive the check could not finish.
    pub fn failed(file: &Path, status: ArchiveStatus, error: impl Into<String>) -> Self {
        Self {
            file: file.display().to_string(),
            status,
            error: Some(error.into()),
            writer_version: String::new(),
            bodies: Vec::new(),
            unsupported: Vec::new(),
            lost_parametric: Vec::new(),
            cosmetic: Vec::new(),
            seconds: 0.0,
        }
    }
}

/// Import the archive at `path`, rebuild every body and hold each to its
/// saved solid. Nothing it reads stops it with a panic: a broken archive
/// comes back as a failed report.
pub fn check_archive(path: &Path, tolerances: &Tolerances) -> ArchiveReport {
    let started = std::time::Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(path, tolerances)));
    let mut report = match result {
        Ok(Ok(report)) => report,
        Ok(Err(failure)) => ArchiveReport::failed(path, ArchiveStatus::Failed, failure.to_string()),
        Err(panic) => ArchiveReport::failed(
            path,
            ArchiveStatus::Failed,
            format!("panic: {}", crate::document_import::panic_message(&panic)),
        ),
    };
    report.seconds = started.elapsed().as_secs_f64();
    report
}

fn check(path: &Path, tolerances: &Tolerances) -> Result<ArchiveReport, Failure> {
    let bytes = std::fs::read(path)
        .map_err(|e| Failure::new(ErrorCategory::Archive, format!("{}: {e}", path.display())))?;
    let archive =
        Archive::from_bytes(&bytes).map_err(|e| Failure::new(ErrorCategory::Archive, e))?;
    let source = FcDocument::parse(&archive.document_xml)
        .map_err(|e| Failure::new(ErrorCategory::Parse, e))?;
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    let project = crate::document_import::import_document(&source, &archive.shapes, &name)?;
    let parents =
        ParentMap::build(&source).map_err(|e| Failure::new(ErrorCategory::Conversion, e))?;
    let mut checker = Checker {
        source: &source,
        archive: &archive,
        parents: &parents,
        project: &project,
        tolerances,
        kernel: OgeomKernel::new(),
        saved: BTreeMap::new(),
    };
    let bodies = project
        .bodies
        .iter()
        .map(|body| checker.body(body))
        .collect::<Vec<_>>();
    let mut report = ArchiveReport {
        file: path.display().to_string(),
        status: ArchiveStatus::Partial,
        error: None,
        writer_version: source.program_version.clone(),
        bodies,
        unsupported: Vec::new(),
        lost_parametric: Vec::new(),
        cosmetic: Vec::new(),
        seconds: 0.0,
    };
    let sketch_findings = project.sketches.values().flat_map(|s| &s.findings);
    for finding in project.findings.iter().chain(sketch_findings) {
        match finding_kind(&finding.category) {
            FindingKind::Unsupported => report.unsupported.push(finding.clone()),
            FindingKind::LostParametric => report.lost_parametric.push(finding.clone()),
            FindingKind::Cosmetic => report.cosmetic.push(finding.clone()),
        }
    }
    if !report.bodies.is_empty()
        && report.bodies.iter().all(|b| b.verdict == Verdict::Matches)
        && report.unsupported.is_empty()
        && report.lost_parametric.is_empty()
    {
        report.status = ArchiveStatus::Complete;
    }
    Ok(report)
}

struct Checker<'a> {
    source: &'a FcDocument,
    archive: &'a Archive,
    parents: &'a ParentMap,
    project: &'a ImportedProject,
    tolerances: &'a Tolerances,
    kernel: OgeomKernel,
    /// The saved solid of each source object measured so far, or why it
    /// could not be.
    saved: BTreeMap<String, Option<Result<Measures, String>>>,
}

impl Checker<'_> {
    fn body(&mut self, body: &ConvertedBody) -> BodyResult {
        let mut result = BodyResult {
            name: body.name.clone(),
            kind: body.kind,
            shape_of: body.shape_of.clone(),
            features: body.features.len(),
            verdict: Verdict::NoReference,
            differences: Vec::new(),
            failure: None,
            saved: None,
            built: None,
            first_divergence: None,
        };
        match body.kind {
            BodyKind::SavedShape => {
                result.verdict = Verdict::SavedShape;
                return result;
            }
            BodyKind::LinkedCopy => {
                result.verdict = Verdict::LinkedCopy;
                return result;
            }
            BodyKind::Features => {}
        }
        let saved = self.saved_measures(&body.shape_of);
        if let Some(Ok(saved)) = saved {
            result.saved = Some(saved);
        }
        if let Some(failure) = self.project.build_failures.get(&body.shape_of) {
            result.verdict = Verdict::Failed;
            result.failure = Some(failure.reason.clone());
        } else {
            match self.built_measures(body) {
                Err(reason) => {
                    result.verdict = Verdict::Failed;
                    result.failure = Some(reason);
                }
                Ok(built) => {
                    result.built = Some(built);
                    match saved {
                        None => result.verdict = Verdict::NoReference,
                        Some(Err(reason)) => {
                            result.verdict = Verdict::Failed;
                            result.failure = Some(format!("saved solid: {reason}"));
                        }
                        Some(Ok(saved)) => {
                            result.differences = compare(&saved, &built, self.tolerances);
                            result.verdict = if result.differences.is_empty() {
                                Verdict::Matches
                            } else {
                                Verdict::Differs
                            };
                        }
                    }
                }
            }
        }
        if matches!(result.verdict, Verdict::Differs | Verdict::Failed) {
            result.first_divergence = self.divergence(body);
        }
        result
    }

    /// The body's own rebuilt solid, measured.
    fn built_measures(&mut self, body: &ConvertedBody) -> Result<Measures, String> {
        let project = self.project;
        let document = &project.document;
        let geometry = document
            .derived()
            .imported_geometry(body.body)
            .ok_or("the body has no solid")?;
        let blob = document
            .derived()
            .imported_brep_blob(body.body)
            .ok_or("the body has no solid")?;
        Measures::of_blob(
            &mut self.kernel,
            blob,
            geometry.bounds_mm,
            geometry.mesh.face_surfaces.len(),
        )
    }

    /// The solid the archive saved for `object`, placed where its groups
    /// put it and measured; `None` when it saved none.
    fn saved_measures(&mut self, object: &str) -> Option<Result<Measures, String>> {
        if let Some(known) = self.saved.get(object) {
            return known.clone();
        }
        let measured = self.measure_saved(object);
        self.saved.insert(object.into(), measured.clone());
        measured
    }

    fn measure_saved(&mut self, object: &str) -> Option<Result<Measures, String>> {
        let text = self
            .source
            .object(object)
            .and_then(|o| o.shape_file("Shape").ok())
            .and_then(|file| self.archive.shape(file).ok())?;
        if text.trim().is_empty() {
            return None;
        }
        let frame: Placement = match self.parents.placement(self.source, object) {
            Ok(frame) => frame,
            Err(e) => return Some(Err(e.to_string())),
        };
        let measured = project_archive::restate::restate(text, frame)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                self.kernel
                    .read_brep(&text, &TessellationSettings::default())
                    .map_err(|e| e.to_string())
            })
            .and_then(|read| {
                let bounds = read.bounds_mm.or_else(|| read.mesh.bounds());
                Measures::of_blob(
                    &mut self.kernel,
                    &read.brep_blob,
                    bounds,
                    read.mesh.face_surfaces.len(),
                )
            });
        Some(measured)
    }

    /// The first source feature of `body` whose step strays from the solid
    /// the archive saved for it, building the history a step at a time;
    /// when no step carries a saved solid, the feature the build stops at.
    fn divergence(&mut self, body: &ConvertedBody) -> Option<Divergence> {
        let project = self.project;
        let document = &project.document;
        let names: BTreeMap<FeatureId, &str> = project
            .objects
            .iter()
            .filter_map(|(name, object)| match object {
                ImportedObject::Solid {
                    body: b,
                    feature: Some(feature),
                } if *b == body.body => Some((*feature, name.as_str())),
                _ => None,
            })
            .collect();
        let name_of = |feature: Option<FeatureId>| {
            feature
                .and_then(|f| names.get(&f).map(|n| n.to_string()))
                .or_else(|| {
                    feature
                        .and_then(|f| document.get_feature_meta(f))
                        .map(|m| m.name.clone())
                })
                .unwrap_or_else(|| body.name.clone())
        };
        let plan = match wb_design::body_build_ops(document, body.body) {
            Ok(plan) => plan,
            Err(error) => {
                return Some(Divergence {
                    feature: name_of(error.feature),
                    kind: DivergenceKind::Fails,
                    differences: Vec::new(),
                    reason: Some(format!("planning: {error}")),
                    located_by: Located::BuildFailure,
                });
            }
        };
        let detail = TessellationSettings::default();
        let mut stepped = false;
        for name in &body.features {
            let Some(ImportedObject::Solid {
                feature: Some(feature),
                ..
            }) = project.objects.get(name)
            else {
                continue;
            };
            let Some(last) = plan.op_features.iter().rposition(|f| f == feature) else {
                continue;
            };
            let Some(Ok(saved)) = self.saved_measures(name) else {
                continue;
            };
            stepped = true;
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                build_driver::execute(&mut self.kernel, &up_to(&plan, last + 1, &detail), None)
            }));
            let built = match built {
                Ok(Ok(built)) => built,
                Ok(Err(error)) => {
                    return Some(Divergence {
                        feature: name_of(plan.op_features.get(error.op_index).copied()),
                        kind: DivergenceKind::Fails,
                        differences: Vec::new(),
                        reason: Some(error.to_string()),
                        located_by: Located::IntermediateShapes,
                    });
                }
                Err(panic) => {
                    return Some(Divergence {
                        feature: name.clone(),
                        kind: DivergenceKind::Fails,
                        differences: Vec::new(),
                        reason: Some(format!(
                            "kernel panic: {}",
                            crate::document_import::panic_message(&panic)
                        )),
                        located_by: Located::IntermediateShapes,
                    });
                }
            };
            let measured = Measures::of_blob(
                &mut self.kernel,
                &built.brep_blob,
                built.bounds_mm,
                built.mesh.face_surfaces.len(),
            );
            let differences = match measured {
                Ok(built) => compare(&saved, &built, self.tolerances),
                Err(reason) => {
                    return Some(Divergence {
                        feature: name.clone(),
                        kind: DivergenceKind::Fails,
                        differences: Vec::new(),
                        reason: Some(reason),
                        located_by: Located::IntermediateShapes,
                    });
                }
            };
            if !differences.is_empty() {
                return Some(Divergence {
                    feature: name.clone(),
                    kind: DivergenceKind::Differs,
                    differences,
                    reason: None,
                    located_by: Located::IntermediateShapes,
                });
            }
        }
        if stepped {
            return None;
        }
        // No step to hold the history to: name where the whole build stops.
        let failure = project.build_failures.get(&body.shape_of)?;
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            build_driver::execute(
                &mut self.kernel,
                &up_to(&plan, plan.ops.len(), &detail),
                None,
            )
        }));
        let feature = match built {
            Ok(Err(error)) => name_of(plan.op_features.get(error.op_index).copied()),
            _ => body.name.clone(),
        };
        Some(Divergence {
            feature,
            kind: DivergenceKind::Fails,
            differences: Vec::new(),
            reason: Some(failure.reason.clone()),
            located_by: Located::BuildFailure,
        })
    }
}

/// The first `ops` ops of `plan`, asked nothing else.
fn up_to<'a>(
    plan: &'a core_document::BuildPlan,
    ops: usize,
    detail: &'a TessellationSettings,
) -> Chain<'a> {
    Chain {
        ops: &plan.ops[..ops],
        op_features: &plan.op_features[..ops],
        detail,
        range: None,
        probes: &[],
        projections: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_of_an_unknown_category_counts_as_unsupported() {
        assert_eq!(finding_kind("constraint"), FindingKind::LostParametric);
        assert_eq!(finding_kind("expression"), FindingKind::LostParametric);
        assert_eq!(finding_kind("dimension label"), FindingKind::Cosmetic);
        assert_eq!(finding_kind("saved shape"), FindingKind::Unsupported);
        assert_eq!(
            finding_kind("a category added later"),
            FindingKind::Unsupported
        );
    }
}
