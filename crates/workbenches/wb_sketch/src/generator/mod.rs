//! Generated profiles: a sketch whose curves are made from a few numbers
//! (an involute gear, a chain sprocket, a stepped shaft, a shaft's
//! keyway) rather than drawn.
//!
//! The numbers live on the sketch ([`crate::SketchFeature::generator`]);
//! the curves are made again from them whenever they change, by hand, by a
//! formula (every number is a parameter formulas can set) or by a command.
//! Everything that takes a sketch takes a generated one: a Pad pads the
//! gear, a Revolution turns the shaft. The curves carry ids derived from
//! the sketch's own, so making them again from the same numbers gives the
//! same sketch.

mod fit;
mod gear;
pub mod keyway;
mod loads;
#[cfg(feature = "egui")]
pub(crate) mod panel;
mod shaft;
mod sprocket;

use core_document::expr::Dim;
use core_document::{FeatureNode, Parameter, WorkbenchFeature};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::sketch::{Arc, BSpline, Circle, GeometryElement, Line, Point, Sketch, Vec2D};

pub use gear::GearSpec;
pub use keyway::{BoreKeyway, KeySize, KeywaySpec};
pub use loads::{ShaftAnalysis, ShaftForce, ShaftLoads, analyse};
pub use shaft::{ShaftSection, ShaftSpec};
pub use sprocket::{SPROCKET_CHAINS, SprocketSpec};

/// What a generated sketch is made from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Generator {
    Gear(GearSpec),
    Sprocket(SprocketSpec),
    Shaft(ShaftSpec),
    Keyway(KeywaySpec),
}

impl Generator {
    /// The generator a command or tool names: `gear`, `sprocket` or
    /// `shaft`, with its defaults.
    pub fn named(kind: &str) -> Option<Self> {
        Some(match kind {
            "gear" => Generator::Gear(GearSpec::default()),
            "sprocket" => Generator::Sprocket(SprocketSpec::default()),
            "shaft" => Generator::Shaft(ShaftSpec::default()),
            "keyway" => Generator::Keyway(KeywaySpec::default()),
            _ => return None,
        })
    }

