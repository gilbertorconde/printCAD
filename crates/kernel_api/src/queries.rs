//! The queries a built solid or mesh answers, which the kernel adapter
//! implements.

use crate::*;

pub trait KernelQueries: Send + Sync {
    /// The wall thickness of `target` at each point, `NaN` where none is
    /// found within reach.
    fn thickness(
        &self,
        _target: DistanceTarget<'_>,
        _request: &ThicknessRequest,
    ) -> KernelResult<Vec<f64>> {
        Err(KernelError::Unsupported("thickness".into()))
    }

    /// The sections of `mesh` by the plane through `origin` square to
    /// `normal`, moved along it by each of `offsets`: one list of chains
    /// per offset, in their order. Each chain runs with the material on
    /// its left seen from the side `normal` points to.
    fn mesh_sections(
        &self,
        _mesh: &TriMesh,
        _origin: [f64; 3],
        _normal: [f64; 3],
        _offsets: &[f64],
    ) -> KernelResult<Vec<Vec<SectionChain>>> {
        Err(KernelError::Unsupported("mesh_sections".into()))
    }

    /// The shadow `mesh` casts on `request.plane` seen along its normal,
    /// and an outline fitted around it within `request`'s band.
    fn mesh_shadow(&self, _mesh: &TriMesh, _request: &ShadowRequest) -> KernelResult<ShadowAnswer> {
        Err(KernelError::Unsupported("mesh_shadow".into()))
    }

    /// Lines and arcs fitted to a 2D chain of points, every point within
    /// `fit.tolerance` of its piece and the pieces meeting end to end: a
    /// polyline whose bulges give the arcs, or a circle where one holds a
    /// closed chain.
    fn fit_chain_2d(&self, _points: &[[f64; 2]], _fit: &ChainFit) -> KernelResult<DrawingShape> {
        Err(KernelError::Unsupported("fit_chain_2d".into()))
    }

    /// `start` with `ops` applied in turn: a mesh body's history.
    fn build_mesh_chain(&self, _start: &TriMesh, _ops: &[MeshOp]) -> KernelResult<MeshChainAnswer> {
        Err(KernelError::Unsupported("build_mesh_chain".into()))
    }

    /// The mesh as a file of `format` (`stl`, `3mf`, `ply` or `obj`),
    /// its triangles as they are, in millimetres.
    fn write_mesh(&self, _mesh: &TriMesh, _format: &str) -> KernelResult<Vec<u8>> {
        Err(KernelError::Unsupported("write_mesh".into()))
    }

    /// A solid trimmed to where a scan breaks off.
    fn trim_to_scan(&self, _request: &TrimRequest) -> KernelResult<TrimAnswer> {
        Err(KernelError::Unsupported("trim_to_scan".into()))
    }

    /// A DXF drawing of polylines in millimetres: `visible` continuous,
    /// `hidden` dashed, each on a layer of its own.
    fn write_dxf(
        &self,
        _visible: &[Vec<[f64; 2]>],
        _hidden: &[Vec<[f64; 2]>],
    ) -> KernelResult<String> {
        Err(KernelError::Unsupported("write_dxf".into()))
    }

    /// A drawing as an R2000 DXF, each curve and text as itself on its
    /// layer (a layer draws dashed when its curves are hidden), its unit
    /// declared from `unit_mm` (1 millimetres, 25.4 inches, none
    /// unitless).
    fn write_dxf_drawing(&self, _drawing: &Drawing2d) -> KernelResult<String> {
        Err(KernelError::Unsupported("write_dxf_drawing".into()))
    }

    /// A STEP file of one part, `part`, holding `items` as its construction
    /// geometry and no solid.
    fn write_construction_step(
        &self,
        _part: &str,
        _items: &[ConstructionItem],
    ) -> KernelResult<Vec<u8>> {
        Err(KernelError::Unsupported("write_construction_step".into()))
    }

