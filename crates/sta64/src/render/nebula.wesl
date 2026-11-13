// Nebulae drawn as volumes of gas, marched per pixel through a class's field.
//
// Ported from Armada II Refit's render3d/shaders/world_nebula.hlsl (MIT, Copyright (c)
// 2026 Cedric Haase), itself Nebulae.asi: the envelope of domes over the map plane,
// warped thresholded noise, self-shading a step toward the gas's light, filaments,
// fringe, knots and lanes, a brighter core, two hues and a glow, the key light
// scattered forward, the near fade and edge-on dimming, flow that churns the gas,
// and optical depth that dims what lies behind. The march stops at the scene's depth.
// RND-05 adds the core's pulse, the ring's colours about the core, the swirl (the gas
// wanders round two circles about its place) and the lightning (up to four flashes
// that light the gas about them), as Refit's gas_colour and march do.
// RND-15 adds the ships' shadows (`key_lit`, after Refit's world_nebula.hlsl).
// Not ported yet: temporal smoothing, the baked volume (the gas is
// evaluated directly at each step).

struct NebulaParams {
    eye: vec4<f32>,       // xyz the camera; w the near fade (world units)
    gas_a: vec4<f32>,     // rgb the gas; w brightness
    gas_b: vec4<f32>,     // rgb the second hue; w hue scale
    glow: vec4<f32>,      // rgb the densest gas; w gamma on the density
    noise: vec4<f32>,     // 1/tile, warp, coverage, softness
    seed: vec4<f32>,      // xyz the noise domain's offset; w detail
    env: vec4<f32>,       // xy the envelope's origin (x, z); zw 1/its size
    vert: vec4<f32>,      // x the field's mean y; y the domes' height; w edge-on scale / reach
    shape: vec4<f32>,     // ridge, hue mix, warp scale, lumps
    knots: vec4<f32>,     // knots, their scale, lanes, their scale
    light: vec4<f32>,     // xyz toward the gas's light times the shade step; w shading
    fine: vec4<f32>,      // x fine detail, y fuzz, z flow (world units), w flow scale
    flow: vec4<f32>,      // x the drift now; w edge warp (world units)
    fil: vec4<f32>,       // filaments, scale, sharpness, height (world units)
    fringe: vec4<f32>,    // x fringe, y its scale
    core: vec4<f32>,      // x core, y its sharpness
    opac: vec4<f32>,      // x optical depth per unit of the densest gas; y the heights' spread
    key: vec4<f32>,       // xyz toward the key light; w its share of the gas's light
    scat: vec4<f32>,      // x 1 + g^2, y 2g (Henyey-Greenstein); z the march's steps (0: 16)
    box_lo: vec4<f32>,    // the field's box, low corner; w the camera's near plane
    box_hi: vec4<f32>,
    cam_fwd: vec4<f32>,   // the camera's forward axis
    pulse: vec4<f32>,     // x Pulse (0: off); y its phase now (radians); z 1 with lightning
    ring: vec4<f32>,      // x how much the ring tints (0: off); y where (core profile); z its width; w the colours' phase now
    rpatch: vec4<f32>,    // x the patches' scale (of the tile); y how much of the ring they cover; z their softness; w the colours' spread
    ring_a: vec4<f32>,    // rgb the ring's four colours, in turn
    ring_b: vec4<f32>,
    ring_c: vec4<f32>,
    ring_d: vec4<f32>,
    swirl: vec4<f32>,     // x Swirl (world units; 0: off); y its phases' scale (of the tile); z, w the two circles' phases now
    flash0: vec4<f32>,    // lightning: xyz where; w how bright now (0: dark)
    flash1: vec4<f32>,
    flash2: vec4<f32>,
    flash3: vec4<f32>,
    flash_c: vec4<f32>,   // rgb the lightning's colour; w 1 / its radius squared
    sh0: vec4<f32>,       // the Key's shadow map (shadows.rs): world -> u,
    sh1: vec4<f32>,       //   v,
    sh2: vec4<f32>,       //   and depth along the Key (0..1 over the hulls)
    shp: vec4<f32>,       //   x 1: there is a map (0: none), y the depth bias, z the penumbra (u, v per depth behind the caster), w a texel (u, v)
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: NebulaParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var s_noise: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var s_noise_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var s_env: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var s_env_sampler: sampler;
// The Key's shadow map: r + g / 2048 the depth of the nearest hull along the Key, 1 where none.
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var s_shadow: texture_2d<f32>;

@if(DEPTH_PREPASS && MULTISAMPLED)
@group(0) @binding(20) var depth_prepass_texture: texture_depth_multisampled_2d;
@if(DEPTH_PREPASS && !MULTISAMPLED)
@group(0) @binding(20) var depth_prepass_texture: texture_depth_2d;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
};

