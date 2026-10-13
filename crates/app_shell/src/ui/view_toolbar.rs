//! The floating view toolbar at the top of the viewport: fit, standard
//! views, draw style, the clipping plane, projection, and in perspective its
//! field of view. With the clipping plane on, a second pill under it sets
//! the plane's axis, position and side; with the measure tool armed, another
//! sets the unit it reads in and starts its chain over.

use egui::{Align2, Area, Context, Vec2};
use settings::{DrawStyle, ProjectionMode};
use ui_kit::tokens::*;
use ui_kit::widgets::{QtyField, ToolButtonState, tool_button, vseparator};

use super::UiCommand;
use super::keymap::Keymap;
use crate::camera::FOV_RANGE_DEG;
use crate::camera::section::{SectionAxis, SectionPlane, SectionToggle};
use crate::orientation_cube::CameraSnapView;

const BUTTON: f32 = 28.0;

enum Item {
    Button {
        icon: &'static str,
        label: &'static str,
        /// The keymap id whose key the tooltip names.
        binding: &'static str,
        on: bool,
        planned: Option<&'static str>,
        command: Option<Box<UiCommand>>,
    },
    Sep,
}

fn button(
    icon: &'static str,
    label: &'static str,
    binding: &'static str,
    command: UiCommand,
) -> Item {
    Item::Button {
        icon,
        label,
        binding,
        on: false,
        planned: None,
        command: Some(Box::new(command)),
    }
}

fn toggled(
    icon: &'static str,
    label: &'static str,
    binding: &'static str,
    on: bool,
    command: UiCommand,
) -> Item {
    Item::Button {
        icon,
        label,
        binding,
        on,
        planned: None,
        command: Some(Box::new(command)),
    }
}

/// What a click in the view picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickFilter {
    /// An edge near the cursor, else the face under it; a double click
    /// the whole body.
    #[default]
    Any,
    Faces,
    Edges,
    /// A click takes the whole body.
    Bodies,
}

impl PickFilter {
    pub const ALL: [PickFilter; 4] = [
        PickFilter::Any,
        PickFilter::Faces,
        PickFilter::Edges,
        PickFilter::Bodies,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PickFilter::Any => "Faces and edges",
            PickFilter::Faces => "Faces only",
            PickFilter::Edges => "Edges only",
            PickFilter::Bodies => "Whole bodies",
        }
    }

    /// Whether an edge near the cursor hovers and picks.
    pub fn edges(self) -> bool {
        matches!(self, PickFilter::Any | PickFilter::Edges)
    }

    /// Whether the face under the cursor hovers and picks.
    pub fn faces(self) -> bool {
        matches!(self, PickFilter::Any | PickFilter::Faces)
    }
}

/// The view state the toolbar shows.
pub struct ViewToolbarState {
    pub pick_filter: PickFilter,
    pub projection: ProjectionMode,
    pub field_of_view_deg: f32,
    pub draw_style: DrawStyle,
    pub section: Option<SectionPlane>,
    /// The box around what the scene draws: the clipping plane's range.
    pub scene_bounds: Option<(glam::Vec3, glam::Vec3)>,
    /// The measure tool, while it is armed.
    pub measure: Option<MeasureBar>,
}

/// What the measure tool's pill shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureBar {
    /// The unit chosen for the readout; `None` follows the document's.
    pub unit: Option<core_document::Unit>,
    pub document_unit: core_document::Unit,
    /// How many picks the chain holds.
    pub picks: usize,
}

