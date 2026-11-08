struct Grid {
    view_proj: mat4x4<f32>, camera_step: vec4<f32>, clip_plane: vec4<f32>,
    anchor_radius: vec4<f32>, u_minor: vec4<f32>, v_major: vec4<f32>,
    center_axis: vec4<f32>, forward_near: vec4<f32>, color_far: vec4<f32>,
    u_axis_color: vec4<f32>, v_axis_color: vec4<f32>, dash: vec4<f32>,
    clip_wedge: vec4<f32>,
}
@group(0) @binding(0) var<uniform> g: Grid;
struct Vertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) plane: vec2<f32>, @location(1) world: vec3<f32>,
}
@vertex fn vs_grid(@builtin(vertex_index) index: u32) -> Vertex {
    let corners = array<vec2<f32>, 6>(vec2(-1., -1.), vec2(1., -1.), vec2(1., 1.), vec2(-1., -1.), vec2(1., 1.), vec2(-1., 1.));
    let corner = corners[index] * g.anchor_radius.w;
    let world = g.center_axis.xyz + g.u_minor.xyz * corner.x + g.v_major.xyz * corner.y;
    let rel = world - g.anchor_radius.xyz;
    var clip = g.view_proj * vec4(world, 1.);
    clip.y = -clip.y;
    clip.z += 2e-5 * clip.w;
    return Vertex(clip, vec2(dot(rel, g.u_minor.xyz), dot(rel, g.v_major.xyz)), world);
}
fn strength(px: f32) -> f32 {
    return g.u_minor.w * smoothstep(3., 14., px) + (g.v_major.w - g.u_minor.w) * smoothstep(40., 160., px);
}
// How much of a line is drawn `px` pixels along it: all of it while the
// grid is solid, else the drawn part of each period, its ends softened.
fn dash_mask(px: f32) -> f32 {
    if g.dash.x <= 0. { return 1.; }
    let t = fract(px / g.dash.x) * g.dash.x;
    return clamp(g.dash.y + .5 - t, 0., 1.) * clamp(t + .5, 0., 1.);
}
fn coverage(plane: vec2<f32>, spacing: f32) -> vec2<f32> {
    let c = plane / spacing;
    let d = max(fwidth(c), vec2(1e-6));
    let off = abs(fract(c - .5) - .5) / d;
    let lines = vec2(1.) - min(off, vec2(1.));
    // Pixels along each line: a line of constant x runs along y.
    let along = plane / max(fwidth(plane), vec2(1e-9));
    let drawn = max(lines.x * dash_mask(along.y), lines.y * dash_mask(along.x));
    return vec2(drawn, 1. / max(d.x, d.y));
}
fn axis_line(value: f32, at: f32) -> f32 {
    return clamp(1.25 - abs(value - at) / max(fwidth(value), 1e-9), 0., 1.);
}
@fragment fn fs_grid(v: Vertex) -> @location(0) vec4<f32> {
    var alpha = 0.;
    var spacing = g.camera_step.w;
    for (var level = 0; level < 3; level += 1) {
        let c = coverage(v.plane, spacing);
        alpha = max(alpha, c.x * strength(c.y));
        spacing *= 10.;
    }
    let on_u = axis_line(v.plane.y, g.v_axis_color.w) * g.center_axis.w;
    let on_v = axis_line(v.plane.x, g.u_axis_color.w) * g.center_axis.w;
    let a = max(on_u, on_v);
    let axis = select(g.v_axis_color.rgb, g.u_axis_color.rgb, on_u >= on_v);
    let color = mix(g.color_far.rgb, axis, a / max(max(a, alpha), 1e-6));
    alpha = max(alpha, a);
    let r = length(v.world - g.center_axis.xyz) / g.anchor_radius.w;
    let depth = dot(v.world - g.camera_step.xyz, g.forward_near.xyz);
    let span = max(g.color_far.w - g.forward_near.w, 1e-6);
    alpha *= (1. - smoothstep(.45, 1., r)) * (1. - smoothstep(g.forward_near.w + .8 * span, g.color_far.w, depth));
    let cut = dot(g.clip_plane, vec4(v.world, 1.)) < 0. && dot(g.clip_wedge, vec4(v.world, 1.)) < 0.;
    if alpha < 1. / 255. || cut { discard; }
    return vec4(color, alpha);
}
