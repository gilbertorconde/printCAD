//! The status bar: activity or workbench state on the left, selection
//! summary, and mono readouts on the right (coordinates, dimensions,
//! the 6-DoF device, frame counters, the document server).

use axes::AxisSystem;
use core_document::{StatusItems, Unit};
use egui::{RichText, Vec2};
use glam::Vec3;
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{mono_label, vseparator};

use super::overlays::rgb;

/// The document server serving a tab, as the status bar shows it.
#[derive(Debug, Clone, Default)]
pub struct ServerBadge {
    /// What the server calls itself, for the tooltip.
    pub name: String,
    /// The app reads and writes files itself, with no server process.
    pub standalone: bool,
    pub connected: bool,
    /// Other clients editing the same document.
    pub peers: u32,
}

/// Everything the status bar reads this frame.
pub struct StatusBarInputs<'a> {
    pub fps: Option<f32>,
    pub scene_redraws_per_s: u32,
    pub hovered_point: Option<[f32; 3]>,
    pub axis_system: AxisSystem,
    pub display_unit: Unit,
    pub pending_imports: u32,
    pub pending_document_open: u32,
    pub kernel_status: Option<&'a str>,
    pub kernel_cancellable: bool,
    pub kernel_progress: Option<(u64, u64)>,
    pub server: &'a ServerBadge,
    pub document_saving: bool,
    /// Bytes packed into the archive being saved, out of the whole.
    pub save_progress: Option<(u64, u64)>,
    /// The connected 6-DoF mouse, when there is one.
    pub nav_device: Option<&'a str>,
    pub items: Option<&'a StatusItems>,
    pub preselect: Option<&'a str>,
    /// The view's picks, when several: shown when the bench names no
    /// selection of its own.
    pub picks: Option<&'a str>,
    /// "x × y × z" of the selection, already formatted.
    pub dimensions: Option<&'a str>,
    /// The script running, by name, while one is.
    pub script_running: Option<&'a str>,
    /// A recording is on.
    pub recording: bool,
    /// Which of the log, the console and the assistant are showing.
    pub panels: Panels,
    /// The readouts the user keeps on the bar.
    pub shown: settings::StatusBarSettings,
}

/// The panels the bar's buttons open and close.
#[derive(Debug, Default, Clone, Copy)]
pub struct Panels {
    pub log: bool,
    pub console: bool,
    pub assistant: bool,
}

/// What the status bar's buttons asked for.
#[derive(Debug, Default, Clone, Copy)]
pub struct StatusBarResult {
    /// Stop the running kernel job.
    pub cancel_kernel: bool,
    /// Stop the running script.
    pub stop_script: bool,
    /// Stop the recording and save it.
    pub stop_recording: bool,
    /// Open or close the log panel, the console, the assistant.
    pub toggle_log: bool,
    pub toggle_console: bool,
    pub toggle_assistant: bool,
    /// The bar's menu turned a readout on or off: what shows now.
    pub shown: Option<settings::StatusBarSettings>,
}

/// The rows of the bar's menu: each readout's name and its switch.
fn readouts(shown: &mut settings::StatusBarSettings) -> [(&'static str, &mut bool); 9] {
    [
        ("State", &mut shown.state),
        ("Selection", &mut shown.selection),
        ("Panel switches", &mut shown.panel_switches),
        ("6-DoF mouse", &mut shown.device),
        ("Frame rate", &mut shown.performance),
        ("Document server", &mut shown.server),
        ("Mode", &mut shown.mode),
        ("Size", &mut shown.size),
        ("Coordinates", &mut shown.coordinates),
    ]
}

/// The bar's menu: a switch per readout, the menu kept open between them.
fn readout_menu(
    ui: &mut egui::Ui,
    shown: settings::StatusBarSettings,
) -> Option<settings::StatusBarSettings> {
    let mut edited = shown;
    ui_kit::widgets::fitted_menu(ui, |ui| {
        for (label, on) in readouts(&mut edited) {
            let row = egui::Button::selectable(*on, RichText::new(label).font(sans(FONT_SM)));
            if ui.add(row).clicked() {
                *on = !*on;
            }
        }
    });
    (edited != shown).then_some(edited)
}

/// A thin bar, `done` of `total` filled, the counts on hover.
fn progress_bar(ui: &mut egui::Ui, done: u64, total: u64, hover: String) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(96.0, 4.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, BG4);
    let mut fill = rect;
    fill.set_width(rect.width() * (done as f32 / total as f32).clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, 2.0, ACCENT);
    response.on_hover_text(hover);
}

fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn percent(done: u64, total: u64) -> String {
    format!("{:.0}%", 100.0 * done as f64 / total as f64)
}

/// Small text in the bar's type.
fn text(ui: &mut egui::Ui, s: impl Into<String>, color: egui::Color32) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(s).font(sans(FONT_XS)).color(color)).truncate())
}

/// A coloured dot of the bar's size.
fn dot(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(6.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, color);
    response
}

/// A button that reads as text until hovered.
fn quiet_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).font(sans(FONT_XS)).color(TEXT1))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(3.0)
            .min_size(Vec2::new(0.0, 16.0)),
    )
}

/// A panel's switch: its icon, lit while the panel shows. Whether it was
/// clicked.
fn panel_button(ui: &mut egui::Ui, icon: &str, name: &str, open: bool) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
    let response = ui.interact(
        rect,
        egui::Id::new(("status_panel", icon)),
        egui::Sense::click(),
    );
    if open || response.hovered() {
        ui.painter()
            .rect_filled(rect, RADIUS_SM, if open { ACCENT_DIM } else { BG3 });
    }
    let tint = if open {
        ACCENT
    } else if response.hovered() {
        TEXT1
    } else {
        TEXT3
    };
    // Painted, not added: a widget over the button would take its clicks.
    if let Some(image) = ui_kit::icon::image(ui.ctx(), icon, 14.0, tint) {
        image.paint_at(ui, rect.shrink(2.0));
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if open {
            format!("Hide the {}", name.to_lowercase())
        } else {
            format!("Show the {}", name.to_lowercase())
        })
        .clicked()
}

/// Draws the bar; says which of its buttons were pressed.
pub fn draw_status_bar(ui: &mut egui::Ui, inputs: &StatusBarInputs<'_>) -> StatusBarResult {
    let mut result = StatusBarResult::default();
    let mut right = StatusBarResult::default();
    let mut shown = None;
    egui::Panel::bottom("status_bar")
        .exact_size(STATUS_BAR)
        .frame(
            egui::Frame::new()
                .fill(BG0)
                .inner_margin(egui::Margin::symmetric(10, 0))
                .stroke(egui::Stroke::NONE),
        )
        .show(ui, |ui| {
            let rect = ui.max_rect();
            // Under the readouts, so a right click anywhere on the bar's
            // free room opens its menu.
            let background =
                ui.interact(rect, egui::Id::new("status_bar_menu"), egui::Sense::click());
            ui.painter()
                .hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, BORDER));
            // The readouts on the right keep their room; what is happening
            // on the left is cut short before it runs under them.
            egui::containers::Sides::new()
                .height(rect.height())
                .spacing(SPACE_4)
                .shrink_left()
                .truncate()
                .show(
                    ui,
                    |ui| {
                        ui.set_clip_rect(ui.max_rect());
                        ui.spacing_mut().item_spacing.x = SPACE_4;
                        draw_left(ui, inputs, &mut result);
                    },
                    |ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_3;
                        right = draw_right(ui, inputs);
                    },
                );
            background.context_menu(|ui| shown = readout_menu(ui, inputs.shown));
        });
    result.toggle_log = right.toggle_log;
    result.toggle_console = right.toggle_console;
    result.toggle_assistant = right.toggle_assistant;
    result.shown = shown;
    result
}

