// Rocks: the asteroids' and the dilithium moons' meshes with a recipe surface.
//
// Ported from Armada II Refit's render3d/shaders/world_rock.hlsl (MIT, Copyright (c)
// 2026 Cedric Haase), itself Lighting.asi's moon_ps: colour, relief and cut-out from
// cube maps in the direction from the mesh's centre, or triplanar across its extents
// (asteroids); lit with diffuse, a highlight by the texture's brightness and a rim.
// The moons' cracks show the light inside them, at the bottom of a ravine: the view's
// ray is marched down into it (parallax), so its walls hide the light at an angle.

import bevy_pbr::render::mesh_functions::get_world_from_local;

struct RockParams {
    amb: vec4<f32>,       // ambient rgb
    shine: vec4<f32>,     // x specular strength, y its exponent, z rim exponent, w the key's strength
    eye: vec4<f32>,       // the camera, world
    rim: vec4<f32>,       // the rim light's colour
    ddir0: vec4<f32>,     // the key: the way its light travels
    dcol0: vec4<f32>,
    ddir1: vec4<f32>,     // the fill
    dcol1: vec4<f32>,
    row0: vec4<f32>,      // (unused: the instance's axes come from its transform, see `fragment`)
    row1: vec4<f32>,
    row2: vec4<f32>,
    ctr: vec4<f32>,       // the mesh's centre, in the mesh's own space
    glow: vec4<f32>,      // rgb: the glow's colour
    misc: vec4<f32>,      // x relief strength, z 1 when the albedo's alpha cuts, w 1 for triplanar
    ext: vec4<f32>,       // xyz 1 / the mesh's extent per object axis; w the highlight's strength
    core: vec4<f32>,      // rgb the light inside the cracks (0: holes); w the cut's level
    spill: vec4<f32>,     // x the glow's scale; y the white at a crack's core, z the ravine's depth (0: flat), w the light's falloff up the walls
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> k: RockParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var m_albedo: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var m_albedo_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var m_relief: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var m_relief_sampler: sampler;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(6) @interpolate(flat) instance_index: u32,
};

// The instance's object axes in world space (with scale), and its mesh's centre in world
// space: from the mesh's own transform, so every rock of a mesh shares one material (and
// one draw) and nothing is rewritten when a rock tumbles.
var<private> row0: vec3<f32>;
var<private> row1: vec3<f32>;
var<private> row2: vec3<f32>;
var<private> ctr: vec3<f32>;

// The cubes hold the files' left-handed object space; Bevy's is mirrored on Z.
fn mirror(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(v.x, v.y, -v.z);
}

fn to_obj(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(v, row0), dot(v, row1), dot(v, row2));
}

fn glint(n: vec3<f32>, l: vec3<f32>, v: vec3<f32>) -> f32 {
    let h = normalize(l + v);
    if dot(n, l) > 0.0 {
        return pow(max(0.0, dot(n, h)), k.shine.y);
    }
    return 0.0;
}

fn shade(n: vec3<f32>, wp: vec3<f32>, t: vec4<f32>, gk: f32) -> vec3<f32> {
    let v = normalize(k.eye.xyz - wp);
    var sum = k.amb.rgb;
    var spec = vec3<f32>(0.0);
    let l0 = -k.ddir0.xyz;
    let c0 = k.dcol0.rgb * k.shine.w;
    sum += c0 * max(0.0, dot(n, l0));
    spec += c0 * glint(n, l0, v);
    let l1 = -k.ddir1.xyz;
    sum += k.dcol1.rgb * max(0.0, dot(n, l1));
    spec += k.dcol1.rgb * glint(n, l1, v);
    sum += k.rim.rgb * pow(1.0 - saturate(dot(n, v)), k.shine.z);
    let gloss = dot(t.rgb, vec3<f32>(0.299, 0.587, 0.114)) * gk;
    return t.rgb * max(sum, vec3<f32>(0.0)) + spec * k.shine.x * gloss;
}

