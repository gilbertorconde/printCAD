//! An inspection's report as text or CSV: every element, constructed
//! element, dimension and tolerance with what it read, PASS or FAIL where it
//! has a band, and the tally; read from the report's own JSON, so every
//! form says the same.

use crate::construction::Construction;
use crate::{
    answer_of, built_at, construction, construction_answer, dimension, dimension_answer,
    dimension_at, element, element_at, tolerance, tolerance_answer, tolerance_at, tolerance_read,
};
use core_document::{BodyId, Document};
use serde_json::{Value, json};

/// Every element, construction, dimension and tolerance of the
/// inspection (of `only`'s scan when given), and how many passed and failed.
pub(super) fn report_of(
    document: &Document,
    only: Option<BodyId>,
    readings: &tolerance_read::Readings,
    kernel: Option<&'static dyn kernel_api::KernelQueries>,
    wait: bool,
) -> Value {
    let mut ordered: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| only.is_none() || n.body == only)
        .collect();
    ordered.sort_by_key(|(id, n)| (n.seq, **id));
    let nodes = || ordered.iter().copied();
    let elements: Vec<Value> = nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == element::KIND)
        .filter_map(|(id, _)| Some(answer_of(*id, &element_at(document, *id)?)))
        .collect();
    let constructions: Vec<Value> = nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == construction::KIND)
        .filter_map(|(id, _)| {
            let c = document.feature_as::<Construction>(*id)?;
            Some(construction_answer(*id, &c, &built_at(document, *id, 0)))
        })
        .collect();
    let dimensions: Vec<Value> = nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == dimension::KIND)
        .filter_map(|(id, _)| {
            let (dim, reading) = dimension_at(document, *id)?;
            Some(dimension_answer(*id, &dim, &reading))
        })
        .collect();
    let tolerances: Vec<Value> = nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == tolerance::KIND)
        .filter_map(|(id, _)| {
            let (t, reading) = tolerance_at(document, *id, readings, kernel, wait)?;
            Some(tolerance_answer(*id, &t, &reading))
        })
        .collect();
    let count = |want: bool| {
        dimensions
            .iter()
            .chain(&tolerances)
            .filter(|d| d["pass"].as_bool() == Some(want))
            .count()
    };
    json!({
        "passed": count(true),
        "failed": count(false),
        "elements": elements,
        "constructions": constructions,
        "dimensions": dimensions,
        "tolerances": tolerances,
    })
}

fn number(v: &Value) -> String {
    v.as_f64().map_or_else(String::new, |x| format!("{x:.4}"))
}

fn verdict(row: &Value) -> &'static str {
    match row["pass"].as_bool() {
        Some(true) => "PASS",
        Some(false) => "FAIL",
        None => "",
    }
}