/// What is happening: the activity, a recording, a script, the selection.
fn draw_left(ui: &mut egui::Ui, inputs: &StatusBarInputs<'_>, result: &mut StatusBarResult) {
    draw_activity(ui, inputs, &mut result.cancel_kernel);
    let shown = inputs.shown;
    if inputs.recording {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_2;
            dot(ui, DANGER);
            text(ui, "Recording", TEXT1);
            if quiet_button(ui, "Stop")
                .on_hover_text("Save what was recorded as a new script")
                .clicked()
            {
                result.stop_recording = true;
            }
        });
    }
    if let Some(script) = inputs.script_running {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_2;
            ui.add(egui::Spinner::new().size(11.0).color(ACCENT));
            text(ui, format!("Running {script}"), TEXT1);
            if quiet_button(ui, "Stop").clicked() {
                result.stop_script = true;
            }
        });
    }
    let (what, name) = match (
        inputs
            .items
            .and_then(|i| i.selection.as_deref())
            .or(inputs.picks),
        inputs.preselect,
    ) {
        (Some(sel), _) => ("Selected", sel),
        (None, Some(pre)) => ("Under the cursor", pre),
        (None, None) => return,
    };
    if !shown.selection {
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_1;
        text(ui, what, TEXT3);
        text(ui, name, TEXT1);
    });
}

/// The readouts, laid out from the right edge: the 6-DoF device, performance,
/// the server, then what the bench or the cursor says.
fn draw_right(ui: &mut egui::Ui, inputs: &StatusBarInputs<'_>) -> StatusBarResult {
    let mut result = StatusBarResult::default();
    let shown = inputs.shown;
    // A separator goes between two groups that both show.
    let mut drawn = false;
    let mut group = |ui: &mut egui::Ui, on: bool| {
        if on && drawn {
            vseparator(ui, 12.0);
        }
        drawn |= on;
        on
    };
    // The panels' switches at the end, the assistant outermost.
    let panels = inputs.panels;
    if group(ui, shown.panel_switches) {
        if crate::platform::offers("app.assistant") {
            result.toggle_assistant = panel_button(ui, "assistant", "Assistant", panels.assistant);
        }
        result.toggle_console = panel_button(ui, "console", "Console", panels.console);
        result.toggle_log = panel_button(ui, "log", "Log panel", panels.log);
    }
    if let Some(device) = inputs.nav_device.filter(|_| group(ui, shown.device)) {
        text(ui, device, TEXT3).on_hover_text("6-DoF mouse connected");
    }

    // Two numbers because they are two things: UI frames presented, and
    // how often the 3D scene was drawn again under them.
    let fps = match inputs.fps {
        Some(fps) if fps > 0.0 => format!("{fps:.0} fps"),
        Some(_) => "… fps".to_string(),
        // The loop is about to sleep; a frozen number would read as a live
        // measurement.
        None => "idle".to_string(),
    };
    let scene = if inputs.fps.is_none() || inputs.scene_redraws_per_s == 0 {
        "cached".to_string()
    } else {
        format!("{}/s", inputs.scene_redraws_per_s)
    };
    if group(ui, shown.performance) {
        mono_label(ui, format!("{fps} · {scene}"), FONT_XS, TEXT3).on_hover_text(
            "Frames drawn per second, and how often the 3D scene was drawn again \
             (cached while nothing in it changes)",
        );
    }
    if group(ui, shown.server) {
        draw_server(ui, inputs.server);
    }

    let coords = coords_text(inputs).filter(|_| shown.coordinates);
    let mode = inputs
        .items
        .and_then(|i| i.mode.as_deref())
        .filter(|_| shown.mode);
    let dimensions = inputs.dimensions.filter(|_| shown.size);
    group(
        ui,
        coords.is_some() || dimensions.is_some() || mode.is_some(),
    );
    if let Some(mode) = mode {
        text(ui, mode, TEXT2);
    }
    if let Some(dim) = dimensions {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_1;
            mono_label(ui, dim, FONT_XS, TEXT2);
            text(ui, "Size", TEXT3);
        })
        .response
        .on_hover_text("The selection's bounding box along X, Y and Z");
    }
    if let Some(coords) = coords {
        mono_label(ui, coords, FONT_XS, TEXT2).on_hover_text("The point under the cursor");
    }
    result
}

