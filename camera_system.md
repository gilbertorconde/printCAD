# Camera

How the viewport camera works. The code is in
`crates/app_shell/src/camera/`.

## State

The camera (`state.rs`, `CadCameraState`) holds:

- the eye position and an orientation quaternion
- the focal distance: how far ahead of the eye the pivot is
- the projection: perspective with a vertical field of view, or
  orthographic with a height in millimetres
- the near and far planes, and the viewport size

The focal point is `eye + forward * focal_distance`. Orbit, zoom and fit all
work around it.

## Axes

Nothing assumes which way is up. The axis preset (`crates/axes`) names a
horizontal, a vertical and a depth axis:

- forward = orientation × (−depth)
- up = orientation × vertical

The default preset is Z up: X right, Z up, depth −Y. Two Y-up presets also
exist.

## Projection

- Switching between perspective and orthographic keeps the size of what is
  at the focal point.
- Changing the field of view (10° to 120°) moves the eye so the focal point
  keeps its size on screen. Only the amount of perspective changes.

## Navigation

| Action | Input | What happens |
| --- | --- | --- |
| Orbit | Middle drag | Turns about the focal point, or about the point picked under the cursor when that option is on |
| Pan | Right drag | Moves the eye; one pixel is one pixel at the focal plane |
| Roll | Left and right drag | Turns about the view direction |
| Zoom | Wheel | Perspective moves the eye; orthographic scales the height. Toward the cursor by default |
| Set pivot | Middle click on the model | The point under the cursor becomes the focal point |
| Set pivot | **H** | The cursor's point on the focal plane becomes the focal point |
| Fit | **F** | Isometric view of the whole scene. The view toolbar can also fit the selection |

A drag starts after 4 pixels, so a click never orbits. Perspective zoom keeps
the focal distance between the configured minimum and maximum (1 mm and
5000 mm by default). While a sketch is open, orbit is turned off.

## Near and far planes

Each frame the near and far planes are fitted to the scene's bounding box:

- **Perspective:** near is a small fraction of the focal distance, pushed out
  toward the model when the whole box is in front of the eye. Far is just past
  the box. The far/near ratio is capped at 100 000 to keep depth precise.
- **Orthographic:** near may sit behind the eye, so geometry behind it still
  draws.

## Animation

Standard views, the orientation cube and entering a sketch animate over
400 ms by default. Orientation is interpolated along the shortest rotation,
and focal distance on a log scale. Any input stops a running animation.

## 6-DoF mouse

`apply_device_motion` runs once per frame. Each device axis is scaled, given
a dead zone, and added to the motion it is assigned to in Preferences: pan,
zoom, turn, tilt or roll. Speeds are per second, so the result does not
depend on the frame rate.

## Clipping plane

The view toolbar's clipping plane (`section.rs`) is a plane square to X, Y
or Z. It starts through the middle of the scene on the axis the view looks
along most directly, and keeps the far half. It belongs to the view, not
the document. The renderer and picking both respect it.