fn rot(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(p, vec3<f32>(0.00, 0.80, 0.60)), dot(p, vec3<f32>(-0.80, 0.36, -0.48)), dot(p, vec3<f32>(-0.60, -0.48, 0.64)));
}
fn rot2(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(p, vec3<f32>(0.00, -0.80, -0.60)), dot(p, vec3<f32>(0.80, 0.36, -0.48)), dot(p, vec3<f32>(0.60, -0.48, 0.64)));
}

fn nz(p: vec3<f32>) -> vec4<f32> {
    return textureSampleLevel(s_noise, s_noise_sampler, p, 0.0);
}

// Two reads of the tile, turned apart, so no lattice lines up along a long path.
fn noise2(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return (nz(a).r + nz(b).r - 1.0) * 0.7071 + 0.5;
}

fn gas_density(p: vec3<f32>, w: vec3<f32>, cover: f32, det: f32) -> f32 {
    let q = rot(p) * k.noise.x + k.seed.xyz;
    let q2 = rot2(p) * (k.noise.x * 0.786) + k.seed.yzx;
    let base = noise2(q + w * k.noise.y, q2 + w.zxy * k.noise.y);
    let f = mix(base, det, k.seed.w);
    var n = (f - 0.5) * 8.0;
    n = mix(n, 2.0 - abs(n) * 2.0, k.shape.x);
    let dens = saturate((n + (cover - 1.0) * 4.0 - k.noise.z) / k.noise.w + 0.5) * smoothstep(0.0, 0.35, cover);
    return pow(dens, k.glow.w);
}

// x density, y shading, z hue, w emission; x < 0 outside the envelope.
fn gas_at(p: vec3<f32>) -> vec4<f32> {
    let q = rot(p) * k.noise.x + k.seed.xyz;
    let w = nz(q * k.shape.z).gba * 2.0 - 1.0;
    let env = textureSampleLevel(s_env, s_env_sampler, (p.xz + w.xz * k.flow.w - k.env.xy) * k.env.zw, 0.0);
    if env.a <= 0.002 && env.g <= 0.002 {
        return vec4<f32>(-1.0, 1.0, 0.0, 1.0);
    }
    let y = p.y - (k.vert.x + (env.b * 2.0 - 1.0) * k.opac.y) + w.y * k.shape.w * k.vert.y * 0.5;
    let hgt = env.r * k.vert.y * (1.0 + w.z * k.shape.w);
    let t = abs(y) / max(hgt, 1.0);
    let cover = env.a * smoothstep(1.0, 0.55, t);
    let det = noise2(q * 3.13 + w.yzx * (k.noise.y * 0.5), rot2(p) * (k.noise.x * 2.71) + k.seed.zxy + w.xzy * (k.noise.y * 0.5));
    var dens = 0.0;
    var ahead = 0.0;
    if cover > 0.001 {
        dens = gas_density(p, w, cover, det);
        ahead = gas_density(p + k.light.xyz, w, cover, det);
    }
    var shade = clamp(1.0 + (dens - ahead) * k.light.w * 2.0, 0.25, 1.6);
    if k.fil.x > 0.0 {
        let fr = nz(q * k.fil.y + w * (k.noise.y * 1.5) + vec3<f32>(0.13, 0.57, 0.91)).r;
        let ridge = saturate(1.0 - abs(fr - 0.5) * 4.0);
        var fl = pow(ridge, k.fil.z) * env.g * smoothstep(1.0, 0.2, abs(y) / k.fil.w);
        fl *= k.fil.x * saturate(det * 1.6);
        dens = dens + fl * (1.0 - dens);
        shade = mix(shade, 1.2, fl);
    }
    if k.fringe.x > 0.0 {
        let fz = noise2(q * k.fringe.y + w * (k.noise.y * 0.7) + vec3<f32>(0.37, 0.11, 0.73),
                        rot2(p) * (k.noise.x * k.fringe.y * 0.786) + k.seed.zxy + vec3<f32>(0.52, 0.23, 0.81));
        let haze = smoothstep(0.40, 0.72, fz) * env.g * smoothstep(1.0, 0.1, abs(y) / max(k.vert.y, 1.0));
        dens = dens + k.fringe.x * haze * (1.0 - dens);
    }
    let kn = nz(q * k.knots.y + vec3<f32>(0.31, 0.17, 0.53)).r;
    let knot = 1.0 + k.knots.x * smoothstep(0.5, 0.8, kn);
    let ln = nz(q * k.knots.w + vec3<f32>(0.71, 0.43, 0.29)).g;
    let lane = 1.0 - k.knots.z * smoothstep(0.35, 0.05, abs(ln - 0.5) * 2.0);
    let core = 1.0 + k.core.x * pow(saturate(env.r) * saturate(1.0 - t), k.core.y);
    let hue = saturate((w.x * k.gas_b.w) * 0.5 + 0.5);
    return vec4<f32>(dens, shade, hue, knot * lane * core);
}

