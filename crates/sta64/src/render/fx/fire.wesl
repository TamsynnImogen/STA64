// Fireballs and shock rings: the explosions of a destroyed unit.
//
// Ported from Armada II Refit's render3d/mods/effects/shaders/effects_explosions.hlsl
// (`fire_ps`, `ring_ps`; MIT, Copyright (c) 2026 Cedric Haase, see NOTICE.md next to
// this file). The fireball is ray-marched through a sphere about the explosion: gas
// from a noise field in its own radii (so the billows grow with it), heated and
// cooling through a black-body ramp, smoke as it cools, with a flash and a halo; the
// light above `hdr.y` is kept in part (`hdr.x`) so the white-hot core goes past white
// and blooms. The shock ring is a band of fire in the plane of the explosion, dark
// inside, flame tongues reaching in from about half the radius, rising to a sharp
// outer edge.
//
// Differences from Refit: its Texture3D of spectral noise is a procedural value noise
// here (no texture to bake or bind), and the sphere's back faces are drawn with no
// depth test where Refit draws a quad that writes the depth the gas sits at, so a
// hull in front of an explosion is covered by it.
//
// Blending is ONE, ONE_MINUS_SRC_ALPHA: rgb is light, premultiplied; alpha what the
// smoke hides of what is behind. The ring writes no alpha, so it only adds.

struct Params {
    eye: vec4<f32>,    // xyz the camera; w the near plane's view depth
    ctr: vec4<f32>,    // xyz the centre; w the fireball's full radius (world units)
    time: vec4<f32>,   // x its radius now (world); y the age over the life (0..1); z the age in seconds; w Brightness
    seed: vec4<f32>,   // xyz this explosion's offset into the noise; w the noise's scale (tiles across the fireball)
    shape: vec4<f32>,  // x Smoke; y Turbulence; z Cooling; w Opacity (of the hot gas)
    misc: vec4<f32>,   // x Flash; y Glow (the halo); z hdr: how far the hottest gas goes past white; w the light it starts at
    look: vec4<f32>,   // xyz the view's forward axis; w 0 fireball, 1 ring
    ring: vec4<f32>,   // xyz its plane's normal; w its radius now (world)
    ringb: vec4<f32>,  // x its width (of the radius); y how bright now; z its max radius; w seed
    ringc: vec4<f32>,  // rgb its colour; w Breakup
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: Params;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
};

const STEPS: i32 = 24;

