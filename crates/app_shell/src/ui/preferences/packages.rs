//! The Workbench packages pages: installed packages, the store's
//! listings and the stores.

use super::*;

/// Installed workbench packages: whether each loads, what it may reach,
/// removing it, and installing another. Installs, updates and removals
/// take effect at once; turning one on or off and what it may reach, when
/// the draft is applied.
pub(super) fn packages_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    packages: &[workbenches::PackageStatus],
    store: &crate::app::packages::StoreView,
    filter: &str,
) {
    use workbenches::PackageState;
    if filter.is_empty() {
        match state.tab {
            1 => return browse_page(ui, state, packages, store),
            2 => return stores_page(ui, state, store),
            _ => {}
        }
    }
    let words = ["workbench packages plugins install remove extensions"];
    if !pref_matches(filter, &words)
        && !packages
            .iter()
            .any(|p| pref_matches(filter, &[&p.name, &p.id]))
    {
        return;
    }
    ui.label(
        RichText::new(
            "Workbenches others made, each run apart from the app and reaching only its own \
             folder and what you allow here. Installs, updates and removals take effect at once; \
             turning a package on or off and what it may reach, when you apply.",
        )
        .font(sans(FONT_XS))
        .color(TEXT3),
    );
    ui.add_space(SPACE_1);
    if packages.is_empty() {
        ui.label(
            RichText::new("No packages are installed.")
                .font(sans(FONT_SM))
                .color(TEXT2),
        );
    }
    let draft = &mut state.draft.packages;
    for package in packages {
        Card::new().padding(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&package.name)
                        .font(sans_semibold(FONT_SM))
                        .color(TEXT1),
                );
                ui.label(
                    RichText::new(&package.version)
                        .font(mono(FONT_XS))
                        .color(TEXT3),
                );
                let (text, color) = match &package.state {
                    PackageState::Loaded => ("Loaded", SUCCESS),
                    PackageState::Disabled => ("Turned off", TEXT3),
                    PackageState::Failed(_) => ("Did not load", DANGER),
                };
                ui_kit::widgets::badge(ui, text, color);
                if store.lists(package).is_some() {
                    ui_kit::widgets::badge(ui, "Listed", ACCENT)
                        .on_hover_text("In the workbench store's list");
                }
            });
            if let Some(why) = store.lists(package).and_then(|l| l.removed.as_deref()) {
                ui.label(
                    RichText::new(format!("Taken off the store's list: {why}"))
                        .font(sans(FONT_XS))
                        .color(WARNING),
                );
            }
            if !package.description.is_empty() {
                ui.label(
                    RichText::new(&package.description)
                        .font(sans(FONT_XS))
                        .color(TEXT2),
                );
            }
            if let PackageState::Failed(reason) = &package.state {
                ui.label(RichText::new(reason).font(sans(FONT_XS)).color(DANGER));
            }
            ui.label(
                RichText::new(format!("{} · {}", package.id, package.dir.display()))
                    .font(mono(FONT_XS))
                    .color(TEXT3),
            );
            if let Some(source) = &package.source {
                ui.label(
                    RichText::new(format!("github.com/{} · {}", source.repo, source.tag))
                        .font(mono(FONT_XS))
                        .color(TEXT3),
                );
            }
            if let Some(tag) = &package.update
                && primary_button(ui, &format!("Update to {tag}")).clicked()
            {
                state.package_request =
                    Some(super::super::UiCommand::UpdatePackage(package.id.clone()));
            }
            let mut enabled = draft.enabled(&package.id);
            if ui_kit::widgets::check_row(ui, &mut enabled, "Turned on").changed() {
                draft.disabled.retain(|d| d != &package.id);
                if !enabled {
                    draft.disabled.push(package.id.clone());
                }
            }
            let mut grant = draft.grant(&package.id);
            let mut changed = false;
            if package.requested.save_dialog {
                changed |= ui_kit::widgets::check_row(
                    ui,
                    &mut grant.save_dialog,
                    "May ask where to save a file",
                )
                .changed();
            }
            if package.requested.open_dialog {
                changed |= ui_kit::widgets::check_row(
                    ui,
                    &mut grant.open_dialog,
                    "May ask for a file to read",
                )
                .changed();
            }
            if package.requested.helper {
                changed |= ui_kit::widgets::check_row(
                    ui,
                    &mut grant.helper,
                    "May run the programs it ships (they run outside its sandbox)",
                )
                .changed();
            }
            if package.requested.network {
                changed |=
                    ui_kit::widgets::check_row(ui, &mut grant.network, "May use the network")
                        .changed();
            }
            if changed {
                draft.grants.insert(package.id.clone(), grant);
            }
            if ui_kit::widgets::small_secondary_button(ui, "Remove").clicked() {
                state.package_request =
                    Some(super::super::UiCommand::RemovePackage(package.id.clone()));
            }
        });
        ui.add_space(SPACE_1);
    }
    ui.add_space(SPACE_2);
    {
        ui.label(
            RichText::new("Install from GitHub")
                .font(sans_semibold(FONT_SM))
                .color(TEXT1),
        );
        ui.label(
            RichText::new(
                "A repository whose releases carry a .pcbench file: its address takes the latest \
                 release, a release's address that one.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.package_repo)
                    .hint_text("https://github.com/owner/repo")
                    .font(mono(FONT_SM))
                    .desired_width(320.0),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let typed = !state.package_repo.trim().is_empty();
            let click = ui
                .add_enabled(typed, |ui: &mut egui::Ui| secondary_button(ui, "Install"))
                .clicked();
            if typed && (click || enter) {
                state.package_request = Some(super::super::UiCommand::InstallPackageFromGithub(
                    std::mem::take(&mut state.package_repo),
                ));
            }
        });
    }
    ui.add_space(SPACE_1);
    ui.horizontal(|ui| {
        if primary_button(ui, "Install from a file…").clicked() {
            state.package_request = Some(super::super::UiCommand::InstallPackage);
        }
        if packages.iter().any(|p| p.source.is_some())
            && secondary_button(ui, "Check for updates").clicked()
        {
            state.package_request = Some(super::super::UiCommand::CheckPackageUpdates);
        }
    });
    ui_kit::widgets::check_row(
        ui,
        &mut state.draft.packages.check_updates,
        "Check for updates of packages from GitHub when printCAD starts",
    );
}

