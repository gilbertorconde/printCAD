//! The sketcher's Preferences page, declared as panel blocks: the solver,
//! snap and view switches, splines, printing checks, appearance and the
//! palette, read-only.

use bench_api::panel;
use bench_api::{
    Caption, Check, ColorField, Flow, PanelEvent, Quantity, Select, SettingsGroup, Swatch,
    Swatches, Widget,
};

use crate::SketchOptions;
use crate::appearance::GridLines;

/// A switch the page shows: its id, words, hint and the option it sets.
type Switch = (
    &'static str,
    &'static str,
    &'static str,
    fn(&mut SketchOptions) -> &mut bool,
);

const SOLVER: [Switch; 10] = [
    (
        "auto_constraints",
        "Auto constraints",
        "Horizontal, vertical, parallel and perpendicular on snapped lines while drawing, and coincident or on-object where a drag lets go",
        |o| &mut o.auto_constraints,
    ),
    (
        "continue_drawing",
        "Continue drawing",
        "A drawing tool stays in hand after its shape; off, the pointer goes back to Select",
        |o| &mut o.continue_drawing,
    ),
    (
        "continue_constraining",
        "Continue constraining",
        "A constraint tool waits for the next elements after its constraint; off, it is put away",
        |o| &mut o.continue_constraining,
    ),
    (
        "hint_lines",
        "Hint lines",
        "Dashed guides for level, plumb, parallel, perpendicular and extension snaps",
        |o| &mut o.hint_lines,
    ),
    (
        "arc_helpers",
        "Arc helpers",
        "While drawing, each arc's whole circle shows as a dashed guide a point can snap onto",
        |o| &mut o.arc_helpers,
    ),
    (
        "avoid_redundant_auto",
        "Avoid redundant auto constraints",
        "An auto constraint the solver calls redundant is dropped again",
        |o| &mut o.avoid_redundant_auto,
    ),
    (
        "auto_remove_redundant",
        "Auto remove redundants",
        "A new constraint takes away the older ones it makes redundant; a drawn shape drops its own constraints the sketch already enforces",
        |o| &mut o.auto_remove_redundant,
    ),
    (
        "ask_dimension_value",
        "Ask for a dimension's value",
        "A new dimension opens its value editor, by its label",
        |o| &mut o.ask_dimension_value,
    ),
    (
        "section_on_open",
        "Section view on open",
        "Editing a sketch cuts the view at its plane; Section view turns it off for one sketch",
        |o| &mut o.section_on_open,
    ),
    (
        "shade_regions",
        "Shade closed regions",
        "While editing, what the profile closes is shaded: what a pad takes",
        |o| &mut o.shade_regions,
    ),
];

const SPLINES: [Switch; 6] = [
    (
        "spline_polygon",
        "Control polygon",
        "The dashed polygon through a spline's control points",
        |o| &mut o.spline_polygon,
    ),
    (
        "spline_degree",
        "Degree",
        "Each spline's degree, by its middle",
        |o| &mut o.spline_degree,
    ),
    (
        "spline_multiplicity",
        "Knot multiplicity",
        "How many times each inner knot repeats, by the knot",
        |o| &mut o.spline_multiplicity,
    ),
    (
        "spline_weights",
        "Weights",
        "Each control point's weight, by the point",
        |o| &mut o.spline_weights,
    ),
    ("spline_comb", "Curvature comb", "", |o| &mut o.spline_comb),
    ("spline_knots", "Show knots", "", |o| &mut o.spline_knots),
];

const APPEARANCE: [Switch; 2] = [
    (
        "dimension_dashed",
        "Dashed dimension lines",
        "Tones the dimensions down against the geometry",
        |o| &mut o.appearance.dimension_dashed,
    ),
    ("axis_dashed", "Dashed axes", "", |o| {
        &mut o.appearance.axis_dashed
    }),
];

/// Every switch the page shows.
fn every_switch() -> impl Iterator<Item = &'static Switch> {
    SOLVER.iter().chain(&SPLINES).chain(&APPEARANCE)
}

/// A number the page shows: its id, label, hint, unit, decimals, drag
/// speed, range and the option it sets.
struct Number {
    id: &'static str,
    label: &'static str,
    hint: &'static str,
    unit: &'static str,
    decimals: usize,
    speed: f64,
    range: (f64, f64),
    get: fn(&SketchOptions) -> f64,
    set: fn(&mut SketchOptions, f64),
}

