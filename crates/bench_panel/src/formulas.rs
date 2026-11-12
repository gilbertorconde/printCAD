//! Formula fields over a document: typed text read against what its
//! formulas last came to, and the names a formula can read.

/// How a formula field reads typed text: against what the document's
/// formulas last came to, for a field holding `dim`.
pub struct DocumentFormulas<'a> {
    pub document: &'a core_document::Document,
    pub dim: core_document::expr::Dim,
}

impl ui_kit::widgets::FormulaHost for DocumentFormulas<'_> {
    fn evaluate(&self, text: &str) -> Result<f64, String> {
        self.document
            .evaluate_formula(text, Some(self.dim))
            .map(|q| q.value)
    }

    fn is_constant(&self, text: &str) -> bool {
        core_document::expr::is_constant(text)
    }

    fn candidates(&self) -> Vec<ui_kit::completion::Candidate> {
        formula_candidates(self.document)
    }
}

/// Every name a formula can read in `document`, with what it comes to,
/// for completion while a formula is typed.
pub fn formula_candidates(
    document: &core_document::Document,
) -> Vec<ui_kit::completion::Candidate> {
    let unit = document.display_unit();
    let mut out: Vec<(u64, ui_kit::completion::Candidate)> = Vec::new();
    for (id, node) in document.feature_tree().all_nodes() {
        for slot in document.evaluated_slots(*id) {
            let Some(name) = &slot.name else {
                continue;
            };
            out.push((
                node.seq,
                ui_kit::completion::Candidate {
                    text: format!(
                        "{}.{}",
                        core_document::expr::quote_name(&node.name),
                        core_document::expr::quote_name(name)
                    ),
                    detail: match &slot.result {
                        Ok(q) => q.display(unit, 3),
                        Err(_) => "error".to_string(),
                    },
                },
            ));
        }
    }
    out.sort_by(|(a, x), (b, y)| a.cmp(b).then(x.text.cmp(&y.text)));
    out.into_iter().map(|(_, c)| c).collect()
}
