#version 450

// Reuses the same per-vertex format as the solid mesh pipeline so we can
// share the vertex buffer; only the position is consulted.
layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_normal;

struct Light {
    vec4 direction_intensity;
    vec4 color_enabled;
};

// Push-constant block must match the solid pipeline's layout exactly because
// both pipelines share a `vk::PipelineLayout`. Unused fields are tolerated.
layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    vec4 camera_pos;
    Light light_main;
    Light light_back;
    Light light_fill;
    vec4 ambient;
    vec4 draw_color;
} pc;

void main() {
    vec4 clip = pc.view_proj * vec4(in_pos, 1.0);
    // Tiny toward-near-plane pull so LEQUAL can tie on-surface edges. Keep this
    // small: a large nudge stacks with raster bias and lets lines pass depth
    // where they should be occluded.
    const float EDGE_CLIP_Z_EPS = 4.5e-5;
    clip.z -= EDGE_CLIP_Z_EPS * clip.w;
    gl_Position = clip;
}