/// The rows of a report, each `[section, name, what, value, target,
/// deviation, verdict, note]`.
pub(crate) fn rows(report: &Value) -> Vec<[String; 8]> {
    let mut out = Vec::new();
    let list = |key: &str| report[key].as_array().cloned().unwrap_or_default();
    for e in list("elements") {
        let mut note = format!(
            "σ {}, {} of {} points",
            number(&e["sigma"]),
            e["used"],
            e["points"]
        );
        if let Some(d) = e["diameters"].as_object() {
            note.push_str(&format!(
                ", Ø {} to {}",
                number(&d["min"]),
                number(&d["max"])
            ));
        }
        out.push([
            "element".into(),
            e["name"].as_str().unwrap_or_default().into(),
            e["kind"].as_str().unwrap_or_default().into(),
            number(&e["form"]),
            String::new(),
            String::new(),
            String::new(),
            format!("form; {note}"),
        ]);
    }
    for c in list("constructions") {
        out.push([
            "construction".into(),
            c["name"].as_str().unwrap_or_default().into(),
            c["made"].as_str().unwrap_or_default().into(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            c["error"].as_str().unwrap_or_default().into(),
        ]);
    }
    for d in list("dimensions") {
        let band = format!("+{} / -{}", number(&d["plus"]), number(&d["minus"]));
        out.push([
            "dimension".into(),
            d["name"].as_str().unwrap_or_default().into(),
            d["kind"].as_str().unwrap_or_default().into(),
            number(&d["value"]),
            number(&d["nominal"]),
            number(&d["deviation"]),
            verdict(&d).into(),
            d["error"].as_str().map_or(band, str::to_string),
        ]);
    }
    for t in list("tolerances") {
        let mut note = t["error"]
            .as_str()
            .or(t["note"].as_str())
            .unwrap_or_default()
            .to_string();
        if t["characteristic"] == "position" {
            note.push_str(&format!(
                "; bonus {} mm; allowed {} mm",
                number(&t["bonus"]),
                number(&t["allowed"])
            ));
        }
        for field in [
            "datum_labels",
            "basic",
            "material",
            "unequal",
            "datum_shift",
        ] {
            if !t[field].is_null() && t[field] != serde_json::json!([]) {
                note.push_str(&format!("; {field}: {}", t[field]));
            }
        }
        out.push([
            "tolerance".into(),
            t["name"].as_str().unwrap_or_default().into(),
            t["characteristic"].as_str().unwrap_or_default().into(),
            number(&t["value"]),
            number(&t["limit"]),
            String::new(),
            verdict(&t).into(),
            note,
        ]);
    }
    out
}

/// The report as CSV, a header row and one row per entry.
pub fn csv(report: &Value) -> String {
    let quote = |s: &str| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut out = String::from("section,name,what,value,nominal or limit,deviation,verdict,note\n");
    for row in rows(report) {
        let line: Vec<String> = row.iter().map(|c| quote(c)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// The report as plain text: a heading, the units, a line per entry and
/// the tally.
pub fn text(report: &Value, title: &str) -> String {
    let mut out = format!("{title}\nLengths in mm, angles in degrees.\n");
    let mut section = String::new();
    for row in rows(report) {
        if row[0] != section {
            section = row[0].clone();
            out.push_str(&format!("\n{}s\n", capitalised(&section)));
        }
        let mut line = format!("  {:<20} {:<16} {:>10}", row[1], row[2], row[3]);
        if !row[4].is_empty() {
            line.push_str(&format!("  of {:>9}", row[4]));
        }
        if !row[5].is_empty() {
            line.push_str(&format!("  {:>+10}", row[5]));
        }
        if !row[6].is_empty() {
            line.push_str(&format!("  {}", row[6]));
        }
        if !row[7].is_empty() {
            line.push_str(&format!("  ({})", row[7]));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.push_str(&format!(
        "\n{} passed, {} failed\n",
        report["passed"], report["failed"]
    ));
    out
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A report's rows come out in CSV quoted where they must be, and in
    /// text under their sections with their verdicts and the tally.
    #[test]
    fn a_report_reads_as_csv_and_text() {
        let report = json!({
            "passed": 1, "failed": 1,
            "elements": [{"name": "Plane 1", "kind": "plane", "form": 0.01, "sigma": 0.002,
                          "used": 90, "points": 100}],
            "dimensions": [{"name": "Distance 1", "kind": "distance", "value": 30.0,
                            "nominal": 30.0, "deviation": 0.0, "plus": 0.1, "minus": 0.1,
                            "pass": true}],
            "tolerances": [{"name": "Flatness 1", "characteristic": "flatness",
                            "value": 0.06, "limit": 0.05, "pass": false,
                            "note": "read at its corners, roughly"}],
        });
        let csv = csv(&report);
        assert_eq!(csv.lines().count(), 4);
        assert!(csv.contains("\"read at its corners, roughly\""));
        let text = text(&report, "Inspection");
        assert!(text.contains("Dimensions\n") && text.contains("PASS") && text.contains("FAIL"));
        assert!(text.ends_with("1 passed, 1 failed\n"));
    }
}
