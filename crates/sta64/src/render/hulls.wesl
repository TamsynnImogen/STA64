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
// Not ported (see TODO.md): the Key's shadow map, planet shadows, the clustered point
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
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: HullParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var tex_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var nmap: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var nmap_sampler: sampler;

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

// Everything after the normal: the lights, the texture, the night lights.
fn shade(n: vec3<f32>, wp: vec3<f32>, t: vec4<f32>) -> vec4<f32> {
    let v = normalize(k.eye.xyz - wp);
    var sum = k.amb.rgb;
    var spec = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        let l = k.dvec[j].xyz;
        let c = k.dcol[j].rgb * mix(1.0, k.misc.w, k.dcol[j].a);
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
