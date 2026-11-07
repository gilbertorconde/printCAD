//! The floating view toolbar at the top of the viewport: fit, standard
//! views, draw style, the clipping plane, the measure tool, the print bed
//! and annotations, projection, and in perspective its field of view. In
//! a viewport too narrow for all of it, the less used groups fold into a
//! menu at its end. With the clipping plane on, a second pill under it sets
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
    /// What clicks in the view pick.
    PickFilter,
    /// The perspective's field of view, dragged or typed.
    FieldOfView,
}

impl Item {
    /// How wide the item draws in the pill, spacing after it included.
    fn width(&self) -> f32 {
        match self {
            Item::Button { .. } => BUTTON + ITEM_GAP,
            Item::PickFilter => BUTTON + 12.0 + ITEM_GAP,
            Item::FieldOfView => FOV_WIDTH + ITEM_GAP,
        }
    }
}

/// A run of the pill's items: set off from the next by a line, and folded
/// into the overflow menu as one when the view is too narrow for it, the
/// highest `fold` first.
struct Group {
    items: Vec<Item>,
    /// 0 never folds; the larger, the sooner it goes.
    fold: u8,
    /// Groups of one section sit together with no line between them.
    section: u8,
}

const ITEM_GAP: f32 = 2.0;
const FOV_WIDTH: f32 = 58.0;
/// A line between sections: the room each side and the line itself.
const SEP_WIDTH: f32 = 3.0 + 1.0 + 3.0 + 2.0 * ITEM_GAP;
/// The frame's margin and stroke on both sides.
const PILL_FRAME: f32 = 2.0 * (3.0 + 1.0);
/// The pill keeps this far in from each side of the viewport.
const PILL_INSET: f32 = SPACE_2;