/// The workbench stores: every package their lists hold, to read about
/// and install, under a word of care about software from other people.
pub(super) fn browse_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    packages: &[workbenches::PackageStatus],
    store: &crate::app::packages::StoreView,
) {
    use crate::app::packages::store_name;
    // The page on screen with a store not fetched yet fetches by itself.
    if store.unlooked() {
        state.package_request = Some(super::super::UiCommand::LookAtStores);
    }
    ui_kit::widgets::note_card(
        ui,
        ui_kit::widgets::Note::Warning,
        Some("Software from other people"),
        "These workbenches are made by people outside printCAD. A store checks that each \
         is what it says it is; it does not check what its code does. Install the ones you \
         trust, from stores you trust, and allow each only what it needs: the network lets it \
         reach the internet, and running its own programs takes it outside its sandbox.",
    );
    ui.add_space(SPACE_1);

    let listings: Vec<(&str, &workbenches::Listing)> = store
        .listings()
        // A package taken off a list shows only to whoever has it.
        .filter(|(_, l)| l.removed.is_none() || packages.iter().any(|p| p.id == l.id))
        .collect();
    let mut categories: Vec<&str> = listings
        .iter()
        .flat_map(|(_, l)| l.categories.iter().map(String::as_str))
        .collect();
    categories.sort_unstable();
    categories.dedup();
    let category_names: Vec<String> = std::iter::once("Every category".to_string())
        .chain(categories.iter().map(|c| c.replace('-', " ")))
        .collect();
    let category_options: Vec<(usize, &str)> = category_names
        .iter()
        .enumerate()
        .map(|(i, name)| (i, name.as_str()))
        .collect();
    let names: Vec<String> = std::iter::once("Every store".to_string())
        .chain(
            store
                .stores
                .iter()
                .map(|(url, s)| store_name(url, s.index.as_ref())),
        )
        .collect();
    let store_options: Vec<(usize, &str)> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (i, name.as_str()))
        .collect();
    if state.store_category >= category_options.len() {
        state.store_category = 0;
    }
    if state.store_pick >= store_options.len() {
        state.store_pick = 0;
    }
    let several = store.stores.len() > 1;
    let looking = store.stores.iter().any(|(_, s)| s.looking);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.store_query)
                .hint_text("Search the stores")
                .font(sans(FONT_SM))
                .desired_width(220.0),
        );
        ui_kit::widgets::select_field(
            ui,
            "store_category",
            &mut state.store_category,
            &category_options,
            150.0,
        );
        if several {
            ui_kit::widgets::select_field(
                ui,
                "store_pick",
                &mut state.store_pick,
                &store_options,
                150.0,
            );
        }
        let refresh = ui.add_enabled(!looking && !store.stores.is_empty(), |ui: &mut egui::Ui| {
            ui_kit::widgets::small_secondary_button(ui, "Refresh")
        });
        if refresh.clicked() {
            state.package_request = Some(super::super::UiCommand::LookAtStores);
        }
    });
    let status = if store.stores.is_empty() {
        "No stores: add one on the Stores tab".to_string()
    } else if looking {
        "Reading the stores' lists…".to_string()
    } else {
        format!(
            "{} package{} listed by {} store{}",
            listings.len(),
            if listings.len() == 1 { "" } else { "s" },
            store.stores.len(),
            if several { "s" } else { "" }
        )
    };
    ui.label(RichText::new(status).font(sans(FONT_XS)).color(TEXT3));
    ui.add_space(SPACE_1);

    let query = state.store_query.trim().to_lowercase();
    let category = (state.store_category > 0).then(|| categories[state.store_category - 1]);
    let picked = (state.store_pick > 0).then(|| store.stores[state.store_pick - 1].0.as_str());
    let shown = listings.iter().filter(|(url, l)| {
        let words = format!("{} {} {}", l.name, l.id, l.description).to_lowercase();
        (query.is_empty() || words.contains(&query))
            && category.is_none_or(|c| l.categories.iter().any(|lc| lc == c))
            && picked.is_none_or(|p| p == *url)
    });
    for (url, listing) in shown {
        let from = several.then(|| {
            let index = store
                .stores
                .iter()
                .find(|(u, _)| u == url)
                .and_then(|(_, s)| s.index.as_ref());
            store_name(url, index)
        });
        listing_card(ui, state, packages, url, from.as_deref(), listing);
        ui.add_space(SPACE_1);
    }
}

