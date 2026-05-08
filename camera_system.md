Most of the FreeCAD wiki and forum pages are blocked by Anubis bot protection, but I got enough from the Gesture Navigation template + general knowledge of FreeCAD's Coin3D-based architecture to give you an accurate spec. Here's a description you can hand to Cursor.

---

# FreeCAD Camera System — Implementation Spec

## Architecture overview

FreeCAD uses **Coin3D** (an Open Inventor implementation) under the hood. The camera is a `SoCamera` node — either `SoPerspectiveCamera` or `SoOrthographicCamera` — and **all navigation is done by mutating the camera's transform and its projection parameters, never by moving the scene**.

Key state held by the camera:
- `position` (eye in world space)
- `orientation` (quaternion — view direction)
- `focalDistance` (scalar — distance from eye to the **point of interest / pivot**)
- `nearDistance`, `farDistance`
- `height` (ortho only) **or** `heightAngle` (perspective only)

The **focal point** is derived: `focalPoint = position + orientation * (0, 0, -focalDistance)`. This is the pivot for rotation and the anchor for zoom.

## Two camera modes, switchable at runtime

1. **Perspective**: uses `heightAngle` (FOV).
2. **Orthographic**: uses `height` (world-space height of the viewport).

When toggling, FreeCAD preserves `position`, `orientation`, and `focalDistance`, and computes the new projection parameter so the framing stays roughly identical at the focal plane:
```text
ortho.height = 2 * focalDistance * tan(persp.heightAngle / 2)
```
And the inverse when going ortho → perspective.

## Zoom

Zoom behaves **differently per camera type** — this is the key trick.

### Orthographic zoom
- Do **not** move the eye.
- Scale `camera.height *= factor` (e.g. `factor = pow(0.95, wheel_steps)`).
- Optionally pan so the world point under the cursor stays under the cursor (zoom-to-cursor).

### Perspective zoom
FreeCAD dollies the camera along its view direction **and** updates `focalDistance` so the pivot stays put in world space:
```text
step = focalDistance * (1 - factor)        # positive = zoom in
position += view_direction * step
focalDistance -= step                       # focal POINT unchanged in world
```
With **zoom-at-cursor** enabled, it also slides `position` and the pivot laterally so the world point under the cursor stays under the cursor.

