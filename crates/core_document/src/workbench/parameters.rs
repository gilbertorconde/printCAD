//! The numbers and inputs of a bench's features that formulas and the
//! property panel reach.

use super::{FeatureReference, PropertyHints, ReferenceChoice};
use crate::feature::FeatureNode;
use crate::runtime::WorkbenchRuntimeContext;
use crate::{Document, FeatureId};

/// The values of an owned feature that formulas and the property panel
/// set and read, and the inputs the panel's Inputs group swaps.
pub trait BenchParameters {
    /// The numbers of an owned feature that formulas may set and read:
    /// where each is in its JSON and what kind of quantity it is. A field
    /// listed here builds from `Document::feature_values`, which has every
    /// formula's current value in.
    fn parameters(&self, _node: &FeatureNode) -> Vec<crate::evaluate::Parameter> {
        Vec::new()
    }

    /// The least and largest values a drag sets `parameter` of the owned
    /// feature `feature` to, where the bench knows them (a fillet's radius
    /// up to the largest that builds): the property panel's drag stops
    /// there, while a typed value may go past.
    fn parameter_drag_range(
        &self,
        _document: &Document,
        _feature: crate::FeatureId,
        _parameter: &crate::evaluate::Parameter,
    ) -> (Option<f64>, Option<f64>) {
        (None, None)
    }

    /// The inputs of an owned feature that can be swapped for others (a
    /// pad's profile sketch), for the property panel to offer. `id` is the
    /// feature `node` describes.
    fn references(
        &self,
        _document: &crate::Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Vec<FeatureReference> {
        Vec::new()
    }

    /// Point input `key` of feature `id` (one [`Self::references`] listed)
    /// at `to`, the bench's own way: its dependencies, what it hides, what
    /// rebuilds. Why not, when it cannot.
    fn set_reference(
        &mut self,
        _ctx: &mut WorkbenchRuntimeContext,
        _id: FeatureId,
        _key: &str,
        _to: ReferenceChoice,
    ) -> Result<(), String> {
        Err("this input cannot be changed here".to_string())
    }

    /// Formulas moved the values of `moved` (any bench's features), or a
    /// formula on one was set or taken away: a bench whose results are
    /// recorded rather than derived (placements a joint solves for, a
    /// formula kept in step with another) records them here. Runs just before an undo step
    /// closes, so the edit that moved a value and what follows from it
    /// are one step; it must record nothing when nothing is out of place.
    fn values_moved(&mut self, _ctx: &mut WorkbenchRuntimeContext, _moved: &[FeatureId]) {}

    /// What the generic property panel should know about this bench's
    /// feature payloads.
    fn property_hints(&self) -> PropertyHints {
        PropertyHints::default()
    }
}