const NUMBERS: [Number; 8] = [
    Number {
        id: "drag_constraint_delay_ms",
        label: "Drag constraint delay",
        hint: "How long a dragged point rests on a target before letting go constrains it there",
        unit: "ms",
        decimals: 0,
        speed: 10.0,
        range: (0.0, 5000.0),
        get: |o| f64::from(o.drag_constraint_delay_ms),
        set: |o, v| o.drag_constraint_delay_ms = v as f32,
    },
    Number {
        id: "min_wall",
        label: "Minimum wall",
        hint: "The wall thickness check marks walls thinner than this",
        unit: "mm",
        decimals: 2,
        speed: 0.01,
        range: (0.01, 100.0),
        get: |o| o.min_wall,
        set: |o, v| o.min_wall = v,
    },
    Number {
        id: "scan_range",
        label: "Scan fit range",
        hint: "A scan's slice draws as fitting where it lies this close to the sketch",
        unit: "mm",
        decimals: 2,
        speed: 0.01,
        range: (0.001, 10.0),
        get: |o| o.scan_range,
        set: |o, v| o.scan_range = v,
    },
    Number {
        id: "label_size",
        label: "Label size",
        hint: "The size of the dimensions' values, growing with the UI scale as the rest of the text does",
        unit: "px",
        decimals: 0,
        speed: 0.2,
        range: (8.0, 40.0),
        get: |o| f64::from(o.appearance.label_size),
        set: |o, v| o.appearance.label_size = v as f32,
    },
    Number {
        id: "dimension_width",
        label: "Dimension lines",
        hint: "How wide the dimension, extension and leader lines draw",
        unit: "px",
        decimals: 1,
        speed: 0.05,
        range: (0.5, 4.0),
        get: |o| f64::from(o.appearance.dimension_width),
        set: |o, v| o.appearance.dimension_width = v as f32,
    },
    Number {
        id: "axis_width",
        label: "Axes",
        hint: "How wide the sketch's axes draw while not picked",
        unit: "px",
        decimals: 1,
        speed: 0.05,
        range: (0.5, 4.0),
        get: |o| f64::from(o.appearance.axis_width),
        set: |o, v| o.appearance.axis_width = v as f32,
    },
    Number {
        id: "grid_opacity",
        label: "Grid",
        hint: "How much the grid's widest lines show, from faint to solid; finer ones show less",
        unit: "",
        decimals: 2,
        speed: 0.01,
        range: (0.05, 1.0),
        get: |o| f64::from(o.appearance.grid_opacity),
        set: |o, v| o.appearance.grid_opacity = v as f32,
    },
    Number {
        id: "region_opacity",
        label: "Region shading",
        hint: "How strongly the regions a profile closes are shaded",
        unit: "",
        decimals: 2,
        speed: 0.01,
        range: (0.02, 0.8),
        get: |o| f64::from(o.appearance.region_opacity),
        set: |o, v| o.appearance.region_opacity = v as f32,
    },
];

const LABEL_FACES: [(bool, &str); 2] = [(true, "Monospace"), (false, "Sans")];

const GRID_LINES: [(GridLines, &str); 3] = [
    (GridLines::Dotted, "Dotted"),
    (GridLines::Dashed, "Dashed"),
    (GridLines::Solid, "Solid"),
];

fn switch(options: &SketchOptions, (id, text, hint, get): &Switch) -> Widget {
    let mut copy = *options;
    let check = Check {
        id: (*id).into(),
        text: (*text).into(),
        on: *get(&mut copy),
        ..Default::default()
    };
    panel::hinted(hint, check.into())
}

fn number(options: &SketchOptions, id: &str) -> Widget {
    let n = NUMBERS
        .iter()
        .find(|n| n.id == id)
        .expect("a listed number");
    let quantity = Quantity {
        id: n.id.into(),
        label: n.label.into(),
        value: (n.get)(options),
        unit: (!n.unit.is_empty()).then(|| n.unit.into()),
        decimals: n.decimals,
        speed: Some(n.speed),
        min: Some(n.range.0),
        max: Some(n.range.1),
        width: Some(120.0),
        ..Default::default()
    };
    panel::hinted(n.hint, quantity.into())
}

fn select<T: PartialEq>(
    id: &str,
    label: &str,
    hint: &str,
    current: &T,
    options: &[(T, &str)],
) -> Widget {
    let select = Select {
        id: id.into(),
        label: label.into(),
        options: options.iter().map(|(_, name)| (*name).into()).collect(),
        selected: options.iter().position(|(v, _)| v == current).unwrap_or(0),
        width: Some(180.0),
        ..Default::default()
    };
    panel::hinted(hint, select.into())
}

fn switches(options: &SketchOptions, ids: &[&str]) -> Vec<Widget> {
    ids.iter()
        .map(|id| {
            let s = every_switch()
                .find(|s| s.0 == *id)
                .expect("a listed switch");
            switch(options, s)
        })
        .collect()
}