/// The Stores tab: the stores Browse reads, as the draft keeps them, each
/// with what its last read found and a Remove; an address to add, and
/// printCAD's own to put back when it was removed.
pub(super) fn stores_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    store: &crate::app::packages::StoreView,
) {
    use crate::app::packages::store_name;
    // What each store holds shows once it is read.
    if store.unlooked() {
        state.package_request = Some(super::super::UiCommand::LookAtStores);
    }
    ui.label(
        RichText::new(
            "Each is the address of a registry's index; Browse lists the packages of all of \
             them. Changes take effect when you apply.",
        )
        .font(sans(FONT_XS))
        .color(TEXT3),
    );
    let mut remove = None;
    for (at, url) in state.draft.packages.stores.iter().enumerate() {
        let known = store.stores.iter().find(|(u, _)| u == url).map(|(_, s)| s);
        Card::new().padding(8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(store_name(url, known.and_then(|s| s.index.as_ref())))
                        .font(sans_semibold(FONT_SM))
                        .color(TEXT1),
                );
                if url == settings::STORE_INDEX {
                    ui_kit::widgets::badge(ui, "printCAD's", ACCENT);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui_kit::widgets::small_secondary_button(ui, "Remove").clicked() {
                        remove = Some(at);
                    }
                });
            });
            ui.label(RichText::new(url).font(mono(FONT_XS)).color(TEXT3));
            let (said, color) = match known {
                None => ("Read when you apply".to_string(), TEXT3),
                Some(s) if s.looking => ("Reading…".to_string(), TEXT3),
                Some(s) => match (&s.error, &s.index) {
                    (Some(e), Some(_)) => (format!("Showing the list as last read: {e}"), WARNING),
                    (Some(e), None) => (e.clone(), WARNING),
                    (None, Some(index)) => (
                        format!(
                            "{} package{}, as of {}",
                            index.packages.len(),
                            if index.packages.len() == 1 { "" } else { "s" },
                            index.generated.replace('T', " ").trim_end_matches('Z')
                        ),
                        TEXT3,
                    ),
                    (None, None) => ("Not read yet".to_string(), TEXT3),
                },
            };
            ui.label(RichText::new(said).font(sans(FONT_XS)).color(color));
        });
        ui.add_space(SPACE_1);
    }
    if let Some(at) = remove {
        state.draft.packages.stores.remove(at);
    }
    let typed = state.store_add.trim().to_string();
    let valid = (typed.starts_with("https://") || typed.starts_with("http://"))
        && !state.draft.packages.stores.contains(&typed);
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut state.store_add)
                .hint_text("https://…/index.json")
                .font(mono(FONT_XS))
                .desired_width(320.0),
        );
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let add = ui
            .add_enabled(valid, |ui: &mut egui::Ui| secondary_button(ui, "Add store"))
            .clicked();
        if valid && (add || enter) {
            state.draft.packages.stores.push(typed.clone());
            state.store_add.clear();
        }
    });
    if !state
        .draft
        .packages
        .stores
        .iter()
        .any(|u| u == settings::STORE_INDEX)
        && ui_kit::widgets::small_secondary_button(ui, "Add printCAD's store").clicked()
    {
        state
            .draft
            .packages
            .stores
            .insert(0, settings::STORE_INDEX.to_string());
    }
}

