# Sketch geometry

`sketch_geometry` provides double-precision planar geometry: analytic curves,
rational B-splines, bounds, projection, intersections and arc-length traversal.
It depends on `sketch_units`, `glam` and `serde`.

`world` distinguishes sketch points, displacement vectors, nonnegative lengths,
signed offsets and pixel scales. Constructors enforce finite values and domain
invariants. Validate deserialized quantities before using them. `sketch_plane`
places local sketch coordinates in a three-dimensional model frame.

Angles are radians. A circular arc has a start angle and signed sweep: positive
is counterclockwise, negative is clockwise. Zero denotes a point; a full turn
covers the circle. Elliptic parameters are cyclic; hyperbolic and parabolic
parameters are monotone. Hyperbolas have an explicit branch.

`bspline` evaluates pole-defined and interpolating curves through a generic
scalar API. `analytic_spline` constructs exact conics. `bspline_window` describes
visible spans, including periodic seams. Representation poles need not be model
points. The implementations document their Piegl–Tiller and Boehm formulas.

`ProfileCurve` and `TrimCurve` traverse carriers by arc length. Geometric contact
does not imply topological identity; the caller validates adjacency and source
addresses before editing a model. Document storage, constraint solving and undo
belong to the application.

Run `cargo test -p sketch_geometry` for geometric equations, finite differences,
angular boundaries, serialization and fixed spline reference data.
Degree conversion, curve joining and editable knot multiplicities are compiled
only with the numerical unit tests; they are outside the production API.
