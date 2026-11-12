# Mesh starts and base solids

**Mesh starts.** A mesh body's mesh history (RFC 0004 milestone 5)
builds from the mesh as imported, kept apart as its start
(`Document::set_mesh_start`, op `SetMeshStart`, saved as
`mesh/<body>.start.bin`); the edited mesh the body draws is derived.
`KernelQueries::build_mesh_chain` applies `kernel_api::MeshOp`s
(`kernel_ogeom/src/mesh_edit.rs`: the start welded once into one
`ogeom::mesh::Editable`, so a `MeshRegion::Vertices` marking of the start
still names its faces after earlier steps), a failing step reported with
the mesh before it.

**Base solids.** An imported, converted or repaired solid takes features
by becoming its body's base: `Document::set_body_base` (op `SetBodyBase`,
its own inverse with `on` flipped) copies the body's solid into
`base_solids` (saved as `brep/<body>.base.bin`/`.colors`, the geometry in
the body's own frame) and `body_solid_is_imported` turns false; dropping
it puts the base back as the body's shape. Design's `DesignFeature::Base`
builds as `SolidOp::Shape` of it and must be first. `take_base` (from
`create_feature` and the borrow command) makes a body based on its first
Design feature, putting the Base first in its history; that tool's Cancel
undoes it, and the Base deletes only once nothing follows it. Repair on a
based body mends the base (`set_base_solid`, derived) and invalidates the
body; `Document::body_health` reads the base's findings where there is
one. The tree puts an imported part's features under its row.
Replace shape… (body menus, `doc.replace_shape`) reads a body's shape
from another file: op `ReplaceBodyShape` carries the file as an asset
(a history barrier, as an import is) and sets `Body.shape_asset`;
`bodies_awaiting_shape` lists bodies whose base (or shape, without a
history) is not yet from that asset, and the host's
`drive_shape_replacements` reads each on the kernel thread
(`OgeomKernel::read_solid`: a STEP/IGES file's first body, a `.brep`
file's shape, a mesh file converted) into the base, invalidating the
body, or into its shape (`recompute::land_shape`, which headless runs
too). `Document::shape_body_from_file` records the same op for a body
holding nothing yet. A stand-in (`Body.stand_in`, op `SetBodyStandIn`,
also a barrier) is a shape asset kept for a body built from features:
`stand_in_due` while its feature is the tip and that feature's build
failed, the same drive reads it into the body's shape, and the next
build that succeeds replaces it (`body_solid_is_imported` ignores it, so
the body still builds). The project import (`project_tools`) gives every
body its tip's saved shape as one, and brings what it cannot rebuild in
as `shape_body_from_file` bodies, the saved `.brp` restated for the
kernel's reader by `project_archive::restate`.
Design's Delete Faces (`DesignFeature::DeleteFaces`, `SolidOp::RemoveFaces`)
takes picked faces away through the kernel's `remove_faces`, the
neighbours closing the openings; Offset Faces and Move Faces
(`SolidOp::OffsetFaces`/`MoveFaces`, a distance, or a translation and a
turn about an axis as a rigid row-major matrix) go through its
`offset_faces`/`move_faces`, the neighbours following on their own
surfaces. Offset Solid (`SolidOp::OffsetSolid`) moves every face through
its `offset_shape`, which refuses free-form faces. Scale (`SolidOp::Scale`,
factors per axis about a point or the bounding box's middle) goes through
its `general_transformed_shape` (`ops::pattern::scaled`). Recognize holes (`design.recognize_holes`,
`wb_design/src/recognize.rs`) asks `KernelQueries::recognize_holes`
(`kernel_ogeom/src/holes.rs`: concave cylinder faces grouped by axis and
radius into whole bores, each end an opening, a flat bottom or a coaxial
cone, from the faces sharing its edges) and adds one Delete Faces for
every hole's faces, then per set of alike holes a hidden sketch of their
centres on the opening plane and a Hole feature.
Design's Project (`DesignFeature::Project`, `SolidOp::Project`,
`kernel_ogeom/src/ops/project.rs`) carries a sketch along its normal onto
picked faces: `split_face` (`Projection::Along`) splits them, and for
Emboss/Deboss each piece whose inner point falls in a profile region is
thickened (`make_thick_sheet`, the side checked by a point) and fused on or
cut away; the kernel's `offset_faces` refuses such regions. Its `wrap`
builds `SolidOp::Wrap` instead (`ops/wrap.rs`, `Development`): the sketch
unrolled onto the faces' cylinder or cone from where its middle lands, or
on any other surface laid as the kernel's `SurfaceSheet` (geodesic polar
coordinates from there, its `+x` the sketch's x; a point carried back by
Gauss-Newton on `chart_point` for Emboss/Deboss), each curve a
`Curve::OnSurface` of its chart curve (on a cylinder the curve's exact
B-spline with mapped control points, on a cone a fit, on a sheet the
kernel's `chart_edges`, cut at a pole), cut at derivative jumps, carrying
that chart curve as its pcurve on each picked face's surface, then
`split_face` with `Projection::OnFace`.
