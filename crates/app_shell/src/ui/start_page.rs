//! The start page: new-document cards, recent files and the learn rail.

use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Align, Layout, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};
use settings::recent::RecentEntry;
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, overline};
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
    /// What the main area shows; UI-local.
    pub view: &'a mut StartView,
    /// The recent documents' previews, loaded once per file version.
    pub thumbnails: &'a mut ThumbnailCache,
}

/// Previews read from the recent documents, keyed by path and the file's
/// modification time, so a save shows its new preview and nothing is read
/// twice.
#[derive(Default)]
pub struct ThumbnailCache {
    loaded: std::collections::HashMap<
        std::path::PathBuf,
        (Option<SystemTime>, Option<egui::TextureHandle>),
    >,
}

impl ThumbnailCache {
    /// The preview of the document at `path`, reading it when the file is
    /// new to the cache or has changed since.
    fn get(&mut self, ctx: &egui::Context, path: &std::path::Path) -> Option<egui::TextureHandle> {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some((seen, texture)) = self.loaded.get(path)
            && *seen == modified
        {
            return texture.clone();
        }
        let texture = core_document::Document::read_thumbnail(path)
            .and_then(|png| tiny_skia::Pixmap::decode_png(&png).ok())
            .map(|pixmap| {
                let size = [pixmap.width() as usize, pixmap.height() as usize];
                let rgba: Vec<u8> = pixmap
                    .pixels()
                    .iter()
                    .flat_map(|p| {
                        let c = p.demultiply();
                        [c.red(), c.green(), c.blue(), c.alpha()]
                    })
                    .collect();
                ctx.load_texture(
                    format!("recent-thumbnail:{}", path.display()),
                    egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                    egui::TextureOptions::LINEAR,
                )
            });
        self.loaded
            .insert(path.to_path_buf(), (modified, texture.clone()));
        texture
    }
}

/// The start page's main area: the cards, or the release notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartView {
    #[default]
    Start,
    WhatsNew,
    /// The exporting walkthrough's steps.
    ExportGuide,
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
        "-".to_string()
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

/// A recent-file card: the document's preview (hatched when it has none),
/// name, age and size.
fn recent_card(
    ui: &mut Ui,
    size: Vec2,
    entry: &RecentEntry,
    now: u64,
    preview: Option<&egui::TextureHandle>,
) -> egui::Response {
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
    let thumb = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 56.0));
    painter.rect_filled(thumb, RADIUS_MD, BG0);
    let name = entry.name();
    if let Some(texture) = preview {
        // Fitted inside the area, keeping the preview's proportions.
        let [w, h] = texture.size().map(|v| v as f32);
        let area = thumb.shrink(6.0);
        let scale = (area.width() / w).min(area.height() / h);
        let shown = Rect::from_center_size(area.center(), vec2(w * scale, h * scale));
        painter.image(
            texture.id(),
            shown,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        hatch(painter, thumb, &name);
    }
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

/// The preview area of a document saved without one: hatched, with the
/// document's name across it.
fn hatch(painter: &egui::Painter, thumb: Rect, name: &str) {
    let step = 10.0;
    let mut x = thumb.left() - thumb.height();
    let stroke = Stroke::new(1.0, with_alpha(BORDER, 0.6));
    let clip = painter.with_clip_rect(thumb);
    while x < thumb.right() {
        clip.line_segment(
            [
                pos2(x, thumb.bottom()),
                pos2(x + thumb.height(), thumb.top()),
            ],
            stroke,
        );
        x += step;
    }
    painter.text(
        thumb.center(),
        egui::Align2::CENTER_CENTER,
        format!("no preview · {name}"),
        mono(FONT_XS),
        TEXT3,
    );
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
            if nav_item(ui, "Start", *inputs.view == StartView::Start).clicked() {
                *inputs.view = StartView::Start;
            }
            if nav_item(ui, "Open file", false).clicked() {
                commands.push(UiCommand::File(FileCommand::Open));
            }
            if nav_item(ui, "Examples", false).clicked() {
                commands.push(UiCommand::StartNew(StartKind::Example(
                    bench_fixtures::Scene::Pocket,
                )));
            }
            if nav_item(ui, "Preferences", false).clicked() {
                result.show_preferences = true;
            }
            if nav_item(ui, "What's new", *inputs.view == StartView::WhatsNew).clicked() {
                *inputs.view = StartView::WhatsNew;
            }

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
                    match *inputs.view {
                        StartView::WhatsNew => {
                            overline(ui, "What's new");
                            super::release_notes::draw(ui);
                            return;
                        }
                        StartView::ExportGuide => {
                            export_guide(ui, inputs.view, commands);
                            return;
                        }
                        StartView::Start => {}
                    }

                    overline(ui, "New");
                    ui.horizontal(|ui| {
                        let size = vec2(card_w, 116.0);
                        if action_card(
                            ui,
                            size,
                            true,
                            Some(landing_icon),
                            &landing_label,
                            "A new body, ready for its first sketch",
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
                        if action_card(
                            ui,
                            size,
                            false,
                            Some("workbench-mesh"),
                            "From mesh",
                            "Import STL / OBJ / 3MF",
                        )
                        .clicked()
                        {
                            commands.push(UiCommand::File(FileCommand::ImportStep));
                        }
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
                                let preview = inputs.thumbnails.get(ui.ctx(), &entry.path);
                                let response = recent_card(
                                    ui,
                                    vec2(card_w, 176.0),
                                    entry,
                                    now,
                                    preview.as_ref(),
                                );
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
                    let learn_w = (width - 2.0 * CARD_GAP) / 3.0;
                    ui.horizontal(|ui| {
                        // Each card opens an example built for it.
                        for (title, subtitle, scene) in [
                            (
                                "Sketcher basics",
                                "A dimensioned sketch, open for editing",
                                bench_fixtures::Scene::Sketch,
                            ),
                            (
                                "Part Design workflow",
                                "Sketch → Pad → Pocket, ready to fillet",
                                bench_fixtures::Scene::Pocket,
                            ),
                        ] {
                            if action_card(ui, vec2(learn_w, 62.0), false, None, title, subtitle)
                                .clicked()
                            {
                                commands.push(UiCommand::StartNew(StartKind::Example(scene)));
                            }
                        }
                        if action_card(
                            ui,
                            vec2(learn_w, 62.0),
                            false,
                            None,
                            "Export for printing",
                            "STL / 3MF and mesh tolerance",
                        )
                        .clicked()
                        {
                            *inputs.view = StartView::ExportGuide;
                        }
                    });
                });
        });
    result
}

