//! The sketcher's tool table: its tools, actions and default keys as the
//! toolbar registers them, each tool's icon and first prompt, and the tool a
//! variant specialises.

use crate::{POLYLINE_ARC_ACTION, SketchWorkbench, commands};
use core_document::{ToolDescriptor, ToolVariant, WorkbenchContext, base_tool_id, tool_variant};

/// Tool ids, labels and icons of the geometry row, in toolbar order.
const GEOMETRY_TOOLS: &[(&str, &str, &str)] = &[
    ("sketch.select", "Select", "select"),
    ("sketch.point", "Point", "point"),
    ("sketch.line", "Line", "line"),
    ("sketch.arc", "Arc", "arc"),
    ("sketch.circle", "Circle", "circle"),
    ("sketch.ellipse", "Ellipse", "ellipse"),
    ("sketch.bspline", "B-spline", "bspline"),
    ("sketch.rect", "Rectangle", "rectangle"),
    ("sketch.polygon", "Regular polygon", "regular-polygon"),
    ("sketch.slot", "Slot", "slot"),
    ("sketch.text", "Text", "sketch-text"),
    ("sketch.fillet", "Fillet", "sketch-fillet"),
    ("sketch.trim", "Trim", "trim"),
    ("sketch.extend", "Extend", "extend"),
    ("sketch.split", "Split", "split"),
    ("sketch.copy", "Copy", "copy-geometry"),
    ("sketch.translate", "Move", "move-geometry"),
    ("sketch.rotate", "Rotate", "rotate-geometry"),
    ("sketch.scale", "Scale", "scale-geometry"),
    ("sketch.offset", "Offset", "offset-geometry"),
    ("sketch.mirror", "Symmetry", "symmetry-geometry"),
];

/// Default keys of the tools; the user can rebind them in Preferences.
/// A plain letter draws or edits, Shift and a letter constrains.
const TOOL_KEYS: &[(&str, &str)] = &[
    ("sketch.point", "O"),
    ("sketch.line", "L"),
    ("sketch.polyline", "P"),
    ("sketch.arc", "A"),
    ("sketch.circle", "C"),
    ("sketch.ellipse", "E"),
    ("sketch.bspline", "B"),
    ("sketch.rect", "R"),
    ("sketch.polygon", "G"),
    ("sketch.slot", "S"),
    ("sketch.trim", "T"),
    ("sketch.external", "X"),
    ("sketch.construction", "N"),
    ("sketch.view_sketch", "V"),
    ("sketch.internal_geometry", "I"),
    ("sketch.constrain.coincident", "Shift+C"),
    ("sketch.constrain.point_on_object", "Shift+O"),
    ("sketch.constrain.vertical", "Shift+V"),
    ("sketch.constrain.horizontal", "Shift+H"),
    ("sketch.constrain.parallel", "Shift+P"),
    ("sketch.constrain.perpendicular", "Shift+N"),
    ("sketch.constrain.tangent", "Shift+T"),
    ("sketch.constrain.equal", "Shift+E"),
    ("sketch.constrain.symmetric", "Shift+S"),
    ("sketch.constrain.block", "Shift+B"),
    ("sketch.constrain.group", "Shift+G"),
    ("sketch.constrain.dimension", "Shift+D"),
    ("sketch.constrain.lock", "Shift+K"),
    ("sketch.constrain.distance_x", "Shift+L"),
    ("sketch.constrain.distance_y", "Shift+I"),
    ("sketch.constrain.radius", "Shift+R"),
    ("sketch.constrain.angle", "Shift+A"),
];

/// The default key of a tool, if it has one.
fn tool_key(id: &str) -> Option<&'static str> {
    TOOL_KEYS.iter().find(|(t, _)| *t == id).map(|(_, k)| *k)
}

/// `tool` with its default key, if it has one.
fn keyed(tool: ToolDescriptor) -> ToolDescriptor {
    match tool_key(&tool.id) {
        Some(key) => tool.shortcut(key),
        None => tool,
    }
}

