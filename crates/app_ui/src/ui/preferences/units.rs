//! The Units page.

use super::*;

pub(super) fn units_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    pref_group(
        ui,
        "Units",
        vec![
            PrefRow::select(
                "Display unit",
                "prefs_unit",
                &mut state.draft_unit,
                &[
                    (Unit::Mm, Unit::Mm.long_label()),
                    (Unit::Cm, Unit::Cm.long_label()),
                    (Unit::M, Unit::M.long_label()),
                    (Unit::In, Unit::In.long_label()),
                    (Unit::Ft, Unit::Ft.long_label()),
                ],
            )
            .hint("Lengths are stored in millimetres; this only changes how they read"),
        ],
        filter,
    );
    pref_block(ui, &[], filter, |ui| {
        ui.label(
            RichText::new("Saved with the document, not the app.")
                .font(mono(FONT_XS))
                .color(TEXT3),
        );
    });
}
