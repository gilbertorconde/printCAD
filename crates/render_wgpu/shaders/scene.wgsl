// The scene's shaders: shaded solids, face-boundary edges, the pick pass
// and the blit that puts the cached scene under the UI.
//
// The camera's `view_proj` is built for a framebuffer whose Y runs down
// (the app's convention); wgpu's clip space has Y up, so every vertex stage
// flips Y on the way out. Depth runs 0 to 1.
//
// The clipping plane is applied per fragment, by discarding what lies on its
// far side, so it works on every backend, the web's included.

struct Light {
    // xyz = direction, w = intensity.
    direction_intensity: vec4<f32>,
    // rgb = colour, a = enabled (>= 0.5).
    color_enabled: vec4<f32>,
}

// Written once per frame.
struct Frame {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_main: Light,
    light_back: Light,
    light_fill: Light,
    // rgb = ambient colour times intensity.
    ambient: vec4<f32>,
    // x = specular exponent, y = specular intensity.
    shading: vec4<f32>,
    // Keeps dot(xyz, p) + w >= 0; (0, 0, 0, 1) keeps everything.
    clip_plane: vec4<f32>,
    // xy = the viewport's size in pixels, z = the edge width in pixels.
    viewport: vec4<f32>,
}

// One per body, at a dynamic offset.
struct Draw {
    // The body's mesh to the scene: identity, or the rigid motion of a
    // body drawing a mesh it shares.
    model: mat4x4<f32>,
    // rgb = the body's colour, highlight mixed in; a = opacity.
    face_color: vec4<f32>,
    // rgb = its edges' colour.
    edge_color: vec4<f32>,
    // The body's id, its UUID as four words.
    object_id: vec4<u32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;

fn to_clip(p: vec3<f32>) -> vec4<f32> {
    var c = frame.view_proj * vec4<f32>(p, 1.0);
    c.y = -c.y;
    return c;
}

fn placed(p: vec3<f32>) -> vec3<f32> {
    return (draw.model * vec4<f32>(p, 1.0)).xyz;
}

fn cut_away(p: vec3<f32>) -> bool {
    return dot(frame.clip_plane.xyz, p) + frame.clip_plane.w < 0.0;
}

// ---------------------------------------------------------------- solids

struct MeshOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
}

@vertex
fn vs_mesh(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
) -> MeshOut {
    var out: MeshOut;
    let world = placed(position);
    out.position = to_clip(world);
    out.world = world;
    out.normal = (draw.model * vec4<f32>(normal, 0.0)).xyz;
    out.color = color;
    return out;
}

fn lambert(light: Light, normal: vec3<f32>) -> vec3<f32> {
    if (light.color_enabled.a < 0.5) {
        return vec3<f32>(0.0);
    }
    let dir = normalize(light.direction_intensity.xyz);
    let ndotl = max(dot(normal, dir), 0.0);
    return light.color_enabled.rgb * light.direction_intensity.w * ndotl;
}

// Blinn-Phong highlight; stronger than Lambert alone for a shaded solid.
fn spec_one(light: Light, normal: vec3<f32>, half_vec: vec3<f32>, shininess: f32) -> vec3<f32> {
    if (light.color_enabled.a < 0.5) {
        return vec3<f32>(0.0);
    }
    let ndoth = max(dot(normal, half_vec), 0.0);
    return pow(ndoth, shininess) * light.direction_intensity.w * light.color_enabled.rgb;
}

@fragment
fn fs_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    // A translucent draw is paint over a surface: its colour is the paint's
    // alone, not the material's underneath.
    let tint = draw.face_color;
    var albedo = in.color * tint.rgb;
    if (tint.a < 1.0) {
        albedo = tint.rgb;
    }
    // Under a clipping plane, the inside of a cut solid shows through the
    // cut: its back faces draw as a flat, darker section so the cut reads
    // as material rather than as a hollow shell.
    if (!front && dot(frame.clip_plane.xyz, frame.clip_plane.xyz) > 0.0) {
        return vec4<f32>(albedo * 0.55, tint.a);
    }
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let view_dir = normalize(frame.camera_pos.xyz - in.world);
    let shininess = max(frame.shading.x, 1.0);
    let spec_k = frame.shading.y;

    let diffuse = frame.ambient.rgb
        + lambert(frame.light_main, n)
        + lambert(frame.light_back, n)
        + lambert(frame.light_fill, n);

    var spec_sum = vec3<f32>(0.0);
    if (spec_k > 1e-6) {
        let l0 = normalize(frame.light_main.direction_intensity.xyz);
        let l1 = normalize(frame.light_back.direction_intensity.xyz);
        let l2 = normalize(frame.light_fill.direction_intensity.xyz);
        spec_sum += spec_one(frame.light_main, n, normalize(l0 + view_dir), shininess);
        spec_sum += spec_one(frame.light_back, n, normalize(l1 + view_dir), shininess);
        spec_sum += spec_one(frame.light_fill, n, normalize(l2 + view_dir), shininess);
    }
    // Neutral grey specular tint, not multiplied by the albedo.
    let spec_tint = vec3<f32>(0.52);
    let color = clamp(albedo * diffuse + spec_k * spec_tint * spec_sum, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(color, tint.a);
}

