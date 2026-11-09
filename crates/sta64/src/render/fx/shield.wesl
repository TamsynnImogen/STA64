// Shield hits: the shell is implied, lit only about where a shot landed.
//
// Ported from Armada II Refit's render3d/mods/effects/shaders/effects_shields.hlsl
// (`shield_ps`; MIT, Copyright (c) 2026 Cedric Haase, see NOTICE.md next to this
// file) with its ART-DIRECTION.md look: a hot spot where the shot landed, a ring
// running out from it, the glow of the rim about it, a lit mist of uneven density
// (four octaves of 3D gradient noise fixed to the shell and drifting slowly), rays
// on the factions that have them (Klingon), and a soft curved edge at the
// silhouette. One draw per hit here (Refit sums up to eight hits on one sphere),
// and the hex lattice stays off, as in Refit's default `Pattern=`. Not ported: the
// shields-down discharges (`down_ps`) and the hull lights.
//
// The sphere is a unit sphere scaled onto the ship's shield ellipsoid; the pixel's
// place on the unit sphere comes from the inverse of that transform (`inv`), as
// Refit passes `o.u` from the vertex shader.

struct Params {
    eye: vec4<f32>,    // xyz the camera; w 1 / the shield's mean radius
    col: vec4<f32>,    // rgb the shield's colour; w overall brightness
    shape: vec4<f32>,  // x 1/spot size, y 1/ring width, z 1/glow reach, w rim power
    mixes: vec4<f32>,  // x spot, y ring, z lattice (unused), w rim: how bright each part is
    core: vec4<f32>,   // x how white the spot's core goes; w the edge's width (in facing)
    mist: vec4<f32>,   // x the mist's brightness, y its share in the rim, z its contrast, w 1 / its reach
    ray: vec4<f32>,    // x the rays' brightness (0: none), y 1 / their length, z their sharpness, w clock (s)
    drift: vec4<f32>,  // xyz the mist's drift now; w its scale
    hit: vec4<f32>,    // xyz where it struck (world); w the spot's brightness now
    now: vec4<f32>,    // x the ring's radius now (shield radii), y its brightness, z the glow's brightness
    unit: vec4<f32>,   // xyz where it struck on the unit sphere; w a seed 0..1
    inv0: vec4<f32>,   // world to unit sphere, the rows of its 3x4
    inv1: vec4<f32>,
    inv2: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: Params;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
};

fn hash33(pin: vec3<f32>) -> vec3<f32> {
    var p = fract(pin * vec3<f32>(0.1031, 0.1030, 0.0973));
    p = p + dot(p, p.yxz + 33.33);
    return fract((p.xxy + p.yxx) * p.zyx) * 2.0 - 1.0;
}

// gradient noise, -1..1 (about), quintic fade: no grid in its zero lines
fn gnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let n000 = dot(hash33(i + vec3<f32>(0.0, 0.0, 0.0)), f - vec3<f32>(0.0, 0.0, 0.0));
    let n100 = dot(hash33(i + vec3<f32>(1.0, 0.0, 0.0)), f - vec3<f32>(1.0, 0.0, 0.0));
    let n010 = dot(hash33(i + vec3<f32>(0.0, 1.0, 0.0)), f - vec3<f32>(0.0, 1.0, 0.0));
    let n110 = dot(hash33(i + vec3<f32>(1.0, 1.0, 0.0)), f - vec3<f32>(1.0, 1.0, 0.0));
    let n001 = dot(hash33(i + vec3<f32>(0.0, 0.0, 1.0)), f - vec3<f32>(0.0, 0.0, 1.0));
    let n101 = dot(hash33(i + vec3<f32>(1.0, 0.0, 1.0)), f - vec3<f32>(1.0, 0.0, 1.0));
    let n011 = dot(hash33(i + vec3<f32>(0.0, 1.0, 1.0)), f - vec3<f32>(0.0, 1.0, 1.0));
    let n111 = dot(hash33(i + vec3<f32>(1.0, 1.0, 1.0)), f - vec3<f32>(1.0, 1.0, 1.0));
    let nx00 = mix(n000, n100, u.x);
    let nx10 = mix(n010, n110, u.x);
    let nx01 = mix(n001, n101, u.x);
    let nx11 = mix(n011, n111, u.x);
    return mix(mix(nx00, nx10, u.y), mix(nx01, nx11, u.y), u.z) * 1.6;
}

