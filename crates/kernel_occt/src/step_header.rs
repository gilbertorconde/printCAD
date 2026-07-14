//! Detect a STEP file's declared length unit by scanning its HEADER/DATA
//! section.
//!
//! STEP (ISO 10303-21) files describe their global length unit in the DATA
//! section using one of two patterns:
//!
//! * `SI_UNIT(.MILLI., .METRE.)` — metric, with an optional prefix.
//! * `CONVERSION_BASED_UNIT('INCH', ...)` or `('FOOT', ...)` — imperial.
//!
//! We don't try to fully parse STEP here. Instead we scan the first ~64 KB of
//! the file (which always covers the HEADER plus the unit declarations near
//! the top of DATA) and look for the first `LENGTH_UNIT` entity. This is
//! good enough for the UI's auto-pick logic; if detection fails we just
//! return `None` and the document keeps its existing display unit.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use kernel_api::LengthUnit;

/// Maximum bytes scanned from the start of the STEP file. The header plus the
/// unit declaration is always well within this window.
const SCAN_BUDGET: usize = 64 * 1024;

/// Read a STEP file from disk and return its declared length unit, when
/// detectable.
pub fn detect_step_unit_from_path(path: &Path) -> io::Result<Option<LengthUnit>> {
    let mut file = File::open(path)?;
    let mut buffer = vec![0u8; SCAN_BUDGET];
    let read = file.read(&mut buffer)?;
    buffer.truncate(read);
    let text = String::from_utf8_lossy(&buffer);
    Ok(detect_step_unit(&text))
}

/// Scan a STEP file's textual prefix and extract its declared length unit.
///
/// The match is case-insensitive and tolerant of whitespace differences. Only
/// length-unit-style entities are considered: `LENGTH_UNIT`/`SI_UNIT` for
/// metric variants, and `CONVERSION_BASED_UNIT('INCH'|'FOOT'|...)` for
/// imperial.
pub fn detect_step_unit(text: &str) -> Option<LengthUnit> {
    let upper = text.to_ascii_uppercase();

    // Strategy: find the first occurrence of either `LENGTH_UNIT(...)` or a
    // `CONVERSION_BASED_UNIT(...)` whose label looks like an imperial length.
    // Whichever appears earlier in the file wins.
    let mut best: Option<(usize, LengthUnit)> = None;

    if let Some((pos, unit)) = scan_si_length_unit(&upper) {
        best = Some((pos, unit));
    }

    if let Some((pos, unit)) = scan_conversion_based_unit(&upper) {
        match best {
            Some((existing_pos, _)) if existing_pos <= pos => {}
            _ => best = Some((pos, unit)),
        }
    }

    best.map(|(_, unit)| unit)
}

/// Look for `( ... LENGTH_UNIT() ... SI_UNIT(.<prefix>., .METRE.) ... )` style
/// constructs and infer the metric prefix.
fn scan_si_length_unit(upper: &str) -> Option<(usize, LengthUnit)> {
    let mut search_from = 0;
    while let Some(rel) = upper[search_from..].find("LENGTH_UNIT") {
        let abs = search_from + rel;
        // Bound the SI_UNIT search to a reasonable window after this token —
        // STEP entity definitions never span more than a few hundred chars.
        let window_end = (abs + 512).min(upper.len());
        let window = &upper[abs..window_end];
        if let Some(si_rel) = window.find("SI_UNIT") {
            let si_abs = abs + si_rel;
            // Extract the parenthesised argument list, e.g. `(.MILLI.,.METRE.)`.
            if let Some(args) = extract_parens(&upper[si_abs..]) {
                if args.contains("METRE") {
                    let unit = if args.contains(".MILLI.") {
                        LengthUnit::Millimetre
                    } else if args.contains(".CENTI.") {
                        LengthUnit::Centimetre
                    } else if args.contains("$,.METRE.")
                        || args.contains("$, .METRE.")
                        || args.contains("$ , .METRE.")
                    {
                        LengthUnit::Metre
                    } else if !has_known_prefix(args) {
                        // No prefix at all → bare SI metre.
                        LengthUnit::Metre
                    } else {
                        // Prefixes we don't model (e.g. micro). Fall back to
                        // metres so the UI still reports something sensible.
                        LengthUnit::Metre
                    };
                    return Some((abs, unit));
                }
            }
        }
        search_from = abs + "LENGTH_UNIT".len();
    }
    None
}

/// Look for `CONVERSION_BASED_UNIT('INCH', ...)` / `('FOOT', ...)` etc.
fn scan_conversion_based_unit(upper: &str) -> Option<(usize, LengthUnit)> {
    let mut search_from = 0;
    while let Some(rel) = upper[search_from..].find("CONVERSION_BASED_UNIT") {
        let abs = search_from + rel;
        if let Some(args) = extract_parens(&upper[abs..]) {
            // The first argument is a quoted unit name.
            let label = first_quoted_label(args).unwrap_or("");
            let unit = match label {
                "INCH" | "INCHES" => Some(LengthUnit::Inch),
                "FOOT" | "FEET" => Some(LengthUnit::Foot),
                "MILLIMETRE" | "MILLIMETER" => Some(LengthUnit::Millimetre),
                "CENTIMETRE" | "CENTIMETER" => Some(LengthUnit::Centimetre),
                "METRE" | "METER" => Some(LengthUnit::Metre),
                _ => None,
            };
            if let Some(u) = unit {
                return Some((abs, u));
            }
        }
        search_from = abs + "CONVERSION_BASED_UNIT".len();
    }
    None
}

fn extract_parens(text: &str) -> Option<&str> {
    let start = text.find('(')?;
    let mut depth = 0i32;
    for (idx, ch) in text[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start + 1..start + idx]);
                }
            }
            _ => {}
        }
    }
    None
}

fn first_quoted_label(args: &str) -> Option<&str> {
    let start = args.find('\'')?;
    let rest = &args[start + 1..];
    let end = rest.find('\'')?;
    Some(&rest[..end])
}

fn has_known_prefix(args: &str) -> bool {
    const PREFIXES: &[&str] = &[
        ".MILLI.", ".CENTI.", ".MICRO.", ".NANO.", ".PICO.", ".KILO.", ".MEGA.", ".DECI.",
        ".HECTO.",
    ];
    PREFIXES.iter().any(|p| args.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MM_HEADER: &str = r#"
ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('STEP AP214'),'1');
ENDSEC;
DATA;
#10 = LENGTH_UNIT(()) ;
#11 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );
ENDSEC;
"#;

    const INCH_HEADER: &str = r#"
ISO-10303-21;
DATA;
#10 = ( LENGTH_UNIT() NAMED_UNIT(#42) CONVERSION_BASED_UNIT('INCH', #99) );
"#;

    const METRE_HEADER: &str = r#"
DATA;
#1 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.) );
"#;

    #[test]
    fn detects_millimetre_si_unit() {
        assert_eq!(detect_step_unit(MM_HEADER), Some(LengthUnit::Millimetre));
    }

    #[test]
    fn detects_inch_conversion_unit() {
        assert_eq!(detect_step_unit(INCH_HEADER), Some(LengthUnit::Inch));
    }

    #[test]
    fn detects_bare_metre() {
        assert_eq!(detect_step_unit(METRE_HEADER), Some(LengthUnit::Metre));
    }

    #[test]
    fn returns_none_when_no_unit() {
        assert_eq!(detect_step_unit("ISO-10303-21;\nDATA;\nENDSEC;"), None);
    }
}