/// The icon of a canonical tool id, for the viewport hint.
pub(crate) fn tool_icon(tool: &str) -> &'static str {
    match tool {
        "sketch.external" => "external-geometry",
        "sketch.arc3" => "arc-3pt",
        "sketch.circle3" => "circle-3pt",
        "sketch.ellipse3" => "ellipse-3pt",
        "sketch.polyline" => "polyline",
        "sketch.ellipse_arc" => "arc-of-ellipse",
        "sketch.parabola" => "arc-of-parabola",
        "sketch.hyperbola" => "arc-of-hyperbola",
        "sketch.rect_center" | "sketch.rect_center3" => "rectangle-centered",
        "sketch.rect3" => "rectangle",
        "sketch.rect_frame" => "offset-geometry",
        "sketch.arc_slot" => "arc-slot",
        "sketch.chamfer" => "sketch-chamfer",
        _ => GEOMETRY_TOOLS
            .iter()
            .find(|(id, _, _)| *id == tool)
            .map(|(_, _, icon)| *icon)
            .unwrap_or("sketch"),
    }
}

/// The name and first prompt of a tool with no shape in progress.
pub(crate) fn idle_hint(tool: &str) -> (&'static str, &'static str) {
    match tool {
        "sketch.point" => ("Point", "Click to place a point"),
        "sketch.line" => ("Line", "Click the start point"),
        "sketch.polyline" => ("Polyline", "Click the start point"),
        "sketch.arc" => ("Arc", "Click the center"),
        "sketch.arc3" => ("Arc", "Click the first endpoint"),
        "sketch.circle" => ("Circle", "Click the center"),
        "sketch.circle3" => ("Circle", "Click a first rim point"),
        "sketch.ellipse" => ("Ellipse", "Click the center"),
        "sketch.ellipse3" => ("Ellipse", "Click one end of the major axis"),
        "sketch.ellipse_arc" => ("Arc of ellipse", "Click the center"),
        "sketch.parabola" => ("Arc of parabola", "Click the vertex"),
        "sketch.hyperbola" => ("Arc of hyperbola", "Click the center"),
        "sketch.bspline" => ("B-spline", "Click its first point"),
        "sketch.rect" => ("Rectangle", "Click the first corner"),
        "sketch.rect_center" | "sketch.rect_center3" => ("Rectangle", "Click the center"),
        "sketch.rect3" => ("Rectangle", "Click the first corner"),
        "sketch.rect_frame" => ("Frame", "Click the first corner"),
        "sketch.polygon" => ("Polygon", "Click the center"),
        "sketch.slot" => ("Slot", "Click the centerline start"),
        "sketch.arc_slot" => ("Arc slot", "Click the arc center"),
        "sketch.text" => (
            "Text",
            "Click where the text starts, a frame's two corners, or the curve to follow",
        ),
        "sketch.fillet" => ("Fillet", "Click a corner, or two curves"),
        "sketch.chamfer" => ("Chamfer", "Click a corner, or two curves"),
        "sketch.trim" => ("Trim", "Click the span to remove, or drag across spans"),
        "sketch.external" => (
            "External geometry",
            "Click edges of a solid to bring them in, or a face for every edge around it; \
             Ctrl picks more",
        ),
        "sketch.extend" => ("Extend", "Click the end to extend"),
        "sketch.split" => ("Split", "Click where to split"),
        "sketch.offset" => ("Offset", "Click the curve to offset"),
        "sketch.translate" => ("Move", "Click the base point"),
        "sketch.rotate" => ("Rotate", "Click the pivot"),
        "sketch.scale" => ("Scale", "Click the base point"),
        "sketch.mirror" => ("Symmetry", "Click a mirror line or its first point"),
        _ => ("Select", "Click to select · drag for a box"),
    }
}