// ----------------------------------------------------------------- edges

// A tiny pull toward the near plane so on-surface edges win the depth tie
// with the face they bound. Keep it small: a large nudge lets lines show
// through what should hide them.
const EDGE_CLIP_Z_EPS: f32 = 4.5e-5;

struct EdgeOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
}

// Each edge segment is one instance, drawn as a quad of six vertices the
// width of `frame.viewport.z` pixels, with square ends so a chain of
// segments meets without gaps at its corners. GPUs draw one-pixel lines
// only, on the web always; a quad draws any width everywhere.
@vertex
fn vs_edge(
    @builtin(vertex_index) vertex: u32,
    @location(0) a_own: vec3<f32>,
    @location(1) b_own: vec3<f32>,
) -> EdgeOut {
    var out: EdgeOut;
    let a = placed(a_own);
    let b = placed(b_own);
    var ca = to_clip(a);
    var cb = to_clip(b);
    var wa = a;
    var wb = b;
    // Clip the segment to the near plane (z >= 0) before dividing by w.
    if (ca.z < 0.0 && cb.z < 0.0) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.world = a;
        return out;
    }
    if (ca.z < 0.0) {
        let t = ca.z / (ca.z - cb.z);
        ca = mix(ca, cb, t);
        wa = mix(a, b, t);
    }
    if (cb.z < 0.0) {
        let t = cb.z / (cb.z - ca.z);
        cb = mix(cb, ca, t);
        wb = mix(b, a, t);
    }
    let half_size = frame.viewport.xy * 0.5;
    let sa = ca.xy / ca.w * half_size;
    let sb = cb.xy / cb.w * half_size;
    var dir = sb - sa;
    let len = length(dir);
    if (len < 1e-6) {
        dir = vec2<f32>(1.0, 0.0);
    } else {
        dir = dir / len;
    }
    // The width is measured along the screen axis nearer the line's
    // direction, as GPUs draw wide lines: a diagonal reads as thin as it
    // always has rather than heavier.
    let half_width = frame.viewport.z * 0.5 * max(abs(dir.x), abs(dir.y));
    // Corners, as two triangles: 0 1 2, 2 1 3. Even corners lie on one
    // side of the segment, odd on the other; 2 and 3 are at `b`.
    var corners = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corners[vertex % 6u];
    let at_b = corner >= 2u;
    let side = select(-1.0, 1.0, (corner & 1u) == 1u);
    let along = select(-1.0, 1.0, at_b);
    var c = select(ca, cb, at_b);
    let offset_px = vec2<f32>(-dir.y, dir.x) * side * half_width + dir * along * half_width;
    c = vec4<f32>(c.xy + offset_px / half_size * c.w, c.z - EDGE_CLIP_Z_EPS * c.w, c.w);
    out.position = c;
    out.world = select(wa, wb, at_b);
    return out;
}

@fragment
fn fs_edge(in: EdgeOut) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    return vec4<f32>(draw.edge_color.rgb, 1.0);
}

// ------------------------------------------------------------------ pick

struct PickOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
}

@vertex
fn vs_pick(@location(0) position: vec3<f32>) -> PickOut {
    var out: PickOut;
    let world = placed(position);
    out.position = to_clip(world);
    out.world = world;
    return out;
}

struct PickTexel {
    @location(0) id: vec4<u32>,
    // The fragment's depth again, its bits in a colour target: a depth
    // texture cannot be copied in part, and the readback wants only the
    // cursor's neighbourhood. An integer target, since GL hardware cannot
    // render to a float one.
    @location(1) depth: u32,
}

// The body's id and its depth; the depth test keeps the nearest.
@fragment
fn fs_pick(in: PickOut) -> PickTexel {
    if (cut_away(in.world)) {
        discard;
    }
    var texel: PickTexel;
    texel.id = draw.object_id;
    texel.depth = bitcast<u32>(in.position.z);
    return texel;
}
