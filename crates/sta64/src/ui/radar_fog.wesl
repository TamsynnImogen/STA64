// Fog of war and shroud overlay for the radar. The four bilinear taps and
// cubic B-spline reconstruction follow render/fog.wesl (MIT; see
// ../render/fog.NOTICE.md).

import bevy_ui_render::ui_vertex_output::UiVertexOutput;

struct FogParams {
    fog: vec4<f32>,      // linear RGB and opacity (0.6)
    shroud: vec4<f32>,   // linear RGB and opacity (1.0)
    mask_uv: vec4<f32>,  // xy offset, zw scale applied to the UI UV
};

@group(1) @binding(0) var<uniform> k: FogParams;
@group(1) @binding(1) var mask: texture_2d<f32>;
@group(1) @binding(2) var mask_s: sampler;

// The red and green channels encode distance to the fog and shroud contours.
fn tap(p: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(mask, mask_s, p / size, 0.0).rg;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let uv = k.mask_uv.xy + in.uv * k.mask_uv.zw;
    let size = vec2<f32>(textureDimensions(mask));
    let t = uv * size - 0.5;
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
    let reconstructed =
        g0.y * (g0.x * tap(vec2<f32>(h0.x, h0.y), size) + g1.x * tap(vec2<f32>(h1.x, h0.y), size)) +
        g1.y * (g0.x * tap(vec2<f32>(h0.x, h1.y), size) + g1.x * tap(vec2<f32>(h1.x, h1.y), size));

    // Evaluate derivatives before the bounds branch so they remain valid in
    // uniform control flow, including pixels outside the mask grid.
    let edge_width = max(vec2<f32>(0.15), fwidth(reconstructed));
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(k.shroud.rgb, 1.0);
    }

    let fog_cover = smoothstep(0.5 - edge_width.x, 0.5 + edge_width.x, reconstructed.x);
    let shroud_cover = smoothstep(0.5 - edge_width.y, 0.5 + edge_width.y, reconstructed.y);
    let fog_alpha = fog_cover * k.fog.a;
    let shroud_alpha = shroud_cover * k.shroud.a;

    // Straight-alpha output for ALPHA_BLENDING: shroud is composited above fog.
    let alpha = shroud_alpha + fog_alpha * (1.0 - shroud_alpha);
    let premultiplied = k.shroud.rgb * shroud_alpha +
        k.fog.rgb * fog_alpha * (1.0 - shroud_alpha);
    let rgb = premultiplied / max(alpha, 0.000001);
    return vec4<f32>(rgb, alpha);
}
