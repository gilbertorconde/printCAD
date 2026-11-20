//! The General page: the interface and the About tab.

use super::*;

pub(super) fn general_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    inputs: &PreferencesInputs<'_>,
    filter: &str,
) {
    match state.tab {
        0 => {
            let draft = &mut state.draft;
            pref_group(
                ui,
                "Interface",
                vec![
                    PrefRow::toggle("Workbench tabs", &mut draft.workbench_tabs)
                        .hint("Choose a workbench from tabs instead of a dropdown"),
                    PrefRow::toggle("Tool panels", &mut draft.tabbed_tool_ribbon)
                        .hint("Group tools into panels with pinned buttons and dropdown menus"),
                    PrefRow::toggle("Tool card", &mut draft.tool_hint.show).hint(
                        "Name the active tool and its keys over the view; drag it to move it",
                    ),
                    PrefRow::toggle("Log panel", &mut draft.rendering.show_log_panel)
                        .hint("Show the in-app log under the viewport"),
                    PrefRow::qty(
                        "Frame rate cap",
                        QtyField::new(&mut draft.fps_cap)
                            .unit("fps")
                            .range(0.0..=480.0)
                            .speed(1.0)
                            .decimals(0),
                    )
                    .hint("0 leaves the rate to the display"),
                ],
                filter,
            );
            let autosave = &mut draft.autosave_minutes;
            pref_group(
                ui,
                "Autosave",
                vec![
                    PrefRow::new("Every", move |ui| {
                        let mut minutes = f64::from(*autosave);
                        let response = ui.add(
                            egui::DragValue::new(&mut minutes)
                                .range(0..=120)
                                .suffix(" min"),
                        );
                        ui_kit::widgets::hold_dragged(ui, &response, &mut minutes, &(0.0..=120.0));
                        *autosave = minutes.round() as _;
                        response.changed()
                    })
                    .hint(
                        "A copy of each edited document, which the start page offers back after \
                         a crash; 0 turns it off",
                    ),
                ],
                filter,
            );
            pref_group(
                ui,
                "Session",
                vec![
                    PrefRow::toggle(
                        "Reopen documents at start",
                        &mut draft.session.reopen_documents,
                    )
                    .hint(
                        "Open again the documents that were open when printCAD closed; off \
                         starts on the start page",
                    ),
                ],
                filter,
            );
            pref_group(
                ui,
                "Files",
                vec![
                    PrefRow::toggle(
                        "Ask before saving over a file from an older version",
                        &mut draft.files.ask_before_saving_older,
                    )
                    .hint(
                        "Once saved by this version, the older one may no longer open it; a file \
                         from a newer version always asks",
                    ),
                    PrefRow::toggle(
                        "Ask what units an STL file is in",
                        &mut draft.files.ask_mesh_units,
                    )
                    .hint("STL says nothing of its units; off reads every one in the units below"),
                    PrefRow::select(
                        "STL units",
                        "prefs_mesh_units",
                        &mut draft.files.mesh_units,
                        &crate::scans::MESH_UNITS,
                    )
                    .hint("What an STL is read in when nothing asks, and the first answer offered"),
                ],
                filter,
            );
            pref_group(
                ui,
                "Diagnostics",
                vec![
                    PrefRow::toggle(
                        "Write a report for every STEP or IGES import",
                        &mut draft.diagnostics.import_report,
                    )
                    .hint(
                        "Everything the reader had to say about the file, written to the temp \
                     dir for sending to the kernel or printCAD developers",
                    ),
                ],
                filter,
            );
        }
        _ => {
            pref_group(
                ui,
                "About",
                vec![
                    PrefRow::text(
                        "Version",
                        format!("printCAD {} · dev", crate::build::version()),
                    ),
                    PrefRow::text("GPU", inputs.gpu_name.unwrap_or("Unknown").to_string()),
                    PrefRow::text(
                        "Graphics API",
                        inputs.graphics_api.unwrap_or("Unknown").to_string(),
                    ),
                    PrefRow::text("Geometry kernel", "ogeom (pure Rust)".to_string()),
                ],
                filter,
            );
        }
    }
}
