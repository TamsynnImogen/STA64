// A quiet, constant shield shell: only its camera-facing silhouette is lit.
// Independent of hit shaders and native game assets.
struct Params { eye: vec4<f32>, };
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: Params;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
};

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    let sight = normalize(k.eye.xyz - in.world_position.xyz);
    let facing = abs(dot(normalize(in.world_normal), sight));
    let rim = smoothstep(0.3, 0.85, 1.0 - facing);
    let light = rim * rim * 0.24;
    return vec4<f32>(vec3<f32>(0.36, 0.69, 1.0) * light, 0.0);
}
