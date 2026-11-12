//! The Generators: an involute gear, a chain sprocket, a stepped shaft and
//! a shaft's keyway, each a sketch made from a few numbers
//! (`wb_sketch::generator`), ready for a Pad (the gear, the sprocket), a
//! Revolution (the shaft) or a Pocket (the keyway).
//!
//! The tool opens the sketcher's plane picker for the selected body, as a
//! new sketch does, the picked face offered first and the generator centred
//! where it was picked; the sketch is made on the plane chosen and opened,
//! where its panel sets the numbers. `design.gear`, `design.sprocket`,
//! `design.shaft` and `design.keyway` make one from a script, on a picked
//! face or a base plane (the shaft stands on XZ, so it turns about Z).

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, InputResult,
    ParamKind, ToolDescriptor, ToolVariant, WorkbenchContext, WorkbenchRuntimeContext,
};
use serde_json::{Map, Value, json};
use wb_sketch::generator::{Generator, new_sketch};
use wb_sketch::sketch::SketchPlane;

use crate::DesignWorkbench;

/// The generators, by variant: (variant, label, icon, command).
const GENERATORS: &[(&str, &str, &str, &str)] = &[
    ("gear", "Involute gear", "involute-gear", "design.gear"),
    ("sprocket", "Sprocket", "sprocket", "design.sprocket"),
    ("shaft", "Shaft", "revolution", "design.shaft"),
    ("keyway", "Keyway", "slot", "design.keyway"),
];

/// The toolbar's Generators dropdown.
pub(crate) fn tool() -> ToolDescriptor {
    ToolDescriptor::new_action("design.generator", "Generators", Some("generators"))
        .icon("involute-gear")
        .variants(
            GENERATORS
                .iter()
                .map(|(id, label, icon, _)| ToolVariant::new(id, label, icon))
                .collect(),
        )
}

/// Register `design.gear`, `design.sprocket`, `design.shaft` and
/// `design.keyway`.
pub(crate) fn register(context: &mut WorkbenchContext) {
    let fields = [
        (
            "design.gear",
            "Make an involute spur gear's profile, outer or internal (ring): a sketch to pad",
            "module, teeth, pressure_angle_deg, profile_shift, addendum and dedendum (in \
             modules), backlash, root_fillet (in modules), undercut (the root a rack cutter \
             leaves; false for a fillet arc), bore, keyway = {on, width, depth} (0 for the \
             standard key's by the bore), internal (true for a ring), rim (a ring's outside \
             diameter)",
        ),
        (
            "design.sprocket",
            "Make a roller chain sprocket's profile (ISO 606 teeth): a sketch to pad",
            "pitch, roller (the roller's diameter), teeth, bore, keyway = {on, width, depth} \
             (0 for the standard key's by the bore)",
        ),
        (
            "design.shaft",
            "Make a stepped shaft's half section: a sketch to revolve about its vertical axis",
            "sections = {{length, diameter, chamfer, fillet}, ...}, start_chamfer, and \
             loads = {bearings = {a, b}, forces = {{at, force, angle_deg}, ...}, torque (N·m), \
             torque_from, torque_to, modulus (GPa)} for its stresses and deflection",
        ),
        (
            "design.keyway",
            "Make the slot of a parallel key in a shaft (DIN 6885): a sketch to pocket",
            "shaft (its diameter, which sizes the standard key), length, width and depth (0 \
             for the standard key's), rounded (false for square ends)",
        ),
    ];
    for (id, summary, extra) in fields {
        let mut spec = CommandSpec::new(id, summary);
        if id == "design.keyway" {
            spec = spec.optional(
                "along",
                ParamKind::List,
                "With face_point: the way it runs on the face, {x, y, z}, such as the \
                 shaft's axis",
            );
        }
        context.register_command(
            prose().explain(
                spec.optional(
                    "body",
                    ParamKind::Id,
                    "The body it goes in; the selected one, else a new one",
                )
                .optional(
                    "plane",
                    ParamKind::String,
                    "The base plane it lies on: XY, XZ or YZ (a gear and a sprocket take XY, \
                     a shaft XZ)",
                )
                .optional(
                    "face_point",
                    ParamKind::List,
                    "Or a face it lies on, centred at this point of it, {x, y, z}, in the \
                     body's own frame",
                )
                .optional(
                    "face_normal",
                    ParamKind::List,
                    "With face_point: the face's outward normal, {x, y, z}",
                )
                .optional("name", ParamKind::String, "Its name in the tree")
                .extra_args(extra)
                .returns("the sketch's id"),
            ),
        );
    }
}