fn key_phase(cv: f32) -> f32 {
    return pow(k.scat.x / max(k.scat.x - k.scat.y * cv, 1e-3), 1.5);
}

// The flow's offset at p: two drifting reads turned apart, so the gas churns.
fn flow_at(p: vec3<f32>) -> vec3<f32> {
    let fd = vec3<f32>(k.flow.x, k.flow.x * 0.71, k.flow.x * 0.37);
    let f = k.noise.x * k.fine.w;
    let a = nz(p * f + fd).gba;
    let b = nz(rot2(p) * (f * 0.786) - fd.zxy + 0.5).gba;
    return clamp((a + b - 1.0) * 1.4142, vec3<f32>(-1.0), vec3<f32>(1.0));
}

// The swirl's offset at p: the read goes round two circles, the second the other way
// and 1.618 times as fast, at phases and a size that vary smoothly from place to place,
// so each speck wanders in loops out of step with its neighbours.
fn swirl_at(p: vec3<f32>) -> vec3<f32> {
    let sn = nz(rot(p) * (k.noise.x * k.swirl.y) + k.seed.yzx + vec3<f32>(0.29, 0.61, 0.17)).gba;
    let a1 = k.swirl.z + sn.x * 6.2832;
    let a2 = -k.swirl.w + sn.y * 6.2832;
    let r = saturate(0.35 + sn.z * 0.9) * k.swirl.x;
    return r * (0.62 * vec3<f32>(cos(a1), 0.4 * sin(a2), sin(a1)) + 0.38 * vec3<f32>(sin(a2), 0.6 * cos(a1), cos(a2)));
}

// About the core: the pulse brightens and dims it, out of step from one nebula to the
// next; the ring tints the gas in patches whose colours go round in turn.
fn core_effects(c_in: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    var c = c_in;
    let e = textureSampleLevel(s_env, s_env_sampler, (p.xz - k.env.xy) * k.env.zw, 0.0);
    let t = abs(p.y - k.vert.x - (e.b * 2.0 - 1.0) * k.opac.y) / max(e.r * k.vert.y, 1.0);
    let cp = saturate(e.r) * saturate(1.0 - t);
    if k.pulse.x > 0.0 {
        let off = nz(p * (k.noise.x * 0.05)).g * 12.566;
        c *= 1.0 + k.pulse.x * sin(k.pulse.y + off) * pow(cp, k.core.y * 0.5);
    }
    if k.ring.x > 0.0 {
        var ring = saturate(1.0 - abs(cp - k.ring.y) / k.ring.z);
        let n = nz(p * (k.noise.x * k.rpatch.x) + vec3<f32>(0.37, 0.11, 0.73)).g;
        ring *= smoothstep(1.0 - k.rpatch.y - k.rpatch.z, 1.0 - k.rpatch.y + k.rpatch.z, n);
        let s = fract(k.ring.w + n * k.rpatch.w) * 4.0;
        let f = fract(s);
        var a = k.ring_a.rgb;
        var b = k.ring_b.rgb;
        if s >= 3.0 {
            a = k.ring_d.rgb;
            b = k.ring_a.rgb;
        } else if s >= 2.0 {
            a = k.ring_c.rgb;
            b = k.ring_d.rgb;
        } else if s >= 1.0 {
            a = k.ring_b.rgb;
            b = k.ring_c.rgb;
        }
        // A tint, not more light: the gas there takes the colour at its own brightness.
        let lum = dot(c, vec3<f32>(0.3, 0.5, 0.2));
        c = mix(c, mix(a, b, smoothstep(0.0, 1.0, f)) * (lum * 1.4), saturate(ring * k.ring.x));
    }
    return c;
}

