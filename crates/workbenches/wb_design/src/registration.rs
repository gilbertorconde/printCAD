//! The bench's tools: their default keys, their registration and when
//! each is enabled.

use crate::{DesignWorkbench, centre, commands, generators, hole_tables, primitive_icon};
use core_document::{
    ToolDescriptor, ToolVariant, WorkbenchContext, WorkbenchRuntimeContext, base_tool_id,
};

/// Primitive shapes offered from the primitive tools' dropdowns.
pub(crate) const PRIMITIVE_SHAPES: &[(&str, &str)] = &[
    ("box", "Box"),
    ("cylinder", "Cylinder"),
    ("sphere", "Sphere"),
    ("cone", "Cone"),
    ("torus", "Torus"),
    ("ellipsoid", "Ellipsoid"),
    ("prism", "Prism"),
    ("wedge", "Wedge"),
    ("tube", "Tube"),
];

fn primitive_variants(subtractive: bool) -> Vec<ToolVariant> {
    PRIMITIVE_SHAPES
        .iter()
        .map(|(id, label)| ToolVariant::new(id, label, primitive_icon(id, subtractive)))
        .collect()
}

/// Default keys of the tools; the user can rebind them in Preferences.
/// Shift and a letter makes the subtractive form of what the letter adds.
pub(crate) const TOOL_KEYS: &[(&str, &str)] = &[
    ("design.new_body", "B"),
    ("design.new_sketch", "S"),
    ("design.pad", "E"),
    ("design.pocket", "Shift+E"),
    ("design.revolve", "R"),
    ("design.groove", "Shift+R"),
    ("design.loft", "L"),
    ("design.subtractive_loft", "Shift+L"),
    ("design.pipe", "W"),
    ("design.subtractive_pipe", "Shift+W"),
    ("design.hole", "Shift+H"),
    ("design.fillet", "U"),
    ("design.chamfer", "C"),
    ("design.mirror", "M"),
];

/// Register `tool` with its default key, if it has one.
fn register(context: &mut WorkbenchContext, tool: ToolDescriptor) {
    let key = TOOL_KEYS.iter().find(|(id, _)| *id == tool.id);
    context.register_tool(match key {
        Some((_, key)) => tool.shortcut(key),
        None => tool,
    });
}

