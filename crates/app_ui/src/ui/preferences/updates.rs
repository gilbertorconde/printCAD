//! The Updates page: the running version and where releases are published.

use super::*;

/// The running version and where releases are published.
pub(super) fn updates_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    release: &package_store::ReleaseCheck,
    filter: &str,
) {
    use package_store::{RELEASES_PAGE, ReleaseCheck};
    let checking = *release == ReleaseCheck::Checking;
    let (status, color) = match release {
        ReleaseCheck::NotChecked => ("Not looked yet".to_string(), TEXT3),
        ReleaseCheck::Checking => ("Looking…".to_string(), TEXT3),
        ReleaseCheck::Found {
            tag, newer: true, ..
        } => (format!("{tag} is out"), SUCCESS),
        ReleaseCheck::Found { tag, .. } => (format!("Up to date ({tag})"), TEXT2),
        ReleaseCheck::Failed(_) => ("Could not look".to_string(), WARNING),
    };
    let failure = match release {
        ReleaseCheck::Failed(e) => Some(e.clone()),
        _ => None,
    };
    let newer_page = match release {
        ReleaseCheck::Found {
            page, newer: true, ..
        } => Some(page.clone()),
        _ => None,
    };
    let check = std::cell::Cell::new(false);
    pref_group(
        ui,
        "Version",
        vec![
            PrefRow::text("This build", crate::build::version().to_string()),
            PrefRow::new("Latest release", |ui| {
                if ui
                    .add_enabled(!checking, |ui: &mut egui::Ui| {
                        ui_kit::widgets::small_secondary_button(ui, "Check now")
                    })
                    .on_hover_text("Look at the releases on GitHub; nothing is downloaded")
                    .clicked()
                {
                    check.set(true);
                }
                if let Some(page) = &newer_page {
                    ui.hyperlink_to(RichText::new("Get it").font(sans(FONT_SM)), page);
                }
                let label = ui.label(RichText::new(&status).font(sans(FONT_SM)).color(color));
                if let Some(e) = &failure {
                    label.on_hover_text(e);
                }
                false
            })
            .hint("Updates are never installed by themselves"),
            PrefRow::new("Releases", |ui| {
                ui.hyperlink_to(
                    RichText::new("github.com/gilbertorconde/printCAD/releases")
                        .font(sans(FONT_SM)),
                    RELEASES_PAGE,
                );
                false
            })
            .hint("Every build is published there"),
        ],
        filter,
    );
    pref_group(
        ui,
        "Looking for updates",
        vec![
            PrefRow::toggle("Look at start", &mut state.draft.updates.check_at_start)
                .hint("Once when printCAD opens; a newer release shows in the log"),
        ],
        filter,
    );
    // The page on screen with nothing looked at yet looks by itself.
    if check.get() || (*release == ReleaseCheck::NotChecked && filter.is_empty()) {
        state.package_request = Some(super::super::UiCommand::CheckForUpdates);
    }
}
