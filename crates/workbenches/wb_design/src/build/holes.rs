//! Hole operations, nut pockets and cosmetic thread geometry.

use super::{
    Mat4, compose_steps, design_features_of_body, distance_to_plane, face_pick_plane, load_sketch,
    normalize, occurrence_steps, start_plane, transform_point,
};
use crate::feature::{DesignFeature, DrillPoint, HoleCut, NutSide, NutTrap, ThreadSpec};
use core_document::{BodyId, Document, FeatureId};
use kernel_api::{
    BooleanOp, ExtrudeTermination, Profile, ProfileSegment, ProfileWire, SolidOp, SweepKind,
};
use wb_sketch::sketch::{GeometryElement, Sketch};
use wb_sketch::{SketchFeature, profile};

/// A hole's drill diameter where it opens, from its thread when it is
/// standards-driven: the tap drill, moved out by the class's allowance (a
/// tapered thread's minor diameter at the face, which its taper narrows
/// from), or else the clearance of its fit (the major diameter where the
/// standard names no clearance).
pub fn hole_diameter(feature: &DesignFeature) -> f32 {
    let DesignFeature::Hole {
        diameter,
        thread,
        threaded,
        fit,
        clearance,
        ..
    } = feature
    else {
        return 0.0;
    };
    let Some((spec, Ok(size))) = thread.as_ref().map(|t| (t, t.resolve())) else {
        return *diameter;
    };
    if let (false, Some(own)) = (*threaded, clearance) {
        return *own;
    }
    if *threaded {
        let drill = if spec.standard.is_tapered() {
            size.minor
        } else {
            size.tap_drill
        };
        (drill + spec.allowance(&size)) as f32
    } else {
        size.clearance.map_or(size.major, |c| c[*fit as usize]) as f32
    }
}

/// The angle a hole's wall leans in by toward its bottom, degrees: a
/// tapped tapered thread's own, else the hole's.
pub(super) fn hole_taper_deg(feature: &DesignFeature) -> f64 {
    let DesignFeature::Hole {
        thread,
        threaded,
        taper_deg,
        ..
    } = feature
    else {
        return 0.0;
    };
    match thread {
        Some(spec) if *threaded && spec.standard.is_tapered() => {
            crate::hole_tables::pipe_taper_deg()
        }
        _ => f64::from(*taper_deg),
    }
}

/// Circle centers + standalone points of a sketch (hole positions).
pub(crate) fn hole_centers(sketch: &Sketch) -> Vec<[f64; 2]> {
    let mut centers = Vec::new();
    let referenced: std::collections::HashSet<_> = sketch
        .geometry
        .iter()
        .flat_map(Sketch::curve_point_ids)
        .collect();
    for element in &sketch.geometry {
        if sketch.is_construction(element.id()) {
            continue;
        }
        match element {
            GeometryElement::Circle(circle) => {
                if let Some(pos) = sketch.point_position(circle.center) {
                    centers.push([pos.x, pos.y]);
                }
            }
            GeometryElement::Point(point) if !referenced.contains(&point.id) => {
                centers.push([point.position.x, point.position.y]);
            }
            _ => {}
        }
    }
    centers
}

/// How many line pieces a cosmetic thread draws per turn.
pub(super) const THREAD_SEGMENTS_PER_TURN: usize = 24;

/// The bores a hole with a cosmetic thread threads, in its body's frame.
pub(crate) struct ThreadedBores {
    /// Each hole's centre on the plane it starts on: where its bore opens.
    pub mouths: Vec<[f64; 3]>,
    /// Down the bores, into the material.
    pub into: [f64; 3],
    /// Across the bores: the sketch plane's X axis.
    pub across: [f64; 3],
    /// The bores' radius, mm.
    pub radius: f64,
    pub pitch: f64,
    pub left_handed: bool,
}

