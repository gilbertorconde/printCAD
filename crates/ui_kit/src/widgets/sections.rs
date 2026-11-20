//! Section headers and the folds that open and close a section.

use super::*;

/// A collapsible section title with a chevron and an optional count. Returns
/// whether the body is open. Drawn as [`section_fold`] with no summary.
pub fn section_header(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    title: &str,
    count: Option<usize>,
    default_open: bool,
) -> bool {
    section_fold(ui, id_salt, title, count, None, default_open)
}

/// A panel section's fold: a hairline across the panel, a chevron, the
/// title in small capitals, its count, and at the right a one-line summary
/// of what it holds while it is closed. The whole row opens and closes it,
/// remembered by `id_salt`. Returns whether the body is open.
pub fn section_fold(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    title: &str,
    count: Option<usize>,
    summary: Option<&str>,
    default_open: bool,
) -> bool {
    section_fold_actions(ui, id_salt, title, count, summary, default_open, &[]).0
}

/// A [`section_fold`] with icon buttons at its right end, before the
/// summary: each `(icon, hint)`. Returns whether the body is open and
/// which action was clicked, by its place.
pub fn section_fold_actions(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    title: &str,
    count: Option<usize>,
    summary: Option<&str>,
    default_open: bool,
    actions: &[(&str, &str)],
) -> (bool, Option<usize>) {
    let id = ui.make_persistent_id(id_salt);
    let mut open = ui
        .data_mut(|d| d.get_persisted::<bool>(id))
        .unwrap_or(default_open);
    ui.add_space(SPACE_1);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SECTION_FOLD),
        Sense::click(),
    );
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, BORDER));
    let hovered = response.hovered();
    let chevron = if open {
        "chevron-down"
    } else {
        "chevron-right"
    };
    let tint = if hovered { TEXT1 } else { TEXT3 };
    if let Some(image) = icon::image(ui.ctx(), chevron, 11.0, tint) {
        image.paint_at(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 6.0, rect.center().y + 1.0),
                Vec2::splat(11.0),
            ),
        );
    }
    let title_color = if hovered { TEXT1 } else { TEXT2 };
    let title =
        ui.painter()
            .layout_no_wrap(title.to_uppercase(), sans_semibold(FONT_XS), title_color);
    let mut x = rect.left() + 16.0;
    let y = rect.center().y + 1.0 - title.size().y / 2.0;
    let title_width = title.size().x;
    ui.painter().galley(egui::pos2(x, y), title, title_color);
    x += title_width + SPACE_2;
    if let Some(n) = count {
        let count = ui
            .painter()
            .layout_no_wrap(n.to_string(), mono(FONT_XS), TEXT3);
        let cy = rect.center().y + 1.0 - count.size().y / 2.0;
        let width = count.size().x;
        ui.painter().galley(egui::pos2(x, cy), count, TEXT3);
        x += width + SPACE_2;
    }
    let mut right = rect.right();
    let mut clicked = None;
    for (i, (icon_name, hint)) in actions.iter().enumerate().rev() {
        let size = Vec2::splat(SECTION_FOLD - 4.0);
        let at = egui::Rect::from_min_size(
            egui::pos2(right - size.x, rect.center().y + 1.0 - size.y / 2.0),
            size,
        );
        right = at.left() - SPACE_1;
        let action = ui
            .interact(at, id.with(("action", i)), Sense::click())
            .on_hover_text(*hint)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if action.hovered() {
            ui.painter()
                .rect_filled(at, CornerRadius::same(RADIUS_SM as u8), BG3);
        }
        let tint = if action.hovered() { TEXT1 } else { TEXT3 };
        if let Some(image) = icon::image(ui.ctx(), icon_name, 12.0, tint) {
            image.paint_at(
                ui,
                egui::Rect::from_center_size(at.center(), Vec2::splat(12.0)),
            );
        }
        if action.clicked() {
            clicked = Some(i);
        }
    }
    if let Some(summary) = summary.filter(|_| !open) {
        let room = (right - x - SPACE_1).max(0.0);
        let job = egui::text::LayoutJob {
            wrap: egui::text::TextWrapping::truncate_at_width(room),
            ..egui::text::LayoutJob::simple_singleline(summary.to_owned(), sans(FONT_XS), TEXT3)
        };
        let galley = ui.painter().layout_job(job);
        let at = egui::pos2(
            right - galley.size().x - SPACE_1,
            rect.center().y + 1.0 - galley.size().y / 2.0,
        );
        ui.painter().galley(at, galley, TEXT3);
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }
    (open, clicked)
}
