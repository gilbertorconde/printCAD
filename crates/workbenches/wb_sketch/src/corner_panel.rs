//! Controls for the selected corner's solved candidate, declared as blocks
//! and drawn by `core_document::panel`.

use crate::SketchWorkbench;
use crate::panel::{button, caption, check, flow, grid, length, note, section};
use crate::rect_panel::paced;
use bench_api::{ButtonStyle, NoteKind, PanelEvent, Tone, Widget};
use core_document::{TaskRequest, WorkbenchRuntimeContext};

impl SketchWorkbench {
    /// The two curves the corner joins, its size, whether the corner point
    /// stays, its problem if it has one, and Apply and Cancel.
    fn corner_widgets(&self, operands: String, speed: f64) -> Vec<Widget> {
        let session = self.session.corner().unwrap();
        let bevel = session.bevel;
        let p = &self.tool_params;
        // Any size typed: the draft, not the field, refuses one it cannot build.
        let size = if bevel {
            length("corner.size", "Setback", p.chamfer_length, None)
        } else {
            length("corner.size", "Radius", p.fillet_radius, None)
        };
        let mut children = vec![
            caption(&operands, Tone::Neutral, false),
            button("corner.again", "Choose again", ButtonStyle::Secondary, true),
            grid(
                "corner_parameters",
                false,
                vec![
                    paced(size, speed),
                    check(
                        "corner.keep",
                        "Corner",
                        "Keep corner",
                        p.corner_keep,
                        true,
                        "The corner stays as a construction point on both curves, with its constraints",
                    ),
                ],
            ),
        ];
        if let Err(reason) = &session.draft.working {
            children.push(note(NoteKind::Warning, reason));
        }
        children.push(flow(
            vec![
                button(
                    "corner.apply",
                    "Apply",
                    ButtonStyle::Primary,
                    session.draft.working.is_ok(),
                ),
                button("corner.cancel", "Cancel", ButtonStyle::Secondary, true),
            ],
            true,
        ));
        let title = if bevel { "Chamfer" } else { "Fillet" };
        vec![section("corner_draft", title, None, None, true, children)]
    }

    pub(crate) fn corner_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) {
        let feature = self.get_active_sketch(ctx).unwrap();
        let operands = self
            .session
            .corner()
            .unwrap()
            .target
            .curves()
            .map(|id| crate::style::element_name(&feature.sketch, id));
        let speed = 1.6 * crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px();
        let widgets = self.corner_widgets(format!("{} + {}", operands[0], operands[1]), speed);
        let out =
            core_document::panel::show(ui, egui::Id::new("sketch_corner_task"), &widgets, None);
        let bevel = self.session.corner().unwrap().bevel;
        let (mut apply, mut cancel, mut changed) = (request.accept, request.cancel, false);
        for event in &out.events {
            match event {
                PanelEvent::Number { id, value } if id == "corner.size" => {
                    let size = if bevel {
                        &mut self.tool_params.chamfer_length
                    } else {
                        &mut self.tool_params.fillet_radius
                    };
                    changed |= *size != *value;
                    *size = *value;
                }
                PanelEvent::Toggle { id, on } if id == "corner.keep" => {
                    changed |= self.tool_params.corner_keep != *on;
                    self.tool_params.corner_keep = *on;
                }
                PanelEvent::Button { id } => match id.as_str() {
                    "corner.again" => {
                        self.discard_corner();
                        return;
                    }
                    "corner.apply" => apply = true,
                    "corner.cancel" => cancel = true,
                    _ => {}
                },
                _ => {}
            }
        }
        if changed {
            self.recalculate_corner();
        }
        let valid = self.session.corner().unwrap().draft.working.is_ok();
        if cancel {
            self.discard_corner();
        } else if apply && valid {
            self.accept_corner(ctx);
        }
    }
}
