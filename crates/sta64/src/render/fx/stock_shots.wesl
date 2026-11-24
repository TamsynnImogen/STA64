// The Stock look's weapon shots: a textured quad, unlit, texture times vertex colour,
// added to the frame (the blend is set in `stock_shots.rs`, ONE, ONE, so the texture's
// alpha darkens nothing and the output's alpha is never read).
//
// The mesh carries the quad's own coordinates: u along a ribbon or streak (0..1, or
// 0..n repeats for a tiled sprite), v across it; a billboard 0..1 both ways. The
// sprite's texture rectangle, flipbook frame included, is one uniform per sprite, so all
// shots of a sprite show the same frame. No fog, no lighting, no tone shaping.

struct Params {
    // Texture rectangle u, v, width, height in texture space.
    rect: vec4<f32>,
    // x: 1 when u repeats inside the rectangle (a tiled sprite).
    flags: vec4<f32>,
    // rgb multiplies the texture and the vertex colour (white for a shot; the shield cap's colour and gain).
    tint: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var shot_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var shot_sampler: sampler;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
};

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    // The derivatives come from the unwrapped coordinate, so a repeat's seam does not pick a coarse mip.
    let dx = dpdx(in.uv) * k.rect.zw;
    let dy = dpdy(in.uv) * k.rect.zw;
    var local = in.uv;
    if k.flags.x > 0.5 {
        local.x = fract(local.x);
    }
    let uv = k.rect.xy + local * k.rect.zw;
    let texel = textureSampleGrad(shot_texture, shot_sampler, uv, dx, dy);
    let emission = texel.rgb * in.color.rgb * k.tint.rgb;
    return vec4<f32>(emission, 0.0);
}