    /// `gear`, `sprocket`, `shaft` or `keyway`.
    pub fn kind(&self) -> &'static str {
        match self {
            Generator::Gear(_) => "gear",
            Generator::Sprocket(_) => "sprocket",
            Generator::Shaft(_) => "shaft",
            Generator::Keyway(_) => "keyway",
        }
    }

    /// What the tree and the panel call it.
    pub fn label(&self) -> &'static str {
        match self {
            Generator::Gear(_) => "Involute gear",
            Generator::Sprocket(_) => "Sprocket",
            Generator::Shaft(_) => "Shaft",
            Generator::Keyway(_) => "Keyway",
        }
    }

    /// Its icon in the set.
    pub fn icon(&self) -> &'static str {
        match self {
            Generator::Gear(_) => "involute-gear",
            Generator::Sprocket(_) => "sprocket",
            Generator::Shaft(_) => "revolution",
            Generator::Keyway(_) => "slot",
        }
    }

    /// The name a new one takes in the tree.
    pub fn base_name(&self) -> &'static str {
        match self {
            Generator::Gear(_) => "Gear",
            Generator::Sprocket(_) => "Sprocket",
            Generator::Shaft(_) => "Shaft",
            Generator::Keyway(_) => "Keyway",
        }
    }

    /// The way its sketch's x runs on `face`, when it has one of its own:
    /// a keyway along the axis of the round face it is cut into.
    pub fn runs_along(&self, face: &core_document::FaceRef) -> Option<[f64; 3]> {
        let Generator::Keyway(_) = self else {
            return None;
        };
        let (_, axis) = face.surface?.axis()?;
        let (a, n) = (
            glam::DVec3::from_array(crate::boundary::widen3(axis)),
            glam::DVec3::from_array(crate::boundary::widen3(face.normal)).normalize_or_zero(),
        );
        let along = (a - n * a.dot(n)).normalize_or_zero();
        (along != glam::DVec3::ZERO).then(|| along.to_array())
    }

    /// The curves its numbers make, or why they make none.
    pub fn outline(&self) -> Result<Outline, String> {
        match self {
            Generator::Gear(spec) => spec.outline(),
            Generator::Sprocket(spec) => spec.outline(),
            Generator::Shaft(spec) => spec.outline(),
            Generator::Keyway(spec) => spec.outline(),
        }
    }

    /// Its numbers as parameters formulas can set, with where each is in
    /// the sketch feature's JSON.
    pub fn parameters(&self) -> Vec<Parameter> {
        let at = |variant: &str, field: &str| format!("/generator/{variant}/{field}");
        let length = |name: &str, label: &str, variant: &str, field: &str| {
            Parameter::new(name, label, Dim::LENGTH, at(variant, field))
        };
        let number = |name: &str, label: &str, variant: &str, field: &str| {
            Parameter::new(name, label, Dim::NUMBER, at(variant, field))
        };
        match self {
            Generator::Gear(_) => vec![
                length("module", "Module", "Gear", "module"),
                Parameter::count("teeth", "Teeth", at("Gear", "teeth")),
                Parameter::new(
                    "pressure_angle",
                    "Pressure angle",
                    Dim::ANGLE,
                    at("Gear", "pressure_angle_deg"),
                ),
                number("profile_shift", "Profile shift", "Gear", "profile_shift"),
                number("addendum", "Addendum", "Gear", "addendum"),
                number("dedendum", "Dedendum", "Gear", "dedendum"),
                length("backlash", "Backlash", "Gear", "backlash"),
                number("root_fillet", "Root fillet", "Gear", "root_fillet"),
                length("bore", "Bore", "Gear", "bore"),
                length("keyway_width", "Keyway width", "Gear", "keyway/width"),
                length("keyway_depth", "Keyway depth", "Gear", "keyway/depth"),
                length("rim", "Ring rim", "Gear", "rim"),
            ],
            Generator::Sprocket(_) => vec![
                length("pitch", "Chain pitch", "Sprocket", "pitch"),
                length("roller", "Roller diameter", "Sprocket", "roller"),
                Parameter::count("teeth", "Teeth", at("Sprocket", "teeth")),
                length("bore", "Bore", "Sprocket", "bore"),
                length("keyway_width", "Keyway width", "Sprocket", "keyway/width"),
                length("keyway_depth", "Keyway depth", "Sprocket", "keyway/depth"),
            ],
            Generator::Keyway(_) => vec![
                length("shaft", "Shaft diameter", "Keyway", "shaft"),
                length("length", "Length", "Keyway", "length"),
                length("width", "Width", "Keyway", "width"),
                length("depth", "Depth", "Keyway", "depth"),
            ],
            Generator::Shaft(spec) => {
                let mut out = vec![length(
                    "start_chamfer",
                    "Start chamfer",
                    "Shaft",
                    "start_chamfer",
                )];
                for i in 0..spec.sections.len() {
                    let n = i + 1;
                    for (field, label) in [
                        ("length", "Length"),
                        ("diameter", "Diameter"),
                        ("chamfer", "Chamfer"),
                        ("fillet", "Fillet"),
                    ] {
                        out.push(Parameter::new(
                            &format!("{field}_{n}"),
                            &format!("{label} {n}"),
                            Dim::LENGTH,
                            format!("/generator/Shaft/sections/{i}/{field}"),
                        ));
                    }
                }
                out
            }
        }
    }

    /// Change the fields `fields` names (`module = 2`, `sections = {...}`),
    /// leaving the rest; a field it does not have is refused by name.
    pub fn merge(
        &mut self,
        fields: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let label = self.label();
        let mut value = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let inner = value
            .as_object_mut()
            .and_then(|o| o.values_mut().next())
            .and_then(|v| v.as_object_mut())
            .ok_or("the generator has no fields")?;
        for (key, given) in fields {
            if !inner.contains_key(key) {
                let known: Vec<&String> = inner.keys().collect();
                return Err(format!(
                    "{label} has no field {key}; it has {}",
                    known
                        .iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            inner.insert(key.clone(), given.clone());
        }
        *self = serde_json::from_value(value).map_err(|e| format!("{label}: {e}"))?;
        Ok(())
    }
}

/// Make the sketch's curves again from its generator. Everything else it
/// held goes: a generated sketch is its numbers. When they make no
/// profile the sketch is left empty, so what is built from it says so,
/// and the reason comes back.
pub fn regenerate(feature: &mut SketchFeature) -> Result<(), String> {
    let Some(generator) = &feature.generator else {
        return Ok(());
    };
    let outline = generator.outline();
    let sketch = &mut feature.sketch;
    sketch.geometry.clear();
    sketch.constraints.clear();
    sketch.construction.clear();
    sketch.external.clear();
    let outline = outline?;
    outline.emit(sketch);
    // Nothing is left for the solver to move: the numbers fix every curve.
    sketch.is_fully_constrained = true;
    Ok(())
}

/// The generated sketch's numbers as formulas see them.
pub fn parameters(feature: &SketchFeature) -> Vec<Parameter> {
    feature
        .generator
        .as_ref()
        .map(Generator::parameters)
        .unwrap_or_default()
}

impl crate::SketchWorkbench {
    /// Whether the sketch open for editing is a generated one. Read off
    /// the JSON rather than the whole sketch: the toolbar asks every frame.
    pub(crate) fn editing_generated(&self, ctx: &core_document::WorkbenchRuntimeContext) -> bool {
        self.active_sketch_id
            .and_then(|id| ctx.document.get_feature_data(id))
            .and_then(|data| data.get("generator"))
            .is_some_and(|g| !g.is_null())
    }
}

/// Whether the node is a generated sketch, and which.
pub fn generator_of(node: &FeatureNode) -> Option<Generator> {
    SketchFeature::from_json(&node.data).ok()?.generator
}

/// A new sketch named `name` on `plane`, made by `generator`.
pub fn new_sketch(
    generator: Generator,
    plane: crate::sketch::SketchPlane,
    name: &str,
) -> Result<SketchFeature, String> {
    let mut sketch = Sketch::new(name);
    sketch.plane = plane;
    let mut feature = SketchFeature::new(sketch, plane);
    feature.generator = Some(generator);
    regenerate(&mut feature)?;
    Ok(feature)
}

/// Register the command that changes a generated sketch's numbers.
pub fn register(context: &mut core_document::WorkbenchContext) {
    use core_document::{CommandSpec, ParamKind};
    context.register_command(
        CommandSpec::new(
            "sketch.generator",
            "Change the numbers a generated sketch (a gear, a sprocket, a shaft, a keyway) is \
             made from, or detach it into a plain sketch",
        )
        .param("sketch", ParamKind::Id, "The generated sketch")
        .optional(
            "detach",
            ParamKind::Bool,
            "Keep the curves as they are and forget the numbers: a plain sketch to edit by hand",
        )
        .extra_args(
            "The numbers to change, such as teeth = 24 or module = 1.5; a shaft takes \
             sections = {{length = 20, diameter = 10, chamfer = 0.5, fillet = 0}, ...}",
        )
        .returns("what the numbers come to: its diameters, or its length")
        .note(
            "Called with only `sketch` it changes nothing and answers the sizes: a gear's \
             base, pitch, root and tip diameters, a sprocket's pitch, root and tip, a \
             shaft's length.",
        )
        .note(
            "A field the generator lacks is refused, naming the ones it has. The sketch's \
             curves are made again from the numbers, so anything drawn in it by hand goes.",
        )
        .note(
            "`detach = true` keeps the curves and drops the numbers for good. Each number \
             is also a parameter by its name (`module`, `teeth`), which `doc.set_formula` \
             binds to a formula.",
        )
        .see_also("design.gear")
        .see_also("design.sprocket")
        .see_also("design.shaft")
        .see_also("doc.set_formula")
        .example(
            "A gear's teeth changed, and the tip diameter with them",
            r#"
            local gear = pc.design.gear{module = 1.5, teeth = 20}
            assert(pc.sketch.generator{sketch = gear}.tip_diameter == 1.5 * 22)
            local size = pc.sketch.generator{sketch = gear, teeth = 30}
            assert(size.pitch_diameter == 1.5 * 30 and size.tip_diameter == 1.5 * 32)
            pc.sketch.generator{sketch = gear, detach = true}
            assert(pc.doc.feature{id = gear}.fields.generator == nil, "a plain sketch")
            "#,
        ),
    );
}

/// Run `sketch.generator`.
pub fn command(
    args: &core_document::CommandArgs,
    ctx: &mut core_document::WorkbenchRuntimeContext,
) -> core_document::CommandResult {
    use core_document::{Args, CommandError, FeatureId};
    let a = Args(args);
    let id = FeatureId(a.id("sketch")?);
    let data = ctx
        .document
        .get_feature_data(id)
        .ok_or_else(|| CommandError::bad("sketch", "is not a feature of this document"))?;
    let mut feature = SketchFeature::from_json(data)
        .map_err(|_| CommandError::bad("sketch", "is not a sketch"))?;
    let Some(generator) = feature.generator.as_mut() else {
        return Err(CommandError::bad("sketch", "is not a generated sketch"));
    };
    let fields: serde_json::Map<String, serde_json::Value> = args
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "sketch" | "detach"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // The answer is what builds: the numbers formulas set, with these.
    let mut built = crate::stored_sketch(ctx.document, id)
        .and_then(|f| f.generator)
        .unwrap_or_else(|| generator.clone());
    generator.merge(&fields).map_err(CommandError::failed)?;
    built.merge(&fields).map_err(CommandError::failed)?;
    let answer = summary(&built);
    if a.opt_bool("detach")?.unwrap_or(false) {
        feature.generator = None;
    } else {
        regenerate(&mut feature).map_err(CommandError::failed)?;
    }
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    Ok(answer)
}

/// What a generator's numbers come to, for the panel and the command's
/// answer: a gear's and a sprocket's circles, a shaft's length.
pub fn summary(generator: &Generator) -> serde_json::Value {
    use serde_json::json;
    match generator {
        Generator::Gear(spec) => match spec
            .geometry()
            .and_then(|g| spec.tip_and_root_diameters().map(|d| (g, d)))
        {
            Ok((g, (tip, root))) => {
                let mut out = json!({
                    "pitch_diameter": 2.0 * g.pitch_radius,
                    "base_diameter": 2.0 * g.base_radius,
                    "tip_diameter": tip,
                    "root_diameter": root,
                });
                if spec.undercut && !spec.internal {
                    out["undercut"] = json!(spec.is_undercut());
                }
                if spec.internal {
                    out
                } else {
                    with_keyway(out, &spec.keyway, spec.bore)
                }
            }
            Err(why) => json!({ "error": why }),
        },
        Generator::Sprocket(spec) => match spec.geometry() {
            Ok(g) => with_keyway(
                json!({
                    "pitch_diameter": g.pitch_diameter,
                    "tip_diameter": g.tip_diameter,
                    "root_diameter": g.root_diameter,
                }),
                &spec.keyway,
                spec.bore,
            ),
            Err(why) => json!({ "error": why }),
        },
        Generator::Keyway(spec) => match spec.size() {
            Ok((width, depth)) => json!({ "width": width, "depth": depth }),
            Err(why) => json!({ "error": why }),
        },
        Generator::Shaft(spec) => {
            let mut out = json!({
                "length": spec.sections.iter().map(|s| s.length).sum::<f64>(),
            });
            if spec.loads.any() {
                out["loads"] = match analyse(spec, &spec.loads) {
                    Ok(a) => {
                        let (moment, moment_at) = a.max_moment();
                        let (stress, stress_at) = a.max_stress();
                        let (sag, sag_at) = a.max_deflection();
                        json!({
                            "reactions_n": a.reactions,
                            "max_moment_nm": moment / 1e3,
                            "max_moment_at": moment_at,
                            "max_stress_mpa": stress,
                            "max_stress_at": stress_at,
                            "max_deflection_mm": sag,
                            "max_deflection_at": sag_at,
                        })
                    }
                    Err(why) => json!({ "error": why }),
                };
            }
            out
        }
    }
}

/// `out` with a bore keyway's width and depth, when it has one.
fn with_keyway(mut out: serde_json::Value, keyway: &BoreKeyway, bore: f64) -> serde_json::Value {
    if keyway.on
        && let Ok((width, depth)) = keyway.size(bore)
    {
        out["keyway_width"] = serde_json::json!(width);
        out["keyway_depth"] = serde_json::json!(depth);
    }
    out
}

/// A count as JSON holds it: a formula may leave `12.0`, a hand edit
/// `12`, a stray negative is none at all.
pub(crate) fn count<'de, D: Deserializer<'de>>(de: D) -> Result<u32, D::Error> {
    let value = f64::deserialize(de)?;
    Ok(if value.is_finite() && value > 0.0 {
        value.round().min(f64::from(u32::MAX)) as u32
    } else {
        0
    })
}

/// A 2D point, in millimetres, double precision while it is worked out.
pub type P2 = [f64; 2];

pub(crate) fn polar(r: f64, angle: f64) -> P2 {
    [r * angle.cos(), r * angle.sin()]
}

pub(crate) fn rotate(p: P2, angle: f64) -> P2 {
    let (s, c) = angle.sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}

pub(crate) fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

pub(crate) fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

pub(crate) fn scale(a: P2, k: f64) -> P2 {
    [a[0] * k, a[1] * k]
}

pub(crate) fn norm(a: P2) -> f64 {
    a[0].hypot(a[1])
}

pub(crate) fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// One piece of a loop, from its joint to the next one.
#[derive(Debug, Clone, PartialEq)]
pub enum Edge {
    Line,
    /// A circular arc about `center`, the short way round.
    Arc {
        center: P2,
    },
    /// A clamped cubic B-spline; these are its control points between the
    /// two joints, which are its first and last.
    Spline {
        inner: Vec<P2>,
    },
}

/// A closed loop: `joints[i]` to `joints[i + 1]` (wrapping) along
/// `edges[i]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loop {
    pub joints: Vec<P2>,
    pub edges: Vec<Edge>,
}

impl Loop {
    /// Continue the loop from its last joint along `edge` to `to`; the
    /// loop closes from its last joint back to its first with
    /// [`Loop::close`].
    pub fn push(&mut self, edge: Edge, to: P2) {
        self.edges.push(edge);
        self.joints.push(to);
    }

    pub fn start(at: P2) -> Self {
        Self {
            joints: vec![at],
            edges: Vec::new(),
        }
    }

    /// Close the loop along `edge`: its last point is where it began, so
    /// that point goes and the edge returns to the first.
    pub fn close(&mut self, edge: Edge) {
        self.joints.pop();
        self.edges.push(edge);
    }

    /// The loop as a closed polyline, every curve sampled `per_curve`
    /// times: for measuring what it encloses.
    pub fn polyline(&self, per_curve: usize) -> Vec<P2> {
        let n = self.joints.len();
        let mut out = Vec::new();
        for (i, edge) in self.edges.iter().enumerate() {
            let a = self.joints[i];
            let b = self.joints[(i + 1) % n];
            match edge {
                Edge::Line => out.push(a),
                Edge::Arc { center } => {
                    let (a0, sweep) = arc_span(*center, a, b);
                    let r = norm(sub(a, *center));
                    for k in 0..per_curve {
                        let t = a0 + sweep * k as f64 / per_curve as f64;
                        out.push(add(*center, polar(r, t)));
                    }
                }
                Edge::Spline { inner } => {
                    let mut poles = vec![a];
                    poles.extend(inner.iter().copied());
                    poles.push(b);
                    for k in 0..per_curve {
                        out.push(fit::evaluate(&poles, k as f64 / per_curve as f64));
                    }
                }
            }
        }
        out
    }
}

/// The start angle and signed sweep of the short arc about `center` from
/// `a` to `b`.
pub(crate) fn arc_span(center: P2, a: P2, b: P2) -> (f64, f64) {
    let (va, vb) = (sub(a, center), sub(b, center));
    let a0 = va[1].atan2(va[0]);
    let sweep = cross(va, vb).atan2(va[0] * vb[0] + va[1] * vb[1]);
    (a0, sweep)
}

/// The area a closed polyline encloses (positive counter-clockwise).
pub fn shoelace(points: &[P2]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| cross(points[i], points[(i + 1) % n]))
        .sum::<f64>()
        / 2.0
}

