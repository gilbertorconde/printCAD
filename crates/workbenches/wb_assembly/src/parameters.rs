//! The numbers joints and couplings list for formulas: a drive's, and
//! each joint kind's.

use crate::{AssemblyWorkbench, COUPLING_KIND, Coupling, Drive, JointFeature, JointKind};
use core_document::WorkbenchFeature;

/// A drive's numbers, those it has: what it holds the motion at and its
/// limits.
fn drive_parameters(
    variant: &str,
    drive: &Drive,
    dim: core_document::expr::Dim,
) -> Vec<core_document::Parameter> {
    named_drive_parameters(&format!("/kind/{variant}/drive"), ("", ""), drive, dim)
}

/// The same for a drive stored at `base`, its keys and labels starting
/// with `key` and `label` (the bare words, capitalized, when `key` is
/// empty).
fn named_drive_parameters(
    base: &str,
    (key, label): (&str, &str),
    drive: &Drive,
    dim: core_document::expr::Dim,
) -> Vec<core_document::Parameter> {
    use core_document::Parameter;
    let mut out = Vec::new();
    let named = |name: &str| {
        if key.is_empty() {
            (name.to_string(), capitalized(name))
        } else {
            (format!("{key}_{name}"), format!("{label} {name}"))
        }
    };
    if drive.to.is_some() {
        let (name, text) = named("drive");
        out.push(Parameter::new(&name, &text, dim, format!("{base}/to")));
    }
    if drive.limits.is_some() {
        for (end, word) in [(0, "lowest"), (1, "highest")] {
            let (name, text) = named(word);
            out.push(Parameter::new(
                &name,
                &text,
                dim,
                format!("{base}/limits/{end}"),
            ));
        }
    }
    out
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

impl AssemblyWorkbench {
    /// The numbers a joint's kind has, for formulas.
    fn kind_parameters(&self, node: &core_document::FeatureNode) -> Vec<core_document::Parameter> {
        use core_document::Parameter;
        use core_document::expr::Dim;
        match JointFeature::from_json(&node.data).map(|j| j.kind) {
            Ok(JointKind::Mate { .. }) => {
                vec![Parameter::new(
                    "offset",
                    "Offset",
                    Dim::LENGTH,
                    "/kind/Mate/offset",
                )]
            }
            Ok(JointKind::Angle { .. }) => {
                vec![Parameter::new(
                    "angle",
                    "Angle",
                    Dim::ANGLE,
                    "/kind/Angle/degrees",
                )]
            }
            Ok(JointKind::Hinge { drive, .. }) => {
                let mut out = vec![Parameter::new(
                    "offset",
                    "Height",
                    Dim::LENGTH,
                    "/kind/Hinge/offset",
                )];
                out.extend(drive_parameters("Hinge", &drive, Dim::ANGLE));
                out
            }
            Ok(JointKind::Slider { drive, .. }) => drive_parameters("Slider", &drive, Dim::LENGTH),
            Ok(JointKind::Fixed { .. }) => ["x", "y", "z"]
                .into_iter()
                .enumerate()
                .map(|(k, axis)| {
                    Parameter::new(
                        &format!("shift_{axis}"),
                        &format!("Shift {axis}"),
                        Dim::LENGTH,
                        format!("/kind/Fixed/shift/{k}"),
                    )
                })
                .collect(),
            Ok(JointKind::Align { turn, slide, .. }) => {
                let mut out =
                    named_drive_parameters("/kind/Align/turn", ("turn", "Turn"), &turn, Dim::ANGLE);
                out.extend(named_drive_parameters(
                    "/kind/Align/slide",
                    ("slide", "Slide"),
                    &slide,
                    Dim::LENGTH,
                ));
                out
            }
            Ok(JointKind::Distance { .. }) => {
                vec![Parameter::new(
                    "offset",
                    "Distance",
                    Dim::LENGTH,
                    "/kind/Distance/offset",
                )]
            }
            Ok(JointKind::Tangent { .. }) => {
                vec![Parameter::new(
                    "radius",
                    "Radius",
                    Dim::LENGTH,
                    "/kind/Tangent/radius",
                )]
            }
            Ok(JointKind::Cam { .. }) => {
                vec![Parameter::new(
                    "radius",
                    "Radius",
                    Dim::LENGTH,
                    "/kind/Cam/radius",
                )]
            }
            _ => Vec::new(),
        }
    }
}

impl AssemblyWorkbench {
    pub(crate) fn parameters_of(
        &self,
        node: &core_document::FeatureNode,
    ) -> Vec<core_document::Parameter> {
        use core_document::Parameter;
        use core_document::expr::Dim;
        if node.workbench_id.as_str() == COUPLING_KIND {
            return match Coupling::from_json(&node.data) {
                Ok(c) => {
                    let (label, length) = c.gearing.ratio_label();
                    let dim = if length { Dim::LENGTH } else { Dim::NUMBER };
                    vec![Parameter::new("ratio", label, dim, "/ratio")]
                }
                Err(_) => Vec::new(),
            };
        }
        let ends = [
            ("moving_end", "Moving end offset", "/ends/0"),
            ("fixed_end", "Fixed end offset", "/ends/1"),
        ]
        .map(|(name, label, pointer)| Parameter::new(name, label, Dim::LENGTH, pointer));
        let mut out = self.kind_parameters(node);
        if JointFeature::from_json(&node.data).is_ok_and(|j| j.kind != JointKind::Ground) {
            out.extend(ends);
        }
        out
    }
}
