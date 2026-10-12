# Camera

How the viewport camera works. The code is in
`crates/app_shell/src/camera/`. The numerical core lives in the shared
`crates/viewport_camera` crate; the app converts its state and routes input.

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

`view_projection` maps to a Y-down clip space, so screen Y grows
downward everywhere; the renderer's vertex shaders flip it to the GPU's
Y-up clip space on the way out. Projecting a point to the screen and
back (`core_document::runtime`, `world_to_viewport` and
`viewport_to_plane`) goes through helpers that use the same matrix; code
that maps between the screen and the world uses them rather than its own
maths.

- Switching between perspective and orthographic keeps the size of what is
  at the focal point.
- Changing the field of view (10° to 120°) moves the eye so the focal point
  keeps its size on screen. Only the amount of perspective changes.

## Navigation

| Action | Input | What happens |
| --- | --- | --- |
| Orbit | Middle drag | Turns about the focal point, or about the point picked under the cursor when that option is on |
| Orbit | Left drag on the orientation cube | Turns about the focal point, as a middle drag does |
| Pan | Right drag | Moves the eye; one pixel is one pixel at the focal plane |
| Roll | Left and right drag | Turns about the view direction |
| Zoom | Wheel | Perspective moves the eye; orthographic scales the height. Toward the cursor by default |
| Set pivot | Middle click on the model | The point under the cursor becomes the focal point |
| Set pivot | **H** | The cursor's point on the focal plane becomes the focal point |
| Fit | **F** | Isometric view of the whole scene. The view toolbar can also fit the selection |
| Home | The house by the orientation cube | The same isometric view of the whole scene, animated |
| Standard view | Click on the orientation cube | Turns to that face, edge or corner; a double click fits the whole scene there too |

A drag starts after 4 logical pixels, so a click never orbits. Perspective zoom keeps
the focal distance between the configured minimum and maximum (1 mm and
5000 mm by default). While a sketch is open, orbit (the cube's drag and home
included) is turned off.

Pan and orbit keep the camera and anchor from the press and use the total
pointer displacement. Results are independent of event frequency, and a picked
anchor keeps its off-centre screen position. UI scale is accounted for: orbit
speed uses logical pixels, and pan follows the scene by the physical displacement.
Wheel input during a hold changes its total zoom; the next pointer event keeps it.

New settings use world-up orbit: yaw turns around the preset's vertical axis,
pitch stops exactly at the top and bottom, and a saved roll does not accumulate.
An explicit saved Camera-up setting keeps free rotation around the screen axes.
Changing view, resizing, changing UI scale or losing focus ends the held snapshot.
Pan, zoom and roll stay available while a sketch locks out orbit.

The adapter freezes an `f64` model origin at the press and performs the hold
in a local `f32` frame, so model coordinates do not swallow small pointer motions.
Rendering retains the app's Vulkan projection and clipping convention.

Fit accounts for the smaller viewport dimension and shows a bounding sphere
whole in both portrait and landscape windows. View commands can frame models
beyond the wheel's focal-distance limits; those limits govern zoom gestures.

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
and focal distance on a log scale. A zero duration applies the endpoint on the
next camera update. Any navigation input stops a running animation.

The shared crate also provides smoothing and optional momentum primitives.
The app uses immediate pointer navigation with no inertia tail.

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
