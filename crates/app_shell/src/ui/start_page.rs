//! The start page: new-document cards, recent files and the learn rail.

use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Align, Layout, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};
use settings::recent::RecentEntry;
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, overline, planned};
use ui_kit::{mono, sans, sans_medium, sans_semibold};

use super::{FileCommand, StartKind, UiCommand};

pub struct StartPageInputs<'a> {
    pub recent: &'a [RecentEntry],
    /// Substring filter over recent names; UI-local.
    pub search: &'a mut String,
    /// The document the New cards start from, and the benches whose cards
    /// they are.
    pub document: &'a core_document::Document,
    pub registry: &'a core_document::DocumentService,
}

#[derive(Default)]
pub struct StartPageResult {
    pub show_preferences: bool,
}

const RAIL_WIDTH: f32 = 260.0;
const CARD_GAP: f32 = 14.0;

/// "2 hours ago", "Yesterday", "3 days ago", "Last week", "2 weeks ago",
/// "Last month" or the date's year-month.
fn humanize_age(then_ms: u64, now_ms: u64) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    let minutes = secs / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 2 {
        "Just now".to_string()
    } else if hours < 1 {
        format!("{minutes} minutes ago")
    } else if hours < 2 {
        "1 hour ago".to_string()
    } else if days < 1 {
        format!("{hours} hours ago")
    } else if days < 2 {
        "Yesterday".to_string()
    } else if days < 7 {
        format!("{days} days ago")
    } else if days < 14 {
        "Last week".to_string()
    } else if days < 30 {
        format!("{} weeks ago", days / 7)
    } else if days < 60 {
        "Last month".to_string()
    } else if days < 365 {
        format!("{} months ago", days / 30)
    } else {
        format!("{} years ago", days / 365)
    }
}

fn humanize_size(bytes: u64) -> String {
    if bytes == 0 {
        "—".to_string()
    } else if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A card-shaped button: title, subtitle, optional icon.
fn action_card(
    ui: &mut Ui,
    size: Vec2,
    primary: bool,
    icon: Option<&str>,
    title: &str,
    subtitle: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let (fill, border) = if primary {
        (ACCENT_FAINT, ACCENT)
    } else if hovered {
        (BG2, BORDER_STRONG)
    } else {
        (BG1, BORDER)
    };
    ui.painter().rect(
        rect,
        RADIUS_MD,
        fill,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    let icon_rect = Rect::from_min_size(rect.min + vec2(14.0, 14.0), Vec2::splat(20.0));
    if let Some(tex) = icon.and_then(|icon| ui_kit::icon::texture(ui.ctx(), icon)) {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(
            tex.id(),
            icon_rect,
            uv,
            if primary { ACCENT } else { TEXT2 },
        );
    }
    let painter = ui.painter();
    let x = rect.left() + 14.0;
    let title_pos = pos2(x, rect.bottom() - 42.0);
    painter.text(
        title_pos,
        egui::Align2::LEFT_TOP,
        title,
        sans_semibold(FONT_MD),
        TEXT1,
    );
    painter.text(
        pos2(x, rect.bottom() - 24.0),
        egui::Align2::LEFT_TOP,
        subtitle,
        sans(FONT_XS),
        TEXT3,
    );
    response
}

/// A recent-file card: hatched thumbnail area, name, age and size.
fn recent_card(ui: &mut Ui, size: Vec2, entry: &RecentEntry, now: u64) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        RADIUS_MD,
        BG1,
        Stroke::new(1.0, if hovered { BORDER_STRONG } else { BORDER }),
        egui::StrokeKind::Inside,
    );
    // The thumbnail area: hatched until documents carry a preview.
    // PLANNED: a rendered preview saved with the document.
    let thumb = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 56.0));
    painter.rect_filled(thumb, RADIUS_MD, BG0);
    let step = 10.0;
    let mut x = thumb.left() - thumb.height();
    let hatch = Stroke::new(1.0, with_alpha(BORDER, 0.6));
    let clip = painter.with_clip_rect(thumb);
    while x < thumb.right() {
        clip.line_segment(
            [
                pos2(x, thumb.bottom()),
                pos2(x + thumb.height(), thumb.top()),
            ],
            hatch,
        );
        x += step;
    }
    let name = entry.name();
    painter.text(
        thumb.center(),
        egui::Align2::CENTER_CENTER,
        format!("thumbnail · {name}"),
        mono(FONT_XS),
        TEXT3,
    );
    let x = rect.left() + 12.0;
    painter.text(
        pos2(rect.center().x, thumb.bottom() + 10.0),
        egui::Align2::CENTER_TOP,
        &name,
        sans_semibold(FONT_MD),
        TEXT1,
    );
    painter.text(
        pos2(x, rect.bottom() - 22.0),
        egui::Align2::LEFT_TOP,
        humanize_age(entry.last_opened_ms, now),
        sans(FONT_XS),
        TEXT3,
    );
    painter.text(
        pos2(rect.right() - 12.0, rect.bottom() - 22.0),
        egui::Align2::RIGHT_TOP,
        humanize_size(entry.size_bytes),
        mono(FONT_XS),
        TEXT3,
    );
    response.on_hover_text(entry.path.display().to_string())
}