    /// An IGES file holding `items` as construction geometry and no solid,
    /// with warnings for geometry or names the writer cannot carry in full.
    fn write_construction_iges(
        &self,
        _part: &str,
        _items: &[ConstructionItem],
    ) -> KernelResult<IgesExport> {
        Err(KernelError::Unsupported("write_construction_iges".into()))
    }

    /// An upright frame inferred from the scan's faces, walls and open rims.
    fn orient(&self, _mesh: &TriMesh) -> KernelResult<Orientation> {
        Err(KernelError::Unsupported("orient".into()))
    }

    /// The best mirror plane seeded from the scan's principal frame.
    fn symmetry_plane(&self, _mesh: &TriMesh) -> KernelResult<SymmetryPlane> {
        Err(KernelError::Unsupported("symmetry_plane".into()))
    }

    /// The rigid motion that brings `request`'s points best onto
    /// `target`'s surface.
    fn register(
        &self,
        _target: DistanceTarget<'_>,
        _request: &RegisterRequest,
    ) -> KernelResult<Registration> {
        Err(KernelError::Unsupported("register".into()))
    }

    /// Where `scan`, given in the solid's frame, and the native-format
    /// solid `solid` differ, as regions on a grid of cells `cell` wide,
    /// largest first. Holes in the scan read as the solid.
    fn volume_difference(
        &self,
        _scan: &TriMesh,
        _solid: &[u8],
        _cell: f64,
    ) -> KernelResult<Vec<VolumeRegion>> {
        Err(KernelError::Unsupported("volume_difference".into()))
    }

    /// Each point's signed distance to `target`'s surface, positive
    /// outside it, `NaN` where nothing lies within reach.
    fn signed_distance(
        &self,
        _target: DistanceTarget<'_>,
        _request: &DistanceRequest,
    ) -> KernelResult<Vec<f64>> {
        Err(KernelError::Unsupported("signed_distance".into()))
    }

    /// Grow a region of `request.kind` over `mesh` from its seeds, joining
    /// triangles while they lie on one surface, and fit it.
    fn grow(&self, _mesh: &TriMesh, _request: &GrowRequest) -> KernelResult<GrowAnswer> {
        Err(KernelError::Unsupported("grow".into()))
    }

    /// Every edge two faces of `brep` share, with how they meet there.
    fn continuity(&self, _brep: &[u8]) -> KernelResult<Vec<EdgeContinuity>> {
        Err(KernelError::Unsupported("continuity".into()))
    }

    /// The shape of `request.kind` that fits its points best: orthogonal
    /// least squares, started robustly, with what it rests on.
    fn fit(&self, _request: &FitRequest) -> KernelResult<FitAnswer> {
        Err(KernelError::Unsupported("fit".into()))
    }

    /// `kind` fitted to every one of `points` by `method`, a zone or a
    /// mating fit rather than a mean one; `converged` says whether the zone
    /// is certified optimal. `form` is the zone's two boundaries as signed
    /// distances from the shape, its width their difference.
    fn zone_fit(
        &self,
        _kind: FitKind,
        _method: FitMethod,
        _points: &[[f64; 3]],
    ) -> KernelResult<FitAnswer> {
        Err(KernelError::Unsupported("zone_fit".into()))
    }

    /// The principal curvatures of `brep`'s faces at points on them. Each
    /// entry names a face by a point inside it (the face nearest that
    /// point) and lists points on it, each with the normal it is signed
    /// against, in the shape's own frame; the answer gives, per point, the
    /// largest and the smallest normal curvature (1/mm): negative where the
    /// face bulges toward that normal, positive where it hollows away.
    /// `None` where the surface has no normal (a pole, an apex).
    fn curvature(
        &self,
        _brep: &[u8],
        _faces: &[FacePoints],
    ) -> KernelResult<Vec<Vec<Option<[f64; 2]>>>> {
        Err(KernelError::Unsupported("curvature".into()))
    }

