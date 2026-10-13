//! Drag handles on the feature under edit: a dot at the end of each
//! number the view can show (a pad's or pocket's lengths, a hole's depth,
//! a revolution's or groove's angles round its axis, a draft's angle, a
//! primitive's sizes, a fillet's or chamfer's size), dragged along its
//! line or round its arc in steps, and where a pad, pocket or hole starts
//! along its way or a revolution or groove starts round its axis. Only
//! numbers the feature lists as
//! parameters have one, and none a formula sets. A pad's, pocket's,
//! hole's, revolution's or groove's turns the feature the other way when
//! clicked, or dragged through its start.

use core_document::{
    Document, FeatureId, FeatureNode, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use glam::Vec3;
use serde_json::Value;

use crate::feature::{DesignFeature, EdgeSel, ExtrudeDirection, ExtrudeMode, RevolveMode};

/// How close to a handle, in logical pixels, a press takes hold of it.
const REACH_PX: f32 = 10.0;
/// The smallest number a drag sets.
const LEAST: f32 = 0.01;
/// Line pieces an arc handle's path is drawn with, per full turn.
const ARC_SEGMENTS: f32 = 48.0;
/// An arc handle on a face (a draft's) is this share of the distance
/// from the eye in radius, so it keeps its size on screen.
const ARC_SHARE: f32 = 0.12;

/// Where a handle's number stands in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Shape {
    /// The number `value` stands at `origin + dir * value * scale`; `dir`
    /// of unit length.
    Line { origin: Vec3, dir: Vec3, scale: f32 },
    /// The number, degrees, stands on the circle of `radius` round
    /// `centre`, turned `base_deg + value * scale` from `from`
    /// right-handed about `axis` (both of unit length, square to each
    /// other).
    Arc {
        centre: Vec3,
        axis: Vec3,
        from: Vec3,
        radius: f32,
        base_deg: f32,
        scale: f32,
    },
}

/// Which numbers a drag may set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Range {
    /// From the least up.
    Positive,
    /// Through zero the feature turns the other way and the number stays
    /// above the least; a click turns it too.
    Reversible,
    /// Either side of zero.
    Signed,
}

/// A handle on one number of a feature.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Handle {
    pub feature: FeatureId,
    /// The number's place in the feature's JSON, as its parameter has it.
    pub pointer: String,
    /// What the parameter is called ("Length").
    pub label: String,
    /// Degrees rather than millimetres.
    pub angle: bool,
    pub shape: Shape,
    pub value: f32,
    pub range: Range,
    /// The largest size the number takes.
    pub most: f32,
}

impl Handle {
    pub fn position(&self) -> Vec3 {
        match self.shape {
            Shape::Line { origin, dir, scale } => origin + dir * self.value * scale,
            Shape::Arc {
                base_deg, scale, ..
            } => self.on_arc(base_deg + self.value * scale),
        }
    }

    /// The point of an arc handle's circle `deg` round from its zero.
    fn on_arc(&self, deg: f32) -> Vec3 {
        match self.shape {
            Shape::Arc {
                centre,
                axis,
                from,
                radius,
                ..
            } => {
                let (sin, cos) = deg.to_radians().sin_cos();
                centre + (from * cos + axis.cross(from) * sin) * radius
            }
            Shape::Line { origin, .. } => origin,
        }
    }

    /// Where the handle's path starts: a line's origin, an arc's base.
    pub fn start(&self) -> Vec3 {
        match self.shape {
            Shape::Line { origin, .. } => origin,
            Shape::Arc { base_deg, .. } => self.on_arc(base_deg),
        }
    }

    /// The number with the handle where the ray passes, before stepping:
    /// a line's at the point of its axis nearest the ray, an arc's where
    /// the ray meets its plane, the turn taken nearest the number now.
    fn raw_at(&self, ray_origin: Vec3, ray_dir: Vec3) -> Option<f32> {
        let d = ray_dir.normalize_or_zero();
        match self.shape {
            Shape::Line { origin, dir, scale } => {
                let w = origin - ray_origin;
                let b = dir.dot(d);
                let denom = 1.0 - b * b;
                if denom < 1e-6 {
                    return None;
                }
                let t = (b * d.dot(w) - dir.dot(w)) / denom;
                Some(t / scale)
            }
            Shape::Arc {
                centre,
                axis,
                from,
                base_deg,
                scale,
                ..
            } => {
                let across = d.dot(axis);
                if across.abs() < 1e-3 {
                    return None;
                }
                let hit = ray_origin + d * ((centre - ray_origin).dot(axis) / across);
                let p = hit - centre;
                let mut deg = p.dot(axis.cross(from)).atan2(p.dot(from)).to_degrees();
                let now = base_deg + self.value * scale;
                while deg - now > 180.0 {
                    deg -= 360.0;
                }
                while deg - now < -180.0 {
                    deg += 360.0;
                }
                Some((deg - base_deg) / scale)
            }
        }
    }

