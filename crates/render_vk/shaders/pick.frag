#version 450

layout(location = 0) out uvec4 out_id;

layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    uvec4 object_id;  // Object ID encoded as 4 uints (UUID = 128 bits)
    vec4 clip_plane;  // keeps dot(xyz, p) + w >= 0: a clipped part never picks
} pc;

void main() {
    // Output the object ID directly - the depth is automatically written to depth buffer
    out_id = pc.object_id;
}

