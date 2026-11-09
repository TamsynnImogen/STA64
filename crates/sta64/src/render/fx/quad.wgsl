// Quad effects: the flare, beam, bolt and cone of Armada II Refit's ordnance module,
// the arc flare, spark streak, hot spot and seam of its repair and construction
// welding.
//
// Ported from Refit's render3d/mods/ordnance/shaders/ordnance.hlsl (fx_ps),
// render3d/mods/effects/shaders/effects_repair.hlsl and effects_construction.hlsl
// (MIT, Copyright (c) 2026 Cedric Haase; see NOTICE.md next to this file). The
// per-quad vertex attributes of the HLSL (t0..t4) are one uniform per material here:
// every effect is its own quad, and the kind is uniform, so the branch is uniform too.
//
// The quad is a 2 x 2 rectangle with uv 0..1; each kind maps that to its own
// coordinates, as Refit's `fx.rs` / `construction.rs` built them:
//   0 flare:  xy -1..1.            col, col2 the streak's colour, p3 = core sharpness,
//             glow sharpness, star share, star sharpness; p4 = streak share,
//             streak sharpness, the star's angle.
//   2 beam:   x along it in half widths (-1 at the start's cap .. len + 1), y -1..1;
//             col2 the core's colour; p3 = core width, flow share, flow frequency,
//             flow speed; p4 = helix amplitude, frequency, speed, crackle.
//   3 bolt:   x along its flight in units of its radius (-tail .. 1), y -1..1;
//             col2 the core's colour; p3 = core sharpness, glow sharpness, rays share,
//             rays' angle.
//   6 cone:   x the way along (0 emitter, 1 the beam's end, s_end past it), y -1..1
//             across the wide end; len = the beam's length in wide-end half widths;
//             col2.x = s_end, col2.w = narrow end's half width over the wide end's;
//             p3 = fans (count, light between edges, rate, edge px); p4 = vapour depth,
//             blobs per half width, vapour speed, brightness at the target.
//   7 streak (a spark):   x 0..1 tail to head, y -1..1; len in widths.
//   8 hot spot:           xy -1..1.
//   9 seam:               x 0..1 along, y -1..1; len = cooling rate; col tints it.
//  10 weld beam:          x 0..1 (bee to weld), y -1..1; col.w = phase of the crackle.
//  11 arc flare:          xy -1..1; p3 = core, glow, rays share, rays sharpness; p4.x angle.
//  12 cloak front:        xy -1..1; p3 = bands over the disc, speed, tilt. Not Refit's
//                         shader (its cloak bends the hull in `hulls.hlsl`): the thin
//                         green line along each band of ART-DIRECTION.md's cloak.