### Clamps (this is what prevents your clipping issue)
- `focalDistance` is clamped to a minimum (so you can't dolly through the pivot).
- Wheel zoom is **exponential** (`factor^steps`), never linear — this gives smooth feel across scales.
- Optional zoom-step setting in preferences.

## Rotation (orbit)

- Pivot = the **focal point** (`position + orientation * -Z * focalDistance`).
- Mouse drag → two rotations: yaw around world up (or camera up, configurable) and pitch around the camera's right axis.
- Implemented as quaternion composition; the eye is then re-derived:
  ```text
  position = focalPoint - new_orientation * -Z * focalDistance
  ```
- This guarantees the focal point is invariant under rotation — that's the whole reason FreeCAD stores `focalDistance` rather than a separate pivot.

### Setting the pivot
FreeCAD lets the user set the pivot explicitly, which is the **real fix for the "zoom into the part" problem**:
- **Middle-click on geometry** → raycast hit point becomes the new focal point. `focalDistance` is updated to `|hit - position|`; orientation unchanged.
- **`H` key** → same, using current cursor position.
- **Fit-to-view / Fit-selection** → focal point set to the bbox center of (selection or scene), and `position` / `height` (or `focalDistance`) recomputed so the bbox fills the viewport with a small margin.

## Pan

- Convert mouse delta from pixels → world units at the focal plane:
  ```text
  world_per_pixel = (ortho.height or 2*focalDistance*tan(heightAngle/2)) / viewport_height_px
  ```
- Translate **both** `position` and the focal point by `(-dx, +dy)` in camera space (scaled by `world_per_pixel`).
- Focal distance unchanged.

## Tilt (roll)

- Rotate `orientation` around the camera's **forward (-Z)** axis.
- Triggered in Gesture nav by holding LMB+RMB and moving sideways.

## Near / Far auto-adjustment

Coin3D has `SoCamera::viewAll()` and an "auto clipping" mode that FreeCAD enables. Each frame (or on camera change):
1. Compute scene AABB.
2. Project its 8 corners into camera space; find min/max Z along view direction.
3. Set:
   ```text
   near = max(min_z, focalDistance * NEAR_RATIO)   # NEAR_RATIO ~ 0.001..0.01
   far  = max_z + margin
   ```
4. For perspective, enforce `near/far` ratio cap (e.g. `far/near < 1e5`) to preserve depth precision.

This is **for Z-precision**, not to prevent geometry clipping — geometry clipping is prevented by the **focal-distance clamp + raycast pivot**.

## Navigation styles (input mapping layer)

FreeCAD separates *what the camera does* from *how input triggers it*. Each style is a state machine mapping mouse/keyboard events → `zoom / rotate / pan / tilt` calls. The default **Gesture** style:

| Action | Input |
|---|---|
| Select | LMB click |
| Zoom | Mouse wheel (zoom-at-cursor) |
| Rotate | LMB drag (drag from empty space) |
| Pan | RMB drag |
| Tilt | LMB+RMB drag horizontally |
| Set pivot | MMB click on geometry, or `H` over cursor |
| Fit all | `V, F` or View menu |

Other styles (CAD, Blender, Maya, Touchpad, etc.) just remap inputs to the same underlying camera operations.

## Animation

Transitions (e.g. clicking a face of the Navigation Cube, or `0`–`9` view presets) are **interpolated**:
- Slerp `orientation` from current to target.
- Lerp `position` and `focalDistance`.
- Run over ~300–500 ms with ease-in-out.

## Implementation order for Cursor

1. `Camera` class holding `{position, orientation (quat), focalDistance, projection (persp|ortho), heightAngle, height, near, far}`. Derived getter `focalPoint()`.
2. `pan(dx, dy)`, `orbit(yaw, pitch)`, `roll(angle)`, `dolly(factor, cursor=None)`, `setPivot(worldPoint)`, `fit(aabb)`.
3. Auto near/far pass running each frame from scene AABB.
4. Input layer (NavigationStyle) — start with one mapping, make it swappable.
5. Raycast-on-MMB for pivot setting → this is what kills your clipping problem.
6. View presets + slerp animation.

The single most important takeaway: **store `focalDistance` as first-class camera state, and make every operation preserve or explicitly update it**. That one design choice is what makes orbit, zoom, and pan all behave "like CAD."

Yes — there are several spots where naïve implementations break in subtle ways. Here are the real landmines, in roughly the order they'll bite you:

---

## 1. Quaternion drift and the "gimbal-ish" feel

If you accumulate orbit rotations by **multiplying quaternions every frame**, floating-point error accumulates and the camera slowly tilts/rolls on its own.

**Fixes:**
- Renormalize the quaternion after every composition (`q = normalize(q)`).
- For yaw, decide **once** whether you rotate around **world up** or **camera up** — mixing them feels broken. FreeCAD/CAD convention: yaw around **world Z (or Y)**, pitch around **camera right**. This prevents unwanted roll.
- If you want true trackball behavior (Blender-style), that's a different model entirely (axis = perpendicular to drag in screen space). Don't mix the two.

---

## 2. Pitch singularity at the poles

When the view direction aligns with world up, yaw becomes undefined and the camera "flips."

**Fix:** clamp pitch to `[-89.9°, +89.9°]` if you're using world-up yaw. Or detect proximity to the pole and switch yaw axis to camera-up temporarily. Most CAD apps just clamp.

---

## 3. Zoom-to-cursor math is deceptively tricky

The naive version ("move toward cursor ray") drifts. The correct version:

```text
1. Before zoom: unproject cursor → get world point W on focal plane
2. Apply zoom (scale ortho.height OR dolly along view dir)
3. After zoom: unproject same cursor pixel → get world point W'
4. Pan camera by (W - W') so the point under cursor is invariant
```

For **perspective** zoom-to-cursor, you also need to dolly along the **cursor ray**, not the view direction — otherwise the cursor point slides. Then re-anchor with the pan correction above.

---

## 4. `focalDistance` clamp must survive every operation

It's easy to clamp on dolly but forget that:
- **Pan in screen space** can drag the focal point through geometry if you don't raycast.
- **Setting pivot via raycast** can place focal point *behind* the eye if you click a back-face — validate `dot(hit - position, forward) > 0`.
- **Fit-to-view** can collapse `focalDistance` to near-zero on a tiny selection — enforce a scene-scale-aware minimum, e.g. `max(MIN_ABS, scene_diag * 1e-4)`.

---

## 5. Auto near/far has nasty edge cases

- **Empty scene** → AABB is undefined → don't run the pass; use defaults.
- **Single point / degenerate AABB** → `min_z == max_z` → far/near collapses → division by zero in the projection matrix. Always enforce `far - near >= MIN_RANGE`.
- **Camera inside the AABB** → `min_z` goes negative. Don't naively `max(EPSILON, min_z)`; the right thing is to set near to a fraction of `focalDistance` and far to `max_z`.
- **Huge scene + tiny detail** → `far/near > 1e5` destroys depth precision. Cap the ratio; clip far if needed.
- Recompute on **camera change**, not every frame — recomputing while the user drags can cause visible "breathing" of clipped geometry.

---

## 6. Pan units must scale with zoom

If you pan by a fixed world-unit-per-pixel, panning feels glacial when zoomed out and hyper-twitchy when zoomed in.

**Always** compute pan delta from the focal-plane scale:

```text
ortho:      world_per_px = camera.height / viewport_height_px
perspective: world_per_px = 2 * focalDistance * tan(heightAngle/2) / viewport_height_px
```

Same applies to orbit speed if you want consistent feel — though orbit is usually angular and doesn't need scaling.

---

## 7. Ortho ↔ Perspective switching

The formula `ortho.height = 2 * focalDistance * tan(heightAngle/2)` only matches framing **at the focal plane**. Things in front/behind the focal plane will jump in size. Users expect this — but **don't** also try to reposition the camera to "compensate," it makes it worse.

Also: in ortho, `position` along the view axis is **almost meaningless** for what you see (only affects clipping). But it still matters for the near/far computation. Don't set ortho `position` to "infinity" — keep it at a sane distance from the focal point.

---

## 8. Coordinate system convention — pick once, document, never deviate

- **Right-handed vs left-handed**
- **Up axis**: Y-up (OpenGL/most 3D) or Z-up (CAD/CAM/engineering convention — FreeCAD uses Z-up)
- **Forward axis**: -Z (OpenGL) or +Y (some CAD)

Pick one tuple and enforce it in your `Camera` class. Half the bugs in homemade camera systems come from one function assuming Y-up and another assuming Z-up.

---

## 9. Animation interpolation pitfalls

- Use **slerp** (not lerp) for quaternions. Lerp gives non-uniform angular velocity and can take the long way around.
- Check the **dot product sign** before slerping; if negative, negate one quaternion (quaternions double-cover rotations — `q` and `-q` are the same orientation but slerp between them goes the long way).
- Lerp `focalDistance` in **log space** if zoom levels span orders of magnitude, otherwise the animation feels nonlinear.
- Cancel running animations on user input — don't queue them.

---

## 10. Input event ordering

- Wheel events fire fast — **accumulate** them per frame instead of running zoom math 30× per frame.
- Distinguish **click** from **drag** with a pixel threshold (e.g. 4 px) and a time threshold. Otherwise every click triggers a tiny orbit.
- On touchpads, pinch-zoom and two-finger-pan arrive as different event types per OS — abstract these behind `dolly()` / `pan()` calls early.

---

## 11. Raycast for pivot — pick the right hit

- Use the **first hit along the ray**, not the closest to the camera in world space (subtle difference when the ray origin is inside a bbox).
- If nothing is hit (clicked empty space), **don't** change the pivot — or fall back to "project cursor onto current focal plane." FreeCAD does the latter.
- Beware of clicking on **edges/vertices** in CAD — usually you want to snap to those, not to the face behind them. Implement a priority: vertex > edge > face.

---

## 12. Numerical precision at large scene scales

If your scene is in meters but the user models a building (1000s of units) or a watch part (0.001 units), **single-precision floats break down**:
- Z-fighting
- Jittery orbit
- Cursor unprojection off by pixels

**Fixes:**
- Keep camera math in **doubles**, convert to float only when uploading matrices to GPU.
- For very large scenes, use a **camera-relative origin**: subtract camera position from all geometry before sending to GPU (a.k.a. "floating origin").

---

## The two that will actually bite you first

If I had to bet on which ones break your build first:

1. **Zoom-to-cursor drift** (#3) — looks fine until users notice, then it's infuriating.
2. **`focalDistance` not preserved across some operation** (#4) — manifests as "rotation pivot mysteriously moved" bugs that are hard to reproduce.

Get those two right early and the rest is incremental polish.