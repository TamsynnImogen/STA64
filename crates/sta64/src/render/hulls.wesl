// Hulls: ships, stations and the like, lit as Armada II Refit lights them.
//
// Ported from Armada II Refit's render3d/mods/hulls/shaders/hulls.hlsl (`ps_hull`,
// `ps_bump`, `shade`, `shoulder`; MIT, Copyright (c) 2026 Cedric Haase, see hulls.NOTICE.md
// next to this file), itself Lighting.asi's hull look: diffuse from up to four
// directional lights (the first is the Key), a highlight scaled by the texture's
// brightness, a rim light, the team colour as the diffuse colour, self-illumination
// from the texture's alpha, and the highlight shoulder that keeps the lit colour within
// 1. A bump-mapped hull takes its normal from a map in a tangent frame worked out per
// pixel from how the position and the texture coordinate change across the screen.
//
// RND-15 adds the Key's shadow map (`map_shadow`).
// Not ported (see TODO.md): planet shadows, the clustered point
// lights, the wave light, near/far fade, cloaks, holograms, the build ghost and the
// sprite pass; the vertex stage is Bevy's own.

struct HullParams {
    amb: vec4<f32>,   // ambient rgb; a: self-illumination (0: none; above 1 brighter than the texture)
    tint: vec4<f32>,  // the diffuse colour (team colour on team groups, else white)
    misc: vec4<f32>,  // x: alpha out; y: specular strength; z: its exponent; w: the Key's strength
    rim: vec4<f32>,   // rim light colour; a: its exponent
    eye: vec4<f32>,   // the camera, world
    cut: vec4<f32>,   // x: 1 to cut out at texture alpha 0.5; y: the normal map's strength (0: none); z: the highlight knee
    uv: vec4<f32>,    // xy: the flipbook's UV offset
    dvec: array<vec4<f32>, 4>,  // towards each directional light
    dcol: array<vec4<f32>, 4>,  // its colour; a: 1 for the Key
    sh: array<vec4<f32>, 3>,    // the Key's shadow map (shadows.rs): world -> u, v, depth along the Key
    shp: vec4<f32>,             // x 1: there is a map (0: none); y one texel (u, v); z the depth bias; w how much of the Key a shadow takes
    // The Stock profile (RND-24, render/stock/hulls.rs): the SOD lighting material's own colours.
    mamb: vec4<f32>,  // ambient colour; a: 1 draws the group the stock way (amb.rgb is then the scene's ambient light), 0 Refit's look
    mdif: vec4<f32>,  // diffuse colour
    mspc: vec4<f32>,  // specular colour; a: its exponent
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: HullParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var tex_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var nmap: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var nmap_sampler: sampler;
// The Key's shadow map: r + g / 2048 the depth of the nearest hull along the Key, 1 where none.
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var smap: texture_2d<f32>;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @builtin(front_facing) front: bool,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

fn glint(n: vec3<f32>, l: vec3<f32>, v: vec3<f32>) -> f32 {
    let h = normalize(l + v);
    if dot(n, l) > 0.0 {
        return pow(max(0.0, dot(n, h)), k.misc.z);
    }
    return 0.0;
}

// Lighting.asi's highlight shoulder: the colour as it is up to the knee, then rolling
// off towards 1 with the same slope at the knee, per channel (knee >= 1: a plain clip;
// above 1.5: none).
fn shoulder(x: vec3<f32>, knee: f32) -> vec3<f32> {
    let r = max(1.0 - knee, 1e-4);
    let y = vec3<f32>(knee) + r * (vec3<f32>(1.0) - exp(-(x - vec3<f32>(knee)) / r));
    if knee > 1.5 {
        return max(x, vec3<f32>(0.0));
    }
    if knee >= 1.0 {
        return clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return mix(max(x, vec3<f32>(0.0)), y, step(vec3<f32>(knee), x));
}

fn map_depth(px: vec2<i32>) -> f32 {
    let dim = vec2<i32>(textureDimensions(smap));
    let t = textureLoad(smap, clamp(px, vec2<i32>(0), dim - vec2<i32>(1)), 0);
    return t.r + t.g / 2048.0;
}

// How much of the Key reaches a point: 3x3 texels weighted by where it falls (16 taps),
// the point pushed off the surface along its normal (a texel, more at a grazing angle)
// so a hull does not shadow itself.
fn map_shadow(wp: vec3<f32>, n: vec3<f32>) -> f32 {
    if k.shp.x <= 0.0 {
        return 1.0;
    }
    let to_key = -normalize(vec3<f32>(k.sh[2].x, k.sh[2].y, k.sh[2].z));
    let ndl = dot(n, to_key);
    let texel_w = k.shp.y / max(length(vec3<f32>(k.sh[0].x, k.sh[0].y, k.sh[0].z)), 1e-9);
    let off = texel_w * (0.5 + 1.5 * sqrt(saturate(1.0 - ndl * ndl)));
    let q = vec4<f32>(wp + n * off, 1.0);
    let sc = vec3<f32>(dot(q, k.sh[0]), dot(q, k.sh[1]), dot(q, k.sh[2]));
    if sc.x <= 0.0 || sc.x >= 1.0 || sc.y <= 0.0 || sc.y >= 1.0 {
        return 1.0;
    }
    let size = vec2<f32>(textureDimensions(smap));
    let tt = sc.xy * size - 0.5;
    let f = fract(tt);
    let b = vec2<i32>(floor(tt));
    let d = sc.z - k.shp.z;
    var lit = 0.0;
    for (var y = -1; y <= 2; y++) {
        let wy = select(select(1.0, f.y, y == 2), 1.0 - f.y, y == -1);
        for (var x = -1; x <= 2; x++) {
            let wx = select(select(1.0, f.x, x == 2), 1.0 - f.x, x == -1);
            let z = map_depth(b + vec2<i32>(x, y));
            lit += select(0.0, 1.0, d <= z) * wx * wy;
        }
    }
    return lit / 9.0;
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), 12.92 * x, x <= vec3<f32>(0.0031308));
}

fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// Stock: the original engine's fixed-function lighting from the SOD material, per light
// diffuse colour times N.L, specular added after the texture, the lit colour clipped at 1.
fn shade_stock(n: vec3<f32>, wp: vec3<f32>, t: vec4<f32>) -> vec4<f32> {
    let v = normalize(k.eye.xyz - wp);
    var sum = k.mamb.rgb * k.amb.rgb;
    var spec = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        let l = k.dvec[j].xyz;
        let c = k.dcol[j].rgb;
        sum += k.mdif.rgb * c * max(0.0, dot(n, l));
        let h = normalize(l + v);
        if dot(n, l) > 0.0 {
            spec += c * pow(max(0.0, dot(n, h)), k.mspc.a);
        }
    }
    // The engine multiplies the colours as stored (gamma-space numbers, no linear light):
    // the texture and the team colour are taken as such, the result goes out as such.
    let ts = to_srgb(t.rgb);
    let lit = ts * to_srgb(k.tint.rgb) * sum + spec * k.mspc.rgb * k.misc.y;
    let c = clamp(mix(lit, ts * max(k.amb.a, 1.0), saturate(t.a * min(k.amb.a, 1.0))), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(from_srgb(c), k.misc.x);
}

// Everything after the normal: the lights, the texture, the night lights.
fn shade(n: vec3<f32>, wp: vec3<f32>, t: vec4<f32>) -> vec4<f32> {
    if k.mamb.a > 0.5 {
        return shade_stock(n, wp, t);
    }
    let v = normalize(k.eye.xyz - wp);
    var sum = k.amb.rgb;
    var spec = vec3<f32>(0.0);
    let key = mix(1.0, map_shadow(wp, n), k.shp.w);
    for (var j = 0; j < 4; j++) {
        let l = k.dvec[j].xyz;
        let c = k.dcol[j].rgb * mix(1.0, k.misc.w * key, k.dcol[j].a);
        sum += c * max(0.0, dot(n, l));
        spec += c * glint(n, l, v);
    }
    sum += k.rim.rgb * pow(1.0 - saturate(dot(n, v)), k.rim.a);
    let gloss = dot(t.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let lit = t.rgb * k.tint.rgb * max(sum, vec3<f32>(0.0)) + spec * k.misc.y * gloss;
    let glow = t.rgb * max(k.amb.a, 1.0);
    let c = shoulder(mix(lit, glow, saturate(t.a * min(k.amb.a, 1.0))), k.cut.z);
    return vec4<f32>(c, k.misc.x);
}

@fragment
fn fragment(i: FragIn) -> @location(0) vec4<f32> {
    let uv = i.uv + k.uv.xy;
    let wp = i.world_position.xyz;
    var nn = normalize(i.world_normal);
    if !i.front {
        nn = -nn;
    }
    // All lookups and derivatives up front (uniform control flow).
    let t = textureSample(tex, tex_sampler, uv);
    let m0 = textureSample(nmap, nmap_sampler, uv).rgb * 2.0 - vec3<f32>(1.0);
    let dp1 = dpdx(wp);
    let dp2 = dpdy(wp);
    let du1 = dpdx(uv);
    let du2 = dpdy(uv);
    if k.cut.x > 0.5 && t.a < 0.5 {
        discard;
    }
    var n = nn;
    if k.cut.y > 0.0 {
        // The tangent frame from the screen-space derivatives (no tangents stored).
        let a = cross(dp2, nn);
        let b = cross(nn, dp1);
        let tt = a * du1.x + b * du2.x;
        let bb = a * du1.y + b * du2.y;
        let s = inverseSqrt(max(max(dot(tt, tt), dot(bb, bb)), 1e-20));
        var m = m0;
        m.x *= k.cut.y;
        m.y *= k.cut.y;
        // The map's y runs up the image, as Bevy's normal maps do.
        let nb = tt * s * m.x - bb * s * m.y + nn * m.z;
        let l = length(nb);
        if l > 1e-5 {
            n = nb / l;
        }
    }
    return shade(n, wp, t);
}
