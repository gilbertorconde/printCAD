#version 450

layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_normal;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;

// Light structure (must match fragment shader)
struct Light {
    vec4 direction_intensity;
    vec4 color_enabled;
};

// Push-constant block split into two ranges:
//   - frame range (offset 0, 192 B): updated once per render pass.
//   - draw range  (offset 192, 16 B): updated once per body so the renderer
//     can change colour/highlight without forcing a vertex re-upload.
layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    vec4 camera_pos;
    Light light_main;
    Light light_back;
    Light light_fill;
    vec4 ambient;
    vec4 draw_color;        // xyz = base color; w = highlight flags as float
} pc;

void main() {
    v_world_pos = in_pos;
    v_normal = normalize(in_normal);
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
}
