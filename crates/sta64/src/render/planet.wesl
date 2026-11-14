// Planets: ground, cloud shell and the air past the limb.
//
// Ported from Armada II Refit's render3d/shaders/world_planet.hlsl (MIT, Copyright (c)
// 2026 Cedric Haase), itself Lighting.asi's planet look: a terminator wrapped past
// the edge and reddened at dusk, the atmosphere seen edge-on at the limb, a glint
// off water, a lit cloud shell, and the scattered light of the air past the limb.
// Lighting is the planet's own; Bevy's lights do not touch it. The cities (RND-12) are
// a third layer per hemisphere (mode.z 3): the race's development texture times the
// colony's alloc picture, blended over the ground as the engine's second pass is; the
// Refit look adds `city_night`, Refit's own night-side lights (the original has none).

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

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> p: PlanetParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var tex_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var s_albedo: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var s_albedo_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var s_relief: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var s_relief_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var s_clouds: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var s_clouds_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var city_alloc: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var city_alloc_sampler: sampler;

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
    // Alpha 0: AlphaMode::Add is Bevy's premultiplied blend (src + dst * (1 - alpha)), so any alpha above 0
    // would replace what is behind the halo (a black disc over the planet) instead of adding to it.
    return vec4<f32>(p.haze.rgb * l * dens * p.halo.z * p.shine.w, 0.0);
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


// ---- Refit's night lights (world_planet.hlsl, `city_night`): made here, not read. The
// development texture's alpha is a solid blob per town, so it says only where people live;
// hashes and noise, the sin-fract kind, and Voronoi nets.
fn hash1(q: vec2<f32>) -> f32 {
    return fract(sin(dot(q, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn hash2(q: vec2<f32>) -> vec2<f32> {
    return fract(sin(vec2<f32>(dot(q, vec2<f32>(127.1, 311.7)), dot(q, vec2<f32>(269.5, 183.3)))) * 43758.5453);
}

fn vnoise(q: vec2<f32>) -> f32 {
    let ip = floor(q);
    var f = fract(q);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash1(ip), hash1(ip + vec2<f32>(1.0, 0.0)), f.x), mix(hash1(ip + vec2<f32>(0.0, 1.0)), hash1(ip + vec2<f32>(1.0, 1.0)), f.x), f.y);
}

fn fbm(q: vec2<f32>) -> f32 {
    return vnoise(q) * 0.55 + vnoise(q * 2.03 + vec2<f32>(5.2, 1.3)) * 0.3 + vnoise(q * 4.1 + vec2<f32>(9.1, 3.7)) * 0.15;
}

// The Voronoi net at q: x the distance to the nearest seed, y F2 - F1 (0 on a border between
// two cells), z the nearest cell's hash, w one hash per pair of cells.
fn cells(q: vec2<f32>) -> vec4<f32> {
    let ip = floor(q);
    let fp = fract(q);
    var d1 = 8.0;
    var d2 = 8.0;
    var i1 = 0.0;
    var i2 = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let g = vec2<f32>(f32(x), f32(y));
            let r = g + hash2(ip + g) - fp;
            let d = dot(r, r);
            let id = hash1(ip + g + vec2<f32>(17.0, 31.0));
            if d < d1 {
                d2 = d1;
                i2 = i1;
                d1 = d;
                i1 = id;
            } else if d < d2 {
                d2 = d;
                i2 = id;
            }
        }
    }
    d1 = sqrt(d1);
    return vec4<f32>(d1, sqrt(d2) - d1, i1, fract(i1 + i2));
}

// Lit lines along the borders of the net at q (cells per UV), `width` cells wide, on the share
// `keep` of the borders; fw is a pixel in cells. z of the result is the cell's hash.
fn roads(q: vec2<f32>, fw: f32, width: f32, keep: f32) -> vec2<f32> {
    let c = cells(q);
    let w = max(width, fw);
    return vec2<f32>((1.0 - smoothstep(0.0, w, c.y)) * (width / w) * step(c.w, keep) * (0.4 + 0.6 * fract(c.w * 7.31)), c.z);
}

// Where people live at uv: the development texture's alpha times the colony's alloc picture.
fn town_at(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(tex, tex_sampler, uv, 0.0).a * textureSampleLevel(city_alloc, city_alloc_sampler, uv, 0.0).a;
}

