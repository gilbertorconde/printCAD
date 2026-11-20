//! What a bench contributes to what leaves the app: reports,
//! annotations, threads, exports and print layouts.

use super::*;

/// What a bench adds to printed reports, exported annotations and
/// threads, and what it keeps out of exports and print layouts.
pub trait BenchOutput {
    /// The semantic annotations describing `body`, with face probes in
    /// world coordinates. Asked of every bench, whichever is active.
    fn semantic_pmi(&self, _document: &Document, _body: BodyId) -> crate::SemanticPmi {
        crate::SemanticPmi::default()
    }

    /// The cosmetic threads an owned feature declares: each bore it
    /// threads without cutting the thread, with its designation, in its
    /// body's frame. Asked as the document evaluates, of `document`'s
    /// evaluated values; the parts list, the annotations and export read
    /// the answers (`Document::cosmetic_threads`).
    fn cosmetic_threads(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Vec<crate::ThreadCallout> {
        Vec::new()
    }

    /// What this bench has to report of the document, for a printed report
    /// document: a title, lines of summary, tables of text, and the bodies
    /// to picture. `None` when it has nothing to report.
    fn report(&self, _document: &Document) -> Option<BenchReport> {
        None
    }

    /// Bodies that are not made (bought parts): an export of every
    /// visible body and what goes to the slicer leave them out.
    /// `bought_kinds` are the feature kinds every bench declares bought
    /// (`WorkbenchDescriptor::bought_kinds`).
    fn not_printed(&self, _document: &Document, _bought_kinds: &[WorkbenchId]) -> Vec<BodyId> {
        Vec::new()
    }

    /// The parts to print and how many of each, for a print layout: each
    /// part the bodies of one shape, the first the one printed. `None`
    /// from a bench that keeps no parts list; the first bench to answer
    /// is the one the host lays out.
    fn print_parts(
        &self,
        _document: &Document,
        _bought_kinds: &[WorkbenchId],
    ) -> Option<Vec<PrintPart>> {
        None
    }
}