/// The tool a variant specialises, as the tool state machine knows it;
/// `None` for a tool not the sketcher's.
pub(crate) fn canonical_tool(tool: &str) -> Option<String> {
    let base = base_tool_id(tool);
    if !base.starts_with("sketch.") {
        return None;
    }
    Some(match (base, tool_variant(tool)) {
        ("sketch.arc", Some("3pt")) => "sketch.arc3".to_string(),
        ("sketch.circle", Some("3pt")) => "sketch.circle3".to_string(),
        ("sketch.ellipse", Some("3pt")) => "sketch.ellipse3".to_string(),
        ("sketch.ellipse", Some("arc")) => "sketch.ellipse_arc".to_string(),
        ("sketch.ellipse", Some("parabola")) => "sketch.parabola".to_string(),
        ("sketch.ellipse", Some("hyperbola")) => "sketch.hyperbola".to_string(),
        ("sketch.rect", Some("center")) => "sketch.rect_center".to_string(),
        ("sketch.rect", Some("rounded")) => "sketch.rect_rounded".to_string(),
        ("sketch.rect", Some("3pt")) => "sketch.rect3".to_string(),
        ("sketch.rect", Some("center3pt")) => "sketch.rect_center3".to_string(),
        ("sketch.rect", Some("frame")) => "sketch.rect_frame".to_string(),
        ("sketch.slot", Some("arc")) => "sketch.arc_slot".to_string(),
        ("sketch.fillet", Some("chamfer")) => "sketch.chamfer".to_string(),
        ("sketch.polygon", Some(_)) => "sketch.polygon".to_string(),
        ("sketch.bspline", Some(_)) => "sketch.bspline".to_string(),
        ("sketch.copy", _) => "sketch.translate".to_string(),
        (base, _) => base.to_string(),
    })
}

impl SketchWorkbench {
    pub(crate) fn register_tools(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        context.register_action(
            core_document::ActionDescriptor::new(
                POLYLINE_ARC_ACTION,
                "Polyline: switch between line and arc",
            )
            .category("create.basic")
            .shortcut("M"),
        );
        // Row 0: sketch management, beside the standard tools.
        context.register_tool(
            ToolDescriptor::new_action("sketch.create", "Create sketch", Some("sketch.manage"))
                .icon("sketch-new")
                .row(0),
        );
        let manage = |id: &str, label: &str, icon: &'static str| {
            ToolDescriptor::new_action(id, label, Some("sketch.manage"))
                .icon(icon)
                .row(0)
        };
        context.register_tool(manage("sketch.edit", "Edit sketch", "sketch-edit"));
        context.register_tool(manage("sketch.attach", "Attach sketch", "sketch-map"));
        context.register_tool(
            manage("sketch.reorient", "Reorient sketch", "sketch-reorient").variants(vec![
                ToolVariant::new("flip", "Flip normal", "sketch-reorient"),
                ToolVariant::new("rotate", "Rotate 90°", "sketch-reorient"),
            ]),
        );
        context.register_tool(manage(
            "sketch.mirror_sketch",
            "Mirror sketch",
            "sketch-mirror",
        ));
        context.register_tool(
            ToolDescriptor::new_action("sketch.merge", "Merge sketches", Some("sketch.manage"))
                .icon("sketch-merge")
                .row(0),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.finish", "Close sketch", Some("sketch.close"))
                .icon("sketch-leave")
                .row(0)
                .align_end(),
        );