fn nav_item(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if active {
        ui.painter().rect_filled(rect, RADIUS_SM, BG2);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, RADIUS_SM, with_alpha(BG2, 0.6));
    }
    ui.painter().text(
        pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        sans_medium(FONT_SM),
        if active { TEXT1 } else { TEXT2 },
    );
    response
}

fn search_field(ui: &mut Ui, search: &mut String) {
    let (rect, _) = ui.allocate_exact_size(vec2(240.0, INPUT + 2.0), Sense::hover());
    ui.painter().rect(
        rect,
        5.0,
        BG2,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = SPACE_2;
    ui_kit::icon::draw(&mut inner, "search", 14.0, TEXT3);
    inner.add(
        egui::TextEdit::singleline(search)
            .hint_text("Search recent…")
            .frame(egui::Frame::NONE)
            .font(sans(FONT_SM))
            .desired_width(f32::INFINITY),
    );
}

pub fn draw_start_page(
    ui: &mut Ui,
    inputs: StartPageInputs<'_>,
    commands: &mut Vec<UiCommand>,
) -> StartPageResult {
    let mut result = StartPageResult::default();
    let now = now_ms();
    // The bench a new document lands in names the primary card and the
    // rail's default-workbench note.
    let landing = inputs
        .registry
        .landing_workbench()
        .and_then(|id| inputs.registry.descriptor(&id).cloned());
    let landing_label = landing
        .as_ref()
        .map(|d| d.label.clone())
        .unwrap_or_else(|| "New document".to_string());
    let landing_icon = landing
        .as_ref()
        .map(|d| d.icon)
        .unwrap_or("workbench-print");

    egui::Panel::left("start_rail")
        .exact_size(RAIL_WIDTH)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .stroke(Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(16, 24)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = SPACE_1;
            ui.label(
                RichText::new("Start")
                    .font(sans_semibold(FONT_XL))
                    .color(TEXT1),
            );
            ui.label(
                RichText::new(format!("printCAD {} · dev", env!("CARGO_PKG_VERSION")))
                    .font(sans(FONT_XS))
                    .color(TEXT3),
            );
            ui.add_space(SPACE_4);
            nav_item(ui, "Start", true);
            if nav_item(ui, "Open file", false).clicked() {
                commands.push(UiCommand::File(FileCommand::Open));
            }
            // PLANNED: bundled example documents.
            planned(ui, "opens a bundled example document", |ui| {
                nav_item(ui, "Examples", false)
            });
            if nav_item(ui, "Preferences", false).clicked() {
                result.show_preferences = true;
            }
            // PLANNED: release notes for the running version.
            planned(ui, "shows what changed in this version", |ui| {
                nav_item(ui, "What's new", false)
            });

            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                Card::new().fill(BG2).padding(SPACE_3).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Default workbench")
                            .font(sans(FONT_XS))
                            .color(TEXT3),
                    );
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        ui_kit::icon::draw(ui, landing_icon, 14.0, ACCENT);
                        ui.label(
                            RichText::new(&landing_label)
                                .font(sans_medium(FONT_SM))
                                .color(TEXT1),
                        );
                    });
                });
            });
        });

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BG0)
                .inner_margin(egui::Margin::symmetric(40, 28)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let width = ui.available_width().min(1100.0);
                    let card_w = ((width - 3.0 * CARD_GAP) / 4.0).max(160.0);
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = vec2(CARD_GAP, CARD_GAP);

                    overline(ui, "New");
                    ui.horizontal(|ui| {
                        let size = vec2(card_w, 116.0);
                        if action_card(
                            ui,
                            size,
                            true,
                            Some(landing_icon),
                            &landing_label,
                            "Body + sketch, ready to pad",
                        )
                        .clicked()
                        {
                            commands.push(UiCommand::StartNew(StartKind::Landing));
                        }
                        // One card per way a bench offers to begin.
                        for (workbench, entry) in inputs
                            .registry
                            .menu_items(&core_document::MenuScope::StartPage, inputs.document)
                        {
                            if action_card(
                                ui,
                                size,
                                false,
                                entry.icon,
                                &entry.label,
                                entry.hint.as_deref().unwrap_or(""),
                            )
                            .clicked()
                            {
                                commands.push(UiCommand::StartNew(StartKind::Bench {
                                    workbench: workbench.clone(),
                                    command: entry.id.clone(),
                                }));
                            }
                        }
                        // PLANNED: import STL / 3MF meshes as bodies.
                        planned(ui, "imports an STL or 3MF mesh as a body", |ui| {
                            action_card(
                                ui,
                                size,
                                false,
                                Some("workbench-mesh"),
                                "From mesh",
                                "Import STL / 3MF",
                            )
                        });
                        if action_card(ui, size, false, Some("open"), "Open…", "Browse files")
                            .clicked()
                        {
                            commands.push(UiCommand::File(FileCommand::Open));
                        }
                    });

                    ui.add_space(SPACE_4);
                    ui.horizontal(|ui| {
                        overline(ui, "Recent");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            search_field(ui, inputs.search);
                        });
                    });
                    let needle = inputs.search.trim().to_lowercase();
                    let shown: Vec<&RecentEntry> = inputs
                        .recent
                        .iter()
                        .filter(|e| needle.is_empty() || e.name().to_lowercase().contains(&needle))
                        .collect();
                    if shown.is_empty() {
                        Card::new().padding(SPACE_4).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let text = if inputs.recent.is_empty() {
                                "Documents you open or save show up here."
                            } else {
                                "No recent document matches."
                            };
                            ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT3));
                        });
                    }
                    for row in shown.chunks(4) {
                        ui.horizontal(|ui| {
                            for entry in row {
                                let response = recent_card(ui, vec2(card_w, 176.0), entry, now);
                                if response.clicked() {
                                    commands.push(UiCommand::OpenRecent(entry.path.clone()));
                                }
                                response.context_menu(|ui| {
                                    if ui.button("Remove from recent").clicked() {
                                        commands.push(UiCommand::RemoveRecent(entry.path.clone()));
                                        ui.close();
                                    }
                                });
                            }
                        });
                    }

                    ui.add_space(SPACE_4);
                    overline(ui, "Learn");
                    // PLANNED: guided walkthroughs opened in a document.
                    let learn_w = (width - 2.0 * CARD_GAP) / 3.0;
                    ui.horizontal(|ui| {
                        for (title, subtitle) in [
                            ("Sketcher basics", "Constraints, DoF and the solver"),
                            ("Part Design workflow", "Sketch → Pad → Pocket → Fillet"),
                            ("Export for printing", "STL / 3MF and mesh tolerance"),
                        ] {
                            planned(ui, "opens a guided walkthrough", |ui| {
                                action_card(ui, vec2(learn_w, 62.0), false, None, title, subtitle)
                            });
                        }
                    });
                });
        });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_the_way_people_say_them() {
        let h = 3_600_000;
        assert_eq!(humanize_age(0, 60_000), "Just now");
        assert_eq!(humanize_age(0, 2 * h), "2 hours ago");
        assert_eq!(humanize_age(0, 30 * h), "Yesterday");
        assert_eq!(humanize_age(0, 3 * 24 * h), "3 days ago");
        assert_eq!(humanize_age(0, 9 * 24 * h), "Last week");
        assert_eq!(humanize_age(0, 20 * 24 * h), "2 weeks ago");
    }

    #[test]
    fn sizes_pick_the_readable_unit() {
        assert_eq!(humanize_size(0), "—");
        assert_eq!(humanize_size(340 * 1024), "340 KB");
        assert_eq!(humanize_size(1_258_291), "1.2 MB");
    }
}