fn hash12(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    q = q + dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn hash13(p3: vec3<f32>) -> f32 {
    var p = fract(p3 * 0.1031);
    p = p + dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
}

// Value noise, 0..1, smooth: stands in for Refit's tiling spectral volume.
fn vn3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash13(i), hash13(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
                mix(hash13(i + vec3<f32>(0.0, 1.0, 0.0)), hash13(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(hash13(i + vec3<f32>(0.0, 0.0, 1.0)), hash13(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
                mix(hash13(i + vec3<f32>(0.0, 1.0, 1.0)), hash13(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

// A fractal field (two octaves) about 0..1.
fn fractal(p: vec3<f32>) -> f32 {
    return 0.65 * vn3(p) + 0.35 * vn3(p * 2.01 + 3.7);
}

// The colour of gas at temperature t (0 cold .. 1 white hot): deep red, orange,
// yellow, white, roughly a black body's, with its light rising steeply with t.
fn heat_colour(t: f32) -> vec3<f32> {
    var c = mix(vec3<f32>(0.45, 0.04, 0.0), vec3<f32>(1.0, 0.26, 0.02), clamp(t * 2.5, 0.0, 1.0));
    c = mix(c, vec3<f32>(1.0, 0.6, 0.12), clamp((t - 0.45) * 2.5, 0.0, 1.0));
    c = mix(c, vec3<f32>(1.0, 0.9, 0.7), clamp((t - 0.88) * 8.0, 0.0, 1.0));
    return c * (t * t * 3.0);
}

fn fire(w: vec3<f32>, vp: vec2<f32>) -> vec4<f32> {
    let d = normalize(w - k.eye.xyz);
    let oc = k.eye.xyz - k.ctr.xyz;
    let R = max(k.time.x, 0.01);
    let s = k.time.y;
    let b = dot(oc, d);
    let tc = max(-b, 0.0);                              // the ray's closest approach to the centre
    let dc = length(oc + d * tc);
    let cosv = max(dot(d, k.look.xyz), 0.05);

    // The flash: a white point that blooms and is gone in a fifth of a second, and a
    // halo of the fireball's light about it while it burns.
    let heat = clamp(1.15 - s * s * k.shape.z * 1.2, 0.0, 1.0); // hot through the first half, then cooling to smoke
    let rf = k.ctr.w;
    let q = dc / (rf * 0.55);
    let flash = k.misc.x * exp2(-k.time.z * 9.0) * (exp2(-q * q * 4.0) * 3.0 + exp2(-q * q * 1.5) * 0.6);
    let q2 = dc / (R * 1.1);
    let halo = k.misc.y * heat * heat * exp2(-q2 * q2 * 3.0);
    let glow = vec3<f32>(1.0, 0.9, 0.75) * flash + vec3<f32>(1.0, 0.45, 0.12) * halo;

    var col = vec3<f32>(0.0);
    var tr = 1.0;
    let Rm = R * 1.3;                                    // the march: the noise pushes the gas out past R
    let h = b * b - (dot(oc, oc) - Rm * Rm);
    if h > 0.0 {
        let sq = sqrt(h);
        let t0 = max(-b - sq, k.eye.w / cosv);
        let t1 = -b + sq;
        if t1 > t0 {
            let dt = (t1 - t0) / f32(STEPS);
            var t = t0 + dt * hash12(vp);
            let turb = k.shape.y * (0.7 + 0.8 * s);
            let fade = 1.0 - smoothstep(0.55, 1.0, s);
            let smoke = k.shape.x * smoothstep(0.3, 0.8, s); // the fire first, the smoke as it cools
            let dn = dt / R;                             // a step, in fireball radii
            for (var j = 0; j < STEPS; j = j + 1) {
                let u = (k.eye.xyz + d * t - k.ctr.xyz) / R; // in the fireball's radii
                let r = length(u);
                // Noise in the gas's own coordinates, so its billows grow with it; the
                // warp turns it over as it ages.
                let wp = u * 0.23 + k.seed.zxy;
                let wv = vec3<f32>(vn3(wp), vn3(wp + 17.3), vn3(wp + 41.9)) * 2.0 - 1.0;
                let p = u * k.seed.w + k.seed.xyz + wv * (0.06 + 0.08 * s) + vec3<f32>(0.0, 0.0, s * 0.2);
                let n1 = fractal(p);
                let n2 = vn3(p * 2.13 + 0.37);
                let f = (n1 - 0.5) * 5.0 + (n2 - 0.5) * 2.5; // about a z-score
                let dens = clamp((0.85 - r) * 3.0 + f * turb * 0.3, 0.0, 1.0) * smoothstep(1.3, 1.0, r) * fade;
                let temp = clamp(heat * (1.05 - r * r * 0.7) + f * 0.2 - (1.0 - dens) * 0.3, 0.0, 1.0);
                let e = heat_colour(temp) * dens;
                let sig = dens * (k.shape.w * heat + smoke * (1.0 - temp) * 3.0);
                let a = 1.0 - exp2(-sig * dn * 6.0);
                let sm = vec3<f32>(0.02, 0.015, 0.012) * (0.2 + 2.5 * temp) * smoke; // smoke, lit by the fire inside
                col = col + tr * (e * dn * 1.6 * k.time.w + sm * a);
                tr = tr * (1.0 - a);
                t = t + dt;
            }
        }
    }
    // Saturate by the brightest channel, so a bright gas keeps its hue (orange stays
    // orange) instead of clipping towards a milky white; only the hottest gas is white.
    // In HDR, the light above hdr.w is kept in part (hdr.z) on top of that.
    let L = max(max(col.r, col.g), max(col.b, 1e-4));
    col = col * ((1.0 - exp2(-L * 1.8) + k.misc.z * max(L - k.misc.w, 0.0)) / L);
    col = col + glow * k.time.w * tr;
    return vec4<f32>(col, 1.0 - tr);
}

// A shockwave: a broad band of fire in the plane of the explosion, as stock's disc
// draws it. The class's colour, driven past 1 as stock's is, so its red clips and the
// hottest part goes yellow.
fn ring(w: vec3<f32>) -> vec4<f32> {
    let d = normalize(w - k.eye.xyz);
    let n = k.ring.xyz;
    let dn = dot(d, n);
    let t = dot(k.ctr.xyz - k.eye.xyz, n) / select(1e-4, dn, abs(dn) > 1e-4);
    let x = k.eye.xyz + d * t - k.ctr.xyz;
    // In the radius stock's disc shows, its sixteen-sided outline's: 0.955 of its radius.
    let rr = length(x) / max(k.ring.w * 0.955, 0.01);
    // The noise in the disc's own radii, so the pattern grows with it. The tongues are
    // read off the direction from the centre, moving only slowly along the plane's
    // normal with the radius, so they are long spikes pointing in.
    let dir = x / max(length(x), 1e-4);
    let pt = dir * 3.2 + n * rr * 0.9 + k.ringb.w;
    let zt = (fractal(pt) - 0.5) * 3.5 + (vn3(pt * 2.37 + 0.5) - 0.5) * 0.8 + (vn3(pt * 0.43 + 0.71) - 0.5) * 3.0;
    // The speckle in blotches a little drawn out along the radius, at two scales whose
    // ratio is not whole.
    let ps = dir * 4.1 + n * rr * 6.0 + k.ringb.w * 1.7;
    let zs = clamp((vn3(ps) - 0.5) * 3.5 + (vn3(ps * 2.29 + 0.31) - 0.5) * 2.0, -2.0, 2.0);
    let wd = k.ringb.x;                                         // the band's width, of the radius
    let r0 = 1.0 - wd + 0.06 + zt * wd * 0.13 * k.ringc.w;      // where the fire starts, broken into tongues
    let u = clamp((rr - r0) / (wd * 0.25), 0.0, 1.0);           // 0 at the inner edge .. 1 a little past it
    // Measured off stock's disc: the class's colour times 1 at 0.7 of the radius, about
    // 1.8 at 0.88 and 2.4 at 0.93, darker again just inside the edge, a faint red fringe
    // just outside it.
    var I = u * u * (3.0 - 2.0 * u) * (1.0 + 0.6 * smoothstep(0.68, 0.88, rr) + 0.75 * smoothstep(0.86, 0.93, rr));
    I = I * (1.0 + zs * 0.18 * k.ringc.w);
    I = I * mix(1.0, 0.45, smoothstep(0.94, 0.99, rr));         // darker just inside the edge
    I = I * smoothstep(1.0, 0.985, rr);                         // the edge, sharp
    let fr = 0.3 * smoothstep(1.06, 1.0, rr) * smoothstep(0.97, 1.0, rr) * clamp(1.0 + zs * 0.4, 0.0, 1.0);
    let edge = 1.0 / (0.35 + 0.65 * abs(dn));                   // seen edge on, a disc is a brighter line
    // Redder inside, as stock's texture is.
    let hg = mix(0.75, 1.0, smoothstep(0.75, 0.93, rr));
    let c = k.ringc.rgb * (vec3<f32>(1.0, hg, 1.0) * I / hg + vec3<f32>(1.0, 0.4, 0.4) * fr) * k.ringb.y * min(edge, 2.0);
    if !(t > k.eye.w && max(c.r, max(c.g, c.b)) > 0.002) {
        discard;
    }
    return vec4<f32>(c * k.time.w, 0.0);
}

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    if k.look.w < 0.5 {
        return fire(in.world_position.xyz, in.position.xy);
    }
    return ring(in.world_position.xyz);
}
