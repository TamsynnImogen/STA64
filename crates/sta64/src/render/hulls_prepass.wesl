// Hulls, depth prepass (and shadow depth pass) of a cut-out part: the colour pass's alpha test, so a texel
// it rejects writes no depth and what lies behind it (a moon's glow shell, the sky) is not hidden by it.
// Bevy runs this only for a mask material (`HullMaterial::alpha_mode`); an opaque part keeps the
// depth-only pipeline. The uniform struct is the front of `HullParams` (the buffer is larger).

struct HullParams {
    amb: vec4<f32>,
    tint: vec4<f32>,
    misc: vec4<f32>,
    rim: vec4<f32>,
    eye: vec4<f32>,
    cut: vec4<f32>,   // x: 1 to cut out at texture alpha 0.5
    uv: vec4<f32>,    // xy: the flipbook's UV offset
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: HullParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var tex_sampler: sampler;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@fragment
fn fragment(i: FragIn) {
    let t = textureSample(tex, tex_sampler, i.uv + k.uv.xy);
    if k.cut.x > 0.5 && t.a < 0.5 {
        discard;
    }
}
