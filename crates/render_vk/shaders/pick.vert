#version 450

// Vertex format matches the shared MeshCache layout (position, normal).
layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_normal;

layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    uvec4 object_id;  // Object ID encoded as 4 uints (UUID = 128 bits)
    vec4 clip_plane;  // keeps dot(xyz, p) + w >= 0: a clipped part never picks
} pc;

out gl_PerVertex {
    vec4 gl_Position;
    float gl_ClipDistance[1];
};

void main() {
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
    gl_ClipDistance[0] = dot(pc.clip_plane.xyz, in_pos) + pc.clip_plane.w;
}