/// One package the store lists: what it is, what it asks to reach, and
/// installing or updating it.
pub(super) fn listing_card(
    ui: &mut Ui,
    state: &mut PreferencesState,
    packages: &[workbenches::PackageStatus],
    url: &str,
    from: Option<&str>,
    listing: &workbenches::Listing,
) {
    let installed = packages.iter().find(|p| p.id == listing.id);
    let installable = listing.installable();
    Card::new().padding(10.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&listing.name)
                    .font(sans_semibold(FONT_SM))
                    .color(TEXT1),
            );
            if let Some(release) = &listing.release {
                ui.label(
                    RichText::new(&release.version)
                        .font(mono(FONT_XS))
                        .color(TEXT3),
                );
            }
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match (installed, &installable) {
                    (Some(status), Ok(release)) => {
                        let tag = status.source.as_ref().map_or("", |s| s.tag.as_str());
                        let newer = status.source.is_some()
                            && workbenches::is_newer(&release.tag, tag, &status.version);
                        if newer {
                            if primary_button(ui, &format!("Update to {}", release.tag)).clicked() {
                                state.package_request = Some(
                                    super::super::UiCommand::UpdatePackage(listing.id.clone()),
                                );
                            }
                        } else {
                            ui_kit::widgets::badge(ui, "Installed", SUCCESS);
                        }
                    }
                    (Some(_), Err(_)) => {
                        ui_kit::widgets::badge(ui, "Installed", SUCCESS);
                    }
                    (None, Ok(_)) => {
                        if primary_button(ui, "Install").clicked() {
                            state.package_request = Some(super::super::UiCommand::InstallListed {
                                store: url.to_string(),
                                id: listing.id.clone(),
                            });
                        }
                    }
                    (None, Err(_)) => {}
                },
            );
        });
        if !listing.description.is_empty() {
            ui.label(
                RichText::new(&listing.description)
                    .font(sans(FONT_XS))
                    .color(TEXT2),
            );
        }
        if let Err(why) = &installable {
            let color = if listing.removed.is_some() {
                WARNING
            } else {
                DANGER
            };
            ui.label(RichText::new(why).font(sans(FONT_XS)).color(color));
        }
        // What it asks to reach beyond its own folder, the weightiest first.
        ui.horizontal_wrapped(|ui| {
            if let Some(release) = &listing.release {
                let asks = &release.capabilities;
                if asks.helper {
                    ui_kit::widgets::badge(ui, "Runs its own programs", DANGER)
                        .on_hover_text("Outside its sandbox, once you allow it");
                }
                if asks.network {
                    ui_kit::widgets::badge(ui, "Uses the network", WARNING)
                        .on_hover_text("Once you allow it");
                }
                if asks.save_dialog {
                    ui_kit::widgets::badge(ui, "Asks where to save files", TEXT3);
                }
                if asks.open_dialog {
                    ui_kit::widgets::badge(ui, "Asks for files to read", TEXT3);
                }
                if !(asks.helper || asks.network || asks.save_dialog || asks.open_dialog) {
                    ui.label(
                        RichText::new("Reaches nothing beyond its own folder")
                            .font(sans(FONT_XS))
                            .color(TEXT3),
                    );
                }
            }
            for category in &listing.categories {
                ui_kit::widgets::badge(ui, &category.replace('-', " "), TEXT3);
            }
            if let Some(from) = from {
                ui_kit::widgets::badge(ui, &format!("from {from}"), ACCENT);
            }
        });
        let by = if listing.maintainers.is_empty() {
            String::new()
        } else {
            format!(" · by {}", listing.maintainers.join(", "))
        };
        ui.label(
            RichText::new(format!("{} · {}{by}", listing.id, listing.license))
                .font(mono(FONT_XS))
                .color(TEXT3),
        );
        ui.horizontal(|ui| {
            ui.hyperlink_to(
                RichText::new(format!("github.com/{}", listing.repository)).font(mono(FONT_XS)),
                format!("https://github.com/{}", listing.repository),
            );
            if let Some(homepage) = &listing.homepage {
                ui.hyperlink_to(RichText::new("Documentation").font(sans(FONT_XS)), homepage);
            }
        });
    });
}
