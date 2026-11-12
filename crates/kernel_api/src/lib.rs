//! printCAD's geometry interface: meshes, profiles and solid operations
//! as plain data, with no geometry code beyond what a three-point arc
//! means ([`arc`]), which every reader of a profile shares, and the
//! nearest point of a segment or a triangle ([`nearest`]), which every
//! pick and snap shares, where a ray crosses a triangle ([`ray`]), a
//! polygon's area and what it holds ([`polygon`]), and the box around a
//! mesh moved by a placement ([`Aabb`]). The app's
//! kernel adapter answers it, and workbench packages build their solids
//! as [`SolidOp`] plans through `printcad-bench-api`.
//!
//! Versions follow `printcad-bench-api`'s: 0.1.x speaks the workbench
//! contract `printcad:workbench@0.1`, and a patch release only adds.
//! [`FaceSurface`] and [`ProfileSegment`], which the app sends a package,
//! are `#[non_exhaustive]` unless the `exhaustive` feature (printCAD's
//! own) is on; [`SolidOp`] and the enums inside it keep their variants'
//! fields within a minor version.

pub mod aabb;
pub mod arc;
pub mod naming;
pub mod nearest;
pub mod pmi;
pub mod polygon;
pub mod ray;
pub use aabb::Aabb;
pub use arc::ThreePointArc;
pub use naming::TopoName;
pub use pmi::{PmiDatum, PmiDimension, PmiTolerance, SemanticPmi};

/// How near two ends of curves are one point, in millimetres, wherever
/// curves cross between the kernel and its callers: consecutive segments
/// of a [`ProfileWire`] meet within it, the kernel chains a section's
/// curves ([`SectionRun`]) within it, and ends read off a solid's edges
/// (sectioned, borrowed or projected) that lie within it are one point.
/// A solid's edges meet at a vertex only within its tolerance, which an
/// imported or fuzzily joined solid widens, and points read off its mesh
/// are single precision. A caller deciding what connects on its own
/// terms, as the sketcher's regions do, joins no wider than this, so
/// whatever it closes the kernel closes too.
pub const JOIN_MM: f64 = 1e-3;

mod chain;
mod fit;
mod import;
mod kernel;
mod mesh;
mod mesh_op;
mod point_match;
mod queries;
mod query;
mod solid_op;

pub use chain::*;
pub use fit::*;
pub use import::*;
pub use kernel::*;
pub use mesh::*;
pub use mesh_op::*;
pub use point_match::*;
pub use queries::*;
pub use query::*;
pub use solid_op::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn pipe_json(frame: &str) -> String {
        let plane = r#"{"origin":[0,0,0],"x_axis":[1,0,0],"y_axis":[0,1,0],"normal":[0,0,1]}"#;
        let profile = format!(
            r#"{{"plane":{plane},"wires":[{{"segments":[{{"Circle":{{"center":[0,0],"radius":1}}}}]}}]}}"#
        );
        format!(r#"{{"Pipe":{{"profile":{profile},"spine":{profile},{frame}"op":"NewSolid"}}}}"#)
    }

    fn frame_of(json: &str) -> PipeFrame {
        match serde_json::from_str::<SolidOp>(json).unwrap() {
            SolidOp::Pipe { frame, .. } => frame,
            other => panic!("not a pipe: {other:?}"),
        }
    }

    #[test]
    fn a_pipe_written_with_the_frenet_flag_reads_as_its_frame() {
        assert_eq!(frame_of(&pipe_json(r#""frenet":true,"#)), PipeFrame::Frenet);
        assert_eq!(
            frame_of(&pipe_json(r#""frenet":false,"#)),
            PipeFrame::RotationMinimizing
        );
        assert_eq!(frame_of(&pipe_json("")), PipeFrame::RotationMinimizing);
    }

    #[test]
    fn a_pipe_frame_round_trips() {
        let frame = PipeFrame::Binormal {
            direction: [0.0, 0.0, 1.0],
        };
        let text = serde_json::to_string(&frame).unwrap();
        assert_eq!(frame_of(&pipe_json(&format!(r#""frame":{text},"#))), frame);
    }
}