impl DesignWorkbench {
    pub(crate) fn register_tools(&self, context: &mut WorkbenchContext) {
        commands::register(context);
        // The hole cuts the user keeps, read once as the bench starts.
        hole_tables::user_cut_profiles();
        let action = |id: &str, label: &str, icon: &'static str, category: &str| {
            ToolDescriptor::new_action(id, label, Some(category)).icon(icon)
        };
        // Structure and sketches.
        register(
            context,
            action("design.new_body", "Create body", "body", "structure"),
        );
        register(
            context,
            action(
                "design.new_sketch",
                "Create sketch",
                "sketch-new",
                "structure",
            ),
        );
        register(
            context,
            action(
                "design.edit_sketch",
                "Edit sketch",
                "sketch-edit",
                "structure",
            ),
        );
        register(
            context,
            action(
                "design.map_sketch",
                "Map sketch to face",
                "sketch-map",
                "structure",
            ),
        );
        // Datums.
        register(
            context,
            action("design.datum_point", "Datum point", "datum-point", "datum"),
        );
        register(
            context,
            action("design.datum_line", "Datum line", "datum-line", "datum"),
        );
        register(
            context,
            action("design.datum_plane", "Datum plane", "datum-plane", "datum"),
        );
        register(
            context,
            action(
                "design.coordinate_system",
                "Local coordinate system",
                "coordinate-system",
                "datum",
            ),
        );
        register(context, action("design.clone", "Clone", "clone", "datum"));
        register(
            context,
            action(
                "design.borrow",
                "Borrow geometry",
                "clone-geometry",
                "datum",
            ),
        );
        // Profiles made from numbers.
        register(context, generators::tool());
        generators::register(context);
        // Additive.
        register(context, action("design.pad", "Pad", "pad", "additive"));
        register(
            context,
            action("design.revolve", "Revolution", "revolution", "additive"),
        );
        register(
            context,
            action("design.loft", "Additive loft", "additive-loft", "additive"),
        );
        register(
            context,
            action("design.pipe", "Additive pipe", "additive-pipe", "additive"),
        );
        register(
            context,
            action(
                "design.helix",
                "Additive helix",
                "additive-helix",
                "additive",
            ),
        );
        register(
            context,
            action(
                "design.primitive",
                "Additive primitive",
                "additive-box",
                "additive",
            )
            .variants(primitive_variants(false)),
        );
        // Subtractive.
        register(
            context,
            action("design.pocket", "Pocket", "pocket", "subtractive"),
        );
        register(
            context,
            action("design.hole", "Hole", "hole", "subtractive"),
        );
        register(
            context,
            action("design.groove", "Groove", "groove", "subtractive"),
        );
        register(
            context,
            action(
                "design.subtractive_loft",
                "Subtractive loft",
                "subtractive-loft",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "design.subtractive_pipe",
                "Subtractive pipe",
                "subtractive-pipe",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "design.subtractive_helix",
                "Subtractive helix",
                "subtractive-helix",
                "subtractive",
            ),
        );
        register(
            context,
            action(
                "design.subtractive_primitive",
                "Subtractive primitive",
                "subtractive-box",
                "subtractive",
            )
            .variants(primitive_variants(true)),
        );
        // Transformations.
        register(
            context,
            action("design.mirror", "Mirrored", "mirrored", "transform"),
        );
        register(
            context,
            action(
                "design.linear_pattern",
                "Linear pattern",
                "linear-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "design.polar_pattern",
                "Polar pattern",
                "polar-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "design.circular_pattern",
                "Circular pattern",
                "circular-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "design.point_pattern",
                "Point pattern",
                "point-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "design.path_pattern",
                "Path pattern",
                "path-pattern",
                "transform",
            ),
        );
        register(
            context,
            action(
                "design.multi_transform",
                "Multi-transform",
                "multi-transform",
                "transform",
            ),
        );
        register(
            context,
            action("design.scaled", "Scaled", "scaled", "transform"),
        );
        // Dress-up.
        register(
            context,
            action("design.fillet", "Fillet", "fillet", "dressup"),
        );
        register(
            context,
            action("design.chamfer", "Chamfer", "chamfer", "dressup"),
        );
        register(context, action("design.draft", "Draft", "draft", "dressup"));
        register(
            context,
            action("design.thickness", "Thickness", "thickness", "dressup"),
        );
        register(
            context,
            action(
                "design.offset_faces",
                "Offset faces",
                "offset-geometry",
                "dressup",
            ),
        );
        register(
            context,
            action(
                "design.offset_solid",
                "Offset solid",
                "offset-solid",
                "dressup",
            ),
        );
        register(
            context,
            action(
                "design.scale_body",
                "Scale body",
                "scale-geometry",
                "dressup",
            ),
        );
        register(
            context,
            action(
                "design.move_faces",
                "Move faces",
                "move-geometry",
                "dressup",
            ),
        );
        register(
            context,
            action("design.delete_faces", "Delete faces", "delete", "dressup"),
        );
        register(
            context,
            action(
                "design.project",
                "Project sketch onto faces",
                "sketch-map",
                "dressup",
            ),
        );
        register(
            context,
            action("design.text", "Text on faces", "sketch-text", "dressup"),
        );
        register(
            context,
            action("design.read_hole", "Read round off scan", "hole", "scan"),
        );
        register(
            context,
            action(
                "design.snap_to_scan",
                "Snap face to scan",
                "move-geometry",
                "scan",
            ),
        );
        register(
            context,
            action(
                "design.scan_symmetry",
                "Symmetry plane from scan",
                "datum-plane",
                "scan",
            ),
        );
        register(
            context,
            action(
                "design.read_thread",
                "Read thread off scan",
                "subtractive-helix",
                "scan",
            ),
        );
        register(
            context,
            action(
                "design.scan_deviation",
                "Deviation from scan",
                "measure",
                "scan",
            ),
        );
        register(
            context,
            action(
                "design.recognize_holes",
                "Recognize holes",
                "hole",
                "dressup",
            ),
        );
        // Boolean.
        register(
            context,
            action("design.boolean", "Boolean", "boolean", "boolean"),
        );
        // Measure.
        register(
            context,
            action("design.centre_line", "Centre line", centre::ICON, "measure"),
        );
    }

    pub(crate) fn tool_enabled(&self, tool_id: &str, ctx: &WorkbenchRuntimeContext) -> bool {
        let body = Self::target_body(ctx);
        let has_body = body.is_some();
        let has_sketch = Self::selected_sketch(ctx).is_some();
        let has_solid = body.map(|b| Self::body_has_solid(ctx, b)).unwrap_or(false);
        match base_tool_id(tool_id) {
            "design.new_body" => true,
            "design.scan_deviation"
            | "design.read_hole"
            | "design.read_thread"
            | "design.snap_to_scan"
            | "design.scan_symmetry" => ctx
                .document
                .bodies()
                .iter()
                .any(|b| ctx.document.body_scan(b.id).is_some()),
            "design.edit_sketch" => has_sketch,
            "design.map_sketch" => has_sketch && ctx.selected_face.is_some(),
            "design.new_sketch"
            | "design.primitive"
            | "design.datum_plane"
            | "design.datum_line"
            | "design.datum_point"
            | "design.coordinate_system"
            | "design.generator" => has_body,
            "design.clone" => has_body && !has_solid,
            "design.borrow" => has_body && ctx.document.bodies().len() > 1,
            "design.scaled" => has_solid,
            // A pad or a pocket also takes a flat face of the solid, or a
            // borrow lending one, as its profile.
            "design.pad" => {
                has_sketch
                    || (has_solid && body.is_some_and(|b| Self::profile_without_sketch(ctx, b)))
            }
            "design.pocket" => {
                has_solid
                    && (has_sketch || body.is_some_and(|b| Self::profile_without_sketch(ctx, b)))
            }
            "design.revolve" | "design.loft" | "design.pipe" | "design.helix" => has_sketch,
            "design.groove"
            | "design.hole"
            | "design.project"
            | "design.subtractive_loft"
            | "design.subtractive_pipe"
            | "design.subtractive_helix" => has_sketch && has_solid,
            "design.subtractive_primitive" | "design.text" => has_solid,
            "design.fillet"
            | "design.chamfer"
            | "design.draft"
            | "design.thickness"
            | "design.delete_faces"
            | "design.offset_faces"
            | "design.offset_solid"
            | "design.scale_body"
            | "design.move_faces"
            | "design.recognize_holes"
            | "design.mirror"
            | "design.linear_pattern"
            | "design.polar_pattern"
            | "design.circular_pattern"
            | "design.point_pattern"
            | "design.path_pattern"
            | "design.multi_transform"
            | "design.boolean"
            | "design.centre_line" => has_solid,
            _ => false,
        }
    }
}