/// What a generator makes: closed loops, loops standing as holes in them
/// (a bore with its keyway), whole circles (a bore), and construction
/// circles drawn as guides (a pitch circle).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outline {
    pub loops: Vec<Loop>,
    pub holes: Vec<Loop>,
    pub circles: Vec<(P2, f64)>,
    pub guides: Vec<(P2, f64)>,
}

/// Ids for generated curves, the same each time from the same sketch:
/// the n-th element's id follows from the sketch's.
struct Ids {
    base: u128,
    next: u128,
}

impl Ids {
    fn new(sketch: &Sketch) -> Self {
        Self {
            base: sketch.id.as_u128(),
            next: 0,
        }
    }

    fn take(&mut self) -> Uuid {
        self.next += 1;
        // An odd multiplier walks every value before it repeats.
        Uuid::from_u128(
            self.base.wrapping_add(
                self.next
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835),
            ),
        )
    }
}

fn v(p: P2) -> Vec2D {
    Vec2D::new(p[0], p[1])
}

impl Outline {
    /// Add the outline to `sketch` as points and curves, the joints of a
    /// loop shared by the curves that meet there, so the loop is closed
    /// the way the profile builder reads one.
    pub fn emit(&self, sketch: &mut Sketch) {
        let mut ids = Ids::new(sketch);
        let point = |sketch: &mut Sketch, ids: &mut Ids, p: P2| {
            let id = ids.take();
            sketch.add_geometry(GeometryElement::Point(Point { id, position: v(p) }));
            id
        };
        for lp in self.loops.iter().chain(&self.holes) {
            let joints: Vec<Uuid> = lp
                .joints
                .iter()
                .map(|p| point(sketch, &mut ids, *p))
                .collect();
            let n = joints.len();
            for (i, edge) in lp.edges.iter().enumerate() {
                let (a, b) = (joints[i], joints[(i + 1) % n]);
                let (pa, pb) = (lp.joints[i], lp.joints[(i + 1) % n]);
                let element = match edge {
                    Edge::Line => {
                        let mut line = Line::new(a, b);
                        line.id = ids.take();
                        GeometryElement::Line(line)
                    }
                    Edge::Arc { center } => {
                        let c = point(sketch, &mut ids, *center);
                        // A sketch arc runs counter-clockwise from start
                        // to end.
                        let (_, sweep) = arc_span(*center, pa, pb);
                        let (s, e) = if sweep >= 0.0 { (a, b) } else { (b, a) };
                        let radius = norm(sub(pa, *center)) as f64;
                        let mut arc = Arc::new(c, s, e, radius);
                        arc.id = ids.take();
                        GeometryElement::Arc(arc)
                    }
                    Edge::Spline { inner } => {
                        let mut poles = vec![a];
                        for p in inner {
                            poles.push(point(sketch, &mut ids, *p));
                        }
                        poles.push(b);
                        let mut spline = BSpline::new(poles, false);
                        spline.id = ids.take();
                        GeometryElement::BSpline(spline)
                    }
                };
                sketch.add_geometry(element);
            }
        }
        for (circles, guide) in [(&self.circles, false), (&self.guides, true)] {
            for (center, radius) in circles {
                let c = point(sketch, &mut ids, *center);
                let mut circle = Circle::new(c, *radius);
                circle.id = ids.take();
                let id = sketch.add_geometry(GeometryElement::Circle(circle));
                if guide {
                    sketch.set_construction(id, true);
                }
            }
        }
    }