/// The export walkthrough: each step's title and what it says.
const EXPORT_STEPS: [(&str, &str); 5] = [
    (
        "Show what you want to print",
        "Export writes every visible body, or only the selected one. Hide the \
         rest in the tree first, or tick Selected body only.",
    ),
    (
        "Open File › Export (Ctrl+E)",
        "Pick the format. 3MF keeps each body as its own named, closed mesh \
         and is what current slicers prefer; STL is the one every slicer reads. \
         STEP carries the exact shapes, for other CAD rather than for printing.",
    ),
    (
        "Choose the mesh tolerance",
        "The chord tolerance is how far a facet may stray from the true surface, \
         the angular one how far a curve may turn across one facet. A tenth of \
         your nozzle width is plenty: finer only makes the file larger.",
    ),
    (
        "Name the file",
        "The export runs beside the window and the log says when it is written, \
         with the triangle count.",
    ),
    (
        "Or send it straight to the slicer",
        "File › Send to slicer (Ctrl+P) writes every visible body to a temporary \
         file and opens it in your slicer. Choose the slicer and the format in \
         Preferences › 3D printing; left empty, the system's app for the file \
         opens it.",
    ),
];

/// The steps of exporting a part for a slicer, and a button that walks
/// them on the pocketed example.
fn export_guide(ui: &mut Ui, view: &mut StartView, commands: &mut Vec<UiCommand>) {
    overline(ui, "Export for printing");
    ui.label(
        RichText::new("From a solid to a file a slicer reads")
            .font(sans_semibold(FONT_LG))
            .color(TEXT1),
    );
    let steps = EXPORT_STEPS;
    for (n, (title, body)) in steps.iter().enumerate() {
        Card::new().padding(SPACE_4).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}", n + 1))
                        .font(sans_semibold(FONT_XL))
                        .color(ACCENT),
                );
                ui.add_space(SPACE_2);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(*title)
                            .font(sans_semibold(FONT_MD))
                            .color(TEXT1),
                    );
                    ui.label(RichText::new(*body).font(sans(FONT_SM)).color(TEXT2));
                });
            });
        });
    }
    ui.add_space(SPACE_2);
    ui.horizontal(|ui| {
        if ui_kit::widgets::primary_button(ui, "Try it on the example part").clicked() {
            commands.push(UiCommand::StartNew(StartKind::ExportWalkthrough));
            *view = StartView::Start;
        }
        if ui_kit::widgets::secondary_button(ui, "Back").clicked() {
            *view = StartView::Start;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_walkthrough_reads_as_plain_sentences() {
        for (title, body) in EXPORT_STEPS {
            assert!(!body.contains("  "), "{title}: a run of spaces in {body:?}");
            assert!(body.ends_with('.'), "{title}");
        }
    }

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
        assert_eq!(humanize_size(0), "-");
        assert_eq!(humanize_size(340 * 1024), "340 KB");
        assert_eq!(humanize_size(1_258_291), "1.2 MB");
    }
}
