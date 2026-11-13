// The reduced-resolution nebula march's two screen passes (RND-34, `nebula_lowres.rs`).
//
// `downsample` reduces the scene's depth prepass to the march's target size: the
// farthest depth of each n x n block (reverse-Z: the smallest value; 0 where any pixel
// of the block shows no geometry), which is how far the block's march may run.
// `composite` lays the march's picture over the view at full size. A pixel takes the
// four nearest texels of the small picture, bilinearly, but only those whose depth is
// within 5% of its own; none within 5%: the one nearest in depth. That keeps gas from
// smearing across a hull's edge, the way Refit's own half-resolution composite did.

struct LowView {
    world_from_clip: mat4x4<f32>,
    size: vec4<f32>,       // the march target's width and height, n, the camera's near plane
};

@group(0) @binding(0) var<uniform> v: LowView;
@group(0) @binding(1) var gas: texture_2d<f32>;
@group(0) @binding(2) var low_z: texture_2d<f32>;
@if(MULTISAMPLED)
@group(0) @binding(3) var scene_depth: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(3) var scene_depth: texture_depth_2d;

struct ScreenIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// View depth of a reverse-Z depth value (a very large one for "nothing there").
fn view_depth(z: f32) -> f32 {
    return select(1.0e7, v.size.w / z, z > 0.0);
}

@fragment
fn downsample(i: ScreenIn) -> @location(0) vec4<f32> {
    let n = i32(v.size.z);
    let dim = vec2<i32>(textureDimensions(scene_depth));
    let base = vec2<i32>(i.position.xy) * n;
    var z = 1.0;
    for (var y = 0; y < n; y++) {
        for (var x = 0; x < n; x++) {
            z = min(z, textureLoad(scene_depth, min(base + vec2<i32>(x, y), dim - vec2<i32>(1)), 0));
        }
    }
    return vec4<f32>(z, 0.0, 0.0, 0.0);
}

@fragment
fn composite(i: ScreenIn) -> @location(0) vec4<f32> {
    let vd = view_depth(textureLoad(scene_depth, vec2<i32>(i.position.xy), 0));
    // Where this pixel's centre falls among the small picture's texel centres.
    let lp = i.position.xy / v.size.z - vec2<f32>(0.5);
    let b = floor(lp);
    let f = lp - b;
    let lo = vec2<i32>(b);
    let hi = vec2<i32>(v.size.xy) - vec2<i32>(1);
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    var nearest = vec4<f32>(0.0);
    var best = 1.0e9;
    for (var j = 0; j < 4; j++) {
        let o = vec2<i32>(j & 1, j >> 1);
        let q = clamp(lo + o, vec2<i32>(0), hi);
        let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let g = textureLoad(gas, q, 0);
        let zl = view_depth(textureLoad(low_z, q, 0).r);
        let dd = abs(zl - vd) / max(min(max(zl, vd), 1.0e7), 1.0);
        if dd < 0.05 {
            sum += g * w;
            wsum += w;
        }
        if dd < best {
            best = dd;
            nearest = g;
        }
    }
    return select(nearest, sum / max(wsum, 1e-6), wsum > 1e-4);
}
