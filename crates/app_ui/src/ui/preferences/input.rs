//! The Input page's Mouse and 6-DoF mouse tabs: navigation, the device's
//! motion and the gestures its buttons run.

use super::*;

/// The six ways the puck moves, in the order the device reports them: what
/// the hand does, the drawing that shows it, and the id its chooser
/// needs.
struct Gesture {
    name: &'static str,
    drawing: &'static str,
    assign_id: &'static str,
}

const SIXDOF_GESTURES: [Gesture; 6] = [
    Gesture {
        name: "Push left and right",
        drawing: "motion-push-left-right",
        assign_id: "prefs_sixdof_does_1",
    },
    Gesture {
        name: "Pull up and push down",
        drawing: "motion-pull-up-push-down",
        assign_id: "prefs_sixdof_does_2",
    },
    Gesture {
        name: "Drag front and back",
        drawing: "motion-drag-front-back",
        assign_id: "prefs_sixdof_does_3",
    },
    Gesture {
        name: "Tilt forward and back",
        drawing: "motion-tilt-forward-back",
        assign_id: "prefs_sixdof_does_4",
    },
    Gesture {
        name: "Twist",
        drawing: "motion-twist",
        assign_id: "prefs_sixdof_does_5",
    },
    Gesture {
        name: "Tilt left and right",
        drawing: "motion-tilt-left-right",
        assign_id: "prefs_sixdof_does_6",
    },
];

/// How big the drawing of a movement is: large enough to read the gesture
/// off it, with the movement's own settings beside it.
const GESTURE_DRAWING: f32 = 200.0;

/// One widget id per 6-DoF mouse button row; a chooser needs its own.
const NAV_BUTTON_IDS: [&str; 16] = [
    "prefs_nav_button_1",
    "prefs_nav_button_2",
    "prefs_nav_button_3",
    "prefs_nav_button_4",
    "prefs_nav_button_5",
    "prefs_nav_button_6",
    "prefs_nav_button_7",
    "prefs_nav_button_8",
    "prefs_nav_button_9",
    "prefs_nav_button_10",
    "prefs_nav_button_11",
    "prefs_nav_button_12",
    "prefs_nav_button_13",
    "prefs_nav_button_14",
    "prefs_nav_button_15",
    "prefs_nav_button_16",
];

/// One movement of the puck: a drawing of the gesture, and beside it
/// everything about what that movement does.
///
/// Returns whether it drew, which is how the search leaves out the movements
/// it does not match.
fn movement_card(
    ui: &mut Ui,
    index: usize,
    device: &mut settings::SixDofSettings,
    filter: &str,
) -> bool {
    let gesture = &SIXDOF_GESTURES[index];
    let assigned = device.assign[index];
    if !pref_matches(filter, &[gesture.name, assigned.label(), "reverse speed"]) {
        return false;
    }

    overline(ui, gesture.name);
    ui.add_space(SPACE_1);
    Frame::new()
        .fill(BG1)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_MD as u8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 24.0);
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(GESTURE_DRAWING, GESTURE_DRAWING), Sense::hover());
                if let Some(image) =
                    ui_kit::icon::drawing(ui.ctx(), gesture.drawing, GESTURE_DRAWING)
                {
                    image.paint_at(ui, rect);
                }
                ui.add_space(SPACE_3);
                ui.vertical(|ui| {
                    // The controls are shorter than the drawing beside them,
                    // so they sit in the middle of it rather than at the top.
                    // Row heights come from the design system: 40 plain, 44
                    // with a hint under the label.
                    let rows_height = if assigned == SixDofMotion::None {
                        40.0
                    } else {
                        40.0 + 44.0 + 44.0
                    };
                    ui.add_space(((GESTURE_DRAWING - rows_height) / 2.0).max(0.0));
                    let motions: Vec<(SixDofMotion, &str)> = SixDofMotion::ALL
                        .iter()
                        .map(|motion| (*motion, motion.label()))
                        .collect();
                    let mut rows = vec![PrefRow::select(
                        "What it does",
                        gesture.assign_id,
                        &mut device.assign[index],
                        &motions,
                    )];
                    if assigned != SixDofMotion::None {
                        rows.push(
                            PrefRow::toggle("Reverse it", &mut device.invert[index])
                                .hint("The same movement, the other way"),
                        );
                        rows.push(
                            PrefRow::new("Speed", {
                                let speed = &mut device.speed[index];
                                move |ui| {
                                    QtyField::new(speed)
                                        .unit(assigned.speed_unit())
                                        .range(assigned.speed_range())
                                        .decimals(if assigned == SixDofMotion::Zoom {
                                            2
                                        } else {
                                            0
                                        })
                                        .show(ui)
                                }
                            })
                            .hint("At full deflection"),
                        );
                    }
                    pref_group(ui, "", rows, "");
                });
            });
        });
    ui.add_space(SPACE_2);
    true
}

