# Sketch units

`sketch_units` defines explicit f64 quantities for geometry boundaries:

- `space::Point3` and `space::Vector3`: finite components in a chosen frame;
- `length::Length`: finite, nonnegative length;
- `length::Offset`: finite, signed displacement;
- `scale::PerPx`: finite, strictly positive model units per logical pixel;
- `angle::Angle`: finite radians, retaining accumulated turns.

The caller chooses coordinate frames and physical units. Constructors check
their invariants. Domain types convert explicitly, preserving numerical values;
`.0` exposes those values where needed. There is no `Deref` or implicit arithmetic
between incompatible quantities.

Lengths and scales serialize as bare numbers. Deserialization does not execute
constructors: validate external values before passing them into geometry code.

Run `cargo test -p sketch_units` for constructor and JSON format contracts.
