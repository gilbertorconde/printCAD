//! The corpus report: its counts, the table printed for a person, and the
//! regressions against an earlier run's report.

use super::measure::Tolerances;
use super::{ArchiveReport, ArchiveStatus, Verdict};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// A whole run: every archive's report, the counts over them, and what
/// regressed since the baseline when one was given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusReport {
    pub tolerances: Tolerances,
    pub time_limit_s: f64,
    pub summary: Summary,
    pub archives: Vec<ArchiveReport>,
    #[serde(default)]
    pub regressions: Vec<String>,
}

impl CorpusReport {
    pub fn new(tolerances: Tolerances, time_limit_s: f64, archives: Vec<ArchiveReport>) -> Self {
        Self {
            tolerances,
            time_limit_s,
            summary: Summary::of(&archives),
            archives,
            regressions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub archives: usize,
    pub complete: usize,
    pub partial: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub bodies: usize,
    pub matched: usize,
    pub differ: usize,
    pub bodies_failed: usize,
    pub no_reference: usize,
    pub saved_shapes: usize,
    pub linked_copies: usize,
    pub unsupported: usize,
    pub lost_parametric: usize,
}

impl Summary {
    pub fn of(archives: &[ArchiveReport]) -> Self {
        let mut summary = Self {
            archives: archives.len(),
            ..Self::default()
        };
        for archive in archives {
            *match archive.status {
                ArchiveStatus::Complete => &mut summary.complete,
                ArchiveStatus::Partial => &mut summary.partial,
                ArchiveStatus::Failed => &mut summary.failed,
                ArchiveStatus::TimedOut => &mut summary.timed_out,
            } += 1;
            summary.unsupported += archive.unsupported.len();
            summary.lost_parametric += archive.lost_parametric.len();
            for body in &archive.bodies {
                summary.bodies += 1;
                *match body.verdict {
                    Verdict::Matches => &mut summary.matched,
                    Verdict::Differs => &mut summary.differ,
                    Verdict::Failed => &mut summary.bodies_failed,
                    Verdict::NoReference => &mut summary.no_reference,
                    Verdict::SavedShape => &mut summary.saved_shapes,
                    Verdict::LinkedCopy => &mut summary.linked_copies,
                } += 1;
            }
        }
        summary
    }

    /// One line of the counts that matter.
    pub fn line(&self) -> String {
        format!(
            "{} matched / {} differ / {} failed / {} no reference / {} kept as saved solids \
             ({} bodies); archives: {} complete, {} partial, {} failed, {} timed out; \
             {} unsupported, {} lost parametric",
            self.matched,
            self.differ,
            self.bodies_failed,
            self.no_reference,
            self.saved_shapes,
            self.bodies,
            self.complete,
            self.partial,
            self.failed,
            self.timed_out,
            self.unsupported,
            self.lost_parametric
        )
    }
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Matches => "matches",
        Verdict::Differs => "differs",
        Verdict::NoReference => "no reference",
        Verdict::Failed => "failed",
        Verdict::SavedShape => "saved solid",
        Verdict::LinkedCopy => "linked copy",
    }
}

/// The report as a plain table: a line per body under its archive's line,
/// then what each archive could not carry over.
pub fn table(report: &CorpusReport) -> String {
    let mut rows: Vec<[String; 4]> = Vec::new();
    for archive in &report.archives {
        let name = std::path::Path::new(&archive.file)
            .file_name()
            .map_or(archive.file.clone(), |n| n.to_string_lossy().into_owned());
        let status = match archive.status {
            ArchiveStatus::Complete => "complete",
            ArchiveStatus::Partial => "partial",
            ArchiveStatus::Failed => "FAILED",
            ArchiveStatus::TimedOut => "TIMED OUT",
        };
        let mut detail = format!(
            "{} unsupported, {} lost parametric, {:.1} s",
            archive.unsupported.len(),
            archive.lost_parametric.len(),
            archive.seconds
        );
        if !archive.writer_version.is_empty() {
            write!(detail, ", written by {}", archive.writer_version).unwrap();
        }
        if let Some(error) = &archive.error {
            detail = error.clone();
        }
        rows.push([name, String::new(), status.into(), detail]);
        for body in &archive.bodies {
            let mut detail = body
                .differences
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            if let Some(failure) = &body.failure {
                detail = failure.clone();
            }
            if let Some(divergence) = &body.first_divergence {
                let how = match divergence.kind {
                    super::DivergenceKind::Differs => "first differs at",
                    super::DivergenceKind::Fails => "first fails at",
                };
                write!(detail, " | {how} {}", divergence.feature).unwrap();
                if let Some(first) = divergence.differences.first() {
                    write!(detail, ": {first}").unwrap();
                }
            }
            rows.push([
                String::new(),
                body.name.clone(),
                verdict_name(body.verdict).into(),
                detail,
            ]);
        }
    }
    let mut out = String::new();
    let widths: Vec<usize> = (0..3)
        .map(|i| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain([["archive", "body", "result"][i].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut line = |cells: [&str; 4]| {
        let mut text = String::new();
        for (i, width) in widths.iter().enumerate() {
            write!(text, "{:<width$}  ", cells[i]).unwrap();
        }
        writeln!(out, "{}", (text + cells[3]).trim_end()).unwrap();
    };
    line(["archive", "body", "result", "detail"]);
    for row in &rows {
        line([&row[0], &row[1], &row[2], &row[3]]);
    }
    for archive in &report.archives {
        for (heading, findings) in [
            ("unsupported", &archive.unsupported),
            ("lost parametric", &archive.lost_parametric),
        ] {
            for finding in findings {
                writeln!(
                    out,
                    "{heading}: {}: {} ({}): {}",
                    archive.file, finding.object, finding.category, finding.reason
                )
                .unwrap();
            }
        }
    }
    for regression in &report.regressions {
        writeln!(out, "REGRESSION: {regression}").unwrap();
    }
    writeln!(out, "{}", report.summary.line()).unwrap();
    out
}

/// What was right in `baseline` and is not in `current`: an archive that
/// imported and now fails or times out, a body that matched and now does
/// not. Archives are paired by file name, so a corpus may move between
/// runs; one missing from either run is not compared.
pub fn regressions(baseline: &CorpusReport, current: &CorpusReport) -> Vec<String> {
    let key = |file: &str| {
        std::path::Path::new(file)
            .file_name()
            .map_or(file.to_owned(), |n| n.to_string_lossy().into_owned())
    };
    let mut found = Vec::new();
    for now in &current.archives {
        let Some(before) = baseline
            .archives
            .iter()
            .find(|a| key(&a.file) == key(&now.file))
        else {
            continue;
        };
        let broken =
            |s: ArchiveStatus| matches!(s, ArchiveStatus::Failed | ArchiveStatus::TimedOut);
        if broken(now.status) && !broken(before.status) {
            found.push(format!(
                "{}: imported before, now {:?}: {}",
                key(&now.file),
                now.status,
                now.error.as_deref().unwrap_or("")
            ));
            continue;
        }
        for body in &before.bodies {
            if body.verdict != Verdict::Matches {
                continue;
            }
            let after = now
                .bodies
                .iter()
                .find(|b| b.name == body.name && b.shape_of == body.shape_of);
            match after {
                Some(after) if after.verdict == Verdict::Matches => {}
                Some(after) => found.push(format!(
                    "{}: {} matched before, now {}",
                    key(&now.file),
                    body.name,
                    verdict_name(after.verdict)
                )),
                None => found.push(format!(
                    "{}: {} matched before and is gone",
                    key(&now.file),
                    body.name
                )),
            }
        }
    }
    found
}