    /// The number with the handle where the ray passes, in whole `step`s
    /// and within its range: below zero, through its start, for one that
    /// can go the other way, else never under the least.
    pub fn value_at(&self, ray_origin: Vec3, ray_dir: Vec3, step: f32) -> Option<f32> {
        let raw = self.raw_at(ray_origin, ray_dir)?;
        let step = step.max(LEAST);
        let value = (raw / step).round() * step;
        Some(match self.range {
            Range::Positive => value.clamp(LEAST, self.most),
            Range::Signed => value.clamp(-self.most, self.most),
            Range::Reversible if value.abs() < LEAST => LEAST.copysign(raw),
            Range::Reversible => value.clamp(-self.most, self.most),
        })
    }

    /// The handle as it stands once its feature turns the other way: the
    /// same place, the other way along its path.
    pub fn turn_round(&mut self) {
        match &mut self.shape {
            Shape::Line { dir, .. } => *dir = -*dir,
            Shape::Arc { axis, base_deg, .. } => {
                *axis = -*axis;
                *base_deg = -*base_deg;
            }
        }
        self.value = -self.value;
    }

    /// The number as the handle shows it.
    pub fn text(&self) -> String {
        let digits = format!("{:.2}", self.value);
        let digits = digits.trim_end_matches('0').trim_end_matches('.');
        if self.angle {
            format!("{digits}°")
        } else {
            format!("{digits} mm")
        }
    }
}

/// What a feature's handles are built from: the node, which numbers it
/// lists, and where its body puts things.
struct Maker<'a> {
    document: &'a Document,
    node: &'a FeatureNode,
    feature: FeatureId,
    listed: Vec<String>,
    placement: core_document::BodyPlacement,
}

impl Maker<'_> {
    fn world(&self, p: [f32; 3]) -> Vec3 {
        Vec3::from_array(self.placement.point(p))
    }

    fn world64(&self, p: [f64; 3]) -> Vec3 {
        self.world(p.map(|c| c as f32))
    }

    fn world_dir(&self, d: [f32; 3]) -> Vec3 {
        Vec3::from_array(self.placement.direction(d)).normalize_or_zero()
    }

    fn world_dir64(&self, d: [f64; 3]) -> Vec3 {
        self.world_dir(d.map(|c| c as f32))
    }

    /// A handle on the number at `pointer` (under the feature's variant):
    /// none unless the feature lists it as a parameter that no formula
    /// sets, or for a path that goes nowhere.
    fn handle(&self, pointer: &str, shape: Shape, range: Range) -> Option<Handle> {
        let pointer = format!("/{}{pointer}", self.variant()?);
        if !self.listed.contains(&pointer) || self.node.formulas.contains_key(&pointer) {
            return None;
        }
        let value = self.node.data.pointer(&pointer)?.as_f64()? as f32;
        let label = crate::params::feature_parameters(self.node)
            .into_iter()
            .find(|p| p.pointer == pointer)
            .map_or_else(String::new, |p| p.label);
        let goes = match shape {
            Shape::Line { dir, .. } => dir != Vec3::ZERO,
            Shape::Arc {
                axis, from, radius, ..
            } => axis != Vec3::ZERO && from != Vec3::ZERO && radius > 1e-4,
        };
        goes.then_some(Handle {
            feature: self.feature,
            pointer,
            label,
            angle: matches!(shape, Shape::Arc { .. }),
            shape,
            value,
            range,
            most: f32::INFINITY,
        })
    }

    fn variant(&self) -> Option<&str> {
        self.node
            .data
            .as_object()
            .and_then(|m| m.keys().next())
            .map(String::as_str)
    }
}

/// The handles `feature` offers, looked at from `eye`.
pub(crate) fn handles_of(document: &Document, feature: FeatureId, eye: Vec3) -> Vec<Handle> {
    let Some(node) = document.get_feature_meta(feature) else {
        return Vec::new();
    };
    // Where things stand as the build has them: a start face followed.
    let values = document.feature_values(feature).unwrap_or(&node.data);
    let (Some(body), Ok(part)) = (node.body, DesignFeature::from_json(values)) else {
        return Vec::new();
    };
    let maker = Maker {
        document,
        node,
        feature,
        listed: crate::params::feature_parameters(node)
            .into_iter()
            .map(|p| p.pointer)
            .collect(),
        placement: document.body_placement(body),
    };
    let mut out: Vec<Option<Handle>> = Vec::new();
    match &part {
        DesignFeature::Pad { .. } | DesignFeature::Pocket { .. } => {
            out.extend(extrude_handles(&maker, &part));
        }
        DesignFeature::Hole { .. } => out.extend(hole_handles(&maker, &part)),
        DesignFeature::Revolution { .. } | DesignFeature::Groove { .. } => {
            out.extend(revolve_handles(&maker, &part));
        }
        DesignFeature::Draft { .. } => out.push(draft_handle(&maker, &part, eye)),
        DesignFeature::Primitive { .. } => out.extend(primitive_handles(&maker, &part)),
        DesignFeature::Fillet { edges, .. } => out.push(edge_handle(&maker, edges, "/radius", eye)),
        DesignFeature::Chamfer { edges, .. } => out.push(edge_handle(&maker, edges, "/size", eye)),
        _ => {}
    }
    out.into_iter().flatten().collect()
}

