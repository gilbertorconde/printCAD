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

// Push-constant block split into two ranges:
//   - frame range (offset 0): updated once per render pass.
//   - draw range  (offset MESH_DRAW_PUSH_OFFSET): per-body draw_color.
layout(push_constant) uniform PushConstants {
    mat4 view_proj;
    vec4 camera_pos;
    Light light_main;
    Light light_back;
    Light light_fill;
    vec4 ambient;       // rgb = ambient color * intensity
    vec4 shading;       // x = specular exponent, y = specular intensity, zw unused
    vec4 draw_color;    // xyz = final body color (already highlight-mixed); w unused
} pc;

vec3 lambert(Light light, vec3 normal) {
    if (light.color_enabled.a < 0.5) {
        return vec3(0.0);
    }
    vec3 light_dir = normalize(light.direction_intensity.xyz);
    float intensity = light.direction_intensity.w;
    vec3 color = light.color_enabled.rgb;
    float ndotl = max(dot(normal, light_dir), 0.0);
    return color * intensity * ndotl;
}

// Blinn-Phong highlight; stronger than pure Lambert for a typical CAD shaded solid.
vec3 spec_one(Light light, vec3 normal, vec3 half_vec, float shininess) {
    if (light.color_enabled.a < 0.5) {
        return vec3(0.0);
    }
    float intensity = light.direction_intensity.w;
    vec3 color = light.color_enabled.rgb;
    float ndoth = max(dot(normal, half_vec), 0.0);
    float spec = pow(ndoth, shininess) * intensity;
    return spec * color;
}

void main() {
    vec3 n = normalize(v_normal);
    if (!gl_FrontFacing) {
        n = -n;
    }

    vec3 view_dir = normalize(pc.camera_pos.xyz - v_world_pos);
    float shininess = max(pc.shading.x, 1.0);
    float spec_k = pc.shading.y;

    vec3 diffuse = pc.ambient.rgb
        + lambert(pc.light_main, n)
        + lambert(pc.light_back, n)
        + lambert(pc.light_fill, n);

    vec3 spec_sum = vec3(0.0);
    if (spec_k > 1e-6) {
        vec3 L0 = normalize(pc.light_main.direction_intensity.xyz);
        vec3 L1 = normalize(pc.light_back.direction_intensity.xyz);
        vec3 L2 = normalize(pc.light_fill.direction_intensity.xyz);
        spec_sum += spec_one(pc.light_main, n, normalize(L0 + view_dir), shininess);
        spec_sum += spec_one(pc.light_back, n, normalize(L1 + view_dir), shininess);
        spec_sum += spec_one(pc.light_fill, n, normalize(L2 + view_dir), shininess);
    }

    vec3 albedo = v_color * pc.draw_color.rgb;
    // Neutral grey specular tint (~0.52); not multiplied by albedo.
    const vec3 spec_tint = vec3(0.52);
    vec3 color = clamp(albedo * diffuse + spec_k * spec_tint * spec_sum, 0.0, 1.0);
    out_color = vec4(color, 1.0);
}
