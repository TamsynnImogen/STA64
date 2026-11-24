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

// Gaussian, sigma 3.5 reduced texels (14 screen pixels).
// The outer samples reach 8 reduced texels, or 32 screen pixels.
fn blur(pixel: vec2<i32>, axis: vec2<i32>) -> vec4<f32> {
    var rgb = load_clamped(pixel) * 0.1143;
    rgb += (load_clamped(pixel + axis) + load_clamped(pixel - axis)) * 0.1097;
    rgb += (load_clamped(pixel + axis * 2) + load_clamped(pixel - axis * 2)) * 0.0970;
    rgb += (load_clamped(pixel + axis * 3) + load_clamped(pixel - axis * 3)) * 0.0797;
    rgb += (load_clamped(pixel + axis * 4) + load_clamped(pixel - axis * 4)) * 0.0609;
    rgb += (load_clamped(pixel + axis * 5) + load_clamped(pixel - axis * 5)) * 0.0433;
    rgb += (load_clamped(pixel + axis * 6) + load_clamped(pixel - axis * 6)) * 0.0287;
    rgb += (load_clamped(pixel + axis * 7) + load_clamped(pixel - axis * 7)) * 0.0177;
    rgb += (load_clamped(pixel + axis * 8) + load_clamped(pixel - axis * 8)) * 0.0102;
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
    return vec4<f32>(mix(a, b, f.y) * params.strength.x, 0.0);
}
