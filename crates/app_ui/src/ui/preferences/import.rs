//! The Import / Export page.

use super::*;

pub(super) fn import_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    let t = &mut state.draft.import.tessellation;
    let absolute = t.linear_deflection_mode == LinearDeflectionMode::AbsoluteMm;
    let mut rows = vec![PrefRow::select(
        "Linear deflection",
        "prefs_step_linear",
        &mut t.linear_deflection_mode,
        &[
            (
                LinearDeflectionMode::BboxScaled,
                "Scaled by the bounding box",
            ),
            (LinearDeflectionMode::AbsoluteMm, "Absolute chord height"),
        ],
    )];
    if absolute {
        rows.push(PrefRow::qty(
            "Chord tolerance",
            QtyField::mm(&mut t.chord_tolerance)
                .range(0.001..=5.0)
                .speed(0.01)
                .decimals(3),
        ));
    } else {
        rows.push(PrefRow::qty(
            "Mesh deviation",
            QtyField::new(&mut t.mesh_deviation)
                .range(0.01..=1.0)
                .speed(0.005)
                .decimals(3),
        ));
    }
    rows.push(PrefRow::qty(
        "Angular tolerance",
        QtyField::degrees(&mut t.angular_tolerance_deg).range(0.5..=90.0),
    ));
    rows.push(
        PrefRow::toggle("Weld across faces", &mut t.weld_cross_face)
            .hint("Merge vertices shared by faces with close normals"),
    );
    rows.push(PrefRow::qty(
        "Weld angle threshold",
        QtyField::degrees(&mut t.weld_angle_threshold_deg).range(0.0..=90.0),
    ));
    rows.push(
        PrefRow::toggle("Keep shape snapshots", &mut t.persist_brep_snapshot)
            .hint("Serialize each body's shape and mesh in the background"),
    );
    rows.push(
        PrefRow::toggle("Boundary edges", &mut t.generate_boundary_edges)
            .hint("Face boundaries as edge lines"),
    );
    pref_group(ui, "STEP and IGES import defaults", rows, filter);
    pref_block(ui, &[], filter, |ui| {
        ui.label(
            RichText::new("The import dialog opens with these values.")
                .font(mono(FONT_XS))
                .color(TEXT3),
        );
    });
}