/// Where a hole with a cosmetic thread opens; `None` for a hole with no
/// cosmetic thread, or one whose sketch or thread size is not found.
pub(crate) fn threaded_bores(
    document: &Document,
    feature: &DesignFeature,
) -> Option<ThreadedBores> {
    let DesignFeature::Hole {
        sketch,
        thread: Some(spec),
        ..
    } = feature
    else {
        return None;
    };
    feature.cosmetic_thread()?;
    let (Ok(size), Ok(sketch_feature)) = (spec.resolve(), load_sketch(document, *sketch)) else {
        return None;
    };
    let (plane, into) = hole_plane(document, feature, &sketch_feature).ok()?;
    let mouths = hole_centers(&sketch_feature.sketch)
        .into_iter()
        .map(|centre| {
            std::array::from_fn(|k| {
                plane.origin[k] + plane.x_axis[k] * centre[0] + plane.y_axis[k] * centre[1]
            })
        })
        .collect();
    Some(ThreadedBores {
        mouths,
        into,
        across: plane.x_axis,
        radius: f64::from(hole_diameter(feature)) * 0.5,
        pitch: size.pitch,
        left_handed: spec.left_handed,
    })
}

/// One threaded bore, in its body's frame: the hole that made it (its
/// thread, size and depth), where it opens and how it runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bore {
    pub hole: FeatureId,
    pub mouth: [f64; 3],
    pub into: [f64; 3],
    pub across: [f64; 3],
}

/// The threaded bores feature `id` of `body` makes: a hole with a
/// cosmetic thread its own; a mirror or a pattern the bores of what it
/// repeats (its originals, or every feature before it for the whole body)
/// carried to each copy it keeps. A copy is left out where the body's
/// built solid shows no bore, as when it lands outside the material, and
/// a pattern that follows a path, whose places only the solid gives,
/// threads none.
pub(crate) fn threaded_bores_of(document: &Document, body: BodyId, id: FeatureId) -> Vec<Bore> {
    let features = design_features_of_body(document, body);
    let mut seen = Vec::new();
    bores_of(document, body, &features, id, &mut seen)
}

fn bores_of(
    document: &Document,
    body: BodyId,
    features: &[(FeatureId, DesignFeature)],
    id: FeatureId,
    seen: &mut Vec<FeatureId>,
) -> Vec<Bore> {
    // A feature's history reads only what comes before it; `seen` is the
    // way down here, which guards a document whose originals point the
    // other way.
    if seen.contains(&id) {
        return Vec::new();
    }
    seen.push(id);
    let bores = bores_seen(document, body, features, id, seen);
    seen.pop();
    bores
}

fn bores_seen(
    document: &Document,
    body: BodyId,
    features: &[(FeatureId, DesignFeature)],
    id: FeatureId,
    seen: &mut Vec<FeatureId>,
) -> Vec<Bore> {
    let Some(at) = features.iter().position(|(f, _)| *f == id) else {
        return Vec::new();
    };
    let feature = &features[at].1;
    if let Some(bores) = threaded_bores(document, feature) {
        return bores
            .mouths
            .iter()
            .map(|mouth| Bore {
                hole: id,
                mouth: *mouth,
                into: bores.into,
                across: bores.across,
            })
            .collect();
    }
    let Some(originals) = feature.originals() else {
        return Vec::new();
    };
    let Some(copies) = occurrence_steps(document, feature)
        .ok()
        .and_then(|steps| compose_steps(&steps))
    else {
        return Vec::new();
    };
    let skipped = feature.skipped().unwrap_or(&[]);
    let sources: Vec<FeatureId> = if originals.is_empty() {
        features[..at].iter().map(|(f, _)| *f).collect()
    } else {
        originals.to_vec()
    };
    let source_bores: Vec<Bore> = sources
        .iter()
        .flat_map(|source| bores_of(document, body, features, *source, seen))
        .collect();
    let solid = document.local_geometry(body);
    copies
        .iter()
        .zip(0u32..)
        .skip(1)
        .filter(|(_, index)| !skipped.contains(index))
        .flat_map(|(m, _)| {
            source_bores.iter().map(move |b| Bore {
                hole: b.hole,
                mouth: transform_point(m, b.mouth),
                into: transform_vector(m, b.into),
                across: transform_vector(m, b.across),
            })
        })
        .filter(|b| {
            let radius = features
                .iter()
                .find(|(f, _)| *f == b.hole)
                .map_or(0.0, |(_, h)| f64::from(hole_diameter(h)) * 0.5);
            solid
                .as_ref()
                .is_none_or(|(mesh, _)| bore_drawn(mesh, b, radius))
        })
        .collect()
}