    /// The edges bounding the face of `brep` nearest `near`: a point on
    /// each, halfway along, and its direction there, in the shape's own
    /// frame. A seam (the edge a round face meets itself along) and an
    /// edge with no curve (a pole) are left out.
    fn face_edges(&self, _brep: &[u8], _near: [f64; 3]) -> KernelResult<Vec<([f64; 3], [f64; 3])>> {
        Err(KernelError::Unsupported("face_edges".into()))
    }

    /// The round holes of `brep`, in its own frame, and how many bores it
    /// has that are not holes it can describe (a counterbore, a slot).
    fn recognize_holes(&self, _brep: &[u8]) -> KernelResult<(Vec<RecognizedHole>, usize)> {
        Err(KernelError::Unsupported("recognize_holes".into()))
    }

    /// What `a` and `b` share when `b` sits where `b_in_a` (a rigid
    /// row-major 4×4 matrix) puts it in `a`'s frame; `None` when they only
    /// touch or are apart.
    fn overlap(
        &self,
        _a: &[u8],
        _b: &[u8],
        _b_in_a: &[[f64; 4]; 4],
    ) -> KernelResult<Option<Overlap>> {
        Err(KernelError::Unsupported("overlap".into()))
    }

    /// How near `a` and `b` come when `b` sits where `b_in_a` puts it in
    /// `a`'s frame.
    fn gap(&self, _a: &[u8], _b: &[u8], _b_in_a: &[[f64; 4]; 4]) -> KernelResult<Gap> {
        Err(KernelError::Unsupported("gap".into()))
    }

    /// `brep` mirrored across the plane through `point` with `normal`, in
    /// its own frame, as a snapshot of its own.
    fn mirror(&self, _brep: &[u8], _point: [f64; 3], _normal: [f64; 3]) -> KernelResult<Vec<u8>> {
        Err(KernelError::Unsupported("mirror".into()))
    }

    /// The volume, area and centre of mass of `brep`, in its own frame.
    fn measure(&self, _brep: &[u8]) -> KernelResult<PhysicalProperties> {
        Err(KernelError::Unsupported("measure".into()))
    }

    /// The edge of `brep` nearest `near`, projected orthogonally onto
    /// `plane`. Both are in the shape's own frame.
    fn project_edge(
        &self,
        brep: &[u8],
        near: [f64; 3],
        plane: &ProfilePlane,
    ) -> KernelResult<ProjectedEdge>;

    /// Points and tangents along the edge of `brep` nearest `near` (within
    /// a pick's reach of it), from the start of its curve: one per entry of
    /// `stations`, each a fraction of the edge's length (0 its start, 1 its
    /// end) placed by arc length, not by the curve's parameter. Everything
    /// is in the shape's own frame.
    fn edge_frames(
        &self,
        _brep: &[u8],
        _near: [f64; 3],
        _stations: &[f64],
    ) -> KernelResult<PathFrames> {
        Err(KernelError::Unsupported("points along an edge".into()))
    }

    /// Points and tangents along `wire` on `plane`, walked from its first
    /// segment's start: one per entry of `stations`, each a fraction of the
    /// wire's whole length placed by arc length, as `edge_frames`.
    fn wire_frames(
        &self,
        _plane: &ProfilePlane,
        _wire: &ProfileWire,
        _stations: &[f64],
    ) -> KernelResult<PathFrames> {
        Err(KernelError::Unsupported("points along a wire".into()))
    }

    /// Where every occurrence of a [`SolidOp::Pattern`] with `steps` stands,
    /// as row-major transforms from the original's place, the original's
    /// (the identity) first: what the chain works out, with `brep` (in the
    /// pattern's frame) as the solid a step's edge is found on.
    fn pattern_transforms(
        &self,
        _brep: Option<&[u8]>,
        _steps: &[PatternStep],
    ) -> KernelResult<Vec<[[f64; 4]; 4]>> {
        Err(KernelError::Unsupported("pattern transforms".into()))
    }