/// Draws the pill and pushes the commands of any clicked button.
pub fn draw_view_toolbar(
    ctx: &Context,
    viewport: egui::Rect,
    state: &ViewToolbarState,
    keymap: &Keymap,
    commands: &mut Vec<UiCommand>,
) {
    let with_key = |label: &str, binding: &str| match keymap.text(binding) {
        Some(key) => format!("{label} ({key})"),
        None => label.to_string(),
    };
    let ViewToolbarState {
        pick_filter,
        projection,
        field_of_view_deg,
        draw_style,
        section,
        scene_bounds,
        measure,
    } = *state;
    let ortho = projection == ProjectionMode::Orthographic;
    let items = [
        button("fit-all", "Fit all", "view.fit_all", UiCommand::FitView),
        button(
            "fit-selection",
            "Fit selection",
            "view.fit_selection",
            UiCommand::FitSelection,
        ),
        Item::Sep,
        button(
            "view-iso",
            "Isometric",
            "view.isometric",
            UiCommand::CameraSnap(CameraSnapView::FrontTopRight),
        ),
        button(
            "view-front",
            "Front",
            "view.front",
            UiCommand::CameraSnap(CameraSnapView::Front),
        ),
        button(
            "view-top",
            "Top",
            "view.top",
            UiCommand::CameraSnap(CameraSnapView::Top),
        ),
        button(
            "view-right",
            "Right",
            "view.right",
            UiCommand::CameraSnap(CameraSnapView::Right),
        ),
        button(
            "view-rear",
            "Rear",
            "view.rear",
            UiCommand::CameraSnap(CameraSnapView::Rear),
        ),
        button(
            "view-bottom",
            "Bottom",
            "view.bottom",
            UiCommand::CameraSnap(CameraSnapView::Bottom),
        ),
        button(
            "view-left",
            "Left",
            "view.left",
            UiCommand::CameraSnap(CameraSnapView::Left),
        ),
        Item::Sep,
        toggled(
            "draw-style-shaded",
            "Shaded with edges",
            "view.shaded_edges",
            draw_style == DrawStyle::ShadedEdges,
            UiCommand::SetDrawStyle(DrawStyle::ShadedEdges),
        ),
        toggled(
            "draw-style-flat",
            "Shaded",
            "view.shaded",
            draw_style == DrawStyle::Shaded,
            UiCommand::SetDrawStyle(DrawStyle::Shaded),
        ),
        toggled(
            "draw-style-wireframe",
            "Wireframe",
            "view.wireframe",
            draw_style == DrawStyle::Wireframe,
            UiCommand::SetDrawStyle(DrawStyle::Wireframe),
        ),
        toggled(
            "clipping-plane",
            "Clipping plane",
            "view.clipping_plane",
            section.is_some(),
            UiCommand::SetSection(match section {
                Some(_) => None,
                None => Some(SectionToggle::On),
            }),
        ),
        Item::Sep,
        toggled(
            "view-orthographic",
            "Orthographic",
            "view.orthographic",
            ortho,
            UiCommand::SetProjection(ProjectionMode::Orthographic),
        ),
        toggled(
            "view-perspective",
            "Perspective",
            "view.perspective",
            !ortho,
            UiCommand::SetProjection(ProjectionMode::Perspective),
        ),
    ];

    Area::new(egui::Id::new("view_toolbar"))
        .order(super::hud::VIEW_LAYER)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(viewport.center_top() + Vec2::new(0.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(OVERLAY_CARD)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(7)
                .inner_margin(3)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for item in items {
                            match item {
                                Item::Sep => {
                                    ui.add_space(3.0);
                                    vseparator(ui, 18.0);
                                    ui.add_space(3.0);
                                }
                                Item::Button {
                                    icon,
                                    label,
                                    binding,
                                    on,
                                    planned,
                                    command,
                                } => {
                                    let state = ToolButtonState {
                                        enabled: command.is_some(),
                                        active: on,
                                        planned,
                                        menu: false,
                                    };
                                    if tool_button(
                                        ui,
                                        icon,
                                        &with_key(label, binding),
                                        BUTTON,
                                        state,
                                    )
                                    .clicked()
                                        && let Some(command) = command
                                    {
                                        commands.push(*command);
                                    }
                                }
                            }
                        }
                        ui.add_space(3.0);
                        vseparator(ui, 18.0);
                        ui.add_space(3.0);
                        pick_filter_button(ui, pick_filter, commands);
                        // The perspective's strength, dragged or typed: the
                        // object keeps its size, only the distortion changes.
                        if !ortho {
                            let mut fov = field_of_view_deg;
                            let edit = QtyField::degrees(&mut fov)
                                .range(f64::from(FOV_RANGE_DEG.0)..=f64::from(FOV_RANGE_DEG.1))
                                .decimals(0)
                                .width(58.0)
                                .show_settling(ui);
                            if let Some(response) = &edit.response {
                                response.clone().on_hover_text(
                                    "Field of view: drag to change the perspective; \
                                     the object keeps its size",
                                );
                            }
                            if edit.changed || edit.settled {
                                commands.push(UiCommand::SetFieldOfView {
                                    degrees: fov,
                                    settled: edit.settled,
                                });
                            }
                        }
                    });
                });
        });
    let mut below = 10.0 + BUTTON + 14.0;
    if let Some(plane) = section {
        draw_section_bar(ctx, viewport, below, plane, scene_bounds, commands);
        below += BUTTON + 8.0;
    }
    if let Some(bar) = measure {
        draw_measure_bar(ctx, viewport, below, bar, commands);
    }
}

