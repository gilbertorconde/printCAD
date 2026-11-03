//! Volume errors: where a scan and its reference differ as material, the
//! scan read as a solid on a grid of cells and set against the reference
//! solid. Each region is missing (the reference has it, the scan lacks it)
//! or excess, a layer (a face standing off) or a lump (a boss or a pocket),
//! with its volume and depth; holes in the scan read as the reference.

use kernel_api::{KernelQueries, VolumeRegion};
use serde_json::{Value, json};

use crate::compare::Inputs;

/// A cell is this share of the reference's longest side when none is given.
const CELL_SHARE: f64 = 1.0 / 200.0;

/// The cell a comparison's grid takes when it names none: a two-hundredth
/// of the reference's longest side, no finer than 0.05 mm.
pub fn default_cell(longest_side: f64) -> f64 {
    (longest_side * CELL_SHARE).max(0.05)
}

/// The regions where `inputs`' scan and reference differ, on cells `cell`
/// wide, largest first, in the reference's own frame.
pub fn measure(
    kernel: &dyn KernelQueries,
    inputs: &Inputs,
    cell: f64,
) -> Result<Vec<VolumeRegion>, String> {
    let solid = inputs
        .shape()
        .ok_or("volume errors need a solid reference, not a mesh")?;
    let to = inputs.scan_to_reference();
    let mut scan = inputs.scan_mesh().clone();
    for p in &mut scan.positions {
        *p = to.point(*p);
    }
    kernel
        .volume_difference(&scan, solid, cell)
        .map_err(|e| e.to_string())
}

/// A region's middle, in the reference's frame.
pub fn centre(r: &VolumeRegion) -> [f64; 3] {
    std::array::from_fn(|i| (r.min[i] + r.max[i]) / 2.0)
}

/// What a label on the part says of `r`.
pub fn label(r: &VolumeRegion, unit: core_document::units::Unit) -> String {
    format!(
        "{} {} · {:.0} mm³ · {} deep{}",
        if r.missing { "missing" } else { "excess" },
        if r.layer { "layer" } else { "lump" },
        r.volume,
        core_document::units::show_mm(r.depth, unit),
        if r.unsure { " ?" } else { "" },
    )
}

/// The regions as a script reads them.
pub fn answer(regions: &[VolumeRegion], cell: f64) -> Value {
    let list: Vec<Value> = regions
        .iter()
        .map(|r| {
            json!({
                "side": if r.missing { "missing" } else { "excess" },
                "form": if r.layer { "layer" } else { "lump" },
                "volume": r.volume,
                "depth": r.depth,
                "centre": centre(r),
                "min": r.min,
                "max": r.max,
                "unsure": r.unsure,
            })
        })
        .collect();
    json!({ "cell": cell, "regions": list })
}