@fragment
fn fragment(i: FragIn) -> @location(0) vec4<f32> {
    let wp = i.world_position.xyz;
    let m = get_world_from_local(i.instance_index);
    row0 = m[0].xyz;
    row1 = m[1].xyz;
    row2 = m[2].xyz;
    ctr = (m * vec4<f32>(k.ctr.xyz, 1.0)).xyz;
    let s2 = max(dot(row0, row0), 1e-12);
    // Object space over the extents: each axis -1..1 across the mesh (file space).
    let q = mirror(to_obj(wp - ctr) / s2) * k.ext.xyz;
    let no = mirror(to_obj(normalize(i.world_normal)));
    let cq = clamp(q, vec3<f32>(-1.0), vec3<f32>(1.0));
    let sd = select(vec3<f32>(-1.0), vec3<f32>(1.0), no >= vec3<f32>(0.0));
    let cx = vec3<f32>(sd.x, cq.y, cq.z);
    let cy = vec3<f32>(cq.x, sd.y, cq.z);
    let cz = vec3<f32>(cq.x, cq.y, sd.z);
    let dq = normalize(q);
    // The ravine's view direction and the gradients its taps need, in uniform control
    // flow. A tap at q - vq * t has the gradient ddx(q) - ddx(vq) * t.
    let far_k = saturate((k.spill.z / max(length(fwidth(q)), 1e-6) - 2.0) / 3.0);
    var vq = mirror(to_obj(k.eye.xyz - wp)) * k.ext.xyz;
    vq /= max(length(vq), 1e-12);
    let qx = dpdx(q);
    let qy = dpdy(q);
    let vx = dpdx(vq);
    let vy = dpdy(vq);
    // All lookups up front (uniform control flow for the derivatives).
    let ax = textureSample(m_albedo, m_albedo_sampler, cx);
    let bx = textureSample(m_relief, m_relief_sampler, cx);
    let ay = textureSample(m_albedo, m_albedo_sampler, cy);
    let by = textureSample(m_relief, m_relief_sampler, cy);
    let az = textureSample(m_albedo, m_albedo_sampler, cz);
    let bz = textureSample(m_relief, m_relief_sampler, cz);
    let ad = textureSample(m_albedo, m_albedo_sampler, dq);
    let bd = textureSample(m_relief, m_relief_sampler, dq);
    var a: vec4<f32>;
    var b: vec4<f32>;
    var p: vec3<f32>;
    if k.misc.w > 0.5 {
        // Triplanar (asteroids): blended by how squarely the surface faces each axis.
        var w = pow(abs(no) / max(length(no), 1e-6), vec3<f32>(4.0));
        w /= max(w.x + w.y + w.z, 1e-6);
        a = ax * w.x + ay * w.y + az * w.z;
        b = bx * w.x + by * w.y + bz * w.z;
        p = (bx.rgb - 0.5) * vec3<f32>(0.0, 1.0, 1.0) * w.x
          + (by.rgb - 0.5) * vec3<f32>(1.0, 0.0, 1.0) * w.y
          + (bz.rgb - 0.5) * vec3<f32>(1.0, 1.0, 0.0) * w.z;
    } else {
        a = ad;
        b = bd;
        p = bd.rgb - 0.5;
    }
    a.a = mix(1.0, a.a, k.misc.z);
    // Cracks are holes, or with a light inside (the moons) that light.
    let inside = select(0.0, 1.0, k.core.w > 0.0);
    if max(a.a, inside) < 0.5 {
        discard;
    }
    p *= k.misc.x;
    // To world space without the object's scale.
    let pf = mirror(p);
    let pw = (pf.x * row0 + pf.y * row1 + pf.z * row2) * inverseSqrt(s2);
    let n = normalize(normalize(i.world_normal) + pw);
    var c = shade(n, wp, a, k.ext.w);
    c += k.glow.rgb * b.a * k.spill.x;
    var hot = saturate((b.a - k.core.w) / max(k.core.w, 1e-3));
    var lit = 1.0;
    var wall = c;
    if k.spill.z > 0.0 && k.misc.w < 0.5 {
        // A ravine with the light at its bottom: it opens where the cut does and is
        // deepest at the fissure's core, the walls steep. The view's ray goes down into
        // it until it meets a wall; how deep that is says how much light reaches it, so
        // the walls are dark rock, the light lies at the bottom, and at an angle the near
        // wall hides part of it. Fixed steps, branch-free.
        let dd0 = k.spill.z;
        let lip = k.core.w - 0.06;
        let top = max(2.0 * k.core.w - lip, 1e-3);
        let h0 = dd0 * sqrt(saturate((b.a - lip) / top));
        var hit = h0;
        var found = 0.0;
        var tp = 0.0;
        var dp = h0;
        for (var s = 1; s <= 8; s++) {
            let t = dd0 * f32(s) / 8.0;
            let g = textureSampleGrad(m_relief, m_relief_sampler, q - vq * t, qx - vx * t, qy - vy * t).a;
            let dd = dd0 * sqrt(saturate((g - lip) / top)) - t;
            let f = (1.0 - found) * step(dd, 0.0);
            let x = mix(tp, t, saturate(dp / max(dp - dd, 1e-6)));
            hit = mix(hit, x, f);
            found = max(found, f);
            tp = t;
            dp = dd;
        }
        hit = mix(dd0, hit, found);
        let e = saturate(hit / dd0);
        lit = mix(1.0, pow(e, k.spill.w), far_k);
        hot = mix(hot, e, far_k);
        wall = c * mix(0.5, 0.05, saturate(e * 2.0));
    }
    var core = k.core.rgb * (1.0 + hot) + dot(k.core.rgb, vec3<f32>(0.333)) * hot * hot * k.spill.y;
    core = mix(wall, core, lit);
    c = mix(c, core, (1.0 - a.a) * inside);
    return vec4<f32>(c, 1.0);
}