    /// The area the outline encloses: its loops less its holes and
    /// circles (the bores inside them), or a ring's rim less its loop.
    pub fn area(&self) -> f64 {
        let loops: f64 = self
            .loops
            .iter()
            .map(|l| shoelace(&l.polyline(64)).abs())
            .sum();
        let holes: f64 = self
            .circles
            .iter()
            .map(|(_, r)| std::f64::consts::PI * r * r)
            .chain(self.holes.iter().map(|l| shoelace(&l.polyline(64)).abs()))
            .sum();
        (loops - holes).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regenerated_sketch_is_the_same_sketch() {
        let mut feature = SketchFeature::from_sketch(Sketch::new("g"));
        feature.generator = Generator::named("gear");
        regenerate(&mut feature).unwrap();
        let first = serde_json::to_value(&feature.sketch).unwrap();
        regenerate(&mut feature).unwrap();
        assert_eq!(serde_json::to_value(&feature.sketch).unwrap(), first);
    }

    /// Loads merged into a shaft come back in its summary as what they do.
    #[test]
    fn a_loaded_shaft_reports_its_stress() {
        let mut shaft = Generator::named("shaft").unwrap();
        assert!(summary(&shaft).get("loads").is_none(), "nothing loads it");
        let fields = serde_json::json!({"loads": {
            "bearings": [0.0, 65.0],
            "forces": [{"at": 30.0, "force": 500.0}],
        }});
        shaft.merge(fields.as_object().unwrap()).unwrap();
        let out = summary(&shaft);
        let loads = &out["loads"];
        assert!(loads["max_stress_mpa"].as_f64().unwrap() > 0.0, "{out}");
        let reactions: Vec<f64> = serde_json::from_value(loads["reactions_n"].clone()).unwrap();
        assert!((reactions.iter().sum::<f64>() - 500.0).abs() < 1e-6);
    }

    /// A sprocket's bore takes the standard keyway: a 12 mm bore a 4 mm
    /// key, its keyway 1.8 mm past the bore.
    #[test]
    fn a_sprocket_takes_a_keyway_in_its_bore() {
        let mut generator = Generator::named("sprocket").unwrap();
        let fields = serde_json::json!({"bore": 12.0, "keyway": {"on": true}});
        generator.merge(fields.as_object().unwrap()).unwrap();
        let out = summary(&generator);
        assert_eq!(out["keyway_width"], 4.0, "{out}");
        assert!((out["keyway_depth"].as_f64().unwrap() - 1.8).abs() < 1e-9);
        let outline = generator.outline().unwrap();
        assert!(outline.circles.is_empty() && outline.holes.len() == 1);
    }

    #[test]
    fn every_generator_makes_one_closed_profile() {
        for kind in ["gear", "sprocket", "shaft", "keyway"] {
            let mut feature = SketchFeature::from_sketch(Sketch::new(kind));
            feature.generator = Generator::named(kind);
            regenerate(&mut feature).unwrap();
            let wires = crate::profile::extract_wires(&feature.sketch).unwrap();
            let expect = if matches!(kind, "shaft" | "keyway") {
                1
            } else {
                2
            };
            assert_eq!(wires.len(), expect, "{kind}: its outline and bore");
        }
    }

    #[test]
    fn numbers_it_cannot_make_leave_the_sketch_empty_and_say_why() {
        let mut feature = SketchFeature::from_sketch(Sketch::new("g"));
        feature.generator = Some(Generator::Gear(GearSpec {
            teeth: 2,
            ..GearSpec::default()
        }));
        let why = regenerate(&mut feature).unwrap_err();
        assert!(why.contains("teeth"), "{why}");
        assert!(feature.sketch.geometry.is_empty());
    }

    #[test]
    fn a_field_merges_by_name_and_an_unknown_one_is_refused() {
        let mut generator = Generator::named("gear").unwrap();
        let mut fields = serde_json::Map::new();
        fields.insert("teeth".into(), serde_json::json!(31));
        generator.merge(&fields).unwrap();
        let Generator::Gear(spec) = &generator else {
            unreachable!()
        };
        assert_eq!(spec.teeth, 31);
        fields.insert("colour".into(), serde_json::json!(1));
        assert!(generator.merge(&fields).unwrap_err().contains("colour"));
    }

    #[test]
    fn every_parameter_points_at_a_number() {
        for kind in ["gear", "sprocket", "shaft", "keyway"] {
            let mut feature = SketchFeature::from_sketch(Sketch::new(kind));
            feature.generator = Generator::named(kind);
            let json = feature.to_json();
            for p in parameters(&feature) {
                assert!(
                    json.pointer(&p.pointer).is_some_and(|v| v.is_number()),
                    "{kind}: {}",
                    p.pointer
                );
            }
        }
    }
}