/// The extent of `bounds` (millimetres, document axes) along X, Y and Z,
/// in that order whatever way up the view is, in `unit`.
pub fn size_text(bounds: ([f32; 3], [f32; 3]), unit: Unit) -> String {
    let size = (Vec3::from_array(bounds.1) - Vec3::from_array(bounds.0)).abs();
    format!(
        "{:.1} × {:.1} × {:.1} {}",
        unit.from_mm(size.x),
        unit.from_mm(size.y),
        unit.from_mm(size.z),
        unit.short_label()
    )
}

/// The document server: a dot while all is well, words when it is not.
fn draw_server(ui: &mut egui::Ui, server: &ServerBadge) {
    let (color, words, hover) = if server.standalone {
        (
            WARNING,
            Some("No server".to_string()),
            "The document server is not running: printCAD reads and writes files \
             itself, with no edit log beside them and no one else editing"
                .to_string(),
        )
    } else if !server.connected {
        (
            DANGER,
            Some("Server lost".to_string()),
            format!(
                "The connection to the {} was lost; save to keep your work",
                server.name
            ),
        )
    } else if server.peers > 0 {
        let peers = if server.peers == 1 {
            "1 peer".to_string()
        } else {
            format!("{} peers", server.peers)
        };
        (
            ACCENT,
            Some(peers.clone()),
            format!("Served by the {}, with {peers} editing too", server.name),
        )
    } else {
        (SUCCESS, None, format!("Served by the {}", server.name))
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_1;
        if let Some(words) = words {
            text(ui, words, TEXT2);
        }
        dot(
            ui,
            color.gamma_multiply(if server.connected && !server.standalone {
                0.7
            } else {
                1.0
            }),
        );
    })
    .response
    .on_hover_text(hover);
}

/// Where the cursor is, while it is over something (or what the bench
/// says in its place): each axis by its direction, in the document's unit.
fn coords_text(inputs: &StatusBarInputs<'_>) -> Option<String> {
    if let Some(coords) = inputs.items.and_then(|i| i.coords.as_deref()) {
        return Some(coords.to_owned());
    }
    let pos = inputs.hovered_point?;
    let canonical = inputs
        .axis_system
        .world_to_canonical(Vec3::from_array(pos))
        .to_array();
    let axes = [
        inputs.axis_system.horizontal(),
        inputs.axis_system.vertical(),
        inputs.axis_system.depth(),
    ];
    let unit = inputs.display_unit;
    let values = axes
        .iter()
        .zip(canonical)
        .map(|(axis, mm)| format!("{} {:.2}", axis.signed_label(), unit.from_mm(mm)))
        .collect::<Vec<_>>()
        .join("  ");
    Some(format!("{values} {}", unit.short_label()))
}