/// The notes, related commands and examples of the generator commands.
pub(crate) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "generators.md", include_str!("generators.md"), &[])
}

pub(crate) fn is_command(id: &str) -> bool {
    GENERATORS.iter().any(|(_, _, _, command)| *command == id)
}

/// The plane a generator lies on when no face is given.
fn base_plane(generator: &Generator) -> (SketchPlane, &'static str) {
    match generator {
        Generator::Shaft(_) => (SketchPlane::xz(), "XZ"),
        _ => (SketchPlane::xy(), "XY"),
    }
}

/// The plane of a face, centred on `point` of it.
fn face_plane(point: [f64; 3], normal: [f64; 3]) -> SketchPlane {
    let frame = SketchPlane::from_face(point, normal);
    SketchPlane::from_frame(point, frame.normal, frame.x_axis)
}

/// Add the sketch `generator` makes to `body` on `plane`; with no body,
/// to a new one, made only once the sketch is.
fn add(
    ctx: &mut WorkbenchRuntimeContext,
    body: Option<BodyId>,
    plane: SketchPlane,
    generator: Generator,
    name: Option<&str>,
) -> Result<FeatureId, String> {
    let name = match name {
        Some(name) => name.to_string(),
        None => DesignWorkbench::next_feature_name(ctx, generator.base_name()),
    };
    let feature = new_sketch(generator, plane, &name)?;
    let body = body.unwrap_or_else(|| ctx.document.create_body(None));
    ctx.document
        .add_feature_in_body(feature, name.clone(), Some(body))
        .map_err(|e| format!("Failed to create {name}: {e}"))
}

/// The toolbar's path: the sketcher's plane picker, as a new sketch's,
/// offering the selected face first; the generator `tool` names makes the
/// sketch on the plane chosen, and the sketcher opens it for its numbers.
pub(crate) fn insert(ctx: &mut WorkbenchRuntimeContext, tool: &str) -> InputResult {
    let variant = core_document::tool_variant(tool).unwrap_or("gear");
    let Some(generator) = Generator::named(variant) else {
        return InputResult::consumed();
    };
    DesignWorkbench::start_sketch(ctx, Some(generator.kind()))
}

