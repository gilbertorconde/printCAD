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
| Radial menu | Right press held still | Opens the workbench's slices round the cursor; letting go over one starts it, in the middle nothing. A quick right click is the view's menu |
| Roll | Left and right drag | Turns about the view direction |
| Zoom | Wheel | Perspective moves the eye; orthographic scales the height. Toward the cursor by default |
| Set pivot | Middle click on the model | The point under the cursor becomes the focal point |
| Set pivot | **H** | The cursor's point on the focal plane becomes the focal point |
| Fit | **F** | Isometric view of the whole scene, a feature being edited included. In a sketch being edited it frames the sketch without turning the view. The view toolbar can also fit the selection |
| Home | The house by the orientation cube | The same isometric view of the whole scene, animated |
| Standard view | Click on the orientation cube | Turns to that face, edge or corner; a double click fits the whole scene there too |
| Nearest view | Shift and right click | Turns to the face, edge or corner view nearest the one on screen, about the focal point, the preset's up kept up |
| Zoom | Pinch on a touchpad | As the wheel |

The view toolbar floats over the top of the view. When the view is too
narrow for all of it, its less-used groups (the rear, bottom and left views,
print bed and annotations first, then draw styles, the clipping plane and
the field of view, then the other standard views and the projection) move
one at a time into a **More view tools** menu at its end; fitting, Measure
and the pick filter always stay.

Preferences › Input › Pointing device picks how a touchpad's two-finger
scroll acts. With **Mouse** (the default) it zooms, as a wheel does. With
**Touchpad** it orbits about the focal point, with Shift held pans, and with
Ctrl held zooms; a wheel's notches still zoom, and while a sketch locks out
orbit the scroll pans. The mapping is in one place,
`CameraController::on_viewport_pointer`.

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
Rendering keeps the renderer's projection and clipping convention, the same on every wgpu backend.

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

The view toolbar's clipping plane (`section.rs`) starts square to X, Y or
Z, through the middle of the scene on the axis the view looks along most
directly, and keeps the far half. It belongs to the view, not the
document. The renderer and picking both respect it.

Its pill aims it:

- **X, Y, Z**: square to that axis, through the scene's middle there. The
  offset is the coordinate along the axis.
- **View**: square to the view as it is, through the scene's middle.
- **Face**: on a flat face's plane or a datum plane. A face or datum plane
  already selected is taken at once; otherwise the pill asks for a click on
  one.
- **3 points**: through three points clicked in the view, on faces, edges
  or datums. Three points in a line ask for the third again.

An aimed plane keeps the side away from the eye. The offset moves it along
its normal from where it was aimed; the field takes a formula
(`Plate.thickness / 2`), which the plane follows as the variables change.
The arrow standing on the plane drags the offset in 0.1 mm steps (Escape
puts it back), and Flip keeps the other side. Escape also stops a pick
that aims the plane.

Where it cuts a closed body, the cut shows as a flat cap of material;
where several closed bodies are cut, each cap is hatched at its own
angle so nested parts read apart. A body whose mesh does not close (a
scan's open shell) shows its inside lit, as the shell it is. A sketch's
section view cuts the same way.