/// `v` turned by `m`, its move left out.
fn transform_vector(m: &Mat4, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// Whether `mesh` has a bore of `radius` at `bore`: a point of its rim
/// round the mouth, or of its wall a little way down.
pub(super) fn bore_drawn(mesh: &kernel_api::TriMesh, bore: &Bore, radius: f64) -> bool {
    let reach = (radius * 0.05).max(0.02);
    mesh.positions.iter().any(|p| {
        let rel: [f64; 3] = std::array::from_fn(|k| f64::from(p[k]) - bore.mouth[k]);
        let down: f64 = (0..3).map(|k| rel[k] * bore.into[k]).sum();
        let off: [f64; 3] = std::array::from_fn(|k| rel[k] - bore.into[k] * down);
        let r = (off[0] * off[0] + off[1] * off[1] + off[2] * off[2]).sqrt();
        (-reach..=radius.max(1.0) * 4.0).contains(&down) && (r - radius).abs() <= reach
    })
}

/// A cosmetic thread as lines in its body's frame: at every hole centre a
/// helix of the thread's pitch, just inside the bore so the wall does not
/// hide it, down the thread's length: the thread's own depth when set,
/// never past the hole's end, which for a through hole is where the solid
/// ends along it (`through_length`). Empty for a hole with no cosmetic
/// thread.
pub(crate) fn cosmetic_thread_lines(
    document: &Document,
    body: Option<BodyId>,
    feature: &DesignFeature,
) -> kernel_api::TriMesh {
    match threaded_bores(document, feature) {
        Some(bores) => thread_lines(document, body, feature, bores),
        None => kernel_api::TriMesh::default(),
    }
}

/// The cosmetic threads of the bores feature `id` copies (a mirror's or a
/// pattern's, [`threaded_bores_of`]) as lines, each drawn as its hole's
/// own: a copy across a mirror keeps the hand its thread is cut to.
pub(crate) fn copied_thread_lines(
    document: &Document,
    body: BodyId,
    id: FeatureId,
) -> kernel_api::TriMesh {
    let mut mesh = kernel_api::TriMesh::default();
    for bore in threaded_bores_of(document, body, id) {
        if bore.hole == id {
            continue;
        }
        let Some(hole) = document.feature_as::<DesignFeature>(bore.hole) else {
            continue;
        };
        let Some(own) = threaded_bores(document, &hole) else {
            continue;
        };
        let one = ThreadedBores {
            mouths: vec![bore.mouth],
            into: bore.into,
            across: bore.across,
            ..own
        };
        let lines = thread_lines(document, Some(body), &hole, one);
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(lines.positions);
        mesh.normals.extend(lines.normals);
        mesh.edges.extend(lines.edges.iter().map(|e| e + base));
    }
    mesh
}

/// [`cosmetic_thread_lines`] for the bores `bores` of hole `feature`.
fn thread_lines(
    document: &Document,
    body: Option<BodyId>,
    feature: &DesignFeature,
    bores: ThreadedBores,
) -> kernel_api::TriMesh {
    let mut mesh = kernel_api::TriMesh::default();
    let DesignFeature::Hole {
        depth,
        through_all,
        thread_depth,
        ..
    } = feature
    else {
        return mesh;
    };
    let (into, bore, pitch) = (bores.into, bores.radius, bores.pitch);
    let radius = bore * 0.98;
    if radius <= 0.0 || pitch <= 0.0 {
        return mesh;
    }
    let solid = body.and_then(|b| document.local_geometry(b));
    let (u, v) = (bores.across, cross3(into, bores.across));
    let turn = if bores.left_handed { -1.0 } else { 1.0 };
    for mouth in bores.mouths {
        let hole_length = if *through_all {
            solid
                .as_ref()
                .and_then(|(mesh, bounds)| through_length(mesh, *bounds, mouth, into, bore))
                .unwrap_or(20.0)
        } else {
            f64::from(*depth)
        };
        let length = if *thread_depth > 0.0 {
            f64::from(*thread_depth).min(hole_length)
        } else {
            hole_length
        };
        if length <= 0.0 {
            continue;
        }
        let steps = ((length / pitch) * THREAD_SEGMENTS_PER_TURN as f64).ceil() as usize;
        let base = mesh.positions.len() as u32;
        for i in 0..=steps {
            let down = length * i as f64 / steps as f64;
            let angle = turn * std::f64::consts::TAU * down / pitch;
            let (sin, cos) = angle.sin_cos();
            let point: [f32; 3] = std::array::from_fn(|k| {
                (mouth[k] + into[k] * down + radius * (cos * u[k] + sin * v[k])) as f32
            });
            mesh.positions.push(point);
            mesh.normals.push(into.map(|c| c as f32));
            if i > 0 {
                mesh.edges.extend([base + i as u32 - 1, base + i as u32]);
            }
        }
    }
    mesh
}

/// How far a through hole starting at `start` runs along `into` before it
/// leaves the solid `mesh` (in the body's frame): to the deepest point of
/// its bore's wall, the mesh's points within `bore` of its axis, or, with
/// no bore cut there, to the far side of the solid's bounds.
fn through_length(
    mesh: &kernel_api::TriMesh,
    bounds: Option<([f32; 3], [f32; 3])>,
    start: [f64; 3],
    into: [f64; 3],
    bore: f64,
) -> Option<f64> {
    // How far along the axis a point lies, and how far off it.
    let along = |p: [f64; 3]| {
        let d: [f64; 3] = std::array::from_fn(|k| p[k] - start[k]);
        let down = d[0] * into[0] + d[1] * into[1] + d[2] * into[2];
        let off: [f64; 3] = std::array::from_fn(|k| d[k] - into[k] * down);
        (
            down,
            (off[0] * off[0] + off[1] * off[1] + off[2] * off[2]).sqrt(),
        )
    };
    let deepest = |far: Option<f64>, down: f64| Some(far.map_or(down, |f: f64| f.max(down)));
    let reach = bore * 1.01 + 1e-3;
    let wall = mesh
        .positions
        .iter()
        .map(|p| along(p.map(f64::from)))
        .filter(|(down, off)| *off <= reach && *down > 0.0)
        .map(|(down, _)| down)
        .fold(None, deepest);
    if wall.is_some() {
        return wall;
    }
    let (lo, hi) = bounds.or_else(|| mesh.bounds())?;
    (0..8)
        .map(|corner| {
            along(std::array::from_fn(|k| {
                f64::from(if corner >> k & 1 == 0 { lo[k] } else { hi[k] })
            }))
            .0
        })
        .fold(None, deepest)
        .filter(|far| *far > 0.0)
}

pub(super) fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Translate a hole feature into cut ops: the drill (and its point), then
/// what the hole cut makes of its mouth, then a modeled thread's groove.
pub(super) fn hole_ops(
    document: &Document,
    feature: &DesignFeature,
) -> Result<Vec<SolidOp>, String> {
    let DesignFeature::Hole {
        sketch,
        depth,
        through_all,
        cut,
        reversed,
        thread,
        threaded,
        modeled_thread,
        thread_depth,
        thread_length,
        drill_point,
        point_in_depth,
        nut_trap,
        ..
    } = feature
    else {
        return Err("not a hole feature".into());
    };
    let size = thread.as_ref().map(ThreadSpec::resolve).transpose()?;
    let sketch_feature = load_sketch(document, *sketch)?;
    let (plane, into) = hole_plane(document, feature, &sketch_feature)?;
    let centers = hole_centers(&sketch_feature.sketch);
    if centers.is_empty() {
        return Err("the hole sketch has no circles or points to place holes at".into());
    }
    let diameter = hole_diameter(feature);
    if diameter <= 0.0 {
        return Err("hole diameter must be positive".into());
    }
    let radius = f64::from(diameter) * 0.5;
    let taper = hole_taper_deg(feature);
    if taper.is_nan() || taper.abs() >= 45.0 {
        return Err("hole taper must lie within ±45°".into());
    }
    let tan_taper = taper.to_radians().tan();
    // The drill's radius `down` into the material.
    let radius_at = |down: f64| radius - down * tan_taper;

    let circles_profile = |radius: f64| Profile {
        plane,
        wires: centers
            .iter()
            .map(|center| ProfileWire {
                names: Vec::new(),
                segments: vec![ProfileSegment::Circle {
                    center: *center,
                    radius,
                }],
            })
            .collect(),
    };
    let cut_extrude = |termination: ExtrudeTermination, taper_deg: f64, radius: f64| {
        SolidOp::Sweep {
            profile: circles_profile(radius),
            kind: SweepKind::Extrude {
                termination,
                second_side: None,
                symmetric: false,
                // Holes drill against the sketch normal by default, like
                // pockets.
                reversed: !*reversed,
                taper_deg,
                direction: None,
            },
            op: BooleanOp::Cut,
        }
    };
    // A cone at every center, `down` into the material, from `wide` there
    // to `narrow` `height` further in.
    let cones = |down: f64, wide: f64, narrow: f64, height: f64| {
        centers
            .iter()
            .map(|center| cone_cut(&plane, *center, into, down, wide, narrow, height))
            .collect::<Vec<_>>()
    };

    let mut ops = Vec::new();
    if *through_all {
        ops.push(cut_extrude(ExtrudeTermination::ThroughAll, -taper, radius));
    } else {
        if *depth <= 0.0 {
            return Err("hole depth must be positive".into());
        }
        let depth = f64::from(*depth);
        match drill_point {
            DrillPoint::Flat => {
                if radius_at(depth) <= 0.0 {
                    return Err("the taper closes the hole before its bottom".into());
                }
                ops.push(cut_extrude(
                    ExtrudeTermination::Blind { distance: depth },
                    -taper,
                    radius,
                ));
            }
            DrillPoint::Angled { angle_deg } => {
                if *angle_deg <= 0.0 || *angle_deg >= 180.0 {
                    return Err("the drill point angle must be in (0, 180) degrees".into());
                }
                let tan_half = (f64::from(*angle_deg) * 0.5).to_radians().tan();
                // The point below a wall ending at radius ρ is ρ / tan(half)
                // tall; within the depth, wall and point share it.
                let wall = if *point_in_depth {
                    (depth - radius / tan_half) / (1.0 - tan_taper / tan_half)
                } else {
                    depth
                };
                if wall <= 0.0 {
                    return Err(
                        "the drill point is deeper than the hole; deepen it or put the point \
                         below the depth"
                            .into(),
                    );
                }
                let bottom = radius_at(wall);
                if bottom <= 0.0 {
                    return Err("the taper closes the hole before its bottom".into());
                }
                ops.push(cut_extrude(
                    ExtrudeTermination::Blind { distance: wall },
                    -taper,
                    radius,
                ));
                ops.extend(cones(wall, bottom, 0.0, bottom / tan_half));
            }
        }
    }

    let cut = match cut {
        HoleCut::Seat { seat } => {
            let (spec, size) = thread
                .as_ref()
                .zip(size.as_ref())
                .filter(|(spec, _)| spec.standard.is_metric())
                .ok_or("a screw seat needs the hole sized from an ISO metric thread")?;
            seat.cut(size.major)
                .ok_or_else(|| format!("there is no {} for {}", seat.label(), spec.size))?
        }
        other => *other,
    };
    match cut {
        HoleCut::None | HoleCut::Seat { .. } => {}
        HoleCut::Counterbore {
            diameter: cb_diameter,
            depth: cb_depth,
        }
        | HoleCut::Spotface {
            diameter: cb_diameter,
            depth: cb_depth,
        } => {
            let name = if matches!(cut, HoleCut::Spotface { .. }) {
                "spotface"
            } else {
                "counterbore"
            };
            if cb_diameter <= diameter {
                return Err(format!("{name} diameter must exceed the hole diameter"));
            }
            if cb_depth <= 0.0 {
                return Err(format!("{name} depth must be positive"));
            }
            ops.push(cut_extrude(
                ExtrudeTermination::Blind {
                    distance: f64::from(cb_depth),
                },
                0.0,
                f64::from(cb_diameter) * 0.5,
            ));
        }
        HoleCut::Countersink {
            diameter: cs_diameter,
            angle_deg,
        } => {
            if cs_diameter <= diameter {
                return Err("countersink diameter must exceed the hole diameter".into());
            }
            if angle_deg <= 0.0 || angle_deg >= 180.0 {
                return Err("countersink angle must be in (0, 180) degrees".into());
            }
            // The cone runs from the countersink diameter at the surface down
            // to the hole diameter; its depth follows from the angle.
            let half_angle = f64::from(angle_deg) * 0.5;
            let cs_depth = (f64::from(cs_diameter) - f64::from(diameter)) * 0.5
                / half_angle.to_radians().tan();
            ops.push(cut_extrude(
                ExtrudeTermination::Blind { distance: cs_depth },
                -half_angle,
                f64::from(cs_diameter) * 0.5,
            ));
        }
        HoleCut::Counterdrill {
            diameter: cd_diameter,
            depth: cd_depth,
            angle_deg,
        } => {
            if cd_depth <= 0.0 {
                return Err("counterdrill depth must be positive".into());
            }
            if angle_deg <= 0.0 || angle_deg >= 180.0 {
                return Err("counterdrill angle must be in (0, 180) degrees".into());
            }
            let wide = f64::from(cd_diameter) * 0.5;
            let cd_depth = f64::from(cd_depth);
            let narrow = radius_at(cd_depth);
            if wide <= narrow || cd_diameter <= diameter {
                return Err("counterdrill diameter must exceed the hole diameter".into());
            }
            let tan_half = (f64::from(angle_deg) * 0.5).to_radians().tan();
            ops.push(cut_extrude(
                ExtrudeTermination::Blind { distance: cd_depth },
                0.0,
                wide,
            ));
            ops.extend(cones(cd_depth, wide, narrow, (wide - narrow) / tan_half));
        }
    }
    if let Some(trap) = nut_trap {
        let metric = thread.as_ref().is_some_and(|t| t.standard.is_metric());
        let nominal = size.as_ref().filter(|_| metric).map(|s| s.major);
        let pocket = nut_pocket(trap, nominal)?;
        if pocket.across_flats <= f64::from(diameter) {
            return Err(format!(
                "the nut trap, {:.2} mm across its flats, must be wider than the hole",
                pocket.across_flats
            ));
        }
        let down = match trap.side {
            NutSide::Top => 0.0,
            NutSide::Bottom if *through_all => {
                return Err(
                    "a nut trap at the bottom needs the hole's depth: give the hole \
                     a depth (where the nut sits) rather than through all"
                        .into(),
                );
            }
            NutSide::Bottom => {
                let depth = f64::from(*depth);
                if pocket.depth >= depth {
                    return Err(format!(
                        "the nut trap, {:.2} mm deep, must be shallower than the hole",
                        pocket.depth
                    ));
                }
                depth - pocket.depth
            }
        };
        let mut at = plane;
        for (origin, into) in at.origin.iter_mut().zip(into) {
            *origin += into * down;
        }
        ops.push(SolidOp::Sweep {
            profile: Profile {
                plane: at,
                wires: centers
                    .iter()
                    .map(|center| ProfileWire {
                        names: Vec::new(),
                        segments: hexagon(*center, pocket.across_flats, trap.turn_deg),
                    })
                    .collect(),
            },
            kind: SweepKind::Extrude {
                termination: ExtrudeTermination::Blind {
                    distance: pocket.depth,
                },
                second_side: None,
                symmetric: false,
                reversed: !*reversed,
                taper_deg: 0.0,
                direction: None,
            },
            op: BooleanOp::Cut,
        });
    }
    if *threaded && *modeled_thread {
        let (spec, size) = thread
            .as_ref()
            .zip(size)
            .ok_or("a modeled thread needs a standard size")?;
        // A through hole's whole depth: as far as the solid reaches.
        let hole_depth = if *through_all {
            document
                .get_feature_meta(*sketch)
                .and_then(|n| n.body)
                .and_then(|b| document.imported_geometry(b))
                .and_then(|g| g.bounds_mm)
                .map(|(lo, hi)| {
                    let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
                    f64::from((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
                })
                .ok_or("a through hole's thread needs the solid built first, or a depth given")?
        } else {
            f64::from(*depth)
        };
        let length = match thread_length {
            crate::feature::ThreadLength::Given => f64::from(*thread_depth),
            crate::feature::ThreadLength::HoleDepth => hole_depth,
            crate::feature::ThreadLength::RunOut => hole_depth - 3.0 * size.pitch,
        };
        if length <= 0.0 {
            return Err("give the modeled thread a depth".into());
        }
        let form = ThreadForm {
            wall: radius,
            major: (size.major + spec.allowance(&size)) * 0.5,
            pitch: size.pitch,
            depth: length,
            flank_deg: spec.standard.flank_angle_deg(),
            taper_deg: taper,
            left_handed: spec.left_handed,
        };
        for center in &centers {
            ops.push(thread_cut(&plane, *center, into, &form));
        }
    }
    Ok(ops)
}

/// The plane a hole starts drilling on, in its body's frame, and the way
/// it drills into the material (against the sketch normal unless
/// reversed): the sketch's plane carried along the drill to where the
/// hole's start puts it.
pub(crate) fn hole_plane(
    document: &Document,
    feature: &DesignFeature,
    sketch: &SketchFeature,
) -> Result<(kernel_api::ProfilePlane, [f64; 3]), String> {
    let DesignFeature::Hole {
        reversed, start, ..
    } = feature
    else {
        return Err("not a hole feature".into());
    };
    let mut plane = profile::plane_of(&sketch.plane);
    let into = if *reversed {
        plane.normal
    } else {
        plane.normal.map(|c| -c)
    };
    if start.moves() {
        let way = normalize(into).map_err(|_| "the hole's sketch has no normal".to_string())?;
        let to_start = match (&start.face, start.at) {
            (Some(face), _) => distance_to_plane(plane.origin, way, face_pick_plane(face))?,
            (None, Some(at)) => distance_to_plane(plane.origin, way, start_plane(document, at)?)?,
            (None, None) => 0.0,
        };
        let s = to_start + f64::from(start.offset);
        for (o, w) in plane.origin.iter_mut().zip(way) {
            *o += w * s;
        }
    }
    Ok((plane, into))
}

/// A cone cut `down` along `into` below a hole center: `wide` in radius
/// there, `narrow` `height` further in.
/// A nut trap's pocket: across its flats and how deep, mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NutPocket {
    pub across_flats: f64,
    pub depth: f64,
}

/// The pocket `trap` cuts for a screw of `nominal` diameter (the hole's
/// metric thread's major diameter, when it has one): the nut's across
/// flats and thickness, each with the clearance, unless the trap gives
/// its own.
pub fn nut_pocket(trap: &NutTrap, nominal: Option<f64>) -> Result<NutPocket, String> {
    let clearance = f64::from(trap.clearance);
    if clearance < 0.0 {
        return Err("the nut trap's clearance must not be negative".into());
    }
    let nut = nominal.and_then(|d| trap.standard.nut(d));
    let across_flats = match (trap.across_flats, nut) {
        (Some(own), _) => f64::from(own),
        (None, Some(nut)) => nut.across_flats + clearance,
        (None, None) => {
            return Err(format!(
                "a nut trap needs the hole sized from an ISO metric thread with a {} nut, or \
                 an across-flats of its own",
                trap.standard.label()
            ));
        }
    };
    let depth = match (trap.depth, nut) {
        (Some(own), _) => f64::from(own),
        (None, Some(nut)) => nut.thickness + clearance,
        (None, None) => {
            return Err("a nut trap of its own size needs its depth".into());
        }
    };
    if across_flats <= 0.0 || depth <= 0.0 {
        return Err("the nut trap's size and depth must be positive".into());
    }
    Ok(NutPocket {
        across_flats,
        depth,
    })
}

/// A regular hexagon about `center`, `across_flats` wide between
/// opposite sides, a corner turned `turn_deg` from the X axis.
fn hexagon(center: [f64; 2], across_flats: f64, turn_deg: f32) -> Vec<ProfileSegment> {
    let radius = across_flats / 3f64.sqrt();
    let corner = |i: usize| {
        let a = f64::from(turn_deg).to_radians() + i as f64 * std::f64::consts::FRAC_PI_3;
        [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
    };
    (0..6)
        .map(|i| ProfileSegment::Line {
            start: corner(i),
            end: corner((i + 1) % 6),
        })
        .collect()
}

fn cone_cut(
    plane: &kernel_api::ProfilePlane,
    center: [f64; 2],
    into: [f64; 3],
    down: f64,
    wide: f64,
    narrow: f64,
    height: f64,
) -> SolidOp {
    let at = |k: usize| {
        plane.origin[k] + plane.x_axis[k] * center[0] + plane.y_axis[k] * center[1] + into[k] * down
    };
    SolidOp::Primitive {
        kind: kernel_api::PrimitiveKind::Cone {
            radius1: wide,
            radius2: narrow,
            height,
            angle_deg: 360.0,
        },
        placement: kernel_api::Placement {
            origin: [at(0), at(1), at(2)],
            x_axis: plane.x_axis,
            z_axis: into,
        },
        op: BooleanOp::Cut,
    }
}

/// The internal thread a hole's wall takes.
struct ThreadForm {
    /// The drilled wall's radius at the face.
    wall: f64,
    /// The thread's major radius at the face.
    major: f64,
    pitch: f64,
    /// How far into the material it runs.
    depth: f64,
    /// The included angle between its flanks, degrees.
    flank_deg: f64,
    /// How far its diameters lean in toward the bottom, degrees.
    taper_deg: f64,
    left_handed: bool,
}

/// The groove of an internal thread, cut into a hole's wall: a tooth
/// space of the form's flank angle from inside the drilled wall out to the
/// thread's major radius, flat-topped a pitch's eighth wide there, swept
/// along a helix at the pitch (narrowing along a cone as the wall tapers)
/// from a pitch above the surface to the form's depth into the material.
fn thread_cut(
    plane: &kernel_api::ProfilePlane,
    center: [f64; 2],
    into: [f64; 3],
    form: &ThreadForm,
) -> SolidOp {
    let pitch = form.pitch;
    let at = |k: usize| plane.origin[k] + plane.x_axis[k] * center[0] + plane.y_axis[k] * center[1];
    let x = plane.x_axis;
    let normal = [
        x[1] * into[2] - x[2] * into[1],
        x[2] * into[0] - x[0] * into[2],
        x[0] * into[1] - x[1] * into[0],
    ];
    // The section, in a plane through the hole's axis: x out from the
    // axis, y down it.
    let section = kernel_api::ProfilePlane {
        origin: [at(0), at(1), at(2)],
        x_axis: x,
        y_axis: into,
        normal,
    };
    // The sweep starts a pitch above the face, where a taper leaves every
    // radius wider by a pitch's worth of it.
    let widen = pitch * form.taper_deg.to_radians().tan();
    let wall = form.wall + widen;
    let major = form.major + widen;
    let inner = (wall - 0.1 * pitch).max(0.1 * wall);
    let crest = pitch / 16.0;
    let root =
        (crest + (major - inner) * (form.flank_deg * 0.5).to_radians().tan()).min(0.45 * pitch);
    let start = -pitch;
    let corners = [
        [inner, start - root],
        [major, start - crest],
        [major, start + crest],
        [inner, start + root],
    ];
    let segments = (0..4)
        .map(|i| ProfileSegment::Line {
            start: corners[i],
            end: corners[(i + 1) % 4],
        })
        .collect();
    SolidOp::Sweep {
        profile: Profile {
            plane: section,
            wires: vec![ProfileWire::new(segments)],
        },
        kind: SweepKind::Helix {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            pitch,
            height: form.depth + pitch,
            left_handed: form.left_handed,
            cone_angle_deg: -form.taper_deg,
            reversed: false,
            turns: None,
            growth: None,
        },
        op: BooleanOp::Cut,
    }
}