/// A pad's or pocket's: where it starts along the way it runs, from the
/// profile's plane or the plane or face it starts on; its length from
/// there (half of it, centred), and the second side's the other way.
fn extrude_handles(maker: &Maker, part: &DesignFeature) -> Vec<Option<Handle>> {
    let (sketch, reversed, symmetric, mode, mode2, direction, extras, against, first, second) =
        match part {
            DesignFeature::Pad {
                sketch,
                reversed,
                symmetric,
                mode,
                mode2,
                direction,
                extras,
                ..
            } => (
                sketch, reversed, symmetric, mode, mode2, direction, extras, false, "/length",
                "/length2",
            ),
            DesignFeature::Pocket {
                sketch,
                reversed,
                symmetric,
                mode,
                mode2,
                direction,
                extras,
                ..
            } => (
                sketch, reversed, symmetric, mode, mode2, direction, extras, true, "/depth",
                "/depth2",
            ),
            _ => return Vec::new(),
        };
    if *direction != ExtrudeDirection::Normal {
        return Vec::new();
    }
    let Some((centre, normal)) = sketch.and_then(|s| sketch_centre(maker.document, s)) else {
        return Vec::new();
    };
    let sign = if *reversed != against { -1.0 } else { 1.0 };
    let dir = maker.world_dir(normal) * sign;
    // The plane it starts on, along the profile's normal, as the build
    // carries the profile there.
    let to_start = match (&extras.start_face, extras.start_at) {
        (None, None) => Some(0.0),
        (face, at) => (|| {
            let sketch = crate::build::load_sketch(maker.document, (*sketch)?).ok()?;
            let plane = wb_sketch::profile::plane_of(&sketch.plane);
            let target = match (face, at) {
                (Some(face), _) => crate::build::face_pick_plane(face),
                (None, Some(at)) => crate::build::start_plane(maker.document, at).ok()?,
                (None, None) => return None,
            };
            crate::build::distance_to_plane(plane.origin, plane.normal, target).ok()
        })(),
    };
    let Some(to_start) = to_start else {
        return Vec::new();
    };
    let base = maker.world(centre) + maker.world_dir(normal) * to_start as f32;
    let mut out = vec![maker.handle(
        "/extras/start_offset",
        Shape::Line {
            origin: base,
            dir,
            scale: 1.0,
        },
        Range::Signed,
    )];
    let start = base + dir * extras.start_offset;
    let (first_mode, second_mode) = mode.sides(*mode2);
    if first_mode == ExtrudeMode::Dimension {
        // Centred only in the one mode that offers it.
        let centred = *symmetric && *mode == ExtrudeMode::Dimension && mode2.is_none();
        out.push(maker.handle(
            first,
            Shape::Line {
                origin: start,
                dir,
                scale: if centred { 0.5 } else { 1.0 },
            },
            Range::Reversible,
        ));
    }
    if second_mode == Some(ExtrudeMode::Dimension) {
        out.push(maker.handle(
            second,
            Shape::Line {
                origin: start,
                dir: -dir,
                scale: 1.0,
            },
            Range::Positive,
        ));
    }
    out
}

/// A hole's: where it starts down the drill, from its sketch's plane or
/// the plane or face it starts on, and its depth from there, at its first
/// centre.
fn hole_handles(maker: &Maker, part: &DesignFeature) -> Vec<Option<Handle>> {
    let DesignFeature::Hole {
        sketch,
        through_all,
        start,
        ..
    } = part
    else {
        return Vec::new();
    };
    let Ok(sketch) = crate::build::load_sketch(maker.document, *sketch) else {
        return Vec::new();
    };
    let mut unshifted = part.clone();
    if let DesignFeature::Hole { start, .. } = &mut unshifted {
        start.offset = 0.0;
    }
    let Ok((plane, into)) = crate::build::hole_plane(maker.document, &unshifted, &sketch) else {
        return Vec::new();
    };
    let Some(centre) = crate::build::hole_centers(&sketch.sketch).first().copied() else {
        return Vec::new();
    };
    let at: [f64; 3] = std::array::from_fn(|k| {
        plane.origin[k] + plane.x_axis[k] * centre[0] + plane.y_axis[k] * centre[1]
    });
    let (base, dir) = (maker.world64(at), maker.world_dir64(into));
    let mut out = vec![maker.handle(
        "/start/offset",
        Shape::Line {
            origin: base,
            dir,
            scale: 1.0,
        },
        Range::Signed,
    )];
    if !*through_all {
        out.push(maker.handle(
            "/depth",
            Shape::Line {
                origin: base + dir * start.offset,
                dir,
                scale: 1.0,
            },
            Range::Reversible,
        ));
    }
    out
}

