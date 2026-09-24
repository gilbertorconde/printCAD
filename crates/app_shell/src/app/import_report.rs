//! The file a STEP import's warnings go to, in the shape the kernel's
//! maintainer wants to read: what was read, with what, how it went by kind,
//! then every line.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kernel_api::ImportReport;

/// Writes the report and returns where it went.
pub(crate) fn write(
    source: &Path,
    source_bytes: usize,
    bodies: usize,
    report: &ImportReport,
) -> Result<PathBuf, String> {
    // The temp dir, not state: a report is read once and sent once, and the
    // system clears temp on its own instead of collecting one per import
    // forever.
    let dir = std::env::temp_dir().join("printcad").join("import-reports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "import".to_string());
    let path = dir.join(format!("{stem}-{}.txt", stamp(now)));
    // The report travels away from this machine; a path relative to wherever
    // the app happened to be started says nothing there.
    let absolute = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    std::fs::write(&path, render(&absolute, source_bytes, bodies, now, report))
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// The report as text: a header, the warnings by kind, the faces that
/// will draw with gaps, what the reader skipped, then every warning.
pub(crate) fn render(
    source: &Path,
    source_bytes: usize,
    bodies: usize,
    unix_secs: u64,
    report: &ImportReport,
) -> String {
    let mut out = String::new();
    out.push_str("printCAD import report\n");
    out.push_str("===========================\n");
    out.push_str(&format!("source           {}\n", source.display()));
    out.push_str(&format!(
        "size             {} bytes ({:.1} MB)\n",
        source_bytes,
        source_bytes as f64 / (1024.0 * 1024.0)
    ));
    out.push_str(&format!("read on          {} UTC\n", stamp(unix_secs)));
    out.push_str(&format!("kernel           {}\n", report.kernel));
    out.push_str(&format!("bodies           {bodies}\n"));
    out.push_str(&format!("warnings         {}\n", report.warnings.len()));
    out.push_str(&format!(
        "untrimmed faces  {}\n",
        report.untrimmed_faces.len()
    ));

    if !report.summary.is_empty() {
        out.push_str("\nBy kind: count, worst value (mm), one entity to look at first\n");
        for kind in &report.summary {
            out.push_str(&format!(
                "  {:<16} {:>7}   {:<10}  #{}\n",
                kind.kind,
                kind.count,
                if kind.worst > 0.0 {
                    format!("{:.3e}", kind.worst)
                } else {
                    "-".to_string()
                },
                kind.exemplar
            ));
        }
    }

    if !report.untrimmed_faces.is_empty() {
        out.push_str("\nFaces read without a complete trim (STEP entity ids)\n");
        let ids: Vec<String> = report
            .untrimmed_faces
            .iter()
            .map(|id| format!("#{id}"))
            .collect();
        for line in ids.chunks(8) {
            out.push_str(&format!("  {}\n", line.join(" ")));
        }
    }

    if !report.skipped.is_empty() {
        out.push_str("\nEntity keywords the reader never visited\n");
        for (keyword, count) in &report.skipped {
            out.push_str(&format!("  {count:>7}  {keyword}\n"));
        }
    }

    if !report.warnings.is_empty() {
        out.push_str("\nAll warnings, in the order the reader met them\n");
        for warning in &report.warnings {
            out.push_str(warning);
            out.push('\n');
        }
    }
    out
}

/// `YYYYMMDD-HHMMSS` from seconds since the epoch, without a date crate:
/// the days-to-civil arithmetic is a dozen lines and this is the only
/// place that needs it.
fn stamp(unix_secs: u64) -> String {
    let days = unix_secs / 86_400;
    let secs = unix_secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_api::ImportWarningKind;

    #[test]
    fn the_stamp_reads_as_a_date() {
        // 2026-09-21T16:08:53Z
        assert_eq!(stamp(1_790_006_933), "20260921-160853");
        assert_eq!(stamp(0), "19700101-000000");
        // A leap day, and the day after one.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
    }

    #[test]
    fn the_report_carries_every_section_it_has_something_for() {
        let report = ImportReport {
            kernel: "ogeom 0.1.0".into(),
            summary: vec![ImportWarningKind {
                kind: "vertex-miss".into(),
                count: 2,
                worst: 1.38e-4,
                exemplar: 1_142_600,
            }],
            warnings: vec![
                "#1142600: a curve end misses its vertex by 1.38e-4".into(),
                "#1142601: a curve end misses its vertex by 3.48e-6".into(),
            ],
            untrimmed_faces: vec![77],
            skipped: vec![("DRAUGHTING_MODEL".into(), 3)],
        };
        let text = render(Path::new("/tmp/part.step"), 2048, 5, 0, &report);
        for needle in [
            "source           /tmp/part.step",
            "kernel           ogeom 0.1.0",
            "bodies           5",
            "warnings         2",
            "vertex-miss",
            "1.380e-4",
            "#1142600",
            "#77",
            "DRAUGHTING_MODEL",
            "#1142601: a curve end misses its vertex by 3.48e-6",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    #[test]
    fn a_clean_import_writes_no_sections() {
        let text = render(Path::new("a.step"), 1, 1, 0, &ImportReport::default());
        assert!(!text.contains("By kind"));
        assert!(!text.contains("All warnings"));
    }
}