        // Row 1: geometry. Tools with variants offer them from a dropdown.
        let variants = |id: &str| -> Vec<ToolVariant> {
            match id {
                "sketch.arc" => vec![
                    ToolVariant::new("center", "Center and endpoints", "arc"),
                    ToolVariant::new("3pt", "Three points", "arc-3pt"),
                ],
                "sketch.circle" => vec![
                    ToolVariant::new("center", "Center and rim", "circle"),
                    ToolVariant::new("3pt", "Three points", "circle-3pt"),
                ],
                "sketch.ellipse" => vec![
                    ToolVariant::new("center", "Center and axes", "ellipse"),
                    ToolVariant::new("3pt", "Three points", "ellipse-3pt"),
                    ToolVariant::new("arc", "Arc of ellipse", "arc-of-ellipse"),
                    ToolVariant::new("parabola", "Arc of parabola", "arc-of-parabola"),
                    ToolVariant::new("hyperbola", "Arc of hyperbola", "arc-of-hyperbola"),
                ],
                "sketch.bspline" => vec![
                    ToolVariant::new("open", "Open", "bspline"),
                    ToolVariant::new("periodic", "Periodic", "periodic-bspline"),
                    ToolVariant::new("through", "Through points", "bspline-knot"),
                    ToolVariant::new("through_periodic", "Closed through points", "bspline-knot"),
                ],
                "sketch.rect" => vec![
                    ToolVariant::new("corners", "Two corners", "rectangle"),
                    ToolVariant::new("center", "Center and corner", "rectangle-centered"),
                    ToolVariant::new("rounded", "Rounded", "rounded-rectangle"),
                    ToolVariant::new("3pt", "Three corners", "rectangle"),
                    ToolVariant::new("center3pt", "Center and two corners", "rectangle-centered"),
                    ToolVariant::new("frame", "Frame", "offset-geometry"),
                ],
                "sketch.polygon" => vec![
                    ToolVariant::new("3", "Triangle", "triangle"),
                    ToolVariant::new("4", "Square", "square"),
                    ToolVariant::new("5", "Pentagon", "pentagon"),
                    ToolVariant::new("6", "Hexagon", "hexagon"),
                    ToolVariant::new("7", "Heptagon", "heptagon"),
                    ToolVariant::new("8", "Octagon", "octagon"),
                ],
                "sketch.slot" => vec![
                    ToolVariant::new("straight", "Straight slot", "slot"),
                    ToolVariant::new("arc", "Arc slot", "arc-slot"),
                ],
                "sketch.fillet" => vec![
                    ToolVariant::new("fillet", "Fillet", "sketch-fillet"),
                    ToolVariant::new("chamfer", "Chamfer", "sketch-chamfer"),
                ],
                _ => Vec::new(),
            }
        };
        let category = |id: &str| -> &'static str {
            match id {
                "sketch.select" | "sketch.point" | "sketch.line" => "create.basic",
                "sketch.arc" | "sketch.circle" | "sketch.ellipse" | "sketch.bspline" => {
                    "create.curves"
                }
                "sketch.rect" | "sketch.polygon" | "sketch.slot" => "create.shapes",
                "sketch.text" => "create.text",
                "sketch.fillet" | "sketch.trim" | "sketch.extend" | "sketch.split" => "modify.edit",
                _ => "modify.transform",
            }
        };
        for (id, label, icon) in GEOMETRY_TOOLS {
            let mut tool = keyed(
                ToolDescriptor::new(*id, *label, Some(category(id)))
                    .icon(icon)
                    .variants(variants(id)),
            );
            tool.row = 1;
            if *id == "sketch.split" {
                // The row's planned entries sit after split.
                context.register_tool(tool);
                context.register_tool(keyed(
                    ToolDescriptor::new(
                        "sketch.external",
                        "External geometry",
                        Some("reference.external"),
                    )
                    .icon("external-geometry")
                    .variants(vec![
                        ToolVariant::new("edges", "Projected edges", "external-geometry"),
                        ToolVariant::new(
                            "intersection",
                            "Intersection with faces",
                            "clipping-plane",
                        ),
                    ])
                    .row(1),
                ));
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.external_from",
                        "External from a sketch or datum",
                        Some("reference.external"),
                    )
                    .icon("clone-geometry")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.external_defining",
                        "Toggle external geometry in profiles",
                        Some("reference.external"),
                    )
                    .icon("external-defining")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.carbon_copy",
                        "Carbon copy",
                        Some("reference.external"),
                    )
                    .icon("carbon-copy")
                    .row(1),
                );
                for (id, label, icon) in [
                    ("sketch.select_origin", "Select origin", "select-origin"),
                    (
                        "sketch.select_x_axis",
                        "Select horizontal axis",
                        "select-horizontal-axis",
                    ),
                    (
                        "sketch.select_y_axis",
                        "Select vertical axis",
                        "select-vertical-axis",
                    ),
                ] {
                    context.register_tool(
                        ToolDescriptor::new_action(id, label, Some("reference.external"))
                            .icon(icon)
                            .row(1),
                    );
                }
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.design_values",
                        "Dimension at design values",
                        Some("reference.fit"),
                    )
                    .icon("measure")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new(
                        "sketch.fit_edge",
                        "Fit to a picture's edge",
                        Some("reference.fit"),
                    )
                    .icon("fit-selection")
                    .row(1),
                );
                context.register_tool(
                    ToolDescriptor::new_action(
                        "sketch.fit_scan",
                        "Fit the scan's section",
                        Some("reference.fit"),
                    )
                    .icon("fit-scan")
                    .row(1),
                );
                context.register_tool(keyed(
                    ToolDescriptor::new_action(
                        "sketch.construction",
                        "Construction",
                        Some("create.construction"),
                    )
                    .icon("construction-mode")
                    .row(1),
                ));
                context.register_tool(keyed(
                    ToolDescriptor::new_action(
                        "sketch.internal_geometry",
                        "Show/hide internal geometry",
                        Some("create.construction"),
                    )
                    .icon("show-hide-internal-geometry")
                    .row(1),
                ));
                continue;
            }
            if *id == "sketch.point" {
                context.register_tool(tool);
                context.register_tool(keyed(
                    ToolDescriptor::new("sketch.polyline", "Polyline", Some("create.basic"))
                        .icon("polyline")
                        .row(1),
                ));
                continue;
            }
            context.register_tool(tool);
        }
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.array",
                "Rectangular array",
                Some("modify.transform"),
            )
            .icon("rectangular-array")
            .row(1),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.join", "Join curves", Some("modify.edit"))
                .icon("join-curves")
                .row(1),
        );
        for (id, label, icon) in [
            (
                "sketch.bspline_convert",
                "Convert to B-spline",
                "bspline-convert",
            ),
            (
                "sketch.bspline_degree_up",
                "Raise spline degree",
                "bspline-degree-up",
            ),
            (
                "sketch.bspline_degree_down",
                "Lower spline degree",
                "bspline-degree-down",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("modify.bspline"))
                    .icon(icon)
                    .row(1),
            );
        }
        context.register_tool(
            ToolDescriptor::new("sketch.bspline_knot", "Insert knot", Some("modify.bspline"))
                .icon("bspline-knot")
                .row(1),
        );
        for (id, label, icon) in [
            ("sketch.bspline_comb", "Curvature comb", "bspline-comb"),
            ("sketch.bspline_knots", "Show knots", "bspline-polygon"),
            (
                "sketch.bspline_polygon",
                "Control polygon",
                "bspline-control-polygon",
            ),
            (
                "sketch.bspline_degree_label",
                "Show degree",
                "bspline-degree-label",
            ),
            (
                "sketch.bspline_multiplicity",
                "Show knot multiplicity",
                "bspline-multiplicity",
            ),
            ("sketch.bspline_weights", "Show weights", "bspline-weight"),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("modify.bspline"))
                    .icon(icon)
                    .row(2),
            );
        }
        for (id, label, icon) in [
            (
                "sketch.delete_all_geometry",
                "Delete all geometry",
                "delete-all-geometry",
            ),
            (
                "sketch.delete_all_constraints",
                "Delete all constraints",
                "delete-all-constraints",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("modify.delete"))
                    .icon(icon)
                    .row(1),
            );
        }

        // Row 2: constraints, enabled by the shape of the selection.
        let constraint = |id: &str, label: &str, icon: &'static str, category: &str| {
            keyed(
                ToolDescriptor::new_action(format!("sketch.constrain.{id}"), label, Some(category))
                    .icon(icon)
                    .row(2),
            )
        };
        for (id, label, icon) in [
            ("coincident", "Coincident", "constraint-coincident"),
            (
                "point_on_object",
                "Point on object",
                "constraint-point-on-object",
            ),
            ("vertical", "Vertical", "constraint-vertical"),
            ("horizontal", "Horizontal", "constraint-horizontal"),
            ("parallel", "Parallel", "constraint-parallel"),
            ("perpendicular", "Perpendicular", "constraint-perpendicular"),
            ("tangent", "Tangent", "constraint-tangent"),
            ("equal", "Equal", "constraint-equal"),
            ("symmetric", "Symmetric", "constraint-symmetric"),
            ("block", "Block", "constraint-block"),
            ("group", "Group", "tree-group"),
        ] {
            let mut tool = constraint(id, label, icon, "constraints.geometric");
            if id == "horizontal" {
                tool = tool.variants(vec![
                    ToolVariant::new("horizontal", "Horizontal", "constraint-horizontal"),
                    ToolVariant::new(
                        "horizontal_vertical",
                        "Horizontal or vertical, whichever is nearer",
                        "constraint-horizontal",
                    ),
                ]);
            }
            if id == "point_on_object" {
                tool = tool.variants(vec![
                    ToolVariant::new(
                        "point_on_object",
                        "Point on object",
                        "constraint-point-on-object",
                    ),
                    ToolVariant::new("midpoint", "Midpoint", "constraint-point-on-object"),
                ]);
            }
            context.register_tool(tool);
        }
        // One dimension tool: the distance, radius or angle the selection
        // takes.
        context.register_tool(constraint(
            "dimension",
            "Dimension",
            "dimensional-constraint",
            "dimensions",
        ));
        for (id, label, icon) in [
            ("lock", "Lock", "constraint-lock"),
            ("distance_x", "Horizontal distance", "constraint-distance-x"),
            ("distance_y", "Vertical distance", "constraint-distance-y"),
            ("distance", "Distance", "constraint-distance"),
            ("arc_length", "Arc length", "constraint-arc-length"),
            ("radius", "Radius", "constraint-radius"),
            ("diameter", "Diameter", "constraint-diameter"),
            ("angle", "Angle", "constraint-angle"),
            ("refraction", "Refraction", "constraint-refraction"),
        ] {
            let mut tool = constraint(id, label, icon, "dimensions");
            match id {
                "distance" => {
                    tool = tool.variants(vec![
                        ToolVariant::new("distance", "Distance", "constraint-distance"),
                        ToolVariant::new("gap", "Gap between curves", "constraint-distance"),
                    ]);
                }
                "radius" => {
                    tool = tool.variants(vec![
                        ToolVariant::new("radius", "Radius", "constraint-radius"),
                        ToolVariant::new(
                            "radius_diameter",
                            "Radius or diameter by kind",
                            "constraint-diameter",
                        ),
                        ToolVariant::new(
                            "ellipse_minor",
                            "An ellipse's minor radius",
                            "constraint-radius",
                        ),
                    ]);
                }
                "angle" => {
                    tool = tool.variants(vec![
                        ToolVariant::new("angle", "Between two lines", "constraint-angle"),
                        ToolVariant::new("angle_x", "To the X axis", "constraint-angle"),
                        ToolVariant::new("angle_y", "To the Y axis", "constraint-angle"),
                        ToolVariant::new(
                            "angle_at_point",
                            "Where two curves meet",
                            "constraint-angle",
                        ),
                        ToolVariant::new("arc_angle", "An arc's opening", "constraint-angle"),
                        ToolVariant::new(
                            "angle_three_points",
                            "By three points (arm, corner, arm)",
                            "constraint-angle",
                        ),
                    ]);
                }
                _ => {}
            }
            context.register_tool(tool);
        }
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.remove_axis_alignment",
                "Remove axis alignment",
                Some("constraints.geometric"),
            )
            .icon("toggle-axis-cross")
            .row(2),
        );
        for (id, label, icon) in [
            (
                "sketch.toggle_driving",
                "Toggle driving / reference",
                "toggle-driving",
            ),
            ("sketch.toggle_active", "Toggle active", "toggle-active"),
            (
                "sketch.park_constraints",
                "Park / unpark constraints",
                "pin",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("constraints.toggle"))
                    .icon(icon)
                    .row(2),
            );
        }
        for (id, label, icon) in [
            (
                "sketch.select_conflicting",
                "Select conflicting",
                "select-conflicting",
            ),
            (
                "sketch.select_redundant",
                "Select redundant",
                "select-redundant",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("inspect.select"))
                    .icon(icon)
                    .row(2),
            );
        }
        for (id, label, icon) in [
            (
                "sketch.select_malformed",
                "Select malformed",
                "select-malformed",
            ),
            (
                "sketch.select_unconstrained",
                "Select under-constrained",
                "select-unconstrained",
            ),
            (
                "sketch.select_dof",
                "Elements with DoF",
                "select-elements-with-dof",
            ),
            (
                "sketch.select_constraints",
                "Constraints of the selection",
                "select-constraints",
            ),
            (
                "sketch.select_elements",
                "Elements of the selected constraints",
                "select-elements",
            ),
        ] {
            context.register_tool(
                ToolDescriptor::new_action(id, label, Some("inspect.select"))
                    .icon(icon)
                    .row(2),
            );
        }
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.show_constraints",
                "Show/hide constraints",
                Some("view.display"),
            )
            .icon("show-hide-constraints")
            .row(2),
        );
        context.register_tool(keyed(
            ToolDescriptor::new_action("sketch.view_sketch", "View sketch", Some("view.display"))
                .icon("sketch-view")
                .row(2),
        ));
        context.register_tool(
            ToolDescriptor::new_action("sketch.section_view", "Section view", Some("view.display"))
                .icon("clipping-plane")
                .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.parked_layer",
                "Show parked constraints",
                Some("view.display"),
            )
            .icon("eye")
            .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action(
                "sketch.rendering_order",
                "Rendering order",
                Some("view.display"),
            )
            .icon("rendering-order")
            .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.grid", "Grid", Some("view.snap"))
                .icon("grid")
                .row(2),
        );
        context.register_tool(
            ToolDescriptor::new_action("sketch.snap", "Snap to objects", Some("view.snap"))
                .icon("snap")
                .row(2),
        );
        // Checks and file exports, last in their panels.
        let check = |id: &str, label: &str, icon: &'static str| {
            let category = if id.starts_with("sketch.save_") {
                "sketch.export"
            } else {
                "inspect.check"
            };
            ToolDescriptor::new_action(id, label, Some(category))
                .icon(icon)
                .row(0)
        };
        context.register_tool(check(
            "sketch.validate",
            "Validate sketch",
            "check-geometry",
        ));
        context.register_tool(check(
            "sketch.repair",
            "Repair sketch: join near ends, drop empty and doubled curves",
            "sketch-validate",
        ));
        context.register_tool(check(
            "sketch.wall_thickness",
            "Check wall thickness",
            "thickness",
        ));
        context.register_tool(check("sketch.save_dxf", "Save as DXF…", "save"));
        context.register_tool(check("sketch.save_svg", "Save as SVG…", "save"));
        context.register_tool(check("sketch.save_csv", "Save as CSV…", "save"));
    }
}

