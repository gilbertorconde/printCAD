//! Parameters of the pending rectangle, kept outside the saved sketch. The
//! panel is declared as blocks and drawn by `core_document::panel`, as the
//! sketcher's own task panel is. Lengths take any value typed: the draft,
//! not the field, refuses a size it cannot build, and Apply stays off.
use crate::SketchWorkbench;
use crate::panel::{button, check, choose, degrees, flow, grid, length, note, offset, section};
use crate::rect_session::CornerStyle;
use bench_api::{ButtonStyle, NoteKind, PanelEvent, Widget};
use core_document::{TaskRequest, WorkbenchRuntimeContext};

const CORNERS: [(CornerStyle, &str); 3] = [
    (CornerStyle::Sharp, "Sharp"),
    (CornerStyle::Fillet, "Fillet"),
    (CornerStyle::Chamfer, "Chamfer"),
];

impl SketchWorkbench {
    /// The pending rectangle's fields, its problem if it has one, and Apply
    /// and Cancel. Lengths drag at a pace that follows the view's zoom.
    fn rectangle_widgets(&self, speed: f64) -> Vec<Widget> {
        let session = self.session.rectangle().unwrap();
        let p = session.parameters;
        let mut fields = vec![
            paced(offset("rectangle.x", "Center X", p.x), speed),
            paced(offset("rectangle.y", "Center Y", p.y), speed),
            paced(length("rectangle.width", "Width", p.width, None), speed),
            paced(length("rectangle.height", "Height", p.height, None), speed),
            degrees("rectangle.angle", "Angle", p.angle),
            choose("rectangle.corners", "Corners", &CORNERS, p.corners, 160.0),
        ];
        match p.corners {
            CornerStyle::Sharp => {}
            CornerStyle::Fillet => {
                fields.push(paced(
                    length("rectangle.size", "Radius", p.size, None),
                    speed,
                ));
            }
            CornerStyle::Chamfer => {
                fields.push(paced(
                    length("rectangle.size", "Setback", p.size, None),
                    speed,
                ));
            }
        }
        fields.push(check(
            "rectangle.frame",
            "Frame",
            "Inner contour",
            p.wall.is_some(),
            true,
            "A second rectangle inside, one wall in from the first",
        ));
        if let Some(wall) = p.wall {
            fields.push(paced(length("rectangle.wall", "Wall", wall, None), speed));
        }
        fields.push(check(
            "rectangle.construction",
            "",
            "Construction",
            session.construction,
            true,
            "The rectangle's lines are construction geometry",
        ));
        let mut children = vec![grid("rectangle_parameters", false, fields)];
        if let Err(reason) = &session.draft.working {
            children.push(note(NoteKind::Warning, reason));
        }
        children.push(flow(
            vec![
                button(
                    "rectangle.apply",
                    "Apply",
                    ButtonStyle::Primary,
                    session.draft.working.is_ok(),
                ),
                button("rectangle.cancel", "Cancel", ButtonStyle::Secondary, true),
            ],
            true,
        ));
        vec![section(
            "rectangle_draft",
            "Rectangle",
            None,
            None,
            true,
            children,
        )]
    }

    pub(crate) fn rectangle_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) {
        let feature = self.get_active_sketch(ctx).unwrap();
        let speed = 1.6 * crate::overlay::SketchProjector::new(ctx, feature.plane).units_per_px();
        let widgets = self.rectangle_widgets(speed);
        let out =
            core_document::panel::show(ui, egui::Id::new("sketch_rectangle_task"), &widgets, None);
        let session = self.session.rectangle().unwrap();
        let mut parameters = session.parameters;
        let mut construction = session.construction;
        let before = (parameters, construction);
        let (mut apply, mut cancel) = (request.accept, request.cancel);
        for event in &out.events {
            match event {
                PanelEvent::Number { id, value } => match id.as_str() {
                    "rectangle.x" => parameters.x = *value,
                    "rectangle.y" => parameters.y = *value,
                    "rectangle.width" => parameters.width = *value,
                    "rectangle.height" => parameters.height = *value,
                    "rectangle.angle" => parameters.angle = *value,
                    "rectangle.size" => parameters.size = *value,
                    "rectangle.wall" => {
                        if let Some(wall) = &mut parameters.wall {
                            *wall = *value;
                        }
                    }
                    _ => {}
                },
                PanelEvent::Choice { id, index } if id == "rectangle.corners" => {
                    let corners = CORNERS[*index].0;
                    // A first rounding or bevel takes the tool's own size.
                    if corners != parameters.corners && parameters.size <= 0. {
                        parameters.size = match corners {
                            CornerStyle::Sharp => parameters.size,
                            CornerStyle::Fillet => self.tool_params.fillet_radius,
                            CornerStyle::Chamfer => self.tool_params.chamfer_length,
                        };
                    }
                    parameters.corners = corners;
                }
                PanelEvent::Toggle { id, on } => match id.as_str() {
                    "rectangle.frame" => {
                        parameters.wall = on.then_some(self.tool_params.offset_distance);
                    }
                    "rectangle.construction" => construction = *on,
                    _ => {}
                },
                PanelEvent::Button { id } => match id.as_str() {
                    "rectangle.apply" => apply = true,
                    "rectangle.cancel" => cancel = true,
                    _ => {}
                },
                _ => {}
            }
        }
        if (parameters, construction) != before {
            let session = self.session.rectangle_mut().unwrap();
            session.parameters = parameters;
            session.construction = construction;
            self.recalculate_rectangle();
        }
        let valid = self.session.rectangle().unwrap().draft.working.is_ok();
        if cancel {
            self.discard_rectangle();
        } else if apply && valid {
            self.accept_rectangle(ctx);
        }
    }
}

/// A length field dragged at `speed` units a point.
pub(crate) fn paced(widget: Widget, speed: f64) -> Widget {
    match widget {
        Widget::Quantity(mut quantity) => {
            quantity.speed = Some(speed);
            quantity.into()
        }
        other => other,
    }
}
