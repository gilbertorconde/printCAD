#version 450

layout(location = 0) out vec4 out_color;

// Same push layout as `mesh.frag`: per-draw `draw_color` after frame block
// (view_proj + camera + lights + ambient + shading).
layout(push_constant) uniform PushConstants {
    layout(offset = 208) vec4 draw_color;
} pc;

void main() {
    out_color = vec4(pc.draw_color.rgb, 1.0);
}
