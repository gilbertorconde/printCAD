//! Compile-time embedded toolbar icon SVGs.
//!
//! The icon sources live next to the workbench that owns them
//! (`crates/workbenches/<crate>/src/icons/<tool_id>.svg`) but are embedded
//! into the binary at build time, so icon loading never depends on the
//! process working directory. A missing file is a compile error, which
//! doubles as a completeness check for the icon set.

macro_rules! sketch_icon {
    ($tool:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../workbenches/wb_sketch/src/icons/",
            $tool,
            ".svg"
        ))
    };
}

macro_rules! part_icon {
    ($tool:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../workbenches/wb_part/src/icons/",
            $tool,
            ".svg"
        ))
    };
}

/// Embedded SVG source for a toolbar icon, keyed by workbench + tool id.
pub fn embedded_tool_svg(workbench_id: &str, tool_id: &str) -> Option<&'static str> {
    match workbench_id {
        "wb.sketch" => sketch_tool_svg(tool_id),
        "wb.part" => part_tool_svg(tool_id),
        _ => None,
    }
}

fn sketch_tool_svg(tool_id: &str) -> Option<&'static str> {
    Some(match tool_id {
        "sketch.create" => sketch_icon!("sketch.create"),
        "sketch.select" => sketch_icon!("sketch.select"),
        "sketch.point" => sketch_icon!("sketch.point"),
        "sketch.line" => sketch_icon!("sketch.line"),
        "sketch.arc" => sketch_icon!("sketch.arc"),
        "sketch.arc3" => sketch_icon!("sketch.arc3"),
        "sketch.circle" => sketch_icon!("sketch.circle"),
        "sketch.circle3" => sketch_icon!("sketch.circle3"),
        "sketch.ellipse" => sketch_icon!("sketch.ellipse"),
        "sketch.bspline" => sketch_icon!("sketch.bspline"),
        "sketch.rect" => sketch_icon!("sketch.rect"),
        "sketch.rect_center" => sketch_icon!("sketch.rect_center"),
        "sketch.polygon" => sketch_icon!("sketch.polygon"),
        "sketch.slot" => sketch_icon!("sketch.slot"),
        "sketch.arc_slot" => sketch_icon!("sketch.arc_slot"),
        "sketch.fillet" => sketch_icon!("sketch.fillet"),
        "sketch.chamfer" => sketch_icon!("sketch.chamfer"),
        "sketch.trim" => sketch_icon!("sketch.trim"),
        "sketch.extend" => sketch_icon!("sketch.extend"),
        "sketch.split" => sketch_icon!("sketch.split"),
        "sketch.offset" => sketch_icon!("sketch.offset"),
        "sketch.translate" => sketch_icon!("sketch.translate"),
        "sketch.rotate" => sketch_icon!("sketch.rotate"),
        "sketch.scale" => sketch_icon!("sketch.scale"),
        "sketch.mirror" => sketch_icon!("sketch.mirror"),
        "sketch.construction" => sketch_icon!("sketch.construction"),
        "sketch.finish" => sketch_icon!("sketch.finish"),
        _ => return None,
    })
}

fn part_tool_svg(tool_id: &str) -> Option<&'static str> {
    Some(match tool_id {
        "part.new_body" => part_icon!("part.new_body"),
        "part.new_sketch" => part_icon!("part.new_sketch"),
        "part.datum_plane" => part_icon!("part.datum_plane"),
        "part.datum_line" => part_icon!("part.datum_line"),
        "part.datum_point" => part_icon!("part.datum_point"),
        "part.pad" => part_icon!("part.pad"),
        "part.pocket" => part_icon!("part.pocket"),
        "part.revolve" => part_icon!("part.revolve"),
        "part.groove" => part_icon!("part.groove"),
        "part.loft" => part_icon!("part.loft"),
        "part.pipe" => part_icon!("part.pipe"),
        "part.helix" => part_icon!("part.helix"),
        "part.primitive" => part_icon!("part.primitive"),
        "part.hole" => part_icon!("part.hole"),
        "part.fillet" => part_icon!("part.fillet"),
        "part.chamfer" => part_icon!("part.chamfer"),
        "part.draft" => part_icon!("part.draft"),
        "part.thickness" => part_icon!("part.thickness"),
        "part.mirror" => part_icon!("part.mirror"),
        "part.linear_pattern" => part_icon!("part.linear_pattern"),
        "part.polar_pattern" => part_icon!("part.polar_pattern"),
        "part.multi_transform" => part_icon!("part.multi_transform"),
        "part.boolean" => part_icon!("part.boolean"),
        _ => return None,
    })
}