// How bright the cities are at night at uv: a faint glow over each town, downtown brighter in
// hot spots, a street net beaded with lights and a finer net of lanes in the denser parts fill
// each town, a coarse net of highways runs between neighbouring towns, single lights scatter
// past their edges.
fn city_night(uv: vec2<f32>, fw: f32) -> f32 {
    var ring = array<vec2<f32>, 8>(
        vec2<f32>(1.0, 0.0), vec2<f32>(0.7071, 0.7071), vec2<f32>(0.0, 1.0), vec2<f32>(-0.7071, 0.7071),
        vec2<f32>(-1.0, 0.0), vec2<f32>(-0.7071, -0.7071), vec2<f32>(0.0, -1.0), vec2<f32>(0.7071, -0.7071));
    let m = town_at(uv);
    var near = m;
    var wide = 0.0;
    for (var k = 0; k < 8; k++) {
        near += town_at(uv + ring[k] * (5.0 / 256.0));
        wide += town_at(uv + ring[k] * (14.0 / 256.0));
    }
    near /= 9.0;
    wide /= 8.0;
    let town = saturate((near * 0.7 + m * 0.3) * (0.7 + 1.6 * fbm(uv * 28.0)));
    let wuv = uv + (vec2<f32>(vnoise(uv * 9.0 + vec2<f32>(2.0, 0.0)), vnoise(uv * 9.0 + vec2<f32>(0.0, 7.0))) - 0.5) * 0.035;
    let rs = roads(wuv * 120.0, fw * 180.0, 0.055, 0.85);
    let bead = mix(0.775, 0.55 + 0.45 * vnoise(wuv * 900.0), saturate(2.0 - fw * 1400.0));
    let st = rs.x * smoothstep(0.05, 0.5, town) * (0.45 + 0.55 * rs.y) * bead;
    let lane = roads(wuv * 300.0 + 3.1, fw * 450.0, 0.09, 0.7).x * smoothstep(0.1, 0.6, town);
    var hw = roads(wuv * 20.0, fw * 30.0, 0.03, 0.5).x;
    hw *= smoothstep(0.02, 0.35, max(wide, near)) * (1.0 - 0.7 * smoothstep(0.4, 0.9, town));
    let d1 = cells(uv * 320.0).x;
    let pr = max(0.065, fw * 224.0);
    let pts = exp(-(d1 / pr) * (d1 / pr)) * (0.065 / pr) * (0.065 / pr)
        * step(hash1(floor(uv * 320.0) + vec2<f32>(3.0, 7.0)), (0.08 + 0.9 * saturate(town + wide * 0.3)) * smoothstep(0.01, 0.12, max(wide, near)));
    let core = pow(smoothstep(0.3, 0.9, town), 2.0) * (0.4 + 0.6 * vnoise(uv * 60.0 + vec2<f32>(1.7, 4.4)));
    let glow = smoothstep(0.0, 0.6, town) * 0.12 + saturate(wide) * 0.04;
    return 1.5 * (glow + core * 0.9 + st + lane * 0.8 + hw * 0.6 + pts * (0.8 + town));
}

// The cities under the Refit look (pass 1, blended by alpha over the ground): the development
// texture times the alloc picture, by day lit like the ground, at night lit from within by
// `city_night`, warm, whitening where it is brightest. shine.z is the strength, surf_glow the colour.
fn city(i: FragIn) -> vec4<f32> {
    let s = shade(i.world_position.xyz);
    let t = textureSample(tex, tex_sampler, i.uv) * textureSample(city_alloc, city_alloc_sampler, i.uv);
    let night = (1.0 - s.day) * p.shine.z;
    // A pixel in UV, taken before the branch (derivatives need uniform control flow).
    let fw = max(abs(dpdx(i.uv.x)) + abs(dpdy(i.uv.x)), abs(dpdx(i.uv.y)) + abs(dpdy(i.uv.y)));
    var big = 0.0;
    if night > 0.001 {
        big = city_night(i.uv, fw) * night;
    }
    let e = p.surf_glow.rgb * big + vec3<f32>(saturate(big - 0.9) * 0.4);
    let a = saturate(t.a + max(e.r, max(e.g, e.b)));
    let c = (t.rgb * lit(s) * t.a + e) / max(a, 1e-3);
    return vec4<f32>(max(c, vec3<f32>(0.0)) * p.shine.w, a);
}

// The cities under the Stock look: the development texture times the alloc picture (colour and
// alpha, the engine's stage 1 MODULATE), lit as the ground is and blended over it.
fn stock_city(i: FragIn) -> vec4<f32> {
    let n = normalize(i.world_position.xyz - p.centre.xyz);
    let l = min(vec3<f32>(p.base.w) + p.dcol0.rgb * max(dot(n, p.dvec0.xyz), 0.0) + p.dcol1.rgb * max(dot(n, p.dvec1.xyz), 0.0), vec3<f32>(1.0));
    let t = textureSample(tex, tex_sampler, i.uv) * textureSample(city_alloc, city_alloc_sampler, i.uv);
    let shown = pow(max(t.rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)) * l;
    return vec4<f32>(pow(shown, vec3<f32>(2.2)), t.a);
}

// The Stock look: the engine's fixed pipeline. The texel times the sum of the lights (each at
// its colour, equal weight, plus the constant term), both in display space, clamped to 1.
// mode.z says which mesh this is: 0 ground, 1 cloud shell, 2 halo (nothing drawn).
fn stock(i: FragIn) -> vec4<f32> {
    if p.mode.z > 2.5 {
        return stock_city(i);
    }
    if p.mode.z > 1.5 {
        return vec4<f32>(0.0);
    }
    if p.mode.z > 0.5 && p.mode.w > 0.5 {
        // A cloud shell whose texture is missing would be a white wall: no shell.
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let n = normalize(i.world_position.xyz - p.centre.xyz);
    let l = min(vec3<f32>(p.base.w) + p.dcol0.rgb * max(dot(n, p.dvec0.xyz), 0.0) + p.dcol1.rgb * max(dot(n, p.dvec1.xyz), 0.0), vec3<f32>(1.0));
    let t = textureSample(tex, tex_sampler, i.uv);
    let shown = pow(max(t.rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)) * l;
    return vec4<f32>(pow(shown, vec3<f32>(2.2)), t.a);
}

@fragment
fn fragment(i: FragIn) -> @location(0) vec4<f32> {
    if p.mode.y > 0.5 {
        return stock(i);
    }
    if p.mode.z > 2.5 {
        return city(i);
    }
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
