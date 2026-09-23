//! The floating view toolbar at the top of the viewport: fit, standard
//! views, draw style, the clipping plane, projection, and in perspective its
//! field of view. With the clipping plane on, a second pill under it sets
//! the plane's axis, position and side.

use egui::{Align2, Area, Context, Order, Vec2};
use settings::{DrawStyle, ProjectionMode};
use ui_kit::tokens::*;
use ui_kit::widgets::{QtyField, ToolButtonState, tool_button, vseparator};

use super::UiCommand;
use crate::camera::FOV_RANGE_DEG;
use crate::camera::section::{SectionAxis, SectionPlane, SectionToggle};
use crate::orientation_cube::CameraSnapView;

const BUTTON: f32 = 28.0;

enum Item {
    Button {
        icon: &'static str,
        label: &'static str,
        on: bool,
        planned: Option<&'static str>,
        command: Option<UiCommand>,
    },
    Sep,
}

fn button(icon: &'static str, label: &'static str, command: UiCommand) -> Item {
    Item::Button {
        icon,
        label,
        on: false,
        planned: None,
        command: Some(command),
    }
}

fn toggled(icon: &'static str, label: &'static str, on: bool, command: UiCommand) -> Item {
    Item::Button {
        icon,
        label,
        on,
        planned: None,
        command: Some(command),
    }
}

/// The view state the toolbar shows.
pub struct ViewToolbarState {
    pub projection: ProjectionMode,
    pub field_of_view_deg: f32,
    pub draw_style: DrawStyle,
    pub section: Option<SectionPlane>,
    /// The box around what the scene draws: the clipping plane's range.
    pub scene_bounds: Option<(glam::Vec3, glam::Vec3)>,
}

/// Draws the pill and pushes the commands of any clicked button.
pub fn draw_view_toolbar(
    ctx: &Context,
    viewport: egui::Rect,
    state: &ViewToolbarState,
    commands: &mut Vec<UiCommand>,
) {
    let ViewToolbarState {
        projection,
        field_of_view_deg,
        draw_style,
        section,
        scene_bounds,
    } = *state;
    let ortho = projection == ProjectionMode::Orthographic;
    let items = [
        button("fit-all", "Fit all", UiCommand::FitView),
        button("fit-selection", "Fit selection", UiCommand::FitSelection),
        Item::Sep,
        button(
            "view-iso",
            "Isometric",
            UiCommand::CameraSnap(CameraSnapView::FrontTopRight),
        ),
        button(
            "view-front",
            "Front",
            UiCommand::CameraSnap(CameraSnapView::Front),
        ),
        button(
            "view-top",
            "Top",
            UiCommand::CameraSnap(CameraSnapView::Top),
        ),
        button(
            "view-right",
            "Right",
            UiCommand::CameraSnap(CameraSnapView::Right),
        ),
        button(
            "view-rear",
            "Rear",
            UiCommand::CameraSnap(CameraSnapView::Rear),
        ),
        button(
            "view-bottom",
            "Bottom",
            UiCommand::CameraSnap(CameraSnapView::Bottom),
        ),
        button(
            "view-left",
            "Left",
            UiCommand::CameraSnap(CameraSnapView::Left),
        ),
        Item::Sep,
        toggled(
            "draw-style-shaded",
            "Shaded with edges",
            draw_style == DrawStyle::ShadedEdges,
            UiCommand::SetDrawStyle(DrawStyle::ShadedEdges),
        ),
        toggled(
            "draw-style-flat",
            "Shaded",
            draw_style == DrawStyle::Shaded,
            UiCommand::SetDrawStyle(DrawStyle::Shaded),
        ),
        toggled(
            "draw-style-wireframe",
            "Wireframe",
            draw_style == DrawStyle::Wireframe,
            UiCommand::SetDrawStyle(DrawStyle::Wireframe),
        ),
        toggled(
            "clipping-plane",
            "Clipping plane",
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
            ortho,
            UiCommand::SetProjection(ProjectionMode::Orthographic),
        ),
        toggled(
            "view-perspective",
            "Perspective",
            !ortho,
            UiCommand::SetProjection(ProjectionMode::Perspective),
        ),
    ];

    Area::new(egui::Id::new("view_toolbar"))
        .order(Order::Foreground)
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
                                    if tool_button(ui, icon, label, BUTTON, state).clicked()
                                        && let Some(command) = command
                                    {
                                        commands.push(command);
                                    }
                                }
                            }
                        }
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
    if let Some(plane) = section {
        draw_section_bar(ctx, viewport, plane, scene_bounds, commands);
    }
}

/// The clipping plane's own pill, under the toolbar: its axis, its
/// position along it within the scene, and which side it keeps.
fn draw_section_bar(
    ctx: &Context,
    viewport: egui::Rect,
    plane: SectionPlane,
    bounds: Option<(glam::Vec3, glam::Vec3)>,
    commands: &mut Vec<UiCommand>,
) {
    let set = |plane| UiCommand::SetSection(Some(SectionToggle::Set(plane)));
    Area::new(egui::Id::new("view_toolbar_section"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(viewport.center_top() + Vec2::new(0.0, 10.0 + BUTTON + 14.0))
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
