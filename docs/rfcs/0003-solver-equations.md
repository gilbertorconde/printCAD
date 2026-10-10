# Solver equation inventory

This inventory covers all **42** `ConstraintKind` variants at the captured
application baseline. The compiler and evaluator are in
[solver.rs](../../crates/workbenches/wb_sketch/src/solver.rs); geometric
solver tests are in that file, with internal geometry tests in
[internal.rs](../../crates/workbenches/wb_sketch/src/internal.rs) and linked
array/offset tests in
[tools/tests.rs](../../crates/workbenches/wb_sketch/src/tools/tests.rs).
The [problem contract](0003-solver-contract.md) defines the migration seam.

`L` means the caller's length unit, `rad` radians, `1` dimensionless.
`P` is a point (two coordinates), `R` a scalar radius, `S` a shape
(major/axis x and y, minor), and `t` an auxiliary curve parameter.
The variables column describes dependencies, not how many free variables
the relation adds. Only `t` columns add auxiliary variables. A shape enters
the numerical array only for radius/equality/internal-alignment relations;
otherwise it remains fixed during solve but contributes uncompiled DoF.

Every row count assumes existing geometry of the supported kind. Inactive
and reference dimensions have zero rows. Missing or wrong-kind operands
also have zero rows in the current application. The adapter must report
these omissions; the independent API rejects invalid typed references.
Internal hidden rows are counted separately below.

| Relation | User rows; units | Variables and equation | Branch/domain | Independent tests |
| --- | --- | --- | --- | --- |
| FixedPoint | 2; L | P minus target x/y | Any finite coordinates | `fixed_point_moves_point` |
| Coincident | 2; L | P1 minus P2 x/y | Point-to-point, origin allowed | `coincident_merges_two_points`, reference tests |
| Parallel | 1; 1 | Four P; cross of unit line directions | Infinite lines, either orientation | `parallel_and_perpendicular_pair` |
| Perpendicular | 1; 1 | Four P; dot of unit line directions | Infinite lines | `parallel_and_perpendicular_pair` |
| EqualLength | 1; L | Four P; difference of endpoint distances | Unsigned lengths | `equal_length_matches_two_lines` |
| Length | 1; L | Two P; endpoint distance minus target | Line only | `length_sets_line_length_keeping_direction` |
| EqualRadius | 1; L | Two R; radius difference | Circles/arcs | `equal_radius_matches_two_circles` |
| Radius | 1; L | R minus target | Circles/arcs | `radius_constraint_resizes_circle`, `arc_endpoints_follow_radius_constraint` |
| Diameter | 1; L | 2R minus target | Circles/arcs | `diameter_constraint_resizes_circle` |
| PointOnLine | 1; L | Three P; signed cross divided by line length | Infinite line, axes allowed | `point_on_line_projects_point` |
| PointOnCircle | 1; L | Two P and R; centre distance minus R | Arc means whole circle | `point_on_circle_snaps_to_radius` |
| PointOnEllipse | 1; L | Two P and S; `(sqrt((u/a)^2+(v/b)^2)-1)*b` | S fixed unless otherwise sized | `point_on_ellipse_pulls_point_to_boundary` |
| Horizontal | 1; L | Two P; end y minus start y | Line only | `horizontal_levels_sloped_line` |
| Vertical | 1; L | Two P; end x minus start x | Line only | `vertical_aligns_sloped_line` |
| HorizontalPoints | 1; L | Two P; y difference | Two points | `two_points_are_held_level_and_plumb` |
| VerticalPoints | 1; L | Two P; x difference | Two points | `two_points_are_held_level_and_plumb` |
| Block | 2 per referenced P, plus 1 per R; L | Freeze current positions and radius | Includes standalone P and spline fit points; does not freeze S directly | `block_freezes_line_while_other_geometry_moves`, `length_on_blocked_line_conflicts` |
| Distance | 1; L | Two P; Euclidean distance minus target | Unsigned distance | `distance_constraint_separates_points` |
| DistanceX | 1; L | One/two P; x separation error | Nonnegative absolute, negative signed; missing b uses a from zero | `distance_x_and_y_between_points`, `distance_x_from_origin` |
| DistanceY | 1; L | One/two P; y separation error | Same sign contract as DistanceX | `distance_x_and_y_between_points` |
| Angle | 1; rad | Four P; wrapped atan2(cross,dot) minus target | Directed line angle, periodic | `angle_constraint_rotates_line` |
| AngleToAxis | 1; rad | Two P; wrapped atan2(dy,dx) minus axis+target | Vertical adds π/2 | `angle_to_axis_rotates_line` |
| Tangent | 1; L or 1 | P/R; line-centre distance minus R, circle distance minus radius combination, or endpoint tangent dot/cross | Shared endpoints take precedence; initial closest external/internal branch; ties external | `tangent_line_circle_moves_center_onto_offset`, both circle branches, `two_ends_picked_are_joined_smoothly` |
| Symmetric | 2; L | Four P; midpoint on line, pair normal to line | Infinite mirror line | `symmetric_mirrors_point_about_line`, diagonal test |
| SymmetricAboutPoint | 2; L | Three P; centre minus pair midpoint | Point reflection | `symmetric_about_point_centers_pair` |
| Midpoint | 2; L | Three P; point minus endpoint midpoint | Infinite line reference uses its defined endpoints | `midpoint_pins_point_to_line_center` |
| ArcLength | 1; L | Three P and R; R times CCW sweep minus target | Sweep `(0,2π]`; equal endpoints mean full turn | `an_arc_length_sweeps_the_end_round` |
| Gap | 1; L | P/R; absolute point-line, point-circle, line-circle, midpoint-line or circle-circle gap | Initial inside/nested branch; arcs are whole circles; point-point unsupported; line-line does not add parallelism | `a_gap_keeps_a_circle_off_a_line_and_points_off_both`, both circle gaps, parallel-line gap |
| AngleAtPoint | 1; rad | P and line/circle tangent directions; wrapped directed angle | Point membership is a separate relation; circle tangent is CCW | `an_angle_at_a_point_turns_a_line_against_an_arc` |
| EllipseRadius | 1; L | S; major-vector magnitude or minor minus target | S becomes variable; roles may swap after storage | `an_ellipse_takes_its_radii_and_a_spline_its_length`, internal axis/radius swap tests |
| CurveLength | 1; L | Curve P/R/S; sum of 128 chord lengths minus target | Full curve range, conics between endpoint parameters | `an_ellipse_takes_its_radii_and_a_spline_its_length` |
| PointOnCurve | 2; L | P plus curve variables and **1 t**; coordinate difference | t starts at nearest coarse/fine projection | `a_point_is_held_on_a_spline_and_on_a_parabola` |
| TangentCurves | 3; L,L,1 | Two curves and **2 t**; contact x/y and tangent cross | Initial nearest pair, full curves | `a_line_touches_an_ellipse_and_crosses_a_spline_square` |
| PerpendicularCurves | 3; L,L,1 | Two curves and **2 t**; contact x/y and tangent dot | Same contact search | `a_line_touches_an_ellipse_and_crosses_a_spline_square` |
| EqualEllipse | 2; L | Two S; major and minor differences | Both S become variable | `two_ellipses_are_held_the_same_size` |
| Offset | 2 per line pair, 1 per circle pair; 1/L or L | Line parallelism+midpoint gap; nested circle gap | Circle formula alone does not enforce a shared centre; application keeps other links | `a_linked_offset_follows_its_dimension` |
| Pitch | `d + 2(k-1)`; L | Ordered P; first step length (d=1) or vector (d=2), remaining steps equal first | k selected steps; zero when k=0; columns floor 1; across means first-column steps; f32 direction normalization | `linked_translated_copies_follow_the_original_and_one_pitch`, `a_linked_array_is_spaced_by_its_two_pitches` |
| PolarPitch | `2(n-1)`; rad/L | Centre P and n member P; adjacent angle and equal radii | n≥1, zero for ≤1; periodic directed angles | `linked_rotated_copies_turn_by_one_angle` |
| ArcAngle | 1; rad | Three P; positive CCW sweep minus target | Sweep `(0,2π]`, full turn distinct from zero | `an_arc_opens_to_its_angle_and_three_points_hold_theirs` |
| AngleThreePoints | 1; rad | Three P; wrapped angle of two vertex arms | Periodic directed angle, floor-free atan2 | `an_arc_opens_to_its_angle_and_three_points_hold_theirs` |
| Refraction | 1; 1 | Interface tangent and two ray directions; sin-in minus ratio*sin-out | Rays ordered near/far once; interface line/circle; no membership rows | `a_refraction_bends_the_ray_leaving_the_interface` |
| InternalAlignment | 0/2/4; L | Internal points P and curve S; offset positions or focus square plus opposite midpoint | Major/minor endpoints deduplicated and own curve points omitted; standalone derived ellipse focus/control polygon zero | `a_dimension_on_an_axis_sizes_the_ellipse`, conic tests, focus drag/circle-crossing tests |