struct Params {
    a: vec4<f32>,    // x kind, y brightness, z clock (s), w len
    col: vec4<f32>,  // rgb colour, w seed
    col2: vec4<f32>, // rgb second colour, w extra
    p3: vec4<f32>,
    p4: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> k: Params;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

const PI: f32 = 3.14159265;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let c = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(c), hash21(c + vec2<f32>(1.0, 0.0)), u.x),
               mix(hash21(c + vec2<f32>(0.0, 1.0)), hash21(c + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn rot(uv: vec2<f32>, a: f32) -> vec2<f32> {
    let s = sin(a);
    let c = cos(a);
    return vec2<f32>(c * uv.x - s * uv.y, s * uv.x + c * uv.y);
}

// A flare: a white-hot core, a glow in its colour, a four-pointed star and a long thin
// horizontal streak, as a bright light makes in a lens.
fn flare(uv: vec2<f32>, len: f32, col: vec3<f32>, streak_col: vec3<f32>, p: vec4<f32>, q: vec4<f32>) -> vec3<f32> {
    let r = length(uv);
    let fade = clamp(1.0 - r, 0.0, 1.0);
    let rq = rot(uv, q.z);
    let core = exp(-r * r * p.x);
    let glow = exp(-r * p.y) * fade;
    let star = (exp(-abs(rq.x) * p.w) + exp(-abs(rq.y) * p.w)) * fade * fade;
    let along = clamp(1.0 - abs(uv.x) / len, 0.0, 1.0);
    let streak = exp(-abs(uv.y) * q.y) * along * along
               + 0.6 * exp(-abs(uv.y) * q.y * 4.0) * along * along * along * along;
    return vec3<f32>(1.0) * core * 2.0 + col * (glow + star * p.z) + streak_col * streak * q.x;
}

// A beam from its start to its end: a core in its own colour inside a glow, with
// pulses flowing along it, two strands twisting about it, and a crackle that throws
// the core about. Rounded at both ends.
fn beam(uv: vec2<f32>, len: f32, col: vec3<f32>, core_col: vec3<f32>, seed: f32, p: vec4<f32>, q: vec4<f32>) -> vec3<f32> {
    let t = k.a.z;
    let x = uv.x;
    let y = uv.y;
    let dx = max(0.0, max(-x, x - len));
    let edge = clamp(1.0 - sqrt(dx * dx + y * y), 0.0, 1.0);
    let jag = q.w * (0.55 * sin(x * 1.3 + t * 23.0 + seed) + 0.3 * sin(x * 3.7 - t * 31.0 + seed * 2.0)
                   + 0.15 * sin(x * 9.1 + t * 47.0));
    let yc = y - jag * 0.5;
    let d2 = dx * dx + yc * yc;
    let core = exp(-d2 / max(p.x * p.x, 1e-4));
    let glow = exp(-sqrt(dx * dx + y * y) * 2.2);
    let flow = 0.5 + 0.5 * sin(x * p.z - t * p.w + seed);
    let pulses = 1.0 - p.y + p.y * flow * flow;
    let h1 = q.x * sin(x * q.y - t * q.z + seed);
    let h2 = q.x * sin(x * q.y - t * q.z + seed + PI);
    let sw = max(p.x, 0.04) * 1.2;
    let e1 = (y - h1) / sw;
    let e2 = (y - h2) / sw;
    let strands = (exp(-e1 * e1) + exp(-e2 * e2)) * step(0.001, q.x);
    let ends = clamp(min(x + 1.0, len + 1.0 - x), 0.0, 1.0);
    return (core_col * core * (0.75 + 0.25 * flow) + col * (glow * pulses + strands * 0.8)) * edge * edge * ends;
}

// A bolt in flight: a hot core and a glow at its head, a tail of its colour behind it,
// and rays that turn.
fn bolt(uv: vec2<f32>, tail: f32, col: vec3<f32>, core_col: vec3<f32>, p: vec4<f32>) -> vec3<f32> {
    let r = length(uv);
    let fade = clamp(1.0 - r, 0.0, 1.0);
    let core = exp(-r * r * p.x);
    let glow = exp(-r * p.y) * fade;
    let rq = rot(uv, p.w);
    let rays = (exp(-abs(rq.x) * 14.0) + exp(-abs(rq.y) * 14.0)) * fade * fade * p.z;
    let behind = clamp(-uv.x / max(tail, 0.01), 0.0, 1.0);
    let tw = 0.45 * (1.0 - 0.7 * behind) + 0.02;
    let tl = step(uv.x, 0.0) * exp(-uv.y * uv.y / (tw * tw)) * (1.0 - behind) * (1.0 - behind);
    return core_col * core * 1.5 + col * (glow + rays + tl * 0.7);
}

// One fan of a cone: a wedge from the emitter, hard edged and brightest along its
// edges, softer between them. It fades in, holds and fades out, then comes back at
// another angle and width.
fn cone_fan(v: f32, i: f32, seed: f32, t: f32, rate: f32, interior: f32, fw: f32) -> f32 {
    let ph = t * rate * (0.75 + 0.5 * hash21(vec2<f32>(i, seed))) + hash21(vec2<f32>(seed, i * 7.1 + 0.3));
    let n = floor(ph);
    let x = fract(ph);
    var a = sin(PI * x);
    a = a * a;
    let c = (hash21(vec2<f32>(n + i * 17.0, seed)) * 2.0 - 1.0) * 0.55;
    let w = min(0.18 + 0.5 * hash21(vec2<f32>(n * 1.3 + 3.7, i * 1.7 + seed)), 1.0 - abs(c));
    let d = w - abs(v - c);
    let inside = clamp(d / fw, 0.0, 1.0);
    let edge = exp(-max(d, 0.0) / max(w * 0.1, 1e-4));
    return a * inside * (interior + 1.6 * (1.0 - interior) * edge);
}

// A cone of light through vapour (ART-DIRECTION.md, "Tractor and assimilation
// beams"): fans that come and go, vapour drifting along it, brightest at the
// emitter and thinning towards the target, black within a few pixels of its outline.
fn cone(uv: vec2<f32>, lw: f32, col: vec3<f32>, s_end: f32, w0: f32, seed: f32, p: vec4<f32>, q: vec4<f32>) -> vec3<f32> {
    let t = k.a.z;
    let s = uv.x;
    // the half width here over the wide end's
    let wid = w0 + (1.0 - w0) * s;
    let v = uv.y / max(wid, 1e-4);
    let av = abs(v);
    let fw = max(fwidth(v), 1e-4);
    let e = clamp((1.0 - av) / max(fw * p.w, 1e-4), 0.0, 1.0);
    let edge = e * e * (3.0 - 2.0 * e);
    let efw = fw * max(p.w, 1.0);
    var streak = cone_fan(v, 0.0, seed, t, p.z, p.y, efw) * step(0.5, p.x)
               + cone_fan(v, 1.0, seed, t, p.z, p.y, efw) * step(1.5, p.x)
               + cone_fan(v, 2.0, seed, t, p.z, p.y, efw) * step(2.5, p.x)
               + cone_fan(v, 3.0, seed, t, p.z, p.y, efw) * step(3.5, p.x);
    streak = streak * (0.7 + 0.3 * vnoise(vec2<f32>(v * 40.0 + seed, s * 0.6 + t * 0.35)));
    let x = s * lw * q.y;
    let y = v * q.y * 0.5 + seed;
    let run = t * q.z * q.y;
    let n = 0.5 * vnoise(vec2<f32>(x - run, y + t * 0.05))
          + 0.3 * vnoise(vec2<f32>(x * 2.3 - run * 1.9 + 5.2, y * 2.1 - t * 0.11 + 1.3))
          + 0.2 * vnoise(vec2<f32>(x * 5.1 - run * 3.7 + 11.7, y * 4.7 + t * 0.23 + 7.9));
    let vap = max(1.0 - q.x + q.x * 2.0 * n, 0.0);
    let along = mix(1.0, q.w, pow(clamp(s, 0.0, 1.0), 0.45)) * (1.0 + 1.5 * pow(clamp(1.0 - s, 0.0, 1.0), 3.0));
    let f = clamp((s_end - s) / max(s_end - 1.0 + 0.1, 0.05), 0.0, 1.0);
    let far = f * f * (3.0 - 2.0 * f);
    let near = clamp(s * 40.0, 0.0, 1.0);
    let rim = 1.0 + 0.25 * exp(-clamp(1.0 - av, 0.0, 1.0) * 12.0);
    return col * streak * vap * along * far * near * edge * rim;
}

// ---- welding (repair and construction) -----------------------------------------

fn weld_flare(uv: vec2<f32>, ang: f32, col: vec3<f32>, p: vec4<f32>) -> vec3<f32> {
    let r = length(uv);
    let q = rot(uv, ang);
    let d = vec2<f32>(0.70710678 * (q.x - q.y), 0.70710678 * (q.x + q.y));
    let fade = clamp(1.0 - r, 0.0, 1.0);
    let rays = (exp(-abs(q.y) * p.w) + exp(-abs(q.x) * p.w)) * fade * fade
             + 0.5 * (exp(-abs(d.y) * p.w * 1.5) + exp(-abs(d.x) * p.w * 1.5)) * fade * fade * fade;
    let core = exp(-r * r * p.x);
    let glow = exp(-r * p.y) * fade;
    return vec3<f32>(1.0) * core * 2.0 + col * (glow + rays * p.z);
}

// A spark: a capsule along the streak, brighter at its head.
fn streak_spark(uv: vec2<f32>, len: f32, col: vec3<f32>) -> vec3<f32> {
    let s = uv.x * (len + 2.0) - 1.0;
    let dx = max(0.0, max(-s, s - len));
    let d2 = dx * dx + uv.y * uv.y;
    let head = 0.35 + 0.65 * clamp(s / max(len, 0.01), 0.0, 1.0);
    return col * exp(-d2 * 3.0) * head;
}

fn hot_spot(uv: vec2<f32>, col: vec3<f32>) -> vec3<f32> {
    let r2 = dot(uv, uv);
    return col * (exp(-r2 * 9.0) + 0.25 * exp(-r2 * 2.5)) * clamp(1.0 - r2, 0.0, 1.0);
}

// The black-body ramp of cooling metal, heat 1 white-hot to 0 dark.
fn heat_ramp(h: f32) -> vec3<f32> {
    var c = mix(vec3<f32>(0.35, 0.03, 0.0), vec3<f32>(1.0, 0.28, 0.04), clamp(h * 3.0, 0.0, 1.0));
    c = mix(c, vec3<f32>(1.0, 0.72, 0.30), clamp(h * 3.0 - 1.0, 0.0, 1.0));
    c = mix(c, vec3<f32>(1.0, 0.97, 0.90), clamp(h * 3.0 - 2.0, 0.0, 1.0));
    return c * h;
}

fn seam(uv: vec2<f32>, rate: f32, tint: vec3<f32>) -> vec3<f32> {
    let h = exp(-(1.0 - clamp(uv.x, 0.0, 1.0)) * rate);
    let across = exp(-uv.y * uv.y * 4.0);
    return heat_ramp(h) * tint * across;
}

// A bee's welding beam: a white core in a glow of the colour, crackling along it.
fn weld_beam(uv: vec2<f32>, ph: f32, col: vec3<f32>) -> vec3<f32> {
    let x = uv.x * 12.0;
    let n = 0.55 + 0.25 * sin(x * 1.7 + ph * 9.0) + 0.2 * sin(x * 4.3 - ph * 23.0);
    let y = abs(uv.y + 0.25 * sin(x * 2.9 + ph * 31.0) * (1.0 - abs(uv.y)));
    let core = exp(-y * y * 40.0);
    let glow = exp(-y * y * 4.0);
    let ends = clamp(uv.x * 20.0, 0.0, 1.0) * clamp((1.0 - uv.x) * 6.0 + 0.4, 0.0, 1.0);
    return (vec3<f32>(1.0) * core * 1.5 + col * glow * n) * ends;
}

// The cloak's bands (ART-DIRECTION.md, "Cloaking"): regular bands travelling steadily
// in one direction across the ship, a thin line along each crest.
fn cloak_front(uv: vec2<f32>, col: vec3<f32>, p: vec4<f32>) -> vec3<f32> {
    let r = length(uv);
    let disc = 1.0 - smoothstep(0.7, 1.0, r);
    let s = uv.x * cos(p.z) + uv.y * sin(p.z);
    let band = 0.5 + 0.5 * sin(2.0 * PI * (s * p.x * 0.5 - k.a.z * p.y));
    return col * pow(band, 20.0) * disc;
}

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    let kind = k.a.x;
    let len = k.a.w;
    let q = in.uv * 2.0 - 1.0;
    var c = vec3<f32>(0.0);
    if kind < 0.5 {
        c = flare(q, 1.0, k.col.rgb, k.col2.rgb, k.p3, k.p4);
    } else if kind < 2.5 {
        c = beam(vec2<f32>(-1.0 + (len + 2.0) * in.uv.x, q.y), len, k.col.rgb, k.col2.rgb, k.col.w, k.p3, k.p4);
    } else if kind < 3.5 {
        c = bolt(vec2<f32>(-len + (1.0 + len) * in.uv.x, q.y), len, k.col.rgb, k.col2.rgb, k.p3);
    } else if kind < 6.5 {
        c = cone(vec2<f32>(k.col2.x * in.uv.x, q.y), len, k.col.rgb, k.col2.x, k.col2.w, k.col.w, k.p3, k.p4);
    } else if kind < 7.5 {
        c = streak_spark(vec2<f32>(in.uv.x, q.y), len, k.col.rgb);
    } else if kind < 8.5 {
        c = hot_spot(q, k.col.rgb);
    } else if kind < 9.5 {
        c = seam(vec2<f32>(in.uv.x, q.y), len, k.col.rgb);
    } else if kind < 10.5 {
        c = weld_beam(vec2<f32>(in.uv.x, q.y), k.col.w, k.col.rgb);
    } else if kind < 11.5 {
        c = weld_flare(q, k.p4.x, k.col.rgb, k.p3);
    } else {
        c = cloak_front(q, k.col.rgb, k.p3);
    }
    return vec4<f32>(c * k.a.y, 0.0);
}
