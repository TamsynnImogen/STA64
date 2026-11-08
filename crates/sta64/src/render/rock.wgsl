// Rocks: the asteroids' and the dilithium moons' meshes with a recipe surface.
//
// Ported from Armada II Refit's render3d/shaders/world_rock.hlsl (MIT, Copyright (c)
// 2026 Cedric Haase), itself Lighting.asi's moon_ps: colour, relief and cut-out from
// cube maps in the direction from the mesh's centre, or triplanar across its extents
// (asteroids); lit with diffuse, a highlight by the texture's brightness and a rim.
// The moons' cracks show the light inside them. Not ported yet: the cracks' ravine
// parallax.

struct RockParams {
    amb: vec4<f32>,       // ambient rgb
    shine: vec4<f32>,     // x specular strength, y its exponent, z rim exponent, w the key's strength
    eye: vec4<f32>,       // the camera, world
    rim: vec4<f32>,       // the rim light's colour
    ddir0: vec4<f32>,     // the key: the way its light travels
    dcol0: vec4<f32>,
    ddir1: vec4<f32>,     // the fill
    dcol1: vec4<f32>,
    row0: vec4<f32>,      // the mesh's object axes in world space (with scale)
    row1: vec4<f32>,
    row2: vec4<f32>,
    ctr: vec4<f32>,       // the mesh's centre, world
    glow: vec4<f32>,      // rgb: the glow's colour
    misc: vec4<f32>,      // x relief strength, z 1 when the albedo's alpha cuts, w 1 for triplanar
    ext: vec4<f32>,       // xyz 1 / the mesh's extent per object axis; w the highlight's strength
    core: vec4<f32>,      // rgb the light inside the cracks (0: holes); w the cut's level
    spill: vec4<f32>,     // x the glow's scale; y the white at a crack's core
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> k: RockParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var m_albedo: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var m_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var m_relief: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var m_relief_sampler: sampler;

struct FragIn {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
};

// The cubes hold the files' left-handed object space; Bevy's is mirrored on Z.
fn mirror(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(v.x, v.y, -v.z);
}

fn to_obj(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(v, k.row0.xyz), dot(v, k.row1.xyz), dot(v, k.row2.xyz));
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
    let s2 = max(dot(k.row0.xyz, k.row0.xyz), 1e-12);
    // Object space over the extents: each axis -1..1 across the mesh (file space).
    let q = mirror(to_obj(wp - k.ctr.xyz) / s2) * k.ext.xyz;
    let no = mirror(to_obj(normalize(i.world_normal)));
    let cq = clamp(q, vec3<f32>(-1.0), vec3<f32>(1.0));
    let sd = select(vec3<f32>(-1.0), vec3<f32>(1.0), no >= vec3<f32>(0.0));
    let cx = vec3<f32>(sd.x, cq.y, cq.z);
    let cy = vec3<f32>(cq.x, sd.y, cq.z);
    let cz = vec3<f32>(cq.x, cq.y, sd.z);
    let dq = normalize(q);
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
    let pw = (pf.x * k.row0.xyz + pf.y * k.row1.xyz + pf.z * k.row2.xyz) * inverseSqrt(s2);
    let n = normalize(normalize(i.world_normal) + pw);
    var c = shade(n, wp, a, k.ext.w);
    c += k.glow.rgb * b.a * k.spill.x;
    let hot = saturate((b.a - k.core.w) / max(k.core.w, 1e-3));
    let core = k.core.rgb * (1.0 + hot) + dot(k.core.rgb, vec3<f32>(0.333)) * hot * hot * k.spill.y;
    c = mix(c, core, (1.0 - a.a) * inside);
    return vec4<f32>(c, 1.0);
}
