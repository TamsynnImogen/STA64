// The Key's shadow map: how far from the Key's plane each hull's nearest surface is.
//
// After Armada II Refit's render3d/mods/hulls/shaders/hulls.hlsl `ps_depth` (MIT,
// Copyright (c) 2026 Cedric Haase, see hulls.NOTICE.md next to this file): the map holds
// the depth along the Key, 0 at the plane nearest the Key and 1 at the farthest, and 1
// where no hull is. Refit writes it to an R32F target. Bevy draws a camera into a
// half-float main texture and blits it, so the depth is split in two halves: r is the
// depth rounded to a half float, g what the rounding left over (scaled up), and
// `shadows::DEPTH_RESIDUAL_SCALE` of the Rust side is the scale used here. Read back as
// r + g / scale, it carries about 22 bits.

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
};

const RESIDUAL_SCALE: f32 = 2048.0;

@fragment
fn fragment(i: FragIn) -> @location(0) vec4<f32> {
    // The camera's projection is orthographic and reverse-Z: 1 at its near plane, 0 at
    // its far plane, linear between.
    let d = saturate(1.0 - i.position.z);
    let r = unpack2x16float(pack2x16float(vec2<f32>(d, 0.0))).x;
    return vec4<f32>(r, (d - r) * RESIDUAL_SCALE, 0.0, 1.0);
}