/// The measure tool's pill: the unit its readout is in (the document's
/// unless another is picked) and a button starting the chain over.
fn draw_measure_bar(
    ctx: &Context,
    viewport: egui::Rect,
    top: f32,
    bar: MeasureBar,
    commands: &mut Vec<UiCommand>,
) {
    use core_document::Unit;
    Area::new(egui::Id::new("view_toolbar_measure"))
        .order(super::hud::VIEW_LAYER)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(viewport.center_top() + Vec2::new(0.0, top))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(OVERLAY_CARD)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(7)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(ui_kit::widgets::text("Measure in", FONT_SM, TEXT2));
                        let shown = bar.unit.unwrap_or(bar.document_unit);
                        let document = format!("Document ({})", bar.document_unit.short_label());
                        if ui
                            .selectable_label(bar.unit.is_none(), document)
                            .on_hover_text("Read in the document's unit")
                            .clicked()
                        {
                            commands.push(UiCommand::SetMeasureUnit(None));
                        }
                        for unit in Unit::ALL {
                            if ui
                                .selectable_label(
                                    bar.unit.is_some() && shown == unit,
                                    unit.short_label(),
                                )
                                .on_hover_text(unit.long_label())
                                .clicked()
                            {
                                commands.push(UiCommand::SetMeasureUnit(Some(unit)));
                            }
                        }
                        vseparator(ui, 18.0);
                        if ui
                            .add_enabled(bar.picks > 0, egui::Button::new("Start over"))
                            .on_hover_text("Drop the picks; a click on empty space does too")
                            .clicked()
                        {
                            commands.push(UiCommand::ClearMeasure);
                        }
                    });
                });
        });
}

/// What clicks in the view pick: a button showing the filter, its list
/// below it.
fn pick_filter_button(ui: &mut egui::Ui, filter: PickFilter, commands: &mut Vec<UiCommand>) {
    let state = ToolButtonState {
        enabled: true,
        active: filter != PickFilter::Any,
        planned: None,
        menu: true,
    };
    let response = tool_button(
        ui,
        "select",
        &format!("Pick: {}", filter.label()),
        BUTTON,
        state,
    );
    egui::Popup::menu(&response).show(|ui| {
        for choice in PickFilter::ALL {
            if ui
                .selectable_label(choice == filter, choice.label())
                .clicked()
            {
                commands.push(UiCommand::SetPickFilter(choice));
                ui.close();
            }
        }
    });
}

/// The clipping plane's own pill, under the toolbar: its axis, its
/// position along it within the scene, and which side it keeps.
fn draw_section_bar(
    ctx: &Context,
    viewport: egui::Rect,
    top: f32,
    plane: SectionPlane,
    bounds: Option<(glam::Vec3, glam::Vec3)>,
    commands: &mut Vec<UiCommand>,
) {
    let set = |plane| UiCommand::SetSection(Some(SectionToggle::Set(plane)));
    Area::new(egui::Id::new("view_toolbar_section"))
        .order(super::hud::VIEW_LAYER)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(viewport.center_top() + Vec2::new(0.0, top))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(OVERLAY_CARD)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(7)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(ui_kit::widgets::text("Clip", FONT_SM, TEXT2));
                        for axis in SectionAxis::ALL {
                            if ui
                                .selectable_label(plane.axis == axis, axis.label())
                                .on_hover_text(format!("Cut square to the {} axis", axis.label()))
                                .clicked()
                                && plane.axis != axis
                            {
                                commands.push(set(plane.on_axis(axis, bounds)));
                            }
                        }
                        vseparator(ui, 18.0);
                        let (lo, hi) = plane.range(bounds);
                        let mut offset = plane.offset;
                        let edit = QtyField::mm(&mut offset)
                            .range(f64::from(lo)..=f64::from(hi))
                            .width(72.0)
                            .show_settling(ui);
                        if let Some(response) = &edit.response {
                            response.clone().on_hover_text(format!(
                                "Where the plane cuts along {}: drag across the scene",
                                plane.axis.label()
                            ));
                        }
                        if edit.changed {
                            commands.push(set(SectionPlane { offset, ..plane }));
                        }
                        vseparator(ui, 18.0);
                        if ui
                            .button("Flip")
                            .on_hover_text("Keep the other side of the plane")
                            .clicked()
                        {
                            commands.push(set(SectionPlane {
                                flipped: !plane.flipped,
                                ..plane
                            }));
                        }
                    });
                });
        });
}

#[cfg(test)]
mod pick_filter_tests {
    use super::PickFilter;

    /// Each filter lets through what it names: edges and faces both by
    /// default, one of them alone, or neither when whole bodies are picked.
    #[test]
    fn each_filter_picks_what_it_names() {
        let lets = |f: PickFilter| (f.faces(), f.edges());
        assert_eq!(lets(PickFilter::Any), (true, true));
        assert_eq!(lets(PickFilter::Faces), (true, false));
        assert_eq!(lets(PickFilter::Edges), (false, true));
        assert_eq!(lets(PickFilter::Bodies), (false, false));
        assert_eq!(PickFilter::default(), PickFilter::Any);
    }
}
