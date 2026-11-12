//! What the bench answers for the feature kinds it claims: their info,
//! their drawn lines, the numbers formulas set, and what goes with one
//! deleted.

use crate::{
    InspectionWorkbench, align, clearance, compare, construction, construction::Construction,
    construction::Made, dimension, dimension::DimKind, dimension::Dimension, element,
    element::Element, element_map, making::dimension_at, making::element_at,
    making::shape_revision, overhang, pins, section, shadow, thickness, tolerance,
};
use core_document::{
    Document, FeatureInfo, FeatureNode, PassiveGeometry, PassiveTint, WorkbenchFeature,
    WorkbenchRuntimeContext,
};

impl InspectionWorkbench {
    pub(super) fn info_of(&self, node: &FeatureNode) -> FeatureInfo {
        if node.workbench_id.as_str() == shadow::KIND {
            return FeatureInfo {
                icon: "cut",
                kind_label: "Shadow outline".to_string(),
                family_label: "Inspection shadow".to_string(),
                builds_solid: true,
            };
        }
        if node.workbench_id.as_str() == section::KIND {
            return FeatureInfo {
                icon: "clipping-plane",
                kind_label: "Section".to_string(),
                family_label: "Inspection section".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == pins::KIND {
            return FeatureInfo {
                icon: "datum-point",
                kind_label: "Pinned reading".to_string(),
                family_label: "Inspection pin".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == element_map::KIND {
            return FeatureInfo {
                icon: "constraint-distance",
                kind_label: "Map against an element".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == tolerance::KIND {
            let label = tolerance::Tolerance::from_json(&node.data)
                .map(|t| format!("{} tolerance", t.characteristic.word()))
                .unwrap_or_else(|_| "Tolerance".to_string());
            return FeatureInfo {
                icon: "constraint-perpendicular",
                kind_label: label,
                family_label: "Inspection tolerance".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == thickness::KIND {
            return FeatureInfo {
                icon: "thickness",
                kind_label: "Wall thickness".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == clearance::KIND {
            return FeatureInfo {
                icon: "thickness",
                kind_label: "Fit check".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == overhang::KIND {
            return FeatureInfo {
                icon: "thickness",
                kind_label: "Overhangs".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == align::KIND {
            let label = align::Alignment::from_json(&node.data)
                .map(|a| format!("{} · {}", a.how.word(), a.how.summary()))
                .unwrap_or_else(|_| "Alignment".to_string());
            return FeatureInfo {
                icon: "move-geometry",
                kind_label: label,
                family_label: "Inspection alignment".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == compare::KIND {
            return FeatureInfo {
                icon: "measure",
                kind_label: "Comparison with a reference".to_string(),
                family_label: "Inspection map".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == construction::KIND {
            let Ok(c) = Construction::from_json(&node.data) else {
                return FeatureInfo::fallback(node);
            };
            return FeatureInfo {
                icon: c.made.icon(),
                kind_label: format!("Constructed {}", c.made.word().to_lowercase()),
                family_label: "Inspection element".to_string(),
                builds_solid: false,
            };
        }
        if node.workbench_id.as_str() == dimension::KIND {
            let Ok(dim) = Dimension::from_json(&node.data) else {
                return FeatureInfo::fallback(node);
            };
            return FeatureInfo {
                icon: dim.kind.icon(),
                kind_label: format!("{} dimension", dim.kind.word()),
                family_label: "Inspection dimension".to_string(),
                builds_solid: false,
            };
        }
        let Ok(element) = Element::from_json(&node.data) else {
            return FeatureInfo::fallback(node);
        };
        FeatureInfo {
            icon: element::icon(element.kind),
            kind_label: format!(
                "{} · {}",
                element::kind_word(element.kind),
                element.result.summary()
            ),
            family_label: "Inspection element".to_string(),
            builds_solid: false,
        }
    }

    pub(super) fn passive_of(
        &self,
        document: &Document,
        id: core_document::FeatureId,
        node: &FeatureNode,
    ) -> Option<PassiveGeometry> {
        if node.workbench_id.as_str() == shadow::KIND {
            let s = document.feature_as::<shadow::Shadow>(id)?;
            let made = s.made?;
            return Some(PassiveGeometry {
                mesh: shadow::lines(&made),
                revision: made.key,
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        if node.workbench_id.as_str() == section::KIND {
            let (_, cuts, key) = self.cuts_of(document, id)?;
            return Some(PassiveGeometry {
                mesh: section::lines(&cuts),
                revision: key,
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        let data = document.feature_values(id)?;
        if node.workbench_id.as_str() == dimension::KIND {
            let (_, reading) = dimension_at(document, id)?;
            let reading = reading.ok()?;
            return Some(PassiveGeometry {
                mesh: dimension::line(&reading),
                revision: core_document::data_revision(data)
                    ^ reading.value.to_bits()
                    ^ reading.from[0].to_bits(),
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        if node.workbench_id.as_str() == construction::KIND {
            let built = element_at(document, id)?;
            return Some(PassiveGeometry {
                mesh: built.result.lines(),
                revision: core_document::data_revision(data) ^ shape_revision(&built),
                tint: PassiveTint::External,
                regions: Vec::new(),
            });
        }
        let element = Element::from_json(data).ok()?;
        Some(PassiveGeometry {
            mesh: element.result.lines(),
            revision: core_document::data_revision(data),
            tint: PassiveTint::External,
            regions: Vec::new(),
        })
    }

    pub(super) fn parameters_of(
        &self,
        node: &FeatureNode,
    ) -> Vec<core_document::evaluate::Parameter> {
        use core_document::{evaluate::Parameter, expr::Dim};
        if node.workbench_id.as_str() == construction::KIND {
            return match Construction::from_json(&node.data) {
                Ok(c) if c.made == Made::Plane && c.from.len() == 1 => {
                    vec![Parameter::new("offset", "Offset", Dim::LENGTH, "/offset")]
                }
                _ => Vec::new(),
            };
        }
        if node.workbench_id.as_str() == tolerance::KIND {
            return vec![Parameter::new("limit", "Limit", Dim::LENGTH, "/limit")];
        }
        if node.workbench_id.as_str() == clearance::KIND {
            return vec![
                Parameter::new("min_gap", "Smallest gap", Dim::LENGTH, "/min_gap"),
                Parameter::new("reach", "Reach", Dim::LENGTH, "/reach"),
            ];
        }
        if node.workbench_id.as_str() == overhang::KIND {
            return vec![Parameter::new("limit", "Limit", Dim::ANGLE, "/limit_deg")];
        }
        if node.workbench_id.as_str() == shadow::KIND {
            return vec![
                Parameter::new("play", "Play", Dim::LENGTH, "/play"),
                Parameter::new("tolerance", "Tolerance", Dim::LENGTH, "/tolerance"),
                Parameter::new("round_below", "Round below", Dim::ANGLE, "/round_below_deg"),
                Parameter::new("fill_below", "Fill below", Dim::LENGTH, "/fill_below"),
            ];
        }
        if node.workbench_id.as_str() == section::KIND {
            return vec![
                Parameter::new("offset", "Offset", Dim::LENGTH, "/offset"),
                Parameter::count("count", "Cuts", "/count"),
                Parameter::new("step", "Step", Dim::LENGTH, "/step"),
            ];
        }
        if node.workbench_id.as_str() != dimension::KIND {
            return Vec::new();
        }
        let Ok(dim) = Dimension::from_json(&node.data) else {
            return Vec::new();
        };
        let unit = match dim.kind {
            DimKind::Angle => Dim::ANGLE,
            _ => Dim::LENGTH,
        };
        vec![
            Parameter::new("nominal", "Nominal", unit, "/nominal"),
            Parameter::new("plus", "Plus", unit, "/plus"),
            Parameter::new("minus", "Minus", unit, "/minus"),
        ]
    }

    pub(super) fn delete_with_dependents(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: core_document::FeatureId,
    ) -> bool {
        // Everything made or measured from it goes with it, however deep.
        let mut going = vec![id];
        loop {
            let more: Vec<_> =
                ctx.document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(f, _)| !going.contains(f))
                    .filter(|(_, n)| {
                        let on = match n.workbench_id.as_str() {
                            dimension::KIND => Dimension::from_json(&n.data).map(|d| d.elements),
                            pins::KIND => pins::Pin::from_json(&n.data).map(|p| p.dependencies()),
                            element_map::KIND => element_map::ElementMap::from_json(&n.data)
                                .map(|m| m.dependencies()),
                            tolerance::KIND => {
                                tolerance::Tolerance::from_json(&n.data).map(|t| t.dependencies())
                            }
                            construction::KIND => Construction::from_json(&n.data).map(|c| c.from),
                            _ => return false,
                        };
                        on.is_ok_and(|on| on.iter().any(|f| going.contains(f)))
                    })
                    .map(|(f, _)| *f)
                    .collect();
            if more.is_empty() {
                break;
            }
            going.extend(more);
        }
        for f in going.iter().skip(1).rev() {
            let _ = ctx.document.remove_feature(*f);
        }
        ctx.document.remove_feature(id).is_ok()
    }
}
