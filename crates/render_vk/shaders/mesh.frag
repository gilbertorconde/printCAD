#version 450

layout(location = 0) in vec3 v_world_pos;
layout(location = 1) in vec3 v_normal;
layout(location = 2) in vec3 v_color;

layout(location = 0) out vec4 out_color;

// Light structure: direction_intensity (xyz=dir, w=intensity), color_enabled (rgb=color, a=enabled)
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
    vec4 ambient;  // rgb = ambient color * intensity
} pc;

vec3 compute_light(Light light, vec3 normal) {
    if (light.color_enabled.a < 0.5) {
        return vec3(0.0);
    }
    vec3 light_dir = normalize(light.direction_intensity.xyz);
    float intensity = light.direction_intensity.w;
    vec3 color = light.color_enabled.rgb;
    float ndotl = max(dot(normal, light_dir), 0.0);
    return color * intensity * ndotl;
}

void main() {
    vec3 normal = normalize(v_normal);
    
    // Compute contribution from each light
    vec3 main_contrib = compute_light(pc.light_main, normal);
    vec3 back_contrib = compute_light(pc.light_back, normal);
    vec3 fill_contrib = compute_light(pc.light_fill, normal);
    
    // Combine all lighting
    vec3 lighting = pc.ambient.rgb + main_contrib + back_contrib + fill_contrib;
    
    vec3 color = clamp(v_color * lighting, 0.0, 1.0);
    out_color = vec4(color, 1.0);
}
