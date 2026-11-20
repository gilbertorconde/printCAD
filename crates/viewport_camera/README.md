# viewport_camera

A camera without a window, scene or renderer: pose, projection, rays,
press-relative pan and orbit, zoom, fit, transitions, smoothing and optional
momentum. Dependencies are `glam` and `serde`; the host owns input routing,
anchors, clipping, a floating origin and settings persistence.

This crate is shared with other applications. Change the shared copy first,
then refresh all camera sources and tests together. The packaging includes
the length and scale quantities used by navigation settings in this crate;
the host converts its own units at the boundary. Source formatting follows
this repository. Host-specific code belongs in the app's camera adapter.

`focal_view` is a focal-distance view over that camera, as a CAD viewport
keeps it: an `f64` eye, an orientation relative to the host's axis frame
(`AxisFrame`, three directions), a focal distance, near and far planes fitted
to the scene's box, and the view-projection matrix with a Y-down clip space.
The host passes its preferences as `ViewSettings` and owns the clock.

Keep one `navigation::NavigationDrag` from the press to release. Sample it
with absolute logical-pixel coordinates, and apply the result relative to
the same model origin for the whole gesture. Orbit keeps the anchor's
off-centre screen position, clamps at the poles with a level horizon, and
supports free rotation when requested. Do not accumulate per-frame deltas.
