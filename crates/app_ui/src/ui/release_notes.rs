//! The release notes bundled with the app, shown by the start page's
//! What's new (`crate::build` reads them).

use egui::{RichText, Ui};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, overline};
use ui_kit::{sans, sans_semibold};

use crate::build::{Release, parse};

/// The bundled notes, the running version first.
pub fn bundled() -> Vec<Release<'static>> {
    let mut releases = parse(crate::build::release_notes());
    let running = crate::build::version();
    if let Some(at) = releases.iter().position(|r| r.version == running) {
        let current = releases.remove(at);
        releases.insert(0, current);
    }
    releases
}

/// Every release as a card of topics.
pub fn draw(ui: &mut Ui) {
    let running = crate::build::version();
    for release in bundled() {
        let heading = if release.version == running {
            format!("printCAD {} · this version", release.version)
        } else {
            format!("printCAD {}", release.version)
        };
        ui.label(
            RichText::new(heading)
                .font(sans_semibold(FONT_LG))
                .color(TEXT1),
        );
        Card::new().padding(SPACE_4).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for topic in &release.topics {
                overline(ui, topic.title);
                for change in &topic.changes {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("•").font(sans(FONT_SM)).color(ACCENT));
                        ui.label(RichText::new(*change).font(sans(FONT_SM)).color(TEXT2));
                    });
                }
                ui.add_space(SPACE_2);
            }
        });
        ui.add_space(SPACE_4);
    }
}
