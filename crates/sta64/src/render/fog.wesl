// The fog of war and the shroud: one layer over the map, shaded from the grid's mask.
//
// Ported from Armada II Refit's render3d/mods/overlays/shaders/overlays.hlsl
// (`ps_fog_bspline`, `finish`; MIT, Copyright (c) 2026 Cedric Haase, see fog.NOTICE.md
// next to this file). The mask is a texture with one texel per cell of the grid:
// 1 where the layer hides the cell, 0 where it shows it. Filtered bilinearly that is a
// mask with a corner at every cell; here it is reconstructed with a cubic B-spline
// (four bilinear taps), so a disc of cells reads as a circle, and the edge is a
// smoothstep about a threshold of it, never narrower than a pixel or so (fwidth).
//
// Differences from Refit: its texture also carries the layer's art (alpha 1, rgb the
// art, hidden cells only), which is divided out of the filtered colour; here the layer
// is one colour and the mask is all the texture holds. The rim is kept: Refit's seen
// texels are black, so the art darkens towards an edge, to half at the cut. Here it is
// a shade from the mask: half at the threshold, full where the mask has risen by the
// rim's width above it.
//
// The taps are clamped to the texture, which is exactly the grid: nothing beyond the
// map is drawn, and the clamp keeps the shroud from thinning along its border.

struct FogParams {
    tint: vec4<f32>,  // rgb the layer's colour; w its opacity
    edge: vec4<f32>,  // x the mask level of the edge; y its softness; z the rim's strength; w 1 / the rim's width
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: FogParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var mask: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var mask_s: sampler;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

// One bilinear tap at p, in texel units (a texel's centre at i + 0.5).
fn tap(p: vec2<f32>, size: vec2<f32>) -> f32 {
    return textureSampleLevel(mask, mask_s, p / size, 0.0).r;
}

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(mask));
    let t = in.uv * size - 0.5;
    let f = fract(t);
    let n = t - f;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = n - 0.5 + w1 / g0;
    let h1 = n + 1.5 + w3 / g1;
    let a = g0.y * (g0.x * tap(vec2<f32>(h0.x, h0.y), size) + g1.x * tap(vec2<f32>(h1.x, h0.y), size)) +
            g1.y * (g0.x * tap(vec2<f32>(h0.x, h1.y), size) + g1.x * tap(vec2<f32>(h1.x, h1.y), size));

    let w = max(k.edge.y, fwidth(a));
    let cover = smoothstep(k.edge.x - w, k.edge.x + w, a);
    let shade = clamp(0.5 + 0.5 * (a - k.edge.x) * k.edge.w, 0.0, 1.0);
    let rgb = k.tint.rgb * mix(1.0, shade, k.edge.z);
    return vec4<f32>(rgb, cover * k.tint.a);
}