/// Run `design.gear`, `design.sprocket` or `design.shaft`.
pub(crate) fn command(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let variant = GENERATORS
        .iter()
        .find(|(.., command)| *command == id)
        .map(|(variant, ..)| *variant)
        .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
    let mut generator = Generator::named(variant).expect("a generator per command");
    let own = [
        "body",
        "plane",
        "face_point",
        "face_normal",
        "name",
        "along",
    ];
    let fields: Map<String, Value> = args
        .iter()
        .filter(|(k, _)| !own.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    generator.merge(&fields).map_err(CommandError::failed)?;
    let body = a
        .opt_body(ctx.document, "body")?
        .or(ctx.selected_body_id.map(BodyId));
    let plane = if a.has("face_point") {
        let plane = face_plane(a.vec3("face_point")?, a.vec3("face_normal")?);
        match a.opt_vec3("along")? {
            Some(along) => {
                let along = glam::DVec3::from_array(along);
                let n = glam::DVec3::from_array(plane.normal);
                let x = (along - n * along.dot(n)).normalize_or_zero();
                if x == glam::DVec3::ZERO {
                    return Err(CommandError::bad("along", "must run across the face"));
                }
                SketchPlane::from_frame(plane.origin, plane.normal, x.to_array())
            }
            None => plane,
        }
    } else {
        match a.opt_string("plane")? {
            None => base_plane(&generator).0,
            Some(p) if p.eq_ignore_ascii_case("XY") => SketchPlane::xy(),
            Some(p) if p.eq_ignore_ascii_case("XZ") => SketchPlane::xz(),
            Some(p) if p.eq_ignore_ascii_case("YZ") => SketchPlane::yz(),
            Some(_) => return Err(CommandError::bad("plane", "must be XY, XZ or YZ")),
        }
    };
    let name = a.opt_string("name")?.map(str::to_string);
    let id = add(ctx, body, plane, generator, name.as_deref()).map_err(CommandError::failed)?;
    Ok(json!(id.0.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, WorkbenchFeature};
    use wb_sketch::SketchFeature;

    fn run(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        core_document::testing::call_checked(&mut crate::DesignWorkbench::default(), doc, id, args)
    }

    /// The tool asks the sketcher for its plane picker, the clicked face
    /// offered and followed, the generator carried along; nothing is made
    /// until a plane is chosen.
    #[test]
    fn the_tool_opens_the_plane_picker_for_its_generator() {
        let mut doc = Document::new("g");
        let body = doc.create_body(None);
        let face = core_document::FaceRef {
            point: [1.0, 2.0, 10.0],
            normal: [0.0, 0.0, 1.0],
            surface: None,
            name: 0,
        };
        let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        ctx.selected_body_id = Some(body.0);
        ctx.selected_face = Some(face);
        insert(&mut ctx, "design.generator:sprocket");
        let requests = ctx.take_requests();
        let [core_document::HostRequest::StartOn { workbench, attach }] = requests.as_slice()
        else {
            panic!("one request to start a sketch: {requests:?}")
        };
        assert_eq!(workbench.as_str(), wb_sketch::KIND);
        assert_eq!(attach.body, body.0);
        assert_eq!(attach.face, Some(face));
        assert_eq!(attach.face_origin, core_document::FaceOrigin::OwnSolid);
        assert_eq!(attach.generator, Some("sprocket"));
        assert_eq!(doc.feature_tree().all_nodes().count(), 0);
    }

    #[test]
    fn each_command_makes_its_sketch_with_the_numbers_given() {
        let mut doc = Document::new("g");
        let body = doc.create_body(None);
        let id = run(
            &mut doc,
            "design.gear",
            json!({"body": body.0.to_string(), "teeth": 31, "module": 1.5}),
        )
        .unwrap();
        let id = crate::test_support::made_id(&id);
        let feature = SketchFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        let Some(Generator::Gear(spec)) = &feature.generator else {
            panic!("a gear")
        };
        assert_eq!((spec.teeth, spec.module), (31, 1.5));
        assert!(!feature.sketch.geometry.is_empty());
        assert_eq!(doc.get_feature_meta(id).unwrap().name, "Gear");

        let shaft = run(
            &mut doc,
            "design.shaft",
            json!({"body": body.0.to_string()}),
        )
        .unwrap();
        let shaft = crate::test_support::made_id(&shaft);
        let feature = SketchFeature::from_json(doc.get_feature_data(shaft).unwrap()).unwrap();
        assert_eq!(feature.plane.normal, SketchPlane::xz().normal);
    }

    /// Numbers no profile comes from are refused with what to change, and
    /// leave no body or sketch behind: sizes that overflow, negative
    /// keyway sizes, tooth counts past the limit.
    #[test]
    fn refused_numbers_leave_no_body_behind() {
        let mut doc = Document::new("g");
        let two = |length: f64| json!([{"length": length}, {"length": length, "diameter": 10}]);
        for (id, args, why) in [
            ("design.gear", json!({"teeth": 3, "bore": 20}), "bore"),
            ("design.gear", json!({"module": 1e308}), "too large"),
            ("design.gear", json!({"teeth": 501}), "at most 500"),
            ("design.gear", json!({"teeth": 1e12}), "at most 500"),
            ("design.sprocket", json!({"pitch": 1e308}), "too large"),
            ("design.sprocket", json!({"teeth": 501}), "at most 500"),
            ("design.shaft", json!({"sections": two(1e308)}), "too large"),
            ("design.keyway", json!({"width": -1}), "not negative"),
            ("design.keyway", json!({"depth": -2}), "not negative"),
        ] {
            let err = match run(&mut doc, id, args.clone()) {
                Err(e) => e.to_string(),
                Ok(_) => panic!("{id} {args} accepted"),
            };
            assert!(err.contains(why), "{id} {args}: {err}");
            assert!(doc.bodies().is_empty(), "{id} {args} made a body");
            assert_eq!(doc.feature_tree().all_nodes().count(), 0);
        }
        run(&mut doc, "design.gear", json!({"teeth": 12})).unwrap();
        assert_eq!(doc.bodies().len(), 1, "a gear given no body makes its own");
    }

    #[test]
    fn a_field_the_generator_lacks_is_refused() {
        let mut doc = Document::new("g");
        let err = run(&mut doc, "design.sprocket", json!({"module": 2})).unwrap_err();
        assert!(err.to_string().contains("module"), "{err}");
    }
}
