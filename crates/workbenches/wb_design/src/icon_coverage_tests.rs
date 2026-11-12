use super::*;
use core_document::{Workbench, WorkbenchContext};

#[test]
fn the_bench_and_every_feature_family_name_an_icon_in_the_set() {
    let wb = DesignWorkbench::default();
    assert!(ui_kit::icon::exists(wb.descriptor().icon));
    use core_document::{DatumAttachment, DatumFeature, DatumShape};
    for shape in [
        DatumShape::Plane { size: 1.0 },
        DatumShape::Line { length: 1.0 },
        DatumShape::Point,
        DatumShape::CoordinateSystem { size: 1.0 },
    ] {
        let datum = DatumFeature {
            shape,
            attachment: DatumAttachment::BasePlane(core_document::BasePlane::XY),
            offset: Default::default(),
        };
        let icon = datum_icon(&datum);
        assert!(ui_kit::icon::exists(icon), "unknown icon {icon}");
        // Design presents the core datums it claims.
        let info = wb.feature_info(&core_document::FeatureNode::new(
            FeatureId(uuid::Uuid::new_v4()),
            &datum,
        ));
        assert_eq!(
            (info.icon, info.kind_label.as_str(), info.builds_solid),
            (icon, "Datum", false)
        );
    }
    let node = core_document::FeatureNode::new(
        FeatureId(uuid::Uuid::new_v4()),
        &crate::test_support::pad(FeatureId(uuid::Uuid::new_v4()), 10.0),
    );
    let info = wb.feature_info(&node);
    assert!(
        ui_kit::icon::exists(info.icon),
        "unknown icon {}",
        info.icon
    );
    assert_eq!(info.kind_label, "Pad");
    assert!(info.builds_solid);
    let projection = DesignFeature::Project {
        sketch: FeatureId(uuid::Uuid::new_v4()),
        faces: Vec::new(),
        mode: kernel_api::ProjectMode::Emboss,
        height: 1.0,
        wrap: false,
    };
    assert!(ui_kit::icon::exists(projection.icon()));
}

/// A hole with a cosmetic thread names its designation where the tree
/// says what the feature is; the same hole cutting nothing extra does
/// not.
#[test]
fn a_cosmetic_thread_names_its_designation_in_the_feature_kind() {
    let hole = |cosmetic_thread: bool| {
        crate::test_support::hole(FeatureId(uuid::Uuid::new_v4()), |h| {
            *h.cosmetic_thread = cosmetic_thread;
            *h.diameter = 10.2;
            *h.depth = 10.0;
            *h.through_all = true;
            *h.thread = Some(crate::feature::ThreadSpec::new(
                crate::hole_tables::ThreadStandard::IsoMetricCoarse,
                "M12",
            ));
            *h.threaded = true;
        })
    };
    let kind = |feature: &DesignFeature| {
        let node = core_document::FeatureNode::new(FeatureId(uuid::Uuid::new_v4()), feature);
        DesignWorkbench::default().feature_info(&node).kind_label
    };
    assert_eq!(kind(&hole(true)), "Hole · M12 thread");
    assert_eq!(kind(&hole(false)), "Hole");
}

#[test]
fn every_default_key_lands_on_a_tool_and_no_two_share_one() {
    let mut ctx = WorkbenchContext::default();
    DesignWorkbench::default().configure(&mut ctx);
    for (id, _) in TOOL_KEYS {
        assert!(ctx.tools().iter().any(|t| t.id == *id), "no tool {id}");
    }
    let mut seen = std::collections::HashMap::new();
    let keyed = ctx
        .tools()
        .iter()
        .map(|t| (&t.id, &t.shortcuts))
        .chain(ctx.actions().iter().map(|a| (&a.id, &a.shortcuts)));
    for (id, keys) in keyed {
        for key in keys {
            if let Some(other) = seen.insert(*key, id.clone()) {
                panic!("{id} and {other} share {key}");
            }
        }
    }
}

#[test]
fn every_tool_names_an_icon_in_the_set() {
    let mut ctx = WorkbenchContext::default();
    DesignWorkbench::default().configure(&mut ctx);
    for tool in ctx.tools() {
        let icon = tool
            .icon
            .unwrap_or_else(|| panic!("{} has no icon", tool.id));
        assert!(
            ui_kit::icon::exists(icon),
            "{}: unknown icon {icon}",
            tool.id
        );
        for variant in &tool.variants {
            assert!(
                ui_kit::icon::exists(variant.icon),
                "{}:{}: unknown icon {}",
                tool.id,
                variant.id,
                variant.icon
            );
        }
    }
}
