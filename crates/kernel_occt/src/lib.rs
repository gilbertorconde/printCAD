//! OpenCASCADE-backed implementation of the [`Kernel`] trait.
//!
//! The current scope is intentionally minimal: STEP/STP files can be imported
//! and tessellated for viewport display. Boolean/feature operations remain to
//! be wired in as the kernel matures.

mod ffi;
mod step_header;

use std::ffi::{CStr, CString};
use std::path::Path;
use std::time::Instant;

use kernel_api::{
    BodyHandle, BoolKind, BooleanOp, ChainError, ChamferSpec, EdgeSelection, ExtrudeTermination,
    ImportedBody, ImportedModel, ImportedNode, ImportedNodeKind, Kernel, KernelError, KernelResult,
    LinearDeflectionMode, PrimitiveKind, Profile, ProfilePlane, ProfileSegment, RebuildRequest,
    RebuildResponse, SolidBuildResult, SolidOp, SweepKind, TessellationSettings, TriMesh,
};
use tracing::info;

use std::os::raw::{c_double, c_int};

fn tessellation_linear_for_ffi(detail: &TessellationSettings) -> (c_int, c_double) {
    match detail.linear_deflection_mode {
        LinearDeflectionMode::BboxScaled => (0, detail.mesh_deviation.max(0.001) as c_double),
        LinearDeflectionMode::AbsoluteMm => (1, detail.chord_tolerance.max(0.001) as c_double),
    }
}

fn boundary_edges_for_ffi(detail: &TessellationSettings) -> c_int {
    if detail.generate_boundary_edges {
        1
    } else {
        0
    }
}

pub use step_header::detect_step_unit;

/// OpenCASCADE-backed kernel.
pub struct OcctKernel {
    initialized: bool,
}

impl Default for OcctKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl OcctKernel {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Tessellates one imported BRep body using a pre-snapshotted per-face RGB
    /// table (same order as the OCCT face explorer on the original shape).
    pub fn tessellate_step_brep(
        &self,
        brep_blob: &[u8],
        face_colors: &[[f32; 3]],
        detail: &TessellationSettings,
    ) -> KernelResult<TriMesh> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        tessellate_brep_internal(brep_blob, face_colors, detail)
    }

    /// Read + transfer + `BRepMesh` + extract in one synchronous shot (legacy path).
    /// Useful for tests comparing triangle counts against the deferred pipeline.
    pub fn import_step_full_mesh(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        self.initialize()?;
        import_step_full_mesh_internal(path, detail)
    }

    /// Execute a body's solid-op chain: the first op must be a
    /// shape-producing NewSolid; each subsequent op fuses/cuts a new tool
    /// solid against the accumulated solid or modifies it directly
    /// (dress-ups, patterns, booleans). Errors are attributed to the failing
    /// op's chain index. Returns the final solid's BRep snapshot + render
    /// mesh.
    pub fn execute_solid_chain(
        &mut self,
        ops: &[SolidOp],
        detail: &TessellationSettings,
    ) -> Result<SolidBuildResult, ChainError> {
        self.initialize().map_err(|e| ChainError {
            op_index: 0,
            message: e.to_string(),
        })?;
        execute_solid_chain_internal(ops, detail)
    }
}

impl Kernel for OcctKernel {
    fn name(&self) -> &str {
        "OpenCascade"
    }

    fn initialize(&mut self) -> KernelResult<()> {
        if !self.initialized {
            info!("Initializing OCCT kernel");
            self.initialized = true;
        }
        Ok(())
    }

    fn rebuild(&mut self, _request: &RebuildRequest) -> KernelResult<RebuildResponse> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        Ok(RebuildResponse::default())
    }

    fn tessellate(
        &self,
        _body: BodyHandle,
        _detail: &TessellationSettings,
    ) -> KernelResult<TriMesh> {
        if !self.initialized {
            return Err(KernelError::NotInitialized);
        }
        Ok(TriMesh::default())
    }

    fn import_step(
        &mut self,
        path: &Path,
        detail: &TessellationSettings,
    ) -> KernelResult<ImportedModel> {
        self.initialize()?;
        import_step_brep_only_internal(path, detail)
    }
}

fn tessellate_brep_internal(
    brep_blob: &[u8],
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
) -> KernelResult<TriMesh> {
    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();
    let gen_edges = boundary_edges_for_ffi(detail);

    let mut flat_colors: Vec<f32> = Vec::with_capacity(face_colors.len() * 3);
    for c in face_colors {
        flat_colors.extend_from_slice(c);
    }

    let result = unsafe {
        ffi::printcad_occt_tessellate_brep(
            brep_blob.as_ptr(),
            brep_blob.len(),
            flat_colors.as_ptr(),
            face_colors.len(),
            linear_mode,
            linear_value,
            angular,
            weld_cross_face,
            weld_angle_rad,
            gen_edges,
        )
    };

    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_result(result) };
        return Err(KernelError::Import(message));
    }

    let mesh = if result.body_count > 0 && !result.bodies.is_null() {
        let body = unsafe { &*result.bodies };
        tri_mesh_from_occt_body(body, detail)
    } else {
        TriMesh::default()
    };

    unsafe { ffi::printcad_occt_free_result(result) };

    if mesh.positions.is_empty() {
        return Err(KernelError::Import(
            "tessellation returned an empty mesh".into(),
        ));
    }

    Ok(mesh)
}