/// A revolution's or groove's: where it starts round the axis from the
/// profile's middle, its angle from there the way it turns (half of it,
/// about the midplane), and the second angle the other way.
fn revolve_handles(maker: &Maker, part: &DesignFeature) -> Vec<Option<Handle>> {
    let (DesignFeature::Revolution {
        sketch,
        axis,
        project_axis,
        reversed,
        midplane,
        second_angle_deg,
        start_angle_deg,
        mode,
        ..
    }
    | DesignFeature::Groove {
        sketch,
        axis,
        project_axis,
        reversed,
        midplane,
        second_angle_deg,
        start_angle_deg,
        mode,
        ..
    }) = part
    else {
        return Vec::new();
    };
    let Some(ring) = revolve_ring(maker, *sketch, axis, *project_axis, *reversed) else {
        return Vec::new();
    };
    let mut start = maker.handle("/start_angle_deg", ring.arc(0.0, 1.0), Range::Signed);
    if let Some(h) = &mut start {
        h.most = 360.0;
    }
    let mut out = vec![start];
    if *mode == RevolveMode::Angle {
        let mut angle = maker.handle(
            "/angle_deg",
            ring.arc(*start_angle_deg, if *midplane { 0.5 } else { 1.0 }),
            if *midplane {
                Range::Positive
            } else {
                Range::Reversible
            },
        );
        if let Some(h) = &mut angle {
            h.most = 360.0;
        }
        out.push(angle);
        if second_angle_deg.is_some() {
            let mut second = maker.handle(
                "/second_angle_deg",
                ring.turned_round().arc(-*start_angle_deg, 1.0),
                Range::Positive,
            );
            if let Some(h) = &mut second {
                h.most = 360.0;
            }
            out.push(second);
        }
    }
    out
}

/// The circle a revolution's handles run round: about its axis, through
/// the middle of its profile, turning the way the revolution turns.
#[derive(Debug, Clone, Copy)]
struct Ring {
    centre: Vec3,
    axis: Vec3,
    from: Vec3,
    radius: f32,
}

impl Ring {
    fn arc(&self, base_deg: f32, scale: f32) -> Shape {
        Shape::Arc {
            centre: self.centre,
            axis: self.axis,
            from: self.from,
            radius: self.radius,
            base_deg,
            scale,
        }
    }

    fn turned_round(self) -> Self {
        Self {
            axis: -self.axis,
            ..self
        }
    }
}

fn revolve_ring(
    maker: &Maker,
    sketch: FeatureId,
    axis: &crate::feature::RevolveAxis,
    project: bool,
    reversed: bool,
) -> Option<Ring> {
    let sketch_feature = crate::build::load_sketch(maker.document, sketch).ok()?;
    let (point, way) =
        crate::build::axis_in_sketch(maker.document, &sketch_feature, axis, project).ok()?;
    let plane = wb_sketch::profile::plane_of(&sketch_feature.plane);
    let at = |p: [f64; 2]| -> [f64; 3] {
        std::array::from_fn(|k| plane.origin[k] + plane.x_axis[k] * p[0] + plane.y_axis[k] * p[1])
    };
    let along: [f64; 3] =
        std::array::from_fn(|k| plane.x_axis[k] * way[0] + plane.y_axis[k] * way[1]);
    let pivot = maker.world64(at(point));
    let axis = maker.world_dir64(along) * if reversed { -1.0 } else { 1.0 };
    let (middle, _) = sketch_centre(maker.document, sketch)?;
    let middle = maker.world(middle);
    let foot = pivot + axis * (middle - pivot).dot(axis);
    let out = middle - foot;
    Some(Ring {
        centre: foot,
        axis,
        from: out.normalize_or_zero(),
        radius: out.length(),
    })
}

/// A draft's: its angle, as the first drafted face's line along the pull
/// tilts from the pull toward the face's outside.
fn draft_handle(maker: &Maker, part: &DesignFeature, eye: Vec3) -> Option<Handle> {
    let DesignFeature::Draft {
        neutral,
        faces,
        reversed,
        neutral_plane: None,
        pull: None,
        ..
    } = part
    else {
        return None;
    };
    let face = faces.first()?;
    let pull = maker.world_dir(neutral.normal) * if *reversed { -1.0 } else { 1.0 };
    let outside = maker.world_dir(face.normal);
    let axis = pull.cross(outside);
    if axis.length() < 0.1 {
        return None;
    }
    let centre = maker.world(face.point);
    let mut handle = maker.handle(
        "/angle_deg",
        Shape::Arc {
            centre,
            axis: axis.normalize(),
            from: pull,
            radius: (eye - centre).length() * ARC_SHARE,
            base_deg: 0.0,
            scale: 1.0,
        },
        Range::Positive,
    )?;
    handle.most = 45.0;
    Some(handle)
}