/// Which groups go to the overflow menu for the pill to fit `room`: one
/// at a time, the highest `fold` first and the last of equals first, its
/// button then taking a place at the end; then any folded group that fits
/// again comes back. Groups of `fold` 0 always stay.
fn folded_groups(groups: &[Group], room: f32) -> Vec<bool> {
    let mut folded = vec![false; groups.len()];
    let width = |folded: &[bool]| {
        let mut w = PILL_FRAME;
        let mut section = None;
        for (g, _) in groups.iter().zip(folded).filter(|(_, f)| !**f) {
            if section.is_some_and(|s| s != g.section) {
                w += SEP_WIDTH;
            }
            section = Some(g.section);
            w += g.items.iter().map(Item::width).sum::<f32>();
        }
        if folded.contains(&true) {
            w += SEP_WIDTH + BUTTON + ITEM_GAP;
        }
        w
    };
    let mut order: Vec<usize> = (0..groups.len()).filter(|&i| groups[i].fold > 0).collect();
    order.sort_by_key(|&i| std::cmp::Reverse((groups[i].fold, i)));
    let mut taken = Vec::new();
    for &i in &order {
        if width(&folded) <= room {
            break;
        }
        folded[i] = true;
        taken.push(i);
    }
    // A group folded early may fit again now that later ones went.
    for &i in taken.iter().rev() {
        folded[i] = false;
        if width(&folded) > room {
            folded[i] = true;
        }
    }
    folded
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
    /// The print bed is drawn.
    pub print_bed: bool,
    /// Imported annotations are drawn over the scene.
    pub annotations: bool,
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
        projection,
        draw_style,
        section,
        scene_bounds,
        measure,
        print_bed,
        annotations,
        ..
    } = *state;
    let ortho = projection == ProjectionMode::Orthographic;
    let mut groups = vec![
        Group {
            items: vec![
                button("fit-all", "Fit all", "view.fit_all", UiCommand::FitView),
                button(
                    "fit-selection",
                    "Fit selection",
                    "view.fit_selection",
                    UiCommand::FitSelection,
                ),
            ],
            fold: 0,
            section: 0,
        },
        Group {
            items: vec![
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
            ],
            fold: 1,
            section: 1,
        },
        Group {
            items: vec![
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
            ],
            fold: 3,
            section: 1,
        },
        Group {
            items: vec![
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
            ],
            fold: 2,
            section: 2,
        },
        Group {
            items: vec![toggled(
                "measure",
                "Measure",
                "view.measure",
                measure.is_some(),
                UiCommand::ToggleMeasure,
            )],
            fold: 0,
            section: 3,
        },
        Group {
            items: vec![
                toggled(
                    "print-bed",
                    "Print bed",
                    "view.print_bed",
                    print_bed,
                    UiCommand::TogglePrintBed,
                ),
                toggled(
                    "dimensional-constraint",
                    "Annotations",
                    "view.annotations",
                    annotations,
                    UiCommand::ToggleAnnotations,
                ),
            ],
            fold: 3,
            section: 3,
        },
        Group {
            items: vec![
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
            ],
            fold: 1,
            section: 4,
        },
        Group {
            items: vec![Item::PickFilter],
            fold: 0,
            section: 5,
        },
    ];
    if !ortho {
        groups.push(Group {
            items: vec![Item::FieldOfView],
            fold: 2,
            section: 5,
        });
    }
    // In a view too narrow for the whole pill the less used groups fold
    // into a menu at its end, so the pill never runs past the viewport.
    let fold = folded_groups(&groups, viewport.width() - 2.0 * PILL_INSET);
    let (mut shown, mut folded) = (Vec::new(), Vec::new());
    for (group, fold) in groups.into_iter().zip(fold) {
        if fold {
            folded.push(group);
        } else {
            shown.push(group);
        }
    }

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
                        ui.spacing_mut().item_spacing.x = ITEM_GAP;
                        let mut section = None;
                        for group in shown {
                            if section.is_some_and(|s| s != group.section) {
                                separator(ui);
                            }
                            section = Some(group.section);
                            for item in group.items {
                                draw_item(ui, item, state, &with_key, commands);
                            }
                        }
                        if !folded.is_empty() {
                            separator(ui);
                            overflow_menu(ui, folded, state, &with_key, commands);
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

/// The line between two sections of the pill.
fn separator(ui: &mut egui::Ui) {
    ui.add_space(3.0);
    vseparator(ui, 18.0);
    ui.add_space(3.0);
}

/// One item of the pill, its command pushed when it is used.
fn draw_item(
    ui: &mut egui::Ui,
    item: Item,
    state: &ViewToolbarState,
    with_key: &dyn Fn(&str, &str) -> String,
    commands: &mut Vec<UiCommand>,
) {
    match item {
        Item::Button {
            icon,
            label,
            binding,
            on,
            planned,
            command,
        } => {
            let look = ToolButtonState {
                enabled: command.is_some(),
                active: on,
                planned,
                menu: false,
            };
            if tool_button(ui, icon, &with_key(label, binding), BUTTON, look).clicked()
                && let Some(command) = command
            {
                commands.push(*command);
            }
        }
        Item::PickFilter => pick_filter_button(ui, state.pick_filter, commands),
        Item::FieldOfView => field_of_view(ui, state.field_of_view_deg, commands),
    }
}

/// The perspective's strength, dragged or typed: the object keeps its
/// size, only the distortion changes.
fn field_of_view(ui: &mut egui::Ui, degrees: f32, commands: &mut Vec<UiCommand>) {
    let mut fov = degrees;
    let edit = QtyField::degrees(&mut fov)
        .range(f64::from(FOV_RANGE_DEG.0)..=f64::from(FOV_RANGE_DEG.1))
        .decimals(0)
        .width(FOV_WIDTH)
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

/// The pill's last button when the view is too narrow for all of it: a
/// menu of the folded groups, each button a row with its name, sections
/// set off by lines.
fn overflow_menu(
    ui: &mut egui::Ui,
    folded: Vec<Group>,
    state: &ViewToolbarState,
    with_key: &dyn Fn(&str, &str) -> String,
    commands: &mut Vec<UiCommand>,
) {
    let look = ToolButtonState {
        enabled: true,
        active: false,
        planned: None,
        menu: false,
    };
    let response = tool_button(ui, "more", "More view tools", BUTTON, look);
    super::widget_map::note(ui.ctx(), || "button:view.more".into(), response.rect);
    egui::Popup::menu(&response).show(|ui| {
        ui_kit::widgets::fitted_menu(ui, |ui| {
            let mut section = None;
            for group in folded {
                if section.is_some_and(|s| s != group.section) {
                    ui.separator();
                }
                section = Some(group.section);
                for item in group.items {
                    match item {
                        Item::Button {
                            icon,
                            label,
                            binding,
                            on,
                            planned,
                            command,
                        } => {
                            let text = egui::RichText::new(with_key(label, binding))
                                .font(ui_kit::sans(FONT_SM));
                            let row = match ui_kit::icon::image(ui.ctx(), icon, 16.0, ICON) {
                                Some(image) => egui::Button::image_and_text(image, text),
                                None => egui::Button::new(text),
                            }
                            .selected(on);
                            if ui
                                .add_enabled(command.is_some() && planned.is_none(), row)
                                .clicked()
                                && let Some(command) = command
                            {
                                commands.push(*command);
                                ui.close();
                            }
                        }
                        Item::FieldOfView => {
                            ui.horizontal(|ui| {
                                ui.label(ui_kit::widgets::text("Field of view", FONT_SM, TEXT2));
                                field_of_view(ui, state.field_of_view_deg, commands);
                            });
                        }
                        Item::PickFilter => {
                            for choice in PickFilter::ALL {
                                if ui
                                    .selectable_label(choice == state.pick_filter, choice.label())
                                    .clicked()
                                {
                                    commands.push(UiCommand::SetPickFilter(choice));
                                    ui.close();
                                }
                            }
                        }
                    }
                }
            }
        });
    });
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

#[cfg(test)]
mod fold_tests {
    use super::*;

    /// A wide view shows every group; a narrower one folds the groups that
    /// go first, one at a time, until the rest fits, and the groups that
    /// never fold stay however narrow the view.
    #[test]
    fn groups_fold_until_the_pill_fits() {
        let group = |n: usize, fold: u8, section: u8| Group {
            items: (0..n)
                .map(|_| button("fit-all", "Fit all", "view.fit_all", UiCommand::FitView))
                .collect(),
            fold,
            section,
        };
        let groups = [
            group(2, 0, 0),
            group(4, 1, 1),
            group(3, 3, 1),
            group(4, 2, 2),
        ];
        let folds = |room: f32| folded_groups(&groups, room);
        assert_eq!(folds(2000.0), [false; 4], "nothing folds");
        assert_eq!(folds(400.0), [false, false, true, false], "the first to go");
        assert_eq!(folds(300.0), [false, false, true, true]);
        // The views go too, after which the smaller group folded first fits.
        assert_eq!(
            folds(230.0),
            [false, true, false, true],
            "back once room is made"
        );
        assert_eq!(folds(10.0), [false, true, true, true], "never past fold 0");
    }
}
