// A picture of the minimap drawn additively: the pipeline adds the colour to what is under it
// (blend one, one) and keeps the target's alpha, so the shader returns the picture's colour and
// ignores its alpha. `rect` is the part of the image drawn (u, v, width, height).

import bevy_ui_render::ui_vertex_output::UiVertexOutput;

struct Rect {
    rect: vec4<f32>,
};

@group(1) @binding(0) var<uniform> area: Rect;
@group(1) @binding(1) var picture: texture_2d<f32>;
@group(1) @binding(2) var picture_s: sampler;
@group(1) @binding(3) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let uv = area.rect.xy + in.uv * area.rect.zw;
    let c = textureSample(picture, picture_s, uv);
    let intensity = max(c.r, max(c.g, c.b));
    return vec4<f32>(mix(c.rgb, intensity * tint.rgb, tint.a), 1.0);
}