pub(super) fn input_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    inputs: &PreferencesInputs<'_>,
    filter: &str,
) {
    let draft = &mut state.draft;
    match state.tab {
        0 => {
            let camera = &mut draft.camera;
            pref_group(
                ui,
                "Navigation",
                vec![
                    PrefRow::select(
                        "Pointing device",
                        "prefs_navigation",
                        &mut camera.navigation,
                        &[
                            (NavigationStyle::Mouse, "Mouse"),
                            (NavigationStyle::Touchpad, "Touchpad"),
                        ],
                    )
                    .hint("On a touchpad two fingers orbit, with Shift pan, with Ctrl zoom"),
                    PrefRow::toggle("Zoom to cursor", &mut camera.zoom_to_cursor)
                        .hint("Wheel zoom keeps the point under the cursor still"),
                    PrefRow::toggle("Invert zoom", &mut camera.invert_zoom),
                    PrefRow::qty(
                        "Wheel step factor",
                        QtyField::new(&mut camera.wheel_zoom_factor)
                            .range(0.7..=0.995)
                            .speed(0.005)
                            .decimals(3),
                    )
                    .hint("Scale per wheel notch; closer to 1 zooms slower"),
                    PrefRow::qty(
                        "Orbit sensitivity",
                        QtyField::new(&mut camera.orbit_sensitivity)
                            .range(0.05..=2.0)
                            .speed(0.01),
                    ),
                    PrefRow::toggle(
                        "Orbit around the point under the cursor",
                        &mut camera.orbit_pivot_pick,
                    ),
                    PrefRow::qty(
                        "Pan sensitivity",
                        QtyField::new(&mut camera.pan_sensitivity)
                            .range(0.1..=3.0)
                            .speed(0.01),
                    ),
                    PrefRow::select(
                        "Orbit yaw axis",
                        "prefs_yaw",
                        &mut camera.orbit_yaw_axis,
                        &[
                            (OrbitYawAxis::WorldUp, "World up"),
                            (OrbitYawAxis::CameraUp, "Camera up"),
                        ],
                    ),
                    PrefRow::qty(
                        "Click / drag threshold",
                        QtyField::new(&mut camera.click_drag_threshold_px)
                            .unit("px")
                            .range(2.0..=12.0)
                            .speed(0.5)
                            .decimals(1),
                    ),
                ],
                filter,
            );
        }
        1 => {
            let device = &mut draft.sixdof;
            pref_group(
                ui,
                "6-DoF mouse",
                vec![
                    PrefRow::toggle("Steer the view with a 6-DoF mouse", &mut device.enabled)
                        .hint("A six-axis puck moves the view while it is held"),
                    PrefRow::qty(
                        "Dead zone",
                        QtyField::new(&mut device.dead_zone)
                            .range(0.0..=0.5)
                            .speed(0.005)
                            .decimals(3),
                    )
                    .hint("Deflection below this share of full scale counts as rest"),
                    PrefRow::qty(
                        "Full deflection",
                        QtyField::new(&mut device.full_scale)
                            .range(50.0..=2000.0)
                            .speed(5.0)
                            .decimals(0),
                    )
                    .hint("The reading a fully pushed axis produces"),
                    PrefRow::toggle("One axis at a time", &mut device.dominant_axis)
                        .hint("Only the axis pushed hardest acts, so a gesture stays square"),
                ],
                filter,
            );

            let mut drawn = false;
            for index in 0..SIXDOF_GESTURES.len() {
                drawn |= movement_card(ui, index, device, filter);
            }
            if drawn {
                ui.add_space(SPACE_2);
            }

            // A device with no buttons still gets the two rows a common puck
            // has, so the page is not empty before one is plugged in.
            let count = (inputs.nav_buttons.max(2) as usize).min(NAV_BUTTON_IDS.len());
            if device.buttons.len() < count {
                device
                    .buttons
                    .resize(count, settings::SixDofButtonAction::None);
            }
            let actions: Vec<(settings::SixDofButtonAction, &str)> =
                settings::SixDofButtonAction::ALL
                    .iter()
                    .map(|action| (*action, action.label()))
                    .collect();
            let button_labels: Vec<String> = (1..=count)
                .map(|button| format!("Button {button}"))
                .collect();
            let rows = device
                .buttons
                .iter_mut()
                .take(count)
                .zip(&button_labels)
                .zip(NAV_BUTTON_IDS)
                .map(|((action, label), id)| PrefRow::select(label, id, action, &actions))
                .collect();
            pref_group(ui, "Buttons", rows, filter);
        }
        2 => {
            let words = ["Radial menu", "right button", "slices"];
            let registry = &*inputs.registry;
            pref_block(ui, &words, filter, |ui| radial_page(ui, state, registry));
        }
        _ => {}
    }
}
