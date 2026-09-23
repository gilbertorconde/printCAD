#version 450

layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec3 in_color;

layout(location = 0) out vec3 v_world_pos;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec3 v_color;

// Light structure (must match fragment shader)
struct Light {
    vec4 direction_intensity;
    vec4 color_enabled;
};

// Push-constant block split into two ranges:
//   - frame range (offset 0, 224 B): updated once per render pass.
//   - draw range  (offset 224, 16 B): updated once per body so the renderer
//     can change colour/highlight without forcing a vertex re-upload.
layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    vec4 camera_pos;
    Light light_main;
    Light light_back;
    Light light_fill;
    vec4 ambient;
    vec4 shading;
    vec4 clip_plane;        // keeps dot(xyz, p) + w >= 0; (0,0,0,1) keeps all
    vec4 draw_color;        // xyz = base color; w = highlight flags as float
} pc;

out gl_PerVertex {
    vec4 gl_Position;
    float gl_ClipDistance[1];
};

void main() {
    v_world_pos = in_pos;
    v_normal = normalize(in_normal);
    v_color = in_color;
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
    gl_ClipDistance[0] = dot(pc.clip_plane.xyz, in_pos) + pc.clip_plane.w;
}
