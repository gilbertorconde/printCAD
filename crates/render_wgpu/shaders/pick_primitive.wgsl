// The pick pass's fragment stage on a device that numbers primitives: the
// body's id and depth as `fs_pick` writes them, and the triangle drawn, so
// the host reads the face under the cursor rather than searching for it.
// It is appended to scene.wgsl behind `enable primitive_index;`
// (`PRIMITIVE_PICK_SOURCE` in mesh.rs) and built only where the device has
// the feature.

struct PrimitivePickTexel {
    @location(0) id: vec4<u32>,
    @location(1) depth: u32,
    // The triangle's index in the body's mesh plus one; zero is the
    // cleared texel. A body is one indexed draw from its first index, so
    // the primitive index is the triangle's place in the mesh.
    @location(2) primitive: u32,
}

@fragment
fn fs_pick_primitive(
    in: PickOut,
    @builtin(primitive_index) primitive: u32,
) -> PrimitivePickTexel {
    if (cut_away(in.world)) {
        discard;
    }
    var texel: PrimitivePickTexel;
    texel.id = draw.object_id;
    texel.depth = bitcast<u32>(in.position.z);
    texel.primitive = primitive + 1u;
    return texel;
}
