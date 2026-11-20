//! The 3D printing page: the printer's build volume, the slicer and the
//! print layout's gap.

use super::*;

/// The printer's build volume (what the print-bed overlay draws and the
/// print layout fills), the slicer and the layout's gap.
pub(super) fn printing_page(ui: &mut Ui, state: &mut PreferencesState, filter: &str) {
    let printing = &mut state.draft.printing;
    let [x, y, z] = &mut printing.bed_mm;
    pref_group(
        ui,
        "Build volume",
        vec![
            PrefRow::qty(
                "Bed width (X)",
                QtyField::mm(x).range(10.0..=2000.0).speed(1.0).decimals(0),
            ),
            PrefRow::qty(
                "Bed depth (Y)",
                QtyField::mm(y).range(10.0..=2000.0).speed(1.0).decimals(0),
            ),
            PrefRow::qty(
                "Build height (Z)",
                QtyField::mm(z).range(10.0..=2000.0).speed(1.0).decimals(0),
            ),
            PrefRow::toggle("Origin at the bed centre", &mut printing.origin_center)
                .hint("Off, the origin is the bed's front-left corner"),
            PrefRow::toggle("Show the print bed", &mut printing.show_bed)
                .hint("Draw the build volume around the model; the toolbar toggles it too"),
        ],
        filter,
    );
    let command = &mut printing.slicer_command;
    pref_group(
        ui,
        "Slicer",
        vec![
            PrefRow::new("Slicer command", |ui| {
                ui.add(
                    egui::TextEdit::singleline(command)
                        .hint_text("the system's app for the file")
                        .desired_width(260.0)
                        .font(mono(FONT_SM)),
                )
                .changed()
            })
            .hint("Send to slicer (Ctrl+P) runs this with the model's file; {file} places it"),
            PrefRow::select(
                "Format",
                "prefs_slicer_format",
                &mut printing.slicer_format,
                &[
                    (SlicerFormat::ThreeMf, "3MF: one named object per body"),
                    (SlicerFormat::Stl, "STL: triangles only"),
                ],
            ),
            PrefRow::toggle("Send the print layout", &mut printing.slicer_layout).hint(
                "Each part flat on the bed, as many as the parts list prints, rather than the \
             bodies where they sit",
            ),
        ],
        filter,
    );
    pref_group(
        ui,
        "Print layout",
        vec![
            PrefRow::qty(
                "Gap between parts",
                QtyField::mm(&mut printing.layout_gap_mm)
                    .range(0.0..=50.0)
                    .speed(0.5),
            )
            .hint("Half of it is kept from the bed's edges"),
        ],
        filter,
    );
    let mut rows =
        vec![PrefRow::toggle(
        "Make up for shrinkage",
        &mut printing.apply_compensation,
    )
    .hint(
        "STL, 3MF and Send to slicer scale each body by the factors measured from a scanned \
         print, and draw its round holes and bosses larger or smaller (Inspection's \
         compensation, app.print_compensation)",
    )];
    if let Some(c) = printing.compensation.as_mut() {
        let [x, y, z] = &mut c.scale;
        for (label, factor) in [("Scale X", x), ("Scale Y", y), ("Scale Z", z)] {
            rows.push(PrefRow::qty(
                label,
                QtyField::new(factor)
                    .unit("×")
                    .range(0.5..=2.0)
                    .speed(0.0005),
            ));
        }
        for (label, offset) in [("Holes", &mut c.holes), ("Bosses", &mut c.bosses)] {
            rows.push(
                PrefRow::qty(
                    label,
                    QtyField::new(offset)
                        .unit("mm")
                        .range(-1.0..=1.0)
                        .speed(0.005),
                )
                .hint("Drawn this much larger in radius; negative smaller"),
            );
        }
    }
    pref_group(ui, "Shrinkage", rows, filter);
}
