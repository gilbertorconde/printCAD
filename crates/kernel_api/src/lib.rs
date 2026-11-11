//! printCAD's geometry interface: meshes, profiles and solid operations
//! as plain data, with no geometry code beyond what a three-point arc
//! means ([`arc`]), which every reader of a profile shares. The app's
//! kernel adapter answers it, and workbench packages build their solids
//! as [`SolidOp`] plans through `printcad-bench-api`.
//!
//! Versions follow `printcad-bench-api`'s: 0.1.x speaks the workbench
//! contract `printcad:workbench@0.1`, and a patch release only adds.
//! [`FaceSurface`] and [`ProfileSegment`], which the app sends a package,
//! are `#[non_exhaustive]` unless the `exhaustive` feature (printCAD's
//! own) is on; [`SolidOp`] and the enums inside it keep their variants'
//! fields within a minor version.

pub mod arc;
pub mod naming;
pub mod pmi;
pub use arc::ThreePointArc;
pub use naming::TopoName;
pub use pmi::{PmiDatum, PmiDimension, PmiTolerance, SemanticPmi};

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