## Hidden rules and precision

- Every circular arc adds two L rows `|endpoint-centre|-R` when at least
  one regular user spec exists. DoF always adds them. Internal alignment
  is appended after this gate and alone does not activate circular rules.
- Every conic endpoint adds one L row regardless of user specs: hyperbola
  implicit-function error divided by gradient length, or parabola error
  divided by its slope factor. Ellipse arcs rely on explicit membership
  constraints rather than an extra automatic rule.
- Internal alignment deduplicates target points globally. Ellipse foci
  mentioned by another active constraint, used by a curve or held by the
  gesture participate in the solve; other foci are pinned, omitted from
  residuals and placed after solving. Excluding a constraint can change
  this classification and the number of shape variables.
- Free shapes outside the compiled system add 3 DoF per ellipse or
  hyperbola, 2 per parabola; external curves add none. Contact parameters
  count as ordinary free variables.
- Unit direction floors are `1e-12`, numerical spline direction floor
  `1e-300`. No new degeneracy fallback is introduced by extraction.
- The normal equations use central differences (`1e-6` relative step),
  LM damping `1e-3` initially, 25 retries, diagonal+mean damping floor
  `1e-12`, limits `[1e-12,1e12]`, step cap `100*scale`. Numerical rank uses
  `1e-8`. Default convergence is `1e-9*max(1,|x|∞)`.
- Solver inputs promote f32 geometry to f64. Initial ellipse minor radius
  and pitch direction normalize in f32; storage and axis/focus renaming
  also use f32. Independent f64 inputs must not imitate application
  rounding unless the caller supplied those rounded initial values.

## Coverage limits to preserve visibly

Existing solver tests cover regular relations but do not constitute the
entire deterministic boundary matrix from milestone 4. In particular,
signed DistanceX/Y, all six large angle boundaries, repeated wrap crossings,
partial composite omissions and all internal zero-row variants need the
standalone validation and replay tests. They remain explicit pending work,
rather than being inferred from one successful example per enum variant.