#[cfg(all(test, feature = "egui"))]
mod icon_coverage {
    use super::*;
    use crate::SketchFeature;
    use crate::sketch::{Sketch, SketchPlane};
    use core_document::{FeatureId, Workbench, WorkbenchContext};

    #[test]
    fn the_bench_and_its_feature_name_an_icon_in_the_set() {
        let wb = SketchWorkbench::default();
        assert!(ui_kit::icon::exists(wb.descriptor().icon));
        let node = core_document::FeatureNode::new(
            FeatureId(uuid::Uuid::new_v4()),
            &SketchFeature::new(Sketch::new("s"), SketchPlane::default()),
        );
        assert!(ui_kit::icon::exists(wb.feature_info(&node).icon));
    }

    #[test]
    fn every_default_key_lands_on_a_tool_and_no_two_share_one() {
        let mut ctx = WorkbenchContext::default();
        SketchWorkbench::default().configure(&mut ctx);
        for (id, _) in TOOL_KEYS {
            assert!(ctx.tools().iter().any(|t| t.id == *id), "no tool {id}");
        }
        let mut seen = std::collections::HashMap::new();
        let keyed = ctx
            .tools()
            .iter()
            .map(|t| (&t.id, &t.shortcuts))
            .chain(ctx.actions().iter().map(|a| (&a.id, &a.shortcuts)));
        for (id, keys) in keyed {
            for key in keys {
                if let Some(other) = seen.insert(*key, id.clone()) {
                    panic!("{id} and {other} share {key}");
                }
            }
        }
    }

    #[test]
    fn every_tool_names_an_icon_in_the_set() {
        let mut ctx = WorkbenchContext::default();
        SketchWorkbench::default().configure(&mut ctx);
        for tool in ctx.tools() {
            let icon = tool
                .icon
                .unwrap_or_else(|| panic!("{} has no icon", tool.id));
            assert!(
                ui_kit::icon::exists(icon),
                "{}: unknown icon {icon}",
                tool.id
            );
            for variant in &tool.variants {
                assert!(
                    ui_kit::icon::exists(variant.icon),
                    "{}:{}: unknown icon {}",
                    tool.id,
                    variant.id,
                    variant.icon
                );
            }
        }
    }
}
