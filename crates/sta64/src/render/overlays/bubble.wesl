// The selection bubble (Armada II Stock look): one flat, unlit colour added to the frame.
// The colour arrives in linear space; alpha 0 under the premultiplied blend is a pure add.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> colour: vec4<f32>;

struct FragIn {
    @builtin(position) position: vec4<f32>,
};

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    return vec4<f32>(colour.rgb, 0.0);
}