// The light of the lightning's flashes at p: the gas about each lights up in the
// lightning's colour. Not shadowed by the gas between, and the key's phase leaves it.
fn flashes_at(p: vec3<f32>) -> f32 {
    let d0 = p - k.flash0.xyz;
    let d1 = p - k.flash1.xyz;
    let d2 = p - k.flash2.xyz;
    let d3 = p - k.flash3.xyz;
    return k.flash0.w * exp(-dot(d0, d0) * k.flash_c.w) + k.flash1.w * exp(-dot(d1, d1) * k.flash_c.w)
         + k.flash2.w * exp(-dot(d2, d2) * k.flash_c.w) + k.flash3.w * exp(-dot(d3, d3) * k.flash_c.w);
}

fn shadow_depth(uv: vec2<f32>) -> f32 {
    let dim = textureDimensions(s_shadow);
    let px = clamp(vec2<i32>(floor(uv * vec2<f32>(dim))), vec2<i32>(0), vec2<i32>(dim) - vec2<i32>(1));
    let t = textureLoad(s_shadow, px, 0);
    return t.r + t.g / 2048.0;
}

// How much of the Key reaches p past the hulls (0..1): a ship's shadow through the gas.
// The light is a disc, not a point, so a shadow is sharp next to the hull and widens into
// its penumbra the farther behind the hull the gas is. Two reads, as PCSS does with many:
// the first, anywhere in the widest penumbra possible at p's depth, finds a caster and how
// far in front of p it is; the second, within the penumbra that distance gives, is the
// answer. Each read is a point of a disc picked from the pixel's thresholds u and the step
// kk: one pixel's answer is 0 or 1, and the steps along its ray add them up into a soft
// edge. Outside the map (it covers the hulls in view), the Key is not blocked.
fn key_lit(p: vec3<f32>, u: vec2<f32>, kk: f32) -> f32 {
    if k.shp.x <= 0.0 {
        return 1.0;
    }
    let q = vec4<f32>(p, 1.0);
    let s = vec3<f32>(dot(q, k.sh0), dot(q, k.sh1), dot(q, k.sh2));
    if s.z < k.shp.y || s.x < 0.0 || s.x > 1.0 || s.y < 0.0 || s.y > 1.0 {
        return 1.0;
    }
    let a = fract(u + kk * vec2<f32>(0.3819660, 0.5698403) + vec2<f32>(0.25, 0.75));
    let b = fract(a + vec2<f32>(0.5, 0.5));
    let da = vec2<f32>(cos(a.x * 6.2831853), sin(a.x * 6.2831853)) * sqrt(a.y);
    let db = vec2<f32>(cos(b.x * 6.2831853), sin(b.x * 6.2831853)) * sqrt(b.y);
    // The map is cleared to 1 and its hulls lie well inside 0..1, so a 1 is no hull, not a
    // hull at the far end: gas beyond every hull along the Key (z > 1) would otherwise be
    // in shadow wherever there is no hull at all.
    let z = s.z - k.shp.y;
    let zc = min(z, 0.999);
    let half_texel = 0.5 * k.shp.w;
    var d = shadow_depth(s.xy + da * (half_texel + z * k.shp.z));
    let r = half_texel + select(0.0, (z - d) * k.shp.z, d < zc);
    d = shadow_depth(s.xy + db * r);
    return select(1.0, 0.0, d < zc);
}