// The mist at u (on the unit sphere): 0..2, about 1 on the whole, uneven like lit
// vapour. Four octaves, the domain bent once by a coarser field so the denser parts
// run in wisps; fixed to the shell and drifting slowly through it.
fn mist(u: vec3<f32>) -> f32 {
    var p = u * k.drift.w + k.drift.xyz;
    p = p + 0.45 * gnoise(p * 0.6 + 17.0) * vec3<f32>(0.8, 0.5, -0.35);
    let f = gnoise(p) + 0.5 * gnoise(p * 2.03 + 5.1) + 0.25 * gnoise(p * 4.07 + 9.7) + 0.125 * gnoise(p * 8.11 + 13.3);
    return 2.0 * clamp(0.5 + f * k.mist.z * (0.5 / 1.875), 0.0, 1.0);
}

// The hit's rays at u (unit sphere) and w (world): a sunburst about where it struck,
// clear spokes of uneven width and length out to the rays' length, from just outside
// the spot. Four sines of whole numbers of turns (seeded per hit), each beating
// slowly in strength, so the burst breathes in place and nothing turns.
fn rays(u: vec3<f32>, w: vec3<f32>) -> f32 {
    let d = length(w - k.hit.xyz) * k.eye.w;
    let n = k.unit.xyz;
    let t1 = normalize(cross(n, mix(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), step(0.9, abs(n.y)))));
    let t2 = cross(n, t1);
    let a = atan2(dot(u, t2), dot(u, t1) + 1e-5);
    let s = k.unit.w * 6.2831853;
    let t = k.ray.w;
    let f = sin(a * 7.0 + s) * (0.65 + 0.35 * sin(t * 1.3 + s * 3.0))
          + sin(a * 11.0 + s * 2.0) * (0.65 + 0.35 * sin(t * 1.7 + s * 5.0)) * 0.8
          + sin(a * 17.0 + s * 4.0) * (0.65 + 0.35 * sin(t * 2.3 + s * 7.0)) * 0.6
          + sin(a * 29.0 + s * 6.0) * 0.4;
    // a spoke is the upper half of f, sharpened; the strongest run out furthest
    let spoke = pow(clamp(f * 0.5, 0.0, 1.0), k.ray.z);
    let x = clamp(d * k.ray.y / (0.6 + 0.4 * clamp(f * 0.5, 0.0, 1.0)), 0.0, 1.0);
    let env = smoothstep(0.0, 0.05, d) * (1.0 - x) * (1.0 - x);
    return k.now.z * spoke * env;
}

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    let w = in.world_position.xyz;
    let n = normalize(in.world_normal);
    let v = normalize(k.eye.xyz - w);
    let facing = abs(dot(n, v));
    let rim = pow(1.0 - facing, k.shape.w);

    // What the hit adds at this point: spot, ring, glow, and the mist's light.
    let d = length(w - k.hit.xyz) * k.eye.w;
    let sp = d * k.shape.x;
    let a_spot = k.hit.w * exp(-sp * sp);
    let q = (d - k.now.x) * k.shape.y;
    let a_ring = k.now.y * exp(-q * q) * clamp(d * 4.0, 0.0, 1.0);
    let a_glow = k.now.z * exp(-d * k.shape.z);
    let m_reach = d * k.mist.w;
    let a_haze = k.now.z * exp(-m_reach * m_reach);
    if a_spot + a_ring + a_glow + a_haze * k.mist.x < 0.002 {
        discard;
    }

    let ui = vec3<f32>(dot(k.inv0.xyz, w) + k.inv0.w, dot(k.inv1.xyz, w) + k.inv1.w, dot(k.inv2.xyz, w) + k.inv2.w);
    let u = normalize(ui);
    let m = mist(u);
    let spot = a_spot * k.mixes.x;
    let ring = a_ring * k.mixes.y;
    // with no lattice (Refit's default `Pattern=`) its term is 1, times `Lattice`
    let field = a_glow * (k.mixes.z + rim * k.mixes.w) * mix(1.0, m, k.mist.y);
    let haze = a_haze * k.mist.x * m;
    var ray = 0.0;
    if k.ray.x > 0.0 {
        ray = rays(u, w) * k.ray.x;
    }
    // seen face on, a shell is thin and shows little; at its edge the light runs along it
    var e = (spot + ring + field + haze) * (0.55 + 0.45 * rim + 0.25 * (1.0 - facing)) + ray;
    // ...and stops at the silhouette on a soft but definite curve
    let x = clamp(facing / max(k.core.w, 1e-4), 0.0, 1.0);
    e = e * x * x * (3.0 - 2.0 * x);
    var c = k.col.rgb * e;
    let hot = clamp(spot * spot * k.core.x, 0.0, 1.0); // the core of a hit goes white
    c = mix(c, vec3<f32>(1.0) * e, hot);
    return vec4<f32>(c * k.col.w, 0.0);
}