fn profile_plane_to_ffi(plane: &ProfilePlane) -> ffi::PcadProfilePlane {
    ffi::PcadProfilePlane {
        origin: plane.origin,
        x_axis: plane.x_axis,
        y_axis: plane.y_axis,
        normal: plane.normal,
    }
}

fn boolean_op_to_ffi(op: BooleanOp) -> i32 {
    match op {
        BooleanOp::NewSolid => 0,
        BooleanOp::Fuse => 1,
        BooleanOp::Cut => 2,
    }
}

/// Owned FFI marshalling for one or more profiles. Variable-length segment
/// payloads (B-spline control points, ellipse-arc params) live in separately
/// boxed slices so their addresses stay stable while the outer vectors grow.
#[derive(Default)]
struct ProfileFfi {
    extras: Vec<Box<[f64]>>,
    segments: Vec<Vec<ffi::PcadProfileSegment>>,
    wires: Vec<ffi::PcadProfileWire>,
    planes: Vec<ffi::PcadProfilePlane>,
    wire_offsets: Vec<usize>,
    wire_counts: Vec<usize>,
}

impl ProfileFfi {
    fn new(profile: &Profile) -> Self {
        let mut ffi = Self::default();
        ffi.push_profile(profile);
        ffi
    }

    fn from_sections(sections: &[Profile]) -> Self {
        let mut ffi = Self::default();
        for section in sections {
            ffi.push_profile(section);
        }
        ffi
    }

    fn push_profile(&mut self, profile: &Profile) {
        self.planes.push(profile_plane_to_ffi(&profile.plane));
        self.wire_offsets.push(self.segments.len());
        self.wire_counts.push(profile.wires.len());
        for wire in &profile.wires {
            let segments: Vec<ffi::PcadProfileSegment> = wire
                .segments
                .iter()
                .map(|segment| self.segment_to_ffi(segment))
                .collect();
            self.segments.push(segments);
        }
        // Wire descriptors are rebuilt last so segment vectors are final.
        self.wires = self
            .segments
            .iter()
            .map(|segments| ffi::PcadProfileWire {
                segments: segments.as_ptr(),
                count: segments.len(),
            })
            .collect();
    }