// One step: rgb the light the gas adds over `thick`; a its optical depth. u: the pixel's
// two thresholds; kk: the step's place along the ray (0..1), for the shadow's sampling.
fn march_step(p: vec3<f32>, dist: f32, thick: f32, ph: f32, u: vec2<f32>, kk: f32) -> vec4<f32> {
    var pa = p;
    if k.fine.z > 0.0 {
        pa = p + flow_at(p) * k.fine.z;
    }
    if k.swirl.x > 0.0 {
        pa += swirl_at(p);
    }
    let v = gas_at(pa);
    if v.x <= 0.002 {
        return vec4<f32>(0.0);
    }
    var det = 0.5;
    if k.fine.x > 0.0 || k.fine.y > 0.0 {
        det = noise2(rot(pa) * (k.noise.x * 6.1) + k.seed.zxy, rot2(pa) * (k.noise.x * 5.3) + k.seed.yzx);
    }
    var dn = saturate(v.x * (1.0 + (det - 0.5) * 2.0 * k.fine.x));
    if k.fine.y > 0.0 {
        dn = saturate(dn * (1.0 + k.fine.y * (1.0 - smoothstep(0.0, 0.45, dn)) * clamp((det - 0.5) * 8.0, -1.0, 1.0)));
    }
    let hue = saturate(mix(v.z, dn, k.shape.y));
    let gas = mix(k.gas_a.rgb, k.gas_b.rgb, hue);
    var c = (gas * dn + k.glow.rgb * (dn * dn) * v.w) * v.w * v.y;
    if k.pulse.x > 0.0 || k.ring.x > 0.0 {
        c = core_effects(c, p);
    }
    if k.key.w > 0.0 {
        c *= 1.0 - k.key.w + k.key.w * ph * key_lit(p, u, kk);
    }
    if k.pulse.z > 0.5 {
        let f = flashes_at(p);
        c += k.flash_c.rgb * f * (dn + 0.15 * f * f);
    }
    let near = saturate((dist - k.eye.w * 0.25) / k.eye.w);
    return vec4<f32>(c * (k.gas_a.w * near * thick * k.vert.w), dn * near * thick * k.opac.x);
}


@fragment
fn fragment(i: FragIn) -> @location(0) vec4<f32> {
    let eye = k.eye.xyz;
    let d = normalize(i.world_position.xyz - eye);
    // The ray through the field's box.
    let dz = select(d, vec3<f32>(1e-6), abs(d) < vec3<f32>(1e-6));
    let ta = (k.box_lo.xyz - eye) / dz;
    let tb = (k.box_hi.xyz - eye) / dz;
    let t0 = min(ta, tb);
    let t1 = max(ta, tb);
    let s0 = max(max(t0.x, t0.y), max(t0.z, k.box_lo.w));
    var s1 = min(min(t1.x, t1.y), t1.z);
    // Stop at what the scene shows: reverse-Z depth is near / view depth. (Sample 0
    // serves the single- and the multisampled prepass texture alike.)
    @if(DEPTH_PREPASS)
    {
        let px = vec2<i32>(i.position.xy);
        let z = textureLoad(depth_prepass_texture, px, 0);
        if z > 0.0 {
            let view_depth = k.box_lo.w / z;
            s1 = min(s1, view_depth / max(dot(d, k.cam_fwd.xyz), 1e-4));
        }
    }
    if s1 <= s0 {
        discard;
    }
    // Steps growing geometrically with distance, each jittered within its span.
    // The march's steps: a setting (`RenderQuality::nebula_steps`), 16 by default.
    let steps = select(16, i32(k.scat.z), k.scat.z >= 1.0);
    let q = exp2(log2(s1 / s0) / f32(steps));
    let ph = key_phase(dot(k.key.xyz, d));
    let jit = fract(sin(dot(i.position.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let u = vec2<f32>(jit, fract(sin(dot(i.position.xy, vec2<f32>(39.3468, 11.135))) * 43758.5453));
    var acc = vec4<f32>(0.0);
    var a = s0;
    for (var n = 0; n < steps; n++) {
        let th = a * (q - 1.0);
        let j = fract(jit + f32(n) * 0.6180340);
        let s = a + j * th;
        acc += march_step(eye + d * s, s, th, ph, u, f32(n) / f32(steps));
        a += th;
    }
    // Premultiplied: the gas's light added, what lies behind dimmed by its optical depth.
    let alpha = 1.0 - exp(-acc.a);
    return vec4<f32>(acc.rgb, alpha);
}
