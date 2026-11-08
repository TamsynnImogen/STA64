// Planets: ground, cloud shell and the air past the limb.
//
// Ported from Armada II Refit's render3d/shaders/world_planet.hlsl (MIT, Copyright (c)
// 2026 Cedric Haase), itself Lighting.asi's planet look: a terminator wrapped past
// the edge and reddened at dusk, the atmosphere seen edge-on at the limb, a glint
// off water, a lit cloud shell, and the scattered light of the air past the limb.
// Lighting is the planet's own; Bevy's lights do not touch it. City lights are not
// ported yet (they need the colony's population map).

struct PlanetParams {
    base: vec4<f32>,          // the material's constant term (rgb)
    mat_diffuse: vec4<f32>,   // the material's diffuse colour (rgb)
    dcol0: vec4<f32>,         // the key light's colour; a: the cap on the light
    dcol1: vec4<f32>,         // the fill light's colour
    dvec0: vec4<f32>,         // towards the key, world, unit
    dvec1: vec4<f32>,         // towards the fill
    centre: vec4<f32>,        // the sphere's centre, world
    eye: vec4<f32>,           // the camera, world
    dusk: vec4<f32>,          // rgb: colour where light grazes; a: how far round it wraps
    haze: vec4<f32>,          // rgb: the atmosphere at the limb; a: its exponent
    shine: vec4<f32>,         // x glint, y glint exponent, z city lights, w exposure
    shell: vec4<f32>,         // x: how much of the shell's texture shows
    halo: vec4<f32>,          // x ground radius, y top of the air, z strength, w forward scattering
    mode: vec4<f32>,          // x: 0 ground, 1 cloud shell, 2 halo, 3 recipe surface
    surf_glow: vec4<f32>,     // rgb: the glow's colour; a: how much shows by day
    surf_cloud: vec4<f32>,    // rgb: the clouds' colour; a: their opacity
    surf_drift: vec4<f32>,    // cos, sin of the drift; the shadow's strength; its reach
    surf_misc: vec4<f32>,     // x: the relief's strength; y: 1 with clouds
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: PlanetParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var tex_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var s_albedo: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var s_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var s_relief: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var s_relief_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var s_clouds: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var s_clouds_sampler: sampler;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct Sky {
    light: vec3<f32>,
    glow: vec3<f32>,
    spec: vec3<f32>,
    day: f32,
};

// One light on the sphere normal n with the terrain normal ns: the terrain scales the
// light by how much more or less it faces the light than the sphere there does, up
// to twice; `key` dims the key light (cloud shadow).
fn light_term(col_in: vec3<f32>, dir: vec3<f32>, n: vec3<f32>, ns: vec3<f32>, v: vec3<f32>, rim: f32, first: bool, key: f32, s: ptr<function, Sky>) {
    var col = col_in;
    if first {
        col = col_in * key;
    }
    let ndl = dot(n, dir);
    let ndn = dot(ns, dir);
    let rel = clamp((ndn + 0.3) / max(ndl + 0.3, 0.05), 0.0, 2.0);
    // Wrapped a little past the terminator, as an atmosphere scatters it, and
    // reddened where it grazes: sunset along the terminator.
    let lam = saturate((ndl + p.dusk.a) / (1.0 + p.dusk.a));
    let c = col * mix(p.dusk.rgb, vec3<f32>(1.0), saturate(ndl * 4.0));
    (*s).light += c * p.mat_diffuse.rgb * lam * rel;
    (*s).glow += c * saturate(ndl + 0.35) * rim;
    let h = normalize(dir + v);
    var sp = 0.0;
    if ndl > 0.0 {
        sp = pow(saturate(dot(ns, h)), p.shine.y);
    }
    (*s).spec += col * sp;
    if first {
        (*s).day = saturate(ndl * 4.0 + 0.3);
    }
}

fn shade_n(wp: vec3<f32>, ns: vec3<f32>, key: f32) -> Sky {
    var s: Sky;
    let n = normalize(wp - p.centre.xyz);
    let v = normalize(p.eye.xyz - wp);
    let rim = pow(1.0 - saturate(dot(n, v)), p.haze.a);
    s.light = p.base.rgb;
    s.glow = vec3<f32>(0.0);
    s.spec = vec3<f32>(0.0);
    s.day = 0.0;
    light_term(p.dcol0.rgb, p.dvec0.xyz, n, ns, v, rim, true, key, &s);
    light_term(p.dcol1.rgb, p.dvec1.xyz, n, ns, v, rim, false, key, &s);
    s.glow *= p.haze.rgb;
    return s;
}

fn shade(wp: vec3<f32>) -> Sky {
    return shade_n(wp, normalize(wp - p.centre.xyz), 1.0);
}

// The light on the texture: clamped at the key's own strength rather than 1.
fn lit(s: Sky) -> vec3<f32> {
    return clamp(s.light, vec3<f32>(0.0), vec3<f32>(max(1.0, p.dcol0.a)));
}

fn ground(i: FragIn) -> vec4<f32> {
    let s = shade(i.world_position.xyz);
    let t = textureSample(tex, tex_sampler, i.uv);
    // Water is taken to be where the ground is bluer than it is red or green.
    let wet = saturate((t.b - max(t.r, t.g)) * 8.0 + 0.15) * saturate(1.2 - dot(t.rgb, vec3<f32>(0.333)) * 1.5);
    let c = t.rgb * lit(s) + s.glow + s.spec * p.shine.x * wet;
    return vec4<f32>(max(c, vec3<f32>(0.0)) * p.shine.w, 1.0);
}

fn cloud(i: FragIn) -> vec4<f32> {
    let s = shade(i.world_position.xyz);
    let t = textureSample(tex, tex_sampler, i.uv) * p.shell.x;
    let g = dot(s.glow, vec3<f32>(0.3, 0.5, 0.2));
    let c = t.rgb * lit(s) + s.glow;
    return vec4<f32>(max(c, vec3<f32>(0.0)) * p.shine.w, saturate(t.a + g));
}

// Light scattered toward the eye along each line of sight past the limb: thickest
// just over the ground's edge, gone at the top, brighter looking toward the key.
fn halo_light(i: FragIn) -> vec4<f32> {
    let d = normalize(i.world_position.xyz - p.eye.xyz);
    let lowest = p.eye.xyz + d * dot(p.centre.xyz - p.eye.xyz, d);
    let r = lowest - p.centre.xyz;
    let b = length(r);
    let h = (b - p.halo.x) / (p.halo.y - p.halo.x);
    let a = saturate(1.0 - h);
    let dens = exp(-4.0 * max(h, 0.0)) * a * a * smoothstep(-0.5, 0.0, h);
    let n = r / max(b, 1e-4);
    var l = vec3<f32>(0.0);
    for (var k = 0; k < 2; k++) {
        var col = p.dcol0.rgb;
        var dir = p.dvec0.xyz;
        if k == 1 {
            col = p.dcol1.rgb;
            dir = p.dvec1.xyz;
        }
        let ndl = dot(n, dir);
        let lam = saturate((ndl + 0.3) / 1.3);
        l += col * mix(p.dusk.rgb, vec3<f32>(1.0), saturate(ndl * 3.0 + 0.3)) * lam
            * (1.0 + p.halo.w * pow(saturate(dot(d, dir)), 6.0));
    }
    return vec4<f32>(p.haze.rgb * l * dens * p.halo.z * p.shine.w, 1.0);
}

// The baked cubes are in the files' left-handed space; the world is mirrored on Z.
fn mirror(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(v.x, v.y, -v.z);
}

fn drifted(d: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(p.surf_drift.x * d.x + p.surf_drift.y * d.z, d.y, p.surf_drift.x * d.z - p.surf_drift.y * d.x);
}

// A planet made from a recipe (Surfaces.asi): three cubes sampled in the planet's
// own frame, so the surface has no seams. The relief's normal takes the light; the
// clouds drift about the pole, cast their shadow toward the sun and are lit by the
// sphere; fissures glow, most on the night side.
fn surface(i: FragIn) -> vec4<f32> {
    let wp = i.world_position.xyz;
    let d = normalize(mirror(wp - p.centre.xyz));
    let a = textureSample(s_albedo, s_albedo_sampler, d);
    let b = textureSample(s_relief, s_relief_sampler, d);
    let ns = normalize(mirror(normalize(d + (b.rgb - 0.5) * p.surf_misc.x)));
    let lo = normalize(mirror(p.dvec0.xyz));
    let cl = textureSample(s_clouds, s_clouds_sampler, drifted(d)).r * p.surf_cloud.a * p.surf_misc.y;
    let cs = textureSample(s_clouds, s_clouds_sampler, drifted(normalize(d + (lo - d * dot(lo, d)) * p.surf_drift.w))).r;
    let key = 1.0 - p.surf_drift.z * cs * p.surf_cloud.a * p.surf_misc.y;
    let s = shade_n(wp, ns, key);
    var c = a.rgb * lit(s) + s.glow + s.spec * p.shine.x * a.a;
    c += p.surf_glow.rgb * b.a * mix(p.surf_glow.a, 1.0, 1.0 - s.day);
    let sc = shade(wp);
    c = mix(c, p.surf_cloud.rgb * lit(sc) + sc.glow, cl);
    return vec4<f32>(max(c, vec3<f32>(0.0)) * p.shine.w, 1.0);
}

@fragment
fn fragment(i: FragIn) -> @location(0) vec4<f32> {
    if p.mode.x > 2.5 {
        return surface(i);
    }
    if p.mode.x > 1.5 {
        return halo_light(i);
    }
    if p.mode.x > 0.5 {
        return cloud(i);
    }
    return ground(i);
}
