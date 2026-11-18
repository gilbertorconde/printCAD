// Screen-space line segments drawn among the UI's own shapes: one quad per
// segment, its edges and ends faded over a pixel, dashes cut per fragment.

struct Segment {
    // Start and end, in pixels from the drawing area's corner.
    @location(0) ends: vec4<f32>,
    // Colour as the UI gives it (not linear), alpha apart.
    @location(1) color: vec4<f32>,
    // Thickness in pixels, dash, gap, how far into the pattern it starts.
    @location(2) style: vec4<f32>,
    // The drawing area's size in pixels.
    @location(3) area: vec2<f32>,
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Pixels across from the middle line, and along from the start.
    @location(1) across: f32,
    @location(2) along: f32,
    @location(3) @interpolate(flat) style: vec4<f32>,
    @location(4) @interpolate(flat) length: f32,
}

@vertex
fn vs_line(@builtin(vertex_index) vertex: u32, segment: Segment) -> Varyings {
    let a = segment.ends.xy;
    let b = segment.ends.zw;
    let d = b - a;
    let length = length(d);
    var dir = vec2<f32>(1.0, 0.0);
    if (length > 1e-6) {
        dir = d / length;
    }
    let normal = vec2<f32>(-dir.y, dir.x);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vertex];
    // Half a pixel past each end and a pixel past each side, for the fade.
    let along = mix(-0.5, length + 0.5, corner.x);
    let across = corner.y * (segment.style.x * 0.5 + 1.0);
    let p = a + dir * along + normal * across;
    var out: Varyings;
    out.position = vec4<f32>(
        p.x / segment.area.x * 2.0 - 1.0,
        1.0 - p.y / segment.area.y * 2.0,
        0.0,
        1.0,
    );
    out.color = segment.color;
    out.across = across;
    out.along = along;
    out.style = segment.style;
    out.length = length;
    return out;
}

fn coverage(in: Varyings) -> f32 {
    let half = in.style.x * 0.5;
    let side = clamp(half + 0.5 - abs(in.across), 0.0, 1.0);
    let ends = clamp(min(in.along, in.length - in.along) + 0.5, 0.0, 1.0);
    var dashed = 1.0;
    let period = in.style.y + in.style.z;
    if (in.style.y > 0.0 && period > 0.0) {
        let s = in.style.w + in.along;
        let into = s - floor(s / period) * period;
        dashed = clamp(in.style.y - into + 0.5, 0.0, 1.0) * clamp(into + 0.5, 0.0, 1.0)
            + clamp(into - period + 0.5, 0.0, 1.0);
    }
    return side * ends * dashed;
}

// Onto a plain target: the colour as given, premultiplied.
@fragment
fn fs_line(in: Varyings) -> @location(0) vec4<f32> {
    let a = in.color.a * coverage(in);
    return vec4<f32>(in.color.rgb * a, a);
}

fn linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// Onto an sRGB target, which encodes what it is given.
@fragment
fn fs_line_linear(in: Varyings) -> @location(0) vec4<f32> {
    let a = in.color.a * coverage(in);
    let rgb = vec3<f32>(linear(in.color.r), linear(in.color.g), linear(in.color.b));
    return vec4<f32>(rgb * a, a);
}
