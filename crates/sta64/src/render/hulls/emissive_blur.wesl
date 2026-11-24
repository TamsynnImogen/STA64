// Emissive-only halo. Screen positions use attachment pixel coordinates, including
// a camera viewport offset; all intermediate textures cover the whole target.
struct BlurParams { strength: vec4<f32> };
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: BlurParams;

struct ScreenIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

fn load_clamped(pixel: vec2<i32>) -> vec3<f32> {
    let hi = vec2<i32>(textureDimensions(source)) - vec2<i32>(1);
    return textureLoad(source, clamp(pixel, vec2<i32>(0), hi), 0).rgb;
}

@fragment
fn downsample(input: ScreenIn) -> @location(0) vec4<f32> {
    let p = vec2<i32>(input.position.xy) * 4;
    var rgb = vec3<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            rgb += load_clamped(p + vec2<i32>(x, y));
        }
    }
    return vec4<f32>(rgb / 16.0, 1.0);
}

// A narrow core and a soft outer falloff; the far taps reach 64 screen pixels.
const HALO_WEIGHTS = array<f32, 17>(
    0.086664, 0.083933, 0.076346, 0.065490, 0.053371, 0.041790,
    0.031921, 0.024209, 0.018532, 0.014466, 0.011531, 0.009329,
    0.007592, 0.006162, 0.004959, 0.003943, 0.003092,
);

fn blur(pixel: vec2<i32>, axis: vec2<i32>) -> vec4<f32> {
    var rgb = load_clamped(pixel) * HALO_WEIGHTS[0];
    for (var i = 1; i < 17; i++) {
        let offset = axis * i;
        rgb += (load_clamped(pixel + offset) + load_clamped(pixel - offset)) * HALO_WEIGHTS[i];
    }
    return vec4<f32>(rgb, 1.0);
}

@fragment
fn blur_horizontal(input: ScreenIn) -> @location(0) vec4<f32> {
    return blur(vec2<i32>(input.position.xy), vec2<i32>(1, 0));
}

@fragment
fn blur_vertical(input: ScreenIn) -> @location(0) vec4<f32> {
    return blur(vec2<i32>(input.position.xy), vec2<i32>(0, 1));
}

@fragment
fn composite(input: ScreenIn) -> @location(0) vec4<f32> {
    let reduced = input.position.xy * 0.25 - vec2<f32>(0.5);
    let base = vec2<i32>(floor(reduced));
    let f = fract(reduced);
    let a = mix(load_clamped(base), load_clamped(base + vec2<i32>(1, 0)), f.x);
    let b = mix(load_clamped(base + vec2<i32>(0, 1)), load_clamped(base + vec2<i32>(1, 1)), f.x);
    let light = mix(a, b, f.y);
    // A little neutral light keeps the halo visible when its hue matches the sky.
    let peak = max(max(light.r, light.g), light.b);
    let halo = mix(light, vec3<f32>(peak), 0.12);
    // Thin bands and other small lights cover only a fraction of the blur footprint.
    let lift = 1.0 + 1.4 * (1.0 - smoothstep(0.0, 0.35, peak));
    let falloff = smoothstep(0.0, 0.006, peak);
    return vec4<f32>(halo * params.strength.x * lift * falloff, 0.0);
}
