#version 450

layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec3 in_color;

layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    uvec4 object_id;  // Object ID encoded as 4 uints (UUID = 128 bits)
} pc;

void main() {
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
}

