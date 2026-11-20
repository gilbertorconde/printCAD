//! Threads cut into a cylindrical face, and where a thread sits on the
//! face as the view draws it.

use core_document::{BodyId, Document};
use kernel_api::{FaceProbe, FaceSurface, FaceThread, SolidOp};

use crate::feature::{DesignFeature, FacePick};

/// How many pitches a run-out takes to bring the groove up to the face.
const RUN_OUT_PITCHES: f64 = 2.0;

/// A thread's pitch, depth into the material and flank angle: its
/// standard size's (the depth from its major to its minor diameter), or
/// its own.
pub(crate) fn thread_form(feature: &DesignFeature) -> Result<(f64, f64, f64), String> {
    let DesignFeature::Thread {
        thread,
        pitch,
        depth,
        ..
    } = feature
    else {
        return Err("not a thread feature".into());
    };
    match thread {
        Some(spec) => {
            if spec.standard.is_tapered() {
                return Err(format!(
                    "{} threads taper: cut them with a Hole, which tapers its bore",
                    spec.standard.label()
                ));
            }
            let size = spec.resolve()?;
            Ok((
                size.pitch,
                (size.major - size.minor) * 0.5,
                spec.standard.flank_angle_deg(),
            ))
        }
        None => {
            if *pitch <= 0.0 {
                return Err("the thread's pitch must be more than zero".into());
            }
            if *depth <= 0.0 {
                return Err("the thread's depth must be more than zero".into());
            }
            Ok((f64::from(*pitch), f64::from(*depth), 60.0))
        }
    }
}

/// Whether a thread turns left: its own switch, or its standard size's.
pub(crate) fn thread_left_handed(feature: &DesignFeature) -> bool {
    match feature {
        DesignFeature::Thread {
            thread,
            left_handed,
            ..
        } => *left_handed || thread.as_ref().is_some_and(|t| t.left_handed),
        _ => false,
    }
}

/// What a thread's callout reads: its standard size ("M10", "1/4-20
/// UNC LH"), or its own pitch.
pub(crate) fn thread_designation(feature: &DesignFeature) -> Option<String> {
    let DesignFeature::Thread { thread, pitch, .. } = feature else {
        return None;
    };
    let mut name = match thread {
        Some(spec) => {
            let mut spec = spec.clone();
            // An external thread's class is not a bore's: the size alone.
            spec.class = String::new();
            spec.left_handed = false;
            let full = spec.designation();
            match spec.standard.default_class() {
                "" => full,
                class => full
                    .strip_suffix(&format!("-{class}"))
                    .unwrap_or(&full)
                    .to_string(),
            }
        }
        None => format!("P{pitch}"),
    };
    if thread_left_handed(feature) {
        name.push_str(" LH");
    }
    Some(name)
}

/// The size of `standard` nearest `diameter`: by its major diameter on a
/// shaft, by its minor on a bore.
pub(crate) fn nearest_thread_size(
    standard: crate::hole_tables::ThreadStandard,
    diameter: f64,
    external: bool,
) -> String {
    standard
        .sizes()
        .into_iter()
        .min_by(|a, b| {
            let off = |s: &crate::hole_tables::ThreadSize| {
                let d = if external { s.major } else { s.minor };
                (d - diameter).abs()
            };
            off(a).total_cmp(&off(b))
        })
        .map_or_else(String::new, |s| s.name.to_string())
}

/// The op a thread cuts: none for a cosmetic one, which is only drawn.
pub(super) fn thread_ops(feature: &DesignFeature) -> Result<Vec<SolidOp>, String> {
    let DesignFeature::Thread {
        face,
        length,
        full_length,
        from_far_end,
        chamfer,
        run_out,
        cosmetic,
        ..
    } = feature
    else {
        return Err("not a thread feature".into());
    };
    let (pitch, depth, flank_deg) = thread_form(feature)?;
    if *cosmetic {
        return Ok(Vec::new());
    }
    if !*full_length && *length <= 0.0 {
        return Err("the thread's length must be more than zero".into());
    }
    Ok(vec![SolidOp::Thread {
        face: probe_of(face),
        form: FaceThread {
            pitch,
            depth,
            flank_deg,
            length: (!*full_length).then_some(f64::from(*length)),
            from_far_end: *from_far_end,
            left_handed: thread_left_handed(feature),
            chamfer: *chamfer,
            run_out: if *run_out {
                RUN_OUT_PITCHES * pitch
            } else {
                0.0
            },
        },
    }])
}

fn probe_of(face: &FacePick) -> FaceProbe {
    FaceProbe {
        point: face.point.map(f64::from),
        normal: face.normal.map(f64::from),
        name: face.name,
    }
}

/// Where a thread sits on its face, in its body's frame, as the body's
/// built solid shows the face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ThreadSeat {
    /// The axis where the thread starts, at the face's starting end.
    pub start: [f64; 3],
    /// Along the axis, into the face.
    pub into: [f64; 3],
    /// Square to the axis, toward where the face was picked.
    pub across: [f64; 3],
    pub radius: f64,
    /// How far the thread runs along the face.
    pub length: f64,
    /// The face's own length along its axis.
    pub face_length: f64,
    /// A shaft's face, not a bore's.
    pub external: bool,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Where thread `feature` of `body` sits: on the cylinder of the body's
/// solid nearest where its face was picked, as long as the solid's faces
/// on that cylinder reach; `None` when the solid has no cylinder there.
pub(crate) fn thread_seat(
    document: &Document,
    body: BodyId,
    feature: &DesignFeature,
) -> Option<ThreadSeat> {
    let DesignFeature::Thread {
        face,
        length,
        full_length,
        from_far_end,
        ..
    } = feature
    else {
        return None;
    };
    let (mesh, _) = document.local_geometry(body)?;
    let cylinder = |t: usize| -> Option<([f64; 3], [f64; 3], f64)> {
        let index = *mesh.faces.get(t)? as usize;
        match mesh.face_surfaces.get(index)? {
            FaceSurface::Cylinder {
                origin,
                axis,
                radius,
            } if *radius > 0.0 => {
                let axis = axis.map(f64::from);
                let len = dot(axis, axis).sqrt();
                (len > 0.0).then(|| {
                    (
                        origin.map(f64::from),
                        axis.map(|c| c / len),
                        f64::from(*radius),
                    )
                })
            }
            _ => None,
        }
    };
    let triangles = mesh.indices.as_chunks::<3>().0;
    let corner = |i: u32| mesh.positions[i as usize].map(f64::from);
    let picked = face.point.map(f64::from);
    // How far the picked point lies from a triangle.
    let near = |tri: &[u32; 3]| {
        let [a, b, c] = tri.map(|i| glam::Vec3::from_array(mesh.positions[i as usize]));
        core_document::mesh_query::triangle_distance_sq(glam::Vec3::from_array(face.point), a, b, c)
    };
    let (_, (origin, axis, radius)) = triangles
        .iter()
        .enumerate()
        .filter_map(|(t, tri)| Some((near(tri), cylinder(t)?)))
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    let same = |(o, a, r): ([f64; 3], [f64; 3], f64)| {
        let off: [f64; 3] = std::array::from_fn(|k| o[k] - origin[k]);
        let along = dot(off, axis);
        let square: [f64; 3] = std::array::from_fn(|k| off[k] - axis[k] * along);
        dot(a, axis).abs() > 0.9999
            && dot(square, square).sqrt() < 1e-3 * radius.max(1.0)
            && (r - radius).abs() < 1e-3 * radius.max(1.0)
    };
    let along = |p: [f64; 3]| dot(std::array::from_fn(|k| p[k] - origin[k]), axis);
    let (lo, hi) = triangles
        .iter()
        .enumerate()
        .filter(|(t, _)| cylinder(*t).is_some_and(same))
        .flat_map(|(_, tri)| tri.map(|i| along(corner(i))))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), t| {
            (lo.min(t), hi.max(t))
        });
    if hi <= lo {
        return None;
    }
    let at = along(picked);
    let radial: [f64; 3] = std::array::from_fn(|k| picked[k] - origin[k] - axis[k] * at);
    let reach = dot(radial, radial).sqrt();
    let across = if reach > 1e-9 {
        radial.map(|c| c / reach)
    } else {
        glam::DVec3::from_array(axis)
            .any_orthonormal_vector()
            .to_array()
    };
    let near_low = (at - lo).abs() <= (hi - at).abs();
    let (from, into) = if near_low != *from_far_end {
        (lo, axis)
    } else {
        (hi, axis.map(|c| -c))
    };
    let whole = hi - lo;
    Some(ThreadSeat {
        start: std::array::from_fn(|k| origin[k] + axis[k] * from),
        into,
        across,
        radius,
        length: if *full_length {
            whole
        } else {
            f64::from(*length).min(whole)
        },
        face_length: whole,
        external: dot(face.normal.map(f64::from), across) > 0.0,
    })
}

/// A cosmetic thread as lines in its body's frame: a helix of its pitch
/// along its face, just off the face so the face does not hide it. Empty
/// for a thread that is cut, or whose face the solid does not show.
pub(crate) fn thread_lines(
    document: &Document,
    body: BodyId,
    feature: &DesignFeature,
) -> kernel_api::TriMesh {
    let mut mesh = kernel_api::TriMesh::default();
    let DesignFeature::Thread { cosmetic: true, .. } = feature else {
        return mesh;
    };
    let (Some(seat), Ok((pitch, _, _))) =
        (thread_seat(document, body, feature), thread_form(feature))
    else {
        return mesh;
    };
    let radius = if seat.external {
        seat.radius * 1.01 + 0.02
    } else {
        seat.radius * 0.98
    };
    let u = seat.across;
    let v = super::cross3(seat.into, u);
    let turn = if thread_left_handed(feature) {
        -1.0
    } else {
        1.0
    };
    let steps = ((seat.length / pitch) * super::holes::THREAD_SEGMENTS_PER_TURN as f64)
        .ceil()
        .max(1.0) as usize;
    for i in 0..=steps {
        let down = seat.length * i as f64 / steps as f64;
        let angle = turn * std::f64::consts::TAU * down / pitch;
        let (sin, cos) = angle.sin_cos();
        let point: [f32; 3] = std::array::from_fn(|k| {
            (seat.start[k] + seat.into[k] * down + radius * (cos * u[k] + sin * v[k])) as f32
        });
        mesh.positions.push(point);
        mesh.normals.push(seat.into.map(|c| c as f32));
        if i > 0 {
            mesh.edges.extend([i as u32 - 1, i as u32]);
        }
    }
    mesh
}