/// A primitive's: each size from its placement's origin along the axis it
/// runs (a cone's top radius at its top, a torus's tube radius from its
/// ring).
fn primitive_handles(maker: &Maker, part: &DesignFeature) -> Vec<Option<Handle>> {
    let DesignFeature::Primitive {
        kind,
        placement,
        attached,
        ..
    } = part
    else {
        return Vec::new();
    };
    let placement = attached.as_ref().map_or(*placement, |a| a.placement());
    let origin = maker.world64(placement.origin);
    let x = maker.world_dir64(placement.x_axis);
    let z = maker.world_dir64(placement.z_axis);
    let y = z.cross(x);
    let line = |origin: Vec3, dir: Vec3| Shape::Line {
        origin,
        dir,
        scale: 1.0,
    };
    let size = |field: &str, origin: Vec3, dir: Vec3| {
        let name = kind_name(kind)?;
        maker.handle(
            &format!("/kind/{name}/{field}"),
            line(origin, dir),
            Range::Positive,
        )
    };
    use kernel_api::PrimitiveKind as K;
    match kind {
        K::Box { .. } => vec![
            size("length", origin, x),
            size("width", origin, y),
            size("height", origin, z),
        ],
        K::Cylinder { .. } | K::Prism { .. } => vec![
            size(
                if matches!(kind, K::Prism { .. }) {
                    "circumradius"
                } else {
                    "radius"
                },
                origin,
                x,
            ),
            size("height", origin, z),
        ],
        K::Sphere { .. } => vec![size("radius", origin, x)],
        K::Cone { height, .. } => vec![
            size("radius1", origin, x),
            size("radius2", origin + z * *height as f32, x),
            size("height", origin, z),
        ],
        K::Torus { radius1, .. } => vec![
            size("radius1", origin, x),
            size("radius2", origin + x * *radius1 as f32, x),
        ],
        K::Ellipsoid { .. } => vec![
            size("radius1", origin, x),
            size("radius2", origin, y),
            size("radius3", origin, z),
        ],
        _ => Vec::new(),
    }
}

/// A primitive kind's name in the feature's JSON.
fn kind_name(kind: &kernel_api::PrimitiveKind) -> Option<String> {
    serde_json::to_value(kind)
        .ok()?
        .as_object()?
        .keys()
        .next()
        .cloned()
}

/// A fillet's or chamfer's: beside its first edge, square to it and
/// toward the eye.
fn edge_handle(maker: &Maker, edges: &EdgeSel, pointer: &str, eye: Vec3) -> Option<Handle> {
    let EdgeSel::Edges(picks) = edges else {
        return None;
    };
    let pick = picks.first()?;
    let origin = maker.world(pick.point);
    let along = maker.world_dir(pick.direction);
    let to_eye = eye - origin;
    let dir = (to_eye - along * to_eye.dot(along)).normalize_or_zero();
    maker.handle(
        pointer,
        Shape::Line {
            origin,
            dir,
            scale: 1.0,
        },
        Range::Positive,
    )
}

/// Turn a feature the other way.
pub(crate) fn reverse(feature: &mut DesignFeature) -> bool {
    match feature {
        DesignFeature::Pad { reversed, .. }
        | DesignFeature::Pocket { reversed, .. }
        | DesignFeature::Hole { reversed, .. }
        | DesignFeature::Revolution { reversed, .. }
        | DesignFeature::Groove { reversed, .. } => {
            *reversed = !*reversed;
            true
        }
        _ => false,
    }
}

/// Write `value` into the number at `pointer` of a feature's JSON, kept
/// to a millionth so a step lands on a round number.
pub(crate) fn write(data: &mut Value, pointer: &str, value: f32) -> bool {
    let Some(slot) = data.pointer_mut(pointer) else {
        return false;
    };
    let value = (f64::from(value) * 1e6).round() / 1e6;
    *slot = serde_json::json!(value);
    true
}

/// The middle of a sketch's drawing and its plane's normal, in its body's
/// own frame.
pub(crate) fn sketch_centre(
    document: &Document,
    sketch: FeatureId,
) -> Option<([f32; 3], [f32; 3])> {
    let data = document
        .feature_values(sketch)
        .or_else(|| document.get_feature_data(sketch))?;
    let feature = wb_sketch::SketchFeature::from_json(data).ok()?;
    let points: Vec<Vec3> = wb_sketch::render::sketch_polylines(&feature.sketch, &feature.plane)
        .into_iter()
        .flatten()
        .map(|p| Vec3::from_array(wb_sketch::boundary::narrow3(p)))
        .collect();
    let (lo, hi) = points.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let centre = if points.is_empty() {
        Vec3::from_array(wb_sketch::boundary::narrow3(feature.plane.origin))
    } else {
        (lo + hi) * 0.5
    };
    Some((
        centre.to_array(),
        wb_sketch::boundary::narrow3(feature.plane.normal),
    ))
}