    /// The face of `brep` nearest `near` (in the shape's own frame), as a
    /// native-format snapshot of its own: what a face taken from a solid
    /// keeps once the solid moves on.
    fn face_of(&self, _brep: &[u8], _near: [f64; 3]) -> KernelResult<Vec<u8>> {
        Err(KernelError::Unsupported(
            "taking a face out of a solid".into(),
        ))
    }

    /// The edge of `brep` nearest `near` (in the shape's own frame), as a
    /// native-format snapshot of its own.
    fn edge_of(&self, _brep: &[u8], _near: [f64; 3]) -> KernelResult<Vec<u8>> {
        Err(KernelError::Unsupported(
            "taking an edge out of a solid".into(),
        ))
    }

    /// Where the face of `brep` nearest `near` crosses `plane`, as the
    /// curves the plane cuts from it, in the plane's own 2D coordinates:
    /// each the exact line, circle or ellipse it is, or points along any
    /// other. Empty when the face does not reach the plane. Both are in the
    /// shape's own frame.
    fn section_face(
        &self,
        _brep: &[u8],
        _near: [f64; 3],
        _plane: &ProfilePlane,
    ) -> KernelResult<Vec<ProjectedEdge>> {
        Err(KernelError::Unsupported("face section".into()))
    }

    /// A point on each solid of `brep`, one per solid in the order the
    /// shape holds them, in its own frame: where each piece of a shape of
    /// several stands.
    fn solid_pieces(&self, _brep: &[u8]) -> KernelResult<Vec<[f64; 3]>> {
        Err(KernelError::Unsupported("solid pieces".into()))
    }

    /// Where `plane` crosses `brep`, chained end to end, each curve exact
    /// in the plane's own 2D coordinates and drawn in the shape's own
    /// frame: the whole shape's section, or with `near` only the face
    /// nearest it. Empty when the plane misses.
    fn section(
        &self,
        _brep: &[u8],
        _plane: &ProfilePlane,
        _near: Option<[f64; 3]>,
    ) -> KernelResult<Vec<SectionRun>> {
        Err(KernelError::Unsupported("section".into()))
    }

    /// The boundary loops of the face of `brep` nearest `near`, the outer
    /// one first, each edge in loop order, in the shape's own frame.
    fn face_loops(&self, _brep: &[u8], _near: [f64; 3]) -> KernelResult<Vec<Vec<LoopEdge>>> {
        Err(KernelError::Unsupported("face loops".into()))
    }

    /// The curves of a DXF drawing, given as its text.
    fn read_dxf(&self, _text: &str) -> KernelResult<Drawing2d> {
        Err(KernelError::Unsupported("reading DXF".into()))
    }

    /// The regions `profile` encloses, meshed where they stand in its
    /// plane: what a feature swept from it takes, holes left open.
    fn profile_mesh(&self, _profile: &Profile) -> KernelResult<TriMesh> {
        Err(KernelError::Unsupported("profile mesh".into()))
    }

    /// The medial axis of each region of `profile`, held to `tolerance`
    /// (mm), in the profile plane's own 2D coordinates.
    fn medial_axis(&self, _profile: &Profile, _tolerance: f64) -> KernelResult<Vec<MedialRegion>> {
        Err(KernelError::Unsupported("medial axis".into()))
    }

    /// The centre line of the solid in `brep` from the face `from` names to
    /// the face `to` names, held to `tolerance` (mm). Everything is in the
    /// shape's own frame.
    fn centre_line(
        &self,
        _brep: &[u8],
        _from: &FaceProbe,
        _to: &FaceProbe,
        _tolerance: f64,
    ) -> KernelResult<CentreLine> {
        Err(KernelError::Unsupported("centre line".into()))
    }

    /// What the shape in `brep` answers to `probe`, in the shape's own
    /// frame.
    fn probe(&self, _brep: &[u8], _probe: &ShapeProbe) -> KernelResult<ProbeAnswer> {
        Err(KernelError::Unsupported("probing a shape".into()))
    }
}
