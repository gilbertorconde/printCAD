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

layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    vec4 camera_pos;
    Light light_main;
    Light light_back;
    Light light_fill;
    vec4 ambient;
} pc;

void main() {
    v_world_pos = in_pos;
    v_normal = normalize(in_normal);
    v_color = in_color;
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
}
