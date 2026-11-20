//! The widget vocabulary of the mockups: cards, badges, toggles, quantity
//! and select fields, check rows, note cards, buttons, tool buttons and
//! section headers. Each is a plain function over `egui::Ui` so any crate
//! that draws UI can use it.

use egui::{
    Color32, CornerRadius, Frame, InnerResponse, Layout, Margin, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetText, WidgetType,
};

use crate::icon;
use crate::theme::{mono, sans, sans_semibold};
use crate::tokens::*;

mod buttons;
mod chips;
mod display;
mod formula;
mod prefs;
mod quantity;
mod rows;
mod sections;
mod select;
mod tabs;

pub use buttons::{
    TOOL_MENU_STRIP, ToolButtonState, accent_outline_button, check_row, destructive_button,
    icon_button, primary_button, secondary_button, small_secondary_button, toggle, tool_button,
};
pub use chips::{ChipLook, PickChipOutcome, badges, chip, chip_width, chips, pick_chip};
pub use display::{
    Card, Note, badge, key_chip, mono_label, note_card, overline, planned, status_pill, text,
    vseparator,
};
pub use formula::{FormulaEdit, FormulaField, FormulaHost};
pub use prefs::{PrefRow, pref_block, pref_group, pref_matches};
pub use quantity::{
    QtyEdit, QtyField, QtyValue, hold_dragged, intersect, lock_toggle, qty_readout, step_within,
    wheel_step,
};
pub use rows::{
    LabelColumn, RowLook, field_label, filter_field, list_row_with, row_detail, row_value,
};
pub use sections::{section_fold, section_fold_actions, section_header};
pub use select::{SelectOptions, fitted_menu, select_control, select_field};
pub use tabs::{Tab, TabReorder, TabResponse, tab_plus};

#[cfg(test)]
mod accessibility_tests {
    use super::*;

    /// One of each custom-painted widget, in the order Tab reaches them.
    fn draw(ui: &mut Ui, on: &mut bool) {
        primary_button(ui, "Import");
        tool_button(ui, "pad", "Pad", 28.0, ToolButtonState::default());
        icon_button(ui, "copy", "Copy");
        toggle(ui, on, "Grid");
        check_row(ui, on, "Snap");
        chip(ui, "Line 3", ChipLook::default());
        pick_chip(ui, Some("Face 2"), "Pick a face", false);
        list_row_with(
            ui,
            "row",
            RowLook {
                selected: true,
                ..Default::default()
            },
            ("pad", TEXT2),
            "Pad001",
            |_| {},
            |_| {},
        );
        Tab::new("Part", true).closable("Close Part").show(ui);
        tab_plus(ui, 24.0);
    }

    /// What one frame with `key` pressed reports: each event's role,
    /// label, enabled and checked state.
    fn press(
        ctx: &egui::Context,
        key: egui::Key,
        draw: impl FnMut(&mut Ui),
    ) -> Vec<(WidgetType, String, bool, Option<bool>)> {
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, draw);
        out.textures_delta.clear();
        out.platform_output
            .events
            .into_iter()
            .filter_map(|e| {
                let info = e.widget_info().clone();
                Some((info.typ, info.label?, info.enabled, info.selected))
            })
            .collect()
    }

    /// Every widget names itself with its role, label and state as Tab
    /// walks focus across them.
    #[test]
    fn custom_widgets_report_their_role_label_and_state() {
        use WidgetType::{Button, Checkbox, SelectableLabel};
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut on = true;
        let mut seen = Vec::new();
        for _ in 0..13 {
            seen.extend(press(&ctx, egui::Key::Tab, |ui| draw(ui, &mut on)));
        }
        let want = [
            (Button, "Import", true, None),
            (Button, "Pad", false, None),
            (Button, "Copy", true, None),
            (Checkbox, "Grid", true, Some(true)),
            (Checkbox, "Snap", true, Some(true)),
            (SelectableLabel, "Line 3", true, Some(false)),
            (Button, "Face 2", true, None),
            (Button, "Clear", true, None),
            (SelectableLabel, "Pad001", true, Some(true)),
            (Button, "Part", true, Some(true)),
            (Button, "Close Part", true, None),
            (Button, "+", true, None),
        ];
        let seen: Vec<_> = seen
            .iter()
            .map(|(t, l, e, s)| (*t, l.as_str(), *e, *s))
            .collect();
        assert_eq!(seen[..want.len()], want);
    }

    /// A switch turned from the keyboard reports the state it turned to.
    #[test]
    fn a_switch_reports_its_new_state() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let mut on = true;
        press(&ctx, egui::Key::Tab, |ui| {
            toggle(ui, &mut on, "Grid");
        });
        let seen = press(&ctx, egui::Key::Space, |ui| {
            toggle(ui, &mut on, "Grid");
        });
        assert!(!on);
        assert_eq!(
            seen,
            [(WidgetType::Checkbox, "Grid".into(), true, Some(false))]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::step_within;

    #[test]
    fn a_step_stops_on_a_limit_that_is_no_whole_step() {
        // (from, up, step, most) -> to
        let cases = [
            (4.0, true, 1.0, 5.3, 5.0),
            (5.0, true, 1.0, 5.3, 5.3),
            (5.3, true, 1.0, 5.3, 5.3),
            (5.3, false, 1.0, 5.3, 5.0),
            (5.0, false, 1.0, 5.3, 4.0),
            (4.3, true, 1.0, 9.0, 5.0),
            (0.30000000000000004, true, 0.1, 0.45, 0.4),
            (0.4, true, 0.1, 0.45, 0.45),
            (0.45, false, 0.1, 0.45, 0.4),
        ];
        for (from, up, step, most, want) in cases {
            let got = step_within(from, up, step, 0.0, most);
            assert!(
                (got - want).abs() < 1e-9,
                "{from} {} by {step} under {most}: {got}, not {want}",
                if up { "up" } else { "down" }
            );
        }
    }
}
