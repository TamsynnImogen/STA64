// The map grid's lines (#1033): one constant colour, added to what is behind. Alpha 0: AlphaMode::Add is
// Bevy's premultiplied blend (src + dst * (1 - alpha)), so the colour is added and nothing is replaced.

struct GridParams {
    colour: vec4<f32>, // rgb, linear
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: GridParams;

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(k.colour.rgb, 0.0);
}