/// The handle whose dot is nearest the press at `pos`, within reach.
pub(crate) fn under(
    ctx: &WorkbenchRuntimeContext,
    handles: &[Handle],
    pos: (f32, f32),
) -> Option<usize> {
    let reach = REACH_PX * ctx.pixels_per_point.max(0.5);
    handles
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let (x, y) = ctx.world_to_viewport(h.position().to_array())?;
            let d = (x - pos.0).hypot(y - pos.1);
            (d <= reach).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The handle's path from its start to it: a dashed line, or the arc.
pub(crate) fn overlays(ctx: &WorkbenchRuntimeContext, handle: &Handle) -> Vec<ScreenSpaceOverlay> {
    let color = ctx.sketch_palette.selected;
    let points: Vec<Vec3> = match handle.shape {
        Shape::Line { .. } => vec![handle.start(), handle.position()],
        Shape::Arc {
            base_deg, scale, ..
        } => {
            let sweep = handle.value * scale;
            let pieces = ((sweep.abs() / 360.0) * ARC_SEGMENTS).ceil().max(1.0) as usize;
            (0..=pieces)
                .map(|i| handle.on_arc(base_deg + sweep * i as f32 / pieces as f32))
                .collect()
        }
    };
    let screen: Vec<Option<(f32, f32)>> = points
        .iter()
        .map(|p| ctx.world_to_viewport(p.to_array()))
        .collect();
    screen
        .windows(2)
        .filter_map(|pair| match (pair[0], pair[1]) {
            (Some(a), Some(b)) => {
                Some(ScreenSpaceOverlay::new([a.0, a.1], [b.0, b.1], color, 1.5).dashed(4.0, 3.0))
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn marks(
    ctx: &WorkbenchRuntimeContext,
    handle: &Handle,
    held: bool,
) -> Vec<ScreenSpaceMark> {
    let color = if held {
        ctx.sketch_palette.preselect
    } else {
        ctx.sketch_palette.selected
    };
    ctx.world_to_viewport(handle.position().to_array())
        .map(|(x, y)| vec![ScreenSpaceMark::dot([x, y], 6.0, color)])
        .unwrap_or_default()
}

/// The number beside the handle while it is held.
pub(crate) fn labels(ctx: &WorkbenchRuntimeContext, handle: &Handle) -> Vec<ScreenSpaceLabel> {
    ctx.world_to_viewport(handle.position().to_array())
        .map(|(x, y)| {
            vec![
                ScreenSpaceLabel::new(
                    [x, y - 18.0],
                    handle.text(),
                    ctx.sketch_palette.selected,
                    12.0,
                )
                .pill()
                .mono(),
            ]
        })
        .unwrap_or_default()
}

/// What the status bar says while a handle is held: what it sets, its
/// steps and the key for the fine ones.
pub(crate) fn hint(handle: &Handle, step: f32, fine: f32) -> String {
    let unit = if handle.angle { "°" } else { " mm" };
    let mut text = format!(
        "{} {} · steps of {step}{unit}, Shift: {fine}{unit}",
        handle.label,
        handle.text()
    );
    if handle.range == Range::Reversible {
        text.push_str(" · past the start turns it round");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(scale: f32, range: Range) -> Handle {
        Handle {
            feature: FeatureId(uuid::Uuid::nil()),
            pointer: "/Pad/length".into(),
            label: "Length".into(),
            angle: false,
            shape: Shape::Line {
                origin: Vec3::ZERO,
                dir: Vec3::Z,
                scale,
            },
            value: 10.0,
            range,
            most: f32::INFINITY,
        }
    }

    #[test]
    fn a_ray_across_the_axis_sets_the_value_where_it_passes() {
        let handle = line(1.0, Range::Positive);
        // Looking along -X at height 25.
        let value = handle
            .value_at(Vec3::new(100.0, 0.0, 25.04), Vec3::NEG_X, 0.1)
            .unwrap();
        assert!((value - 25.0).abs() < 1e-4);
        // Half the length shows on each side of a symmetric one.
        let half = line(0.5, Range::Positive);
        let value = half
            .value_at(Vec3::new(100.0, 0.0, 6.0), Vec3::NEG_X, 0.1)
            .unwrap();
        assert!((value - 12.0).abs() < 1e-4);
        // Never below the least, and nothing along the axis itself.
        assert_eq!(
            handle.value_at(Vec3::new(9.0, 0.0, -5.0), Vec3::NEG_X, 0.1),
            Some(LEAST)
        );
        assert_eq!(
            handle.value_at(Vec3::new(0.0, 0.0, 50.0), Vec3::NEG_Z, 0.1),
            None
        );
    }

    /// Coarse steps take whole millimetres, fine ones tenths; a pad's
    /// handle goes below zero, through its start, where it turns round.
    #[test]
    fn a_drag_steps_and_a_reversible_one_goes_through_zero() {
        let pad = line(1.0, Range::Reversible);
        let at = |z: f32, step: f32| pad.value_at(Vec3::new(100.0, 0.0, z), Vec3::NEG_X, step);
        assert_eq!(at(12.34, 1.0), Some(12.0));
        assert!((at(12.34, 0.1).unwrap() - 12.3).abs() < 1e-4);
        assert_eq!(at(-7.6, 1.0), Some(-8.0));
        assert_eq!(at(-0.2, 1.0), Some(-LEAST), "just past the start");
        let mut turned = pad.clone();
        turned.value = 8.0;
        turned.turn_round();
        assert_eq!(turned.position(), Vec3::new(0.0, 0.0, 8.0), "where it was");
        let signed = line(1.0, Range::Signed);
        assert_eq!(
            signed.value_at(Vec3::new(100.0, 0.0, -0.2), Vec3::NEG_X, 1.0),
            Some(0.0),
            "a signed number may be zero"
        );
        let mut part = DesignFeature::from_json(&serde_json::json!({"Pad": {
            "sketch": uuid::Uuid::nil(), "length": 5.0, "reversed": false,
        }}));
        if let Ok(part) = part.as_mut() {
            assert!(reverse(part));
            assert!(matches!(part, DesignFeature::Pad { reversed: true, .. }));
        }
    }

    /// An arc handle reads the angle where the ray meets its plane, keeps
    /// counting past a half turn, and stops at its most.
    #[test]
    fn an_arc_handle_follows_the_turn() {
        let mut arc = Handle {
            angle: true,
            shape: Shape::Arc {
                centre: Vec3::ZERO,
                axis: Vec3::Z,
                from: Vec3::X,
                radius: 10.0,
                base_deg: 0.0,
                scale: 1.0,
            },
            value: 90.0,
            most: 360.0,
            ..line(1.0, Range::Reversible)
        };
        assert!(arc.position().abs_diff_eq(Vec3::new(0.0, 10.0, 0.0), 1e-4));
        // Looking down at (-10, 1): just short of a half turn.
        let down = |x: f32, y: f32, arc: &Handle, step: f32| {
            arc.value_at(Vec3::new(x, y, 50.0), Vec3::NEG_Z, step)
        };
        assert_eq!(down(-10.0, 1.0, &arc, 5.0), Some(175.0));
        // From 175°, a point just past the half turn reads past it.
        arc.value = 175.0;
        assert_eq!(down(-10.0, -1.0, &arc, 5.0), Some(185.0));
        arc.value = 355.0;
        assert_eq!(down(10.0, 0.9, &arc, 5.0), Some(360.0), "at most a turn");
        // Seen edge on, it reads nothing.
        assert_eq!(arc.value_at(Vec3::new(0.0, -50.0, 0.0), Vec3::Y, 1.0), None);
        // Half the angle shows about a midplane.
        let half = Handle {
            shape: Shape::Arc {
                centre: Vec3::ZERO,
                axis: Vec3::Z,
                from: Vec3::X,
                radius: 10.0,
                base_deg: 0.0,
                scale: 0.5,
            },
            value: 90.0,
            ..arc.clone()
        };
        assert_eq!(down(0.0, 10.0, &half, 1.0), Some(180.0));
        // Turned round, an arc runs the other way from the same place.
        let mut turned = half.clone();
        turned.turn_round();
        assert!(turned.position().abs_diff_eq(half.position(), 1e-4));
    }

    /// A body with a 10 x 6 rectangle drawn on XY, and `feature` (JSON,
    /// `SKETCH` standing for the sketch's id) made from it.
    fn made(feature: serde_json::Value) -> (Document, FeatureId) {
        use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, Vec2D};
        let mut sketch = Sketch::new("s");
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 6.0), (0.0, 6.0)].map(|(x, y)| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
        });
        for i in 0..4 {
            sketch.add_geometry(GeometryElement::Line(Line::new(
                corners[i],
                corners[(i + 1) % 4],
            )));
        }
        let plane = sketch.plane;
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let sketch = doc
            .add_feature_in_body(
                wb_sketch::SketchFeature::new(sketch, plane),
                "s".into(),
                Some(body),
            )
            .unwrap();
        let text = feature.to_string().replace("SKETCH", &sketch.0.to_string());
        let feature = DesignFeature::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
        let id = doc
            .add_feature_in_body(feature, "f".into(), Some(body))
            .unwrap();
        (doc, id)
    }

    const EYE: Vec3 = Vec3::new(50.0, -50.0, 40.0);

    /// A pad's start stands on its profile's middle, its length above
    /// the start and the second side's below it; a formula on the length
    /// takes its handle away.
    #[test]
    fn a_pad_offers_each_side_s_length_unless_a_formula_sets_it() {
        let (mut doc, pad) = made(serde_json::json!({"Pad": {
            "sketch": "SKETCH", "length": 4.0, "length2": 3.0, "reversed": false,
            "mode": "TwoLengths",
        }}));
        let offered = handles_of(&doc, pad, EYE);
        let at: Vec<(&str, Vec3)> = offered
            .iter()
            .map(|h| (h.label.as_str(), h.position()))
            .collect();
        assert_eq!(at.len(), 3, "{at:?}");
        assert_eq!(at[0], ("Start offset", Vec3::new(5.0, 3.0, 0.0)));
        assert_eq!(offered[0].range, Range::Signed);
        assert!(
            at[1].1.abs_diff_eq(Vec3::new(5.0, 3.0, 4.0), 1e-4),
            "{at:?}"
        );
        assert_eq!(at[2].0, "Second length");
        assert!(
            at[2].1.abs_diff_eq(Vec3::new(5.0, 3.0, -3.0), 1e-4),
            "{at:?}"
        );
        assert_eq!(offered[1].range, Range::Reversible);
        doc.set_feature_formula(pad, "/Pad/length", Some("2 * 3".into()))
            .unwrap();
        let left: Vec<String> = handles_of(&doc, pad, EYE)
            .into_iter()
            .map(|h| h.label)
            .collect();
        assert_eq!(left, ["Start offset", "Second length"]);
    }

    /// A pad started on a face 7 mm up and 2 mm on from it: the start
    /// handle stands at 9, its length from there.
    #[test]
    fn a_pad_s_start_handle_stands_where_it_starts() {
        let (doc, pad) = made(serde_json::json!({"Pad": {
            "sketch": "SKETCH", "length": 4.0, "reversed": false,
            "extras": {"start_offset": 2.0, "start_face": {
                "point": [1.0, 1.0, 7.0], "normal": [0.0, 0.0, 1.0],
            }},
        }}));
        let at: Vec<Vec3> = handles_of(&doc, pad, EYE)
            .iter()
            .map(Handle::position)
            .collect();
        assert_eq!(at, [Vec3::new(5.0, 3.0, 9.0), Vec3::new(5.0, 3.0, 13.0)]);
    }

    /// A revolution's angle runs round its axis from the profile's
    /// middle, a full turn at most, and a click turns it round.
    #[test]
    fn a_revolution_offers_its_angle_round_the_axis() {
        let (doc, revolution) = made(serde_json::json!({"Revolution": {
            "sketch": "SKETCH", "angle_deg": 90.0, "axis": "SketchY",
        }}));
        let offered = handles_of(&doc, revolution, EYE);
        assert_eq!(offered.len(), 2);
        assert_eq!(offered[0].label, "Start angle");
        assert!(
            offered[0]
                .position()
                .abs_diff_eq(Vec3::new(5.0, 3.0, 0.0), 1e-4)
        );
        let angle = &offered[1];
        assert!(angle.angle && angle.range == Range::Reversible && angle.most == 360.0);
        // The middle (5, 3, 0) a quarter turn about +Y: (0, 3, -5).
        assert!(
            angle
                .position()
                .abs_diff_eq(Vec3::new(0.0, 3.0, -5.0), 1e-4),
            "{:?}",
            angle.position()
        );
        // Started a quarter on, it ends a half turn round.
        let (doc, revolution) = made(serde_json::json!({"Revolution": {
            "sketch": "SKETCH", "angle_deg": 90.0, "axis": "SketchY", "start_angle_deg": 90.0,
        }}));
        let offered = handles_of(&doc, revolution, EYE);
        assert!(
            offered[0]
                .position()
                .abs_diff_eq(Vec3::new(0.0, 3.0, -5.0), 1e-4)
        );
        assert!(
            offered[1]
                .position()
                .abs_diff_eq(Vec3::new(-5.0, 3.0, 0.0), 1e-4),
            "{:?}",
            offered[1].position()
        );
    }

    /// A box's sizes run along its own axes; a blind hole's depth runs
    /// down the drill from its first centre.
    #[test]
    fn a_box_and_a_hole_offer_their_sizes() {
        let (doc, cube) = made(serde_json::json!({"Primitive": {
            "kind": {"Box": {"length": 4.0, "width": 5.0, "height": 6.0}},
            "placement": {"origin": [1.0, 0.0, 0.0], "x_axis": [1.0, 0.0, 0.0], "z_axis": [0.0, 0.0, 1.0]},
            "subtractive": false,
        }}));
        let ends: Vec<Vec3> = handles_of(&doc, cube, EYE)
            .iter()
            .map(Handle::position)
            .collect();
        assert_eq!(
            ends,
            [
                Vec3::new(5.0, 0.0, 0.0),
                Vec3::new(1.0, 5.0, 0.0),
                Vec3::new(1.0, 0.0, 6.0)
            ]
        );
        let (mut doc, hole) = made(serde_json::json!({"Hole": {
            "sketch": "SKETCH", "diameter": 2.0, "depth": 3.0, "through_all": false,
        }}));
        // The rectangle has no centres: nothing to stand on.
        assert!(handles_of(&doc, hole, EYE).is_empty());
        let mut lone = wb_sketch::sketch::Sketch::new("p");
        lone.add_geometry(wb_sketch::sketch::GeometryElement::Point(
            wb_sketch::sketch::Point::new(wb_sketch::sketch::Vec2D::new(2.0, 1.0)),
        ));
        let plane = lone.plane;
        let body = doc.get_feature_meta(hole).and_then(|n| n.body);
        let points = doc
            .add_feature_in_body(wb_sketch::SketchFeature::new(lone, plane), "p".into(), body)
            .unwrap();
        let mut data = doc.get_feature_data(hole).unwrap().clone();
        data["Hole"]["sketch"] = serde_json::json!(points);
        doc.update_feature_data(hole, data).unwrap();
        let offered = handles_of(&doc, hole, EYE);
        assert_eq!(offered.len(), 2);
        assert_eq!(
            offered[0].position(),
            Vec3::new(2.0, 1.0, 0.0),
            "where it starts"
        );
        assert_eq!(
            offered[1].position(),
            Vec3::new(2.0, 1.0, -3.0),
            "down the drill"
        );
    }

    #[test]
    fn the_hint_names_the_number_its_steps_and_the_fine_key() {
        let handle = line(1.0, Range::Reversible);
        let hint = hint(&handle, 1.0, 0.1);
        assert!(hint.starts_with("Length 10 mm"), "{hint}");
        assert!(hint.contains("Shift: 0.1 mm"), "{hint}");
        assert!(hint.contains("turns it round"), "{hint}");
        let mut written = serde_json::json!({"Pad": {"length": 1.0}});
        assert!(write(&mut written, "/Pad/length", 12.3));
        assert_eq!(written["Pad"]["length"], serde_json::json!(12.3));
    }
}