/// The far-left slot: a state dot with a title, or, while the kernel, an
/// open or a save is busy, a spinner or a bar with the announced stage and
/// a Cancel button.
fn draw_activity(ui: &mut egui::Ui, inputs: &StatusBarInputs<'_>, cancel: &mut bool) {
    let busy = inputs.pending_imports > 0 || inputs.pending_document_open > 0;
    if !busy && !inputs.shown.state {
        return;
    }
    if !busy {
        let (color, title) = match inputs.items.and_then(|i| i.state.clone()) {
            Some((rgb_color, title)) => (rgb(rgb_color, 1.0), title),
            None => (SUCCESS, "Ready".to_string()),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            dot(ui, color);
            text(ui, title, TEXT2);
        });
        return;
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        // A stage that announced its counts earns a real bar; anything
        // else keeps the honest spinner.
        match (inputs.kernel_progress, inputs.save_progress) {
            (Some((done, total)), _) if total > 0 => {
                progress_bar(ui, done, total, format!("{done} of {total}"));
                mono_label(ui, percent(done, total), FONT_XS, TEXT2);
            }
            // A save counts bytes, which read better as megabytes.
            (None, Some((done, total))) if total > 0 => {
                progress_bar(
                    ui,
                    done,
                    total,
                    format!("{:.1} of {:.1} MB", megabytes(done), megabytes(total)),
                );
                mono_label(ui, percent(done, total), FONT_XS, TEXT2);
            }
            _ => {
                ui.add(egui::Spinner::new().size(11.0).color(ACCENT));
            }
        }
        let mut parts = Vec::new();
        match inputs.kernel_status {
            Some(status) => parts.push(status.to_owned()),
            // Nothing announced yet: say only what is certain, which is
            // how many jobs are outstanding.
            None if inputs.pending_imports == 1 => parts.push("Working…".to_string()),
            None if inputs.pending_imports > 1 => {
                parts.push(format!("Working… ({} jobs)", inputs.pending_imports));
            }
            None => {}
        }
        if inputs.document_saving {
            parts.push("Saving…".to_string());
        } else if inputs.pending_document_open > 0 {
            parts.push("Opening…".to_string());
        }
        ui.label(
            RichText::new(parts.join(" · "))
                .font(sans(FONT_XS))
                .color(TEXT1),
        );
        if inputs.kernel_cancellable && quiet_button(ui, "Cancel").clicked() {
            *cancel = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(server: &ServerBadge) -> StatusBarInputs<'_> {
        StatusBarInputs {
            fps: Some(60.0),
            scene_redraws_per_s: 0,
            hovered_point: None,
            axis_system: AxisSystem::default(),
            display_unit: Unit::Mm,
            pending_imports: 0,
            pending_document_open: 0,
            kernel_status: None,
            kernel_cancellable: false,
            kernel_progress: None,
            server,
            document_saving: false,
            save_progress: None,
            nav_device: None,
            items: None,
            preselect: None,
            picks: None,
            dimensions: None,
            script_running: None,
            recording: false,
            panels: Panels::default(),
            shown: settings::StatusBarSettings::default(),
        }
    }

    /// One frame of the bar alone in an 800 × 600 window; what it answered.
    fn frame(ctx: &egui::Context, time: f64, events: Vec<egui::Event>) -> StatusBarResult {
        let server = ServerBadge {
            connected: true,
            ..Default::default()
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(800.0, 600.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut result = StatusBarResult::default();
        let mut out = ctx.run_ui(input, |ui| {
            result = draw_status_bar(ui, &inputs(&server));
        });
        out.textures_delta.clear();
        result
    }

    #[test]
    fn the_size_reads_x_then_y_then_z() {
        // A 20 × 30 × 40 box standing up Z, which the default view puts up.
        let bounds = ([5.0, -10.0, 0.0], [25.0, 20.0, 40.0]);
        assert_eq!(size_text(bounds, Unit::Mm), "20.0 × 30.0 × 40.0 mm");
        assert_eq!(size_text(bounds, Unit::Cm), "2.0 × 3.0 × 4.0 cm");
    }

    #[test]
    fn the_panel_switches_answer_a_click() {
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        frame(&ctx, 0.0, vec![]);
        // The switches sit at the bar's right end: the assistant last, the
        // console before it, the log before that, each 18 wide.
        let y = 600.0 - STATUS_BAR / 2.0;
        for (x, which) in [
            (800.0 - 10.0 - 9.0, 0),
            (800.0 - 10.0 - 18.0 - SPACE_3 - 9.0, 1),
            (800.0 - 10.0 - 2.0 * (18.0 + SPACE_3) - 9.0, 2),
        ] {
            let t = 1.0 + which as f64;
            let at = egui::pos2(x, y);
            let press = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(&ctx, t, vec![egui::Event::PointerMoved(at)]);
            frame(&ctx, t + 0.1, vec![press(true)]);
            let result = frame(&ctx, t + 0.2, vec![press(false)]);
            let clicked = [
                result.toggle_assistant,
                result.toggle_console,
                result.toggle_log,
            ][which];
            assert!(clicked, "switch {which} at {at:?} answered {result:?}");
        }
    }
}