/// A palette colour, shown with its hex code.
fn swatch(label: &str, rgb: [f32; 3]) -> Widget {
    let bytes = panel::srgb(rgb);
    Widget::Flow(Flow {
        label: label.into(),
        no_wrap: true,
        children: vec![
            Caption {
                text: format!("#{:02X}{:02X}{:02X}", bytes[0], bytes[1], bytes[2]),
                mono: true,
                small: true,
                ..Default::default()
            }
            .into(),
            Swatches {
                swatches: vec![Swatch {
                    color: bytes,
                    ..Default::default()
                }],
                keys: true,
                ..Default::default()
            }
            .into(),
        ],
    })
}

/// The page's groups; `snap` is whether snapping to objects is on.
pub(crate) fn widgets(options: &SketchOptions, snap: bool) -> Vec<Widget> {
    let group = |title: &str, children: Vec<Widget>| {
        Widget::Settings(SettingsGroup {
            title: title.into(),
            children,
        })
    };
    let mut solver = switches(options, &["auto_constraints"]);
    solver.push(number(options, "drag_constraint_delay_ms"));
    solver.push(select(
        "angle_step_deg",
        "Angle snap step",
        "With Shift held, a segment leaves its start in steps of this angle",
        &options.angle_step_deg,
        &crate::ANGLE_STEPS,
    ));
    solver.extend(switches(
        options,
        &[
            "continue_drawing",
            "continue_constraining",
            "hint_lines",
            "arc_helpers",
            "avoid_redundant_auto",
            "auto_remove_redundant",
            "ask_dimension_value",
        ],
    ));
    solver.push(panel::hinted(
        "Endpoints, midpoints and intersections attract the cursor",
        Check {
            id: "snap".into(),
            text: "Snap to objects".into(),
            on: snap,
            ..Default::default()
        }
        .into(),
    ));
    solver.extend(switches(options, &["section_on_open", "shade_regions"]));
    let look = &options.appearance;
    let appearance = vec![
        number(options, "label_size"),
        select(
            "label_mono",
            "Label face",
            "The face the dimensions' values are set in",
            &look.label_mono,
            &LABEL_FACES,
        ),
        number(options, "dimension_width"),
        switches(options, &["dimension_dashed"]).remove(0),
        number(options, "axis_width"),
        switches(options, &["axis_dashed"]).remove(0),
        number(options, "grid_opacity"),
        select(
            "grid_lines",
            "Grid lines",
            "",
            &look.grid_lines,
            &GRID_LINES,
        ),
        ColorField {
            id: "grid_color".into(),
            label: "Grid colour".into(),
            color: look.grid_color,
        }
        .into(),
        number(options, "region_opacity"),
    ];
    let pal = core_document::SketchPalette::default();
    vec![
        group("Solver & constraints", solver),
        group(
            "Splines",
            switches(
                options,
                &[
                    "spline_polygon",
                    "spline_degree",
                    "spline_multiplicity",
                    "spline_weights",
                    "spline_comb",
                    "spline_knots",
                ],
            ),
        ),
        group(
            "Printing",
            vec![number(options, "min_wall"), number(options, "scan_range")],
        ),
        group("Appearance", appearance),
        group(
            "Colors",
            vec![
                swatch("Geometry", pal.geometry),
                swatch("Construction", pal.construction),
                swatch("External", pal.external),
                swatch("Locked", pal.locked),
                swatch("Fully constrained", pal.fully_constrained),
                swatch("Selected", pal.selected),
                swatch("Preselect", pal.preselect),
                swatch("Constraint", pal.constraint),
                swatch("Reference dimension", pal.reference),
            ],
        ),
    ]
}

/// What a change on the page sets; `snap` is snapping to objects.
pub(crate) fn apply(options: &mut SketchOptions, snap: &mut bool, event: &PanelEvent) {
    match event {
        PanelEvent::Toggle { id, on } if id == "snap" => *snap = *on,
        PanelEvent::Toggle { id, on } => {
            if let Some((.., get)) = every_switch().find(|s| s.0 == id) {
                *get(options) = *on;
            }
        }
        PanelEvent::Number { id, value } => {
            if let Some(n) = NUMBERS.iter().find(|n| n.id == id) {
                (n.set)(options, value.clamp(n.range.0, n.range.1));
            }
        }
        PanelEvent::Choice { id, index } => match id.as_str() {
            "angle_step_deg" => {
                if let Some((v, _)) = crate::ANGLE_STEPS.get(*index) {
                    options.angle_step_deg = *v;
                }
            }
            "label_mono" => {
                if let Some((v, _)) = LABEL_FACES.get(*index) {
                    options.appearance.label_mono = *v;
                }
            }
            "grid_lines" => {
                if let Some((v, _)) = GRID_LINES.get(*index) {
                    options.appearance.grid_lines = *v;
                }
            }
            _ => {}
        },
        PanelEvent::Color { id, color } if id == "grid_color" => {
            options.appearance.grid_color = *color;
        }
        _ => {}
    }
}
