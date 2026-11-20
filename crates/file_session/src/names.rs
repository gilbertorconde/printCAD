//! A document's name from its file's.

/// Derive a user-facing document name from a file name by stripping the
/// known document extensions (case-insensitively), longest match first.
/// Returns the original name when no known extension matches.
pub fn document_name_from_file_name(file_name: &str) -> &str {
    const SUFFIXES: [&str; 4] = [".prtcad.zst", ".prtcad.gz", ".prtcad", ".json"];
    let lowered = file_name.to_ascii_lowercase();
    for suffix in SUFFIXES {
        if let Some(stripped) = lowered.strip_suffix(suffix) {
            return &file_name[..stripped.len()];
        }
    }
    file_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_known_extensions() {
        assert_eq!(document_name_from_file_name("part.prtcad"), "part");
        assert_eq!(document_name_from_file_name("part.prtcad.gz"), "part");
        assert_eq!(document_name_from_file_name("part.prtcad.zst"), "part");
        assert_eq!(document_name_from_file_name("part.json"), "part");
    }

    #[test]
    fn strips_case_insensitively_but_preserves_name_case() {
        assert_eq!(document_name_from_file_name("Bracket.PRTCAD"), "Bracket");
        assert_eq!(
            document_name_from_file_name("Bracket.PrtCad.ZST"),
            "Bracket"
        );
    }

    #[test]
    fn leaves_unknown_extensions_alone() {
        assert_eq!(document_name_from_file_name("part.step"), "part.step");
        assert_eq!(document_name_from_file_name("Untitled"), "Untitled");
        assert_eq!(document_name_from_file_name(""), "");
    }

    #[test]
    fn keeps_inner_dots() {
        assert_eq!(document_name_from_file_name("v1.2.prtcad"), "v1.2");
        // `.gz` alone is not a document extension.
        assert_eq!(document_name_from_file_name("part.gz"), "part.gz");
    }
}