    fn segment_to_ffi(&mut self, segment: &ProfileSegment) -> ffi::PcadProfileSegment {
        let no_extra = (std::ptr::null(), 0usize);
        let (kind, d, (extra, extra_count)) = match segment {
            ProfileSegment::Line { start, end } => {
                (0, [start[0], start[1], end[0], end[1], 0.0, 0.0], no_extra)
            }
            ProfileSegment::Arc { start, mid, end } => (
                1,
                [start[0], start[1], mid[0], mid[1], end[0], end[1]],
                no_extra,
            ),
            ProfileSegment::Circle { center, radius } => {
                (2, [center[0], center[1], *radius, 0.0, 0.0, 0.0], no_extra)
            }
            ProfileSegment::Ellipse {
                center,
                major,
                ratio,
            } => (
                3,
                [center[0], center[1], major[0], major[1], *ratio, 0.0],
                no_extra,
            ),
            ProfileSegment::EllipseArc {
                center,
                major,
                ratio,
                start_param,
                end_param,
            } => (
                4,
                [center[0], center[1], major[0], major[1], *ratio, 0.0],
                self.stash_extra(vec![*start_param, *end_param]),
            ),
            ProfileSegment::BSpline {
                control_points,
                periodic,
            } => (
                5,
                [if *periodic { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0, 0.0, 0.0],
                self.stash_extra(control_points.iter().flatten().copied().collect()),
            ),
        };
        ffi::PcadProfileSegment {
            kind,
            d,
            extra,
            extra_count,
        }
    }

    fn stash_extra(&mut self, values: Vec<f64>) -> (*const c_double, usize) {
        let boxed: Box<[f64]> = values.into_boxed_slice();
        let ptr = boxed.as_ptr();
        let len = boxed.len();
        self.extras.push(boxed);
        (ptr, len)
    }
}

fn mesh_options_for_ffi(detail: &TessellationSettings, want_mesh: bool) -> ffi::PcadMeshOptions {
    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    ffi::PcadMeshOptions {
        want_mesh: if want_mesh { 1 } else { 0 },
        linear_deflection_mode: linear_mode,
        linear_value,
        angular_deflection_rad: (detail.angular_tolerance_deg.max(0.5) as f64).to_radians(),
        weld_cross_face: if detail.weld_cross_face { 1 } else { 0 },
        weld_angle_threshold_rad: (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians(),
        generate_boundary_edges: boundary_edges_for_ffi(detail),
    }
}

fn termination_to_ffi(term: &ExtrudeTermination) -> ffi::PcadTermination {
    match *term {
        ExtrudeTermination::Blind { distance } => ffi::PcadTermination {
            kind: 0,
            distance,
            ..Default::default()
        },
        ExtrudeTermination::ThroughAll => ffi::PcadTermination {
            kind: 1,
            ..Default::default()
        },
        ExtrudeTermination::UpToPlane {
            point,
            normal,
            offset,
        } => ffi::PcadTermination {
            kind: 2,
            plane_point: point,
            plane_normal: normal,
            offset,
            ..Default::default()
        },
        ExtrudeTermination::ToFirst => ffi::PcadTermination {
            kind: 3,
            ..Default::default()
        },
        ExtrudeTermination::ToLast => ffi::PcadTermination {
            kind: 4,
            ..Default::default()
        },
    }
}

fn sweep_desc_to_ffi(kind: &SweepKind) -> ffi::PcadSweepDesc {
    let mut desc = ffi::PcadSweepDesc {
        kind: 0,
        term: ffi::PcadTermination::default(),
        term2: ffi::PcadTermination::default(),
        has_term2: 0,
        symmetric: 0,
        reversed: 0,
        taper_deg: 0.0,
        direction: [0.0; 3],
        has_direction: 0,
        axis_origin: [0.0; 2],
        axis_dir: [0.0; 2],
        angle_deg: 0.0,
        angle2_deg: 0.0,
        has_angle2: 0,
        midplane: 0,
        pitch: 0.0,
        height: 0.0,
        cone_angle_deg: 0.0,
        left_handed: 0,
    };
    match kind {
        SweepKind::Extrude {
            termination,
            second_side,
            symmetric,
            reversed,
            taper_deg,
            direction,
        } => {
            desc.kind = 0;
            desc.term = termination_to_ffi(termination);
            if let Some(second) = second_side {
                desc.term2 = termination_to_ffi(second);
                desc.has_term2 = 1;
            }
            desc.symmetric = *symmetric as i32;
            desc.reversed = *reversed as i32;
            desc.taper_deg = *taper_deg;
            if let Some(dir) = direction {
                desc.direction = *dir;
                desc.has_direction = 1;
            }
        }
        SweepKind::Revolve {
            axis_origin,
            axis_dir,
            angle_deg,
            second_angle_deg,
            midplane,
            reversed,
        } => {
            desc.kind = 1;
            desc.axis_origin = *axis_origin;
            desc.axis_dir = *axis_dir;
            desc.angle_deg = *angle_deg;
            if let Some(second) = second_angle_deg {
                desc.angle2_deg = *second;
                desc.has_angle2 = 1;
            }
            desc.midplane = *midplane as i32;
            desc.reversed = *reversed as i32;
        }
        SweepKind::Helix {
            axis_origin,
            axis_dir,
            pitch,
            height,
            left_handed,
            cone_angle_deg,
            reversed,
        } => {
            desc.kind = 2;
            desc.axis_origin = *axis_origin;
            desc.axis_dir = *axis_dir;
            desc.pitch = *pitch;
            desc.height = *height;
            desc.left_handed = *left_handed as i32;
            desc.cone_angle_deg = *cone_angle_deg;
            desc.reversed = *reversed as i32;
        }
    }
    desc
}

fn primitive_to_ffi(kind: &PrimitiveKind) -> (i32, Vec<f64>) {
    match *kind {
        PrimitiveKind::Box {
            length,
            width,
            height,
        } => (0, vec![length, width, height]),
        PrimitiveKind::Cylinder {
            radius,
            height,
            angle_deg,
        } => (1, vec![radius, height, angle_deg]),
        PrimitiveKind::Sphere {
            radius,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => (2, vec![radius, angle1_deg, angle2_deg, angle3_deg]),
        PrimitiveKind::Cone {
            radius1,
            radius2,
            height,
            angle_deg,
        } => (3, vec![radius1, radius2, height, angle_deg]),
        PrimitiveKind::Torus {
            radius1,
            radius2,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => (
            4,
            vec![radius1, radius2, angle1_deg, angle2_deg, angle3_deg],
        ),
        PrimitiveKind::Ellipsoid {
            radius1,
            radius2,
            radius3,
        } => (5, vec![radius1, radius2, radius3]),
        PrimitiveKind::Prism {
            sides,
            circumradius,
            height,
        } => (6, vec![sides as f64, circumradius, height]),
        PrimitiveKind::Wedge {
            xmin,
            xmax,
            ymin,
            ymax,
            zmin,
            zmax,
            x2min,
            x2max,
            z2min,
            z2max,
        } => (
            7,
            vec![
                xmin, xmax, ymin, ymax, zmin, zmax, x2min, x2max, z2min, z2max,
            ],
        ),
    }
}

fn edge_selection_to_ffi(edges: &EdgeSelection) -> (i32, Vec<f64>) {
    match edges {
        EdgeSelection::All => (0, Vec::new()),
        EdgeSelection::OfFaces(points) => (1, points.iter().flatten().copied().collect()),
        EdgeSelection::Near(points) => (2, points.iter().flatten().copied().collect()),
    }
}

/// One executed chain step's reusable tool solid (`None` for modifiers).
struct ToolSnapshot {
    brep: Vec<u8>,
    subtractive: bool,
}

/// (result BRep, optional tool BRep, optional render mesh) of one chain step.
type StepOutput = (Vec<u8>, Option<Vec<u8>>, Option<TriMesh>);

fn take_sweep_result(
    result: ffi::PrintcadOcctSweepResult,
    op_index: usize,
    want_mesh: bool,
) -> Result<StepOutput, ChainError> {
    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_sweep_result(result) };
        return Err(ChainError { op_index, message });
    }
    if result.brep_blob.is_null() || result.brep_len == 0 {
        unsafe { ffi::printcad_occt_free_sweep_result(result) };
        return Err(ChainError {
            op_index,
            message: "op returned no BRep snapshot".into(),
        });
    }
    let blob = unsafe { std::slice::from_raw_parts(result.brep_blob, result.brep_len) }.to_vec();
    let tool = (!result.tool_blob.is_null() && result.tool_len > 0)
        .then(|| unsafe { std::slice::from_raw_parts(result.tool_blob, result.tool_len) }.to_vec());
    let mesh = want_mesh.then(|| TriMesh {
        positions: copy_vec3_array(result.mesh_positions, result.mesh_vertex_count),
        normals: copy_vec3_array(result.mesh_normals, result.mesh_vertex_count),
        indices: copy_u32_array(result.mesh_indices, result.mesh_index_count),
        edges: copy_u32_array(result.mesh_edges, result.mesh_edge_count * 2),
        colors: Vec::new(),
    });
    unsafe { ffi::printcad_occt_free_sweep_result(result) };
    Ok((blob, tool, mesh))
}

fn execute_solid_chain_internal(
    ops: &[SolidOp],
    detail: &TessellationSettings,
) -> Result<SolidBuildResult, ChainError> {
    let chain_err = |op_index: usize, message: &str| ChainError {
        op_index,
        message: message.into(),
    };
    if ops.is_empty() {
        return Err(chain_err(0, "solid-op chain is empty"));
    }
    match ops[0].boolean_op() {
        Some(BooleanOp::NewSolid) => {}
        Some(_) => return Err(chain_err(0, "first solid op in a chain must be NewSolid")),
        None => {
            return Err(chain_err(
                0,
                "first solid op in a chain must produce a shape",
            ))
        }
    }
    for (index, op) in ops.iter().enumerate().skip(1) {
        if op.boolean_op() == Some(BooleanOp::NewSolid) {
            return Err(chain_err(
                index,
                "only the first op in a chain may be NewSolid",
            ));
        }
    }

    let mut current_blob: Vec<u8> = Vec::new();
    let mut final_mesh = TriMesh::default();
    let mut tools: Vec<Option<ToolSnapshot>> = Vec::with_capacity(ops.len());

    for (index, solid_op) in ops.iter().enumerate() {
        let is_last = index + 1 == ops.len();
        let mesh_opts = mesh_options_for_ffi(detail, is_last);
        let (base_ptr, base_len) = if index == 0 {
            (std::ptr::null(), 0)
        } else {
            (current_blob.as_ptr(), current_blob.len())
        };

        let raw = match solid_op {
            SolidOp::Sweep { profile, kind, op } => {
                let marshalled = ProfileFfi::new(profile);
                let desc = sweep_desc_to_ffi(kind);
                unsafe {
                    ffi::printcad_occt_solid_sweep(
                        base_ptr,
                        base_len,
                        marshalled.planes.as_ptr(),
                        marshalled.wires.as_ptr(),
                        marshalled.wires.len(),
                        &desc,
                        boolean_op_to_ffi(*op),
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Loft {
                sections,
                ruled,
                closed,
                op,
            } => {
                let marshalled = ProfileFfi::from_sections(sections);
                unsafe {
                    ffi::printcad_occt_solid_loft(
                        base_ptr,
                        base_len,
                        marshalled.planes.as_ptr(),
                        marshalled.wires.as_ptr(),
                        marshalled.wire_offsets.as_ptr(),
                        marshalled.wire_counts.as_ptr(),
                        marshalled.planes.len(),
                        *ruled as c_int,
                        *closed as c_int,
                        boolean_op_to_ffi(*op),
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Pipe {
                profile,
                spine,
                frenet,
                op,
            } => {
                let profile_ffi = ProfileFfi::new(profile);
                let spine_ffi = ProfileFfi::new(spine);
                unsafe {
                    ffi::printcad_occt_solid_pipe(
                        base_ptr,
                        base_len,
                        profile_ffi.planes.as_ptr(),
                        profile_ffi.wires.as_ptr(),
                        profile_ffi.wires.len(),
                        spine_ffi.planes.as_ptr(),
                        spine_ffi.wires.as_ptr(),
                        *frenet as c_int,
                        boolean_op_to_ffi(*op),
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Primitive {
                kind,
                placement,
                op,
            } => {
                let (prim_kind, params) = primitive_to_ffi(kind);
                let placement_ffi: [f64; 9] = [
                    placement.origin[0],
                    placement.origin[1],
                    placement.origin[2],
                    placement.x_axis[0],
                    placement.x_axis[1],
                    placement.x_axis[2],
                    placement.z_axis[0],
                    placement.z_axis[1],
                    placement.z_axis[2],
                ];
                unsafe {
                    ffi::printcad_occt_solid_primitive(
                        base_ptr,
                        base_len,
                        prim_kind,
                        params.as_ptr(),
                        params.len(),
                        placement_ffi.as_ptr(),
                        boolean_op_to_ffi(*op),
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Fillet { radius, edges } => {
                let (mode, points) = edge_selection_to_ffi(edges);
                let params = [*radius];
                unsafe {
                    ffi::printcad_occt_dressup(
                        base_ptr,
                        base_len,
                        0,
                        params.as_ptr(),
                        params.len(),
                        mode,
                        points.as_ptr(),
                        points.len() / 3,
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Chamfer { spec, flip, edges } => {
                let (mode, points) = edge_selection_to_ffi(edges);
                let params = match *spec {
                    ChamferSpec::EqualDistance { distance } => [0.0, distance, 0.0, 0.0],
                    ChamferSpec::TwoDistances {
                        distance1,
                        distance2,
                    } => [1.0, distance1, distance2, *flip as i32 as f64],
                    ChamferSpec::DistanceAngle {
                        distance,
                        angle_deg,
                    } => [2.0, distance, angle_deg, *flip as i32 as f64],
                };
                unsafe {
                    ffi::printcad_occt_dressup(
                        base_ptr,
                        base_len,
                        1,
                        params.as_ptr(),
                        params.len(),
                        mode,
                        points.as_ptr(),
                        points.len() / 3,
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Draft {
                angle_deg,
                neutral_point,
                neutral_normal,
                pull_dir,
                faces,
            } => {
                let face_points: Vec<f64> = faces.iter().flatten().copied().collect();
                let pull: Option<[f64; 3]> = *pull_dir;
                unsafe {
                    ffi::printcad_occt_draft(
                        base_ptr,
                        base_len,
                        *angle_deg,
                        neutral_point.as_ptr(),
                        neutral_normal.as_ptr(),
                        pull.as_ref().map_or(std::ptr::null(), |dir| dir.as_ptr()),
                        face_points.as_ptr(),
                        face_points.len() / 3,
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Thickness {
                value,
                open_faces,
                inward,
            } => {
                let face_points: Vec<f64> = open_faces.iter().flatten().copied().collect();
                unsafe {
                    ffi::printcad_occt_thickness(
                        base_ptr,
                        base_len,
                        *value,
                        *inward as c_int,
                        face_points.as_ptr(),
                        face_points.len() / 3,
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Transform {
                transforms,
                originals,
            } => {
                // Whole-body mode (no originals) re-applies the current solid.
                let whole_body;
                let mut tool_solids: Vec<ffi::PcadToolSolid> = Vec::new();
                if originals.is_empty() {
                    whole_body = current_blob.clone();
                    tool_solids.push(ffi::PcadToolSolid {
                        brep: whole_body.as_ptr(),
                        len: whole_body.len(),
                        subtractive: 0,
                    });
                } else {
                    for &orig in originals {
                        let snapshot =
                            tools.get(orig).and_then(|t| t.as_ref()).ok_or_else(|| {
                                chain_err(
                                    index,
                                    "pattern references an op with no reusable tool solid",
                                )
                            })?;
                        tool_solids.push(ffi::PcadToolSolid {
                            brep: snapshot.brep.as_ptr(),
                            len: snapshot.brep.len(),
                            subtractive: snapshot.subtractive as i32,
                        });
                    }
                }
                let flat: Vec<f64> = transforms
                    .iter()
                    .flat_map(|m| m.iter().flatten().copied())
                    .collect();
                unsafe {
                    ffi::printcad_occt_pattern(
                        base_ptr,
                        base_len,
                        tool_solids.as_ptr(),
                        tool_solids.len(),
                        flat.as_ptr(),
                        flat.len() / 16,
                        &mesh_opts,
                    )
                }
            }
            SolidOp::Boolean { tool_brep, kind } => {
                let kind_ffi = match kind {
                    BoolKind::Fuse => 0,
                    BoolKind::Cut => 1,
                    BoolKind::Common => 2,
                };
                unsafe {
                    ffi::printcad_occt_boolean(
                        base_ptr,
                        base_len,
                        tool_brep.as_ptr(),
                        tool_brep.len(),
                        kind_ffi,
                        &mesh_opts,
                    )
                }
            }
        };

        let (blob, tool, mesh) = take_sweep_result(raw, index, is_last)?;
        current_blob = blob;
        tools.push(tool.map(|brep| ToolSnapshot {
            brep,
            subtractive: solid_op.boolean_op() == Some(BooleanOp::Cut),
        }));
        if let Some(mesh) = mesh {
            final_mesh = mesh;
        }
    }

    if final_mesh.positions.is_empty() || final_mesh.indices.is_empty() {
        return Err(chain_err(
            ops.len() - 1,
            "solid-op chain produced an empty render mesh",
        ));
    }

    let bounds_mm = final_mesh.bounds();
    Ok(SolidBuildResult {
        brep_blob: current_blob,
        mesh: final_mesh,
        bounds_mm,
    })
}

fn import_step_brep_only_internal(
    path: &Path,
    detail: &TessellationSettings,
) -> KernelResult<ImportedModel> {
    let rust_total = Instant::now();

    let path_str = path.to_str().ok_or_else(|| {
        KernelError::InvalidInput(format!("STEP path is not valid UTF-8: {}", path.display()))
    })?;
    let c_path = CString::new(path_str).map_err(|err| {
        KernelError::InvalidInput(format!("STEP path contains a NUL byte: {err}"))
    })?;

    let header_start = Instant::now();
    let source_unit = step_header::detect_step_unit_from_path(path).ok().flatten();
    let header_ms = header_start.elapsed().as_secs_f64() * 1000.0;

    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();
    let serialize_brep = if detail.persist_brep_snapshot { 1 } else { 0 };
    let gen_edges = boundary_edges_for_ffi(detail);

    let ffi_start = Instant::now();
    let result = unsafe {
        ffi::printcad_occt_import_step_brep(
            c_path.as_ptr(),
            serialize_brep,
            linear_mode,
            linear_value,
            angular,
            weld_cross_face,
            weld_angle_rad,
            gen_edges,
        )
    };
    let occt_ffi_ms = ffi_start.elapsed().as_secs_f64() * 1000.0;

    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_brep_import_result(result) };
        return Err(KernelError::Import(message));
    }

    let copy_start = Instant::now();
    let nodes = imported_nodes_from_brep_result(&result);
    let mut bodies = Vec::with_capacity(result.body_count);
    if result.body_count > 0 && !result.bodies.is_null() {
        let raw_bodies = unsafe { std::slice::from_raw_parts(result.bodies, result.body_count) };
        for body in raw_bodies {
            let name = if body.name.is_null() {
                None
            } else {
                Some(
                    unsafe { CStr::from_ptr(body.name) }
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            let brep_blob = if body.brep_blob.is_null() || body.brep_len == 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(body.brep_blob, body.brep_len) }.to_vec()
            };
            let face_colors = copy_vec3_array(body.face_colors, body.face_count);
            let bounds_mm = Some((body.bbox_min, body.bbox_max));
            let mesh = tri_mesh_from_brep_inline(body, detail);
            bodies.push(ImportedBody {
                name,
                mesh,
                brep_blob,
                face_colors,
                bounds_mm,
            });
        }
    }
    let rust_copy_ms = copy_start.elapsed().as_secs_f64() * 1000.0;
    unsafe { ffi::printcad_occt_free_brep_import_result(result) };

    let rust_total_ms = rust_total.elapsed().as_secs_f64() * 1000.0;
    info!(
        path = %path.display(),
        header_scan_ms = format!("{header_ms:.2}"),
        occt_brep_ffi_ms = format!("{occt_ffi_ms:.2}"),
        rust_brep_copy_ms = format!("{rust_copy_ms:.2}"),
        rust_import_total_ms = format!("{rust_total_ms:.2}"),
        "STEP fast import (BRep) timing (Rust; C++ stderr: [printcad_import_brep_cpp])"
    );

    let note = if detail.persist_brep_snapshot {
        "BRep snapshot serialized; tessellation may follow in worker"
    } else {
        "session import (mesh inline; BRep snapshot skipped)"
    };
    info!(
        "Imported STEP `{}`: {} bodies ({note}), source unit {:?}",
        path.display(),
        bodies.len(),
        source_unit,
    );

    Ok(ImportedModel {
        bodies,
        nodes,
        source_unit,
    })
}

fn import_step_full_mesh_internal(
    path: &Path,
    detail: &TessellationSettings,
) -> KernelResult<ImportedModel> {
    let rust_total = Instant::now();

    let path_str = path.to_str().ok_or_else(|| {
        KernelError::InvalidInput(format!("STEP path is not valid UTF-8: {}", path.display()))
    })?;
    let c_path = CString::new(path_str).map_err(|err| {
        KernelError::InvalidInput(format!("STEP path contains a NUL byte: {err}"))
    })?;

    let header_start = Instant::now();
    let source_unit = step_header::detect_step_unit_from_path(path).ok().flatten();
    let header_ms = header_start.elapsed().as_secs_f64() * 1000.0;

    let (linear_mode, linear_value) = tessellation_linear_for_ffi(detail);
    let angular = (detail.angular_tolerance_deg.max(0.5) as f64).to_radians();
    let weld_cross_face = if detail.weld_cross_face { 1 } else { 0 };
    let weld_angle_rad = (detail.weld_angle_threshold_deg.max(0.0) as f64).to_radians();
    let gen_edges = boundary_edges_for_ffi(detail);

    let ffi_start = Instant::now();
    let result = unsafe {
        ffi::printcad_occt_import_step(
            c_path.as_ptr(),
            linear_mode,
            linear_value,
            angular,
            weld_cross_face,
            weld_angle_rad,
            gen_edges,
        )
    };
    let occt_ffi_ms = ffi_start.elapsed().as_secs_f64() * 1000.0;

    if !result.error.is_null() {
        let message = unsafe { CStr::from_ptr(result.error) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::printcad_occt_free_result(result) };
        return Err(KernelError::Import(message));
    }

    let copy_start = Instant::now();
    let nodes = imported_nodes_from_import_result(&result);
    let mut bodies = Vec::with_capacity(result.body_count);
    if result.body_count > 0 && !result.bodies.is_null() {
        let raw_bodies = unsafe { std::slice::from_raw_parts(result.bodies, result.body_count) };
        for body in raw_bodies {
            let mesh = tri_mesh_from_occt_body(body, detail);
            let name = if body.name.is_null() {
                None
            } else {
                Some(
                    unsafe { CStr::from_ptr(body.name) }
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            let bounds_mm = mesh.bounds();
            bodies.push(ImportedBody {
                name,
                mesh,
                brep_blob: Vec::new(),
                face_colors: Vec::new(),
                bounds_mm,
            });
        }
    }

    let rust_mesh_copy_ms = copy_start.elapsed().as_secs_f64() * 1000.0;

    unsafe { ffi::printcad_occt_free_result(result) };

    let rust_total_ms = rust_total.elapsed().as_secs_f64() * 1000.0;

    info!(
        path = %path.display(),
        header_scan_ms = format!("{header_ms:.2}"),
        occt_ffi_ms = format!("{occt_ffi_ms:.2}"),
        rust_mesh_copy_ms = format!("{rust_mesh_copy_ms:.2}"),
        rust_import_total_ms = format!("{rust_total_ms:.2}"),
        "STEP full import timing (Rust; C++ stderr: [printcad_import_cpp])"
    );

    info!(
        "Imported STEP `{}` (full mesh): {} bodies, {} triangles, source unit {:?}",
        path.display(),
        bodies.len(),
        bodies
            .iter()
            .map(|b| b.mesh.indices.len() / 3)
            .sum::<usize>(),
        source_unit,
    );

    Ok(ImportedModel {
        bodies,
        nodes,
        source_unit,
    })
}

fn imported_nodes_from_brep_result(
    result: &ffi::PrintcadOcctBrepImportResult,
) -> Vec<ImportedNode> {
    let mut nodes = Vec::new();
    if result.node_count > 0 && !result.nodes.is_null() {
        let raw_nodes = unsafe { std::slice::from_raw_parts(result.nodes, result.node_count) };
        nodes.extend(raw_nodes.iter().map(imported_node_from_ffi));
    }

    if nodes.is_empty() && result.body_count > 0 {
        // Backward-compat fallback while C++ hierarchy export rolls out.
        nodes.extend((0..result.body_count).map(|idx| ImportedNode {
            id: (idx + 1) as u64,
            parent_id: None,
            name: Some(format!("Body {}", idx + 1)),
            kind: ImportedNodeKind::Part,
            visible: true,
            body_index: Some(idx),
            local_transform: None,
        }));
    }
    nodes
}

fn imported_nodes_from_import_result(result: &ffi::PrintcadOcctImportResult) -> Vec<ImportedNode> {
    if result.node_count > 0 && !result.nodes.is_null() {
        let raw_nodes = unsafe { std::slice::from_raw_parts(result.nodes, result.node_count) };
        return raw_nodes.iter().map(imported_node_from_ffi).collect();
    }
    (0..result.body_count)
        .map(|idx| ImportedNode {
            id: (idx + 1) as u64,
            parent_id: None,
            name: Some(format!("Body {}", idx + 1)),
            kind: ImportedNodeKind::Part,
            visible: true,
            body_index: Some(idx),
            local_transform: None,
        })
        .collect()
}

fn imported_node_from_ffi(raw: &ffi::PrintcadOcctImportNode) -> ImportedNode {
    let name = if raw.name.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(raw.name) }
                .to_string_lossy()
                .trim()
                .to_string(),
        )
        .filter(|s| !s.is_empty())
    };
    let kind = match raw.kind {
        0 => ImportedNodeKind::Assembly,
        2 => ImportedNodeKind::Instance,
        _ => ImportedNodeKind::Part,
    };
    let body_index = if raw.body_index >= 0 {
        Some(raw.body_index as usize)
    } else {
        None
    };
    let parent_id = if raw.parent_id >= 0 {
        Some(raw.parent_id as u64)
    } else {
        None
    };
    let local_transform = if raw.has_local_transform != 0 {
        Some([
            [
                raw.local_transform[0],
                raw.local_transform[1],
                raw.local_transform[2],
                raw.local_transform[3],
            ],
            [
                raw.local_transform[4],
                raw.local_transform[5],
                raw.local_transform[6],
                raw.local_transform[7],
            ],
            [
                raw.local_transform[8],
                raw.local_transform[9],
                raw.local_transform[10],
                raw.local_transform[11],
            ],
            [
                raw.local_transform[12],
                raw.local_transform[13],
                raw.local_transform[14],
                raw.local_transform[15],
            ],
        ])
    } else {
        None
    };

    ImportedNode {
        id: raw.id,
        parent_id,
        name,
        kind,
        visible: raw.visible != 0,
        body_index,
        local_transform,
    }
}

fn tri_mesh_from_brep_inline(
    body: &ffi::PrintcadOcctBrepBody,
    detail: &TessellationSettings,
) -> TriMesh {
    if body.mesh_vertex_count == 0 || body.mesh_positions.is_null() {
        return TriMesh::default();
    }
    let occt = ffi::PrintcadOcctBody {
        name: std::ptr::null_mut(),
        positions: body.mesh_positions,
        normals: body.mesh_normals,
        colors: body.mesh_colors,
        indices: body.mesh_indices,
        edges: body.mesh_edges,
        vertex_count: body.mesh_vertex_count,
        index_count: body.mesh_index_count,
        edge_count: body.mesh_edge_count,
    };
    tri_mesh_from_occt_body(&occt, detail)
}

fn tri_mesh_from_occt_body(body: &ffi::PrintcadOcctBody, detail: &TessellationSettings) -> TriMesh {
    let positions = copy_vec3_array(body.positions, body.vertex_count);
    let normals = copy_vec3_array(body.normals, body.vertex_count);
    let mut colors = copy_vec3_array(body.colors, body.vertex_count);
    if colors.len() != positions.len() && !positions.is_empty() {
        colors = vec![[1.0, 1.0, 1.0]; positions.len()];
    }
    let indices = copy_u32_array(body.indices, body.index_count);
    let edges = copy_u32_array(body.edges, body.edge_count * 2);

    let triangle_count = indices.len() / 3;
    if triangle_count > 0 {
        let ratio = positions.len() as f64 / triangle_count as f64;
        let label = if body.name.is_null() {
            "<unnamed>".to_string()
        } else {
            unsafe { CStr::from_ptr(body.name) }
                .to_string_lossy()
                .into_owned()
        };
        tracing::info!(
            body = %label,
            vertices = positions.len(),
            triangles = triangle_count,
            boundary_edges = edges.len() / 2,
            welded = detail.weld_cross_face,
            weld_angle_deg = detail.weld_angle_threshold_deg,
            ratio = format!("{:.2}", ratio),
            "STEP body tessellation profile (vertex/triangle ratio)"
        );
    }

    TriMesh {
        positions,
        normals,
        indices,
        edges,
        colors,
    }
}

fn copy_vec3_array(ptr: *const f32, vertex_count: usize) -> Vec<[f32; 3]> {
    if ptr.is_null() || vertex_count == 0 {
        return Vec::new();
    }
    let scalar_count = vertex_count * 3;
    let slice = unsafe { std::slice::from_raw_parts(ptr, scalar_count) };
    let mut out = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let base = i * 3;
        out.push([slice[base], slice[base + 1], slice[base + 2]]);
    }
    out
}

fn copy_u32_array(ptr: *const u32, count: usize) -> Vec<u32> {
    if ptr.is_null() || count == 0 {
        return Vec::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, count) };
    slice.to_vec()
}
