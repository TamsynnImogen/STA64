//! SOD particle emitters (type-12 nodes naming an `@emitter` of the sprite tables).
//!
//! Rules follow Armada II Refit's reading of the engine
//! (`render3d/src/mods/particles/`, MIT; `spec-particles.md` there):
//!
//! - **Spray**: at most one particle per frame; a timer counts down by the frame
//!   time and fires when it drops below zero. The particle flies along the node's
//!   +Z, tilted by a random pitch and yaw within the spread, at `velocity` units per
//!   second for `lifespan` seconds.
//! - **Cloud**: the owner's list is kept filled to `particle_count`. Particles start
//!   in a cube of half-width `velocity * cloud_radius` around the node with a random
//!   age, drift randomly, and die when old or when they leave `cloud_radius`.
//! - **Beam**: dead in stock, so never made.
//! - Drawing: a camera-facing quad of half size `size + expansion_rate * age`. The
//!   sprite node's colour animation plays on the particle's age; its draw flag is
//!   ignored. The colour is multiplied by 1 + the owner's emitters shown.
//!
//! Time comes from [`AnimClock`]: frames are cut into fixed
//! [`AnimClock::SUBSTEP`]s, and each emitter draws from its own sequence seeded
//! from the clock's seed and its owner's sim id, so a bench run repeats exactly.
//! Particles are a visual effect. They live outside the deterministic simulation.

use crate::clock::AnimClock;
use crate::render::sprites::SpriteTables;
use crate::sim_link::SimEntity;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use sta_assets::a2fmt::spr::{EmitterType, SpriteTable};
use sta_assets::spr_anim::{self, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// The engine's `RandomSequence`, our own instance.
struct Rng(u32);

impl Rng {
    fn draw(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(2273).wrapping_add(31) % 429_397_397;
        self.0 % 100_001
    }
    /// -1..1
    fn signed(&mut self) -> f32 {
        (self.draw() as f32 - 50000.0) * 2e-5
    }
    /// 0..m
    fn up_to(&mut self, m: f32) -> f32 {
        self.draw() as f32 * m * 1e-5
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Blend {
    Additive,
    Translucent,
    Cutout,
}

/// An `@emitter` resolved with its sprite node's look.
pub struct Template {
    kind: EmitterType,
    speed: f32,
    growth: f32,
    life: f32,
    period: f32,
    spread: f32,
    count: i32,
    radius: f32,
    texture: String,
    uv: [f32; 4],
    blend: Blend,
    size: [f32; 2],
    colour: [f32; 3],
    anim: Option<String>,
    sprite_anims: Vec<String>,
}

impl Template {
    pub fn new(table: &SpriteTable, emitter: &str) -> Option<Template> {
        let e = table.emitter(emitter)?;
        if e.emitter_type == EmitterType::Beam {
            return None;
        }
        // No sprite node, no emitter (the engine warns and creates none).
        let nd = table.sprite_node(e.sprite.as_deref()?)?;
        let s = table.sprite(&nd.sprite)?;
        let rw = s.reference[0].filter(|r| *r > 0.0).unwrap_or(128.0);
        let rh = s.reference[1].filter(|r| *r > 0.0).unwrap_or(rw);
        let blend = match s.material.to_ascii_lowercase().as_str() {
            "additive" | "add_nomipf" => Blend::Additive,
            "translucent" => Blend::Translucent,
            _ => Blend::Cutout,
        };
        Some(Template {
            kind: e.emitter_type,
            speed: e.velocity,
            growth: e.expansion_rate,
            life: e.lifespan,
            period: e.firing_period,
            spread: e.spread_angle.to_radians(),
            count: e.particle_count,
            radius: e.cloud_radius,
            texture: s.texture.clone(),
            uv: [s.rect[0] / rw, s.rect[1] / rh, s.rect[2] / rw, s.rect[3] / rh],
            blend,
            size: nd.size,
            colour: nd.colour,
            anim: nd.animation.clone(),
            sprite_anims: s.anims.clone(),
        })
    }
}

/// An emitter node. `owner` groups emitters whose particles share one list.
#[derive(Component)]
pub struct Emitter {
    pub tpl: Arc<Template>,
    pub owner: Entity,
    pub timer: f32,
    /// This emitter's random sequence; seeded by [`simulate`] on first sight.
    pub rng: EmitterRng,
}

/// An emitter's own [`Rng`], empty until seeded.
#[derive(Default)]
pub struct EmitterRng(Option<Rng>);

struct Particle {
    owner: Entity,
    tpl: Arc<Template>,
    pos: Vec3,
    /// Spray: unit flight axis. Cloud: drift per second.
    vel: Vec3,
    centre: Vec3,
    age: f32,
}

#[derive(Resource)]
pub struct Particles {
    /// Frame time not yet run in fixed sub-steps.
    acc: f64,
    /// Emitters seeded so far per owner, for telling an owner's emitters apart.
    ordinals: HashMap<Entity, u32>,
    parts: Vec<Particle>,
    /// One dynamic mesh per (texture, blend).
    batches: HashMap<(String, Blend), (Handle<Mesh>, Entity)>,
    /// Cloud steps per second (stock steps cloud particles once per frame).
    cloud_fps: f32,
}

impl Default for Particles {
    fn default() -> Self {
        Particles {
            acc: 0.0,
            ordinals: HashMap::new(),
            parts: Vec::new(),
            batches: HashMap::new(),
            cloud_fps: 30.0,
        }
    }
}

impl Particles {
    pub fn clear(&mut self) {
        self.parts.clear();
        self.acc = 0.0;
        self.ordinals.clear();
    }
}

/// Fire emitters and age particles.
pub fn simulate(
    clock: Res<AnimClock>,
    mut sys: ResMut<Particles>,
    mut emitters: Query<(&mut Emitter, &GlobalTransform, &InheritedVisibility)>,
    sims: Query<&SimEntity>,
) {
    for (mut em, _, _) in &mut emitters {
        if em.rng.0.is_none() {
            let n = sys.ordinals.entry(em.owner).or_default();
            let ordinal = *n;
            *n += 1;
            let id = sims.get(em.owner).map(|s| s.0).unwrap_or(sta_sim::EntityId(0));
            let key = clock.entity_key(id) ^ (ordinal as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            em.rng.0 = Some(Rng((key % 429_397_397) as u32));
        }
    }
    sys.acc += (clock.frame_dt() as f64).min(0.25);
    while sys.acc >= AnimClock::SUBSTEP {
        sys.acc -= AnimClock::SUBSTEP;
        substep(&mut sys, AnimClock::SUBSTEP as f32, &mut emitters);
    }
}

fn substep(sys: &mut Particles, dt: f32, emitters: &mut Query<(&mut Emitter, &GlobalTransform, &InheritedVisibility)>) {
    let fps = sys.cloud_fps;
    // Age and move; the dead go.
    sys.parts.retain_mut(|p| {
        let t = &p.tpl;
        p.age += dt;
        match t.kind {
            EmitterType::Cloud => {
                p.pos += p.vel * dt;
                p.age < t.life && (p.pos - p.centre).length() <= t.radius
            }
            _ => {
                p.pos += p.vel * (t.speed * dt);
                p.age < t.life
            }
        }
    });
    let mut counts: HashMap<Entity, i32> = HashMap::new();
    for p in &sys.parts {
        *counts.entry(p.owner).or_default() += 1;
    }
    for (mut em, gt, vis) in emitters.iter_mut() {
        if !vis.get() {
            continue;
        }
        let tpl = em.tpl.clone();
        let Some(mut rng) = em.rng.0.take() else { continue };
        let at = gt.translation();
        // Node axes in world space. Our transforms are Z-mirrored SOD transforms,
        // so the SOD node's +Z is the Bevy node's -Z.
        let (rx, ry, rz) = (
            gt.transform_vector(Vec3::X).normalize_or(Vec3::X),
            gt.transform_vector(Vec3::Y).normalize_or(Vec3::Y),
            gt.transform_vector(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z),
        );
        match tpl.kind {
            EmitterType::Spray => {
                em.timer -= dt;
                if em.timer < 0.0 {
                    em.timer = if dt < tpl.period { em.timer + tpl.period } else { tpl.period };
                    let d = if tpl.spread > 0.0 {
                        let a = tpl.spread * rng.signed();
                        let b = tpl.spread * rng.signed();
                        let _ = rng.signed(); // the engine draws a third angle it never uses
                        rx * (a.cos() * b.sin()) - ry * a.sin() + rz * (a.cos() * b.cos())
                    } else {
                        rz
                    };
                    let owner = em.owner;
                    sys.parts.push(Particle {
                        owner,
                        tpl: tpl.clone(),
                        pos: at,
                        vel: d.normalize_or(rz),
                        centre: at,
                        age: 0.0,
                    });
                }
            }
            EmitterType::Cloud => {
                let n = counts.entry(em.owner).or_default();
                let mut guard = 0;
                while *n < tpl.count && guard < 4096 {
                    guard += 1;
                    *n += 1;
                    let r = &mut rng;
                    let v = Vec3::new(r.signed(), r.signed(), r.signed()) * tpl.speed;
                    let o = Vec3::new(r.signed(), r.signed(), r.signed()) * (tpl.speed * tpl.radius);
                    let age = r.up_to(tpl.life);
                    let owner = em.owner;
                    sys.parts.push(Particle {
                        owner,
                        tpl: tpl.clone(),
                        pos: at + o,
                        vel: Vec3::new(v.z, v.y, v.x) * fps,
                        centre: at,
                        age,
                    });
                }
            }
            EmitterType::Beam => {}
        }
        em.rng.0 = Some(rng);
    }
}

fn linear(c: [f32; 3], mult: f32) -> [f32; 4] {
    let l = LinearRgba::from(Color::srgb(c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)));
    [l.red * mult, l.green * mult, l.blue * mult, 1.0]
}

/// Rebuild the particle quads, one mesh per texture and blend mode.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut commands: Commands,
    mut sys: ResMut<Particles>,
    tables: Res<SpriteTables>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    emitters: Query<(&Emitter, &InheritedVisibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut loader: crate::render::textures::LoaderParams,
) {
    let Ok(cam) = camera.single() else { return };
    let (right, up) = (cam.right().as_vec3(), cam.up().as_vec3());
    let table = &tables.0;

    // 1 + the owner's emitters shown (stock submits a list once per emitter).
    let mut mult: HashMap<Entity, f32> = HashMap::new();
    for (e, vis) in &emitters {
        if vis.get() {
            *mult.entry(e.owner).or_insert(1.0) += 1.0;
        }
    }

    #[derive(Default)]
    struct Geo {
        pos: Vec<[f32; 3]>,
        uv: Vec<[f32; 2]>,
        col: Vec<[f32; 4]>,
        idx: Vec<u32>,
    }
    let mut geo: HashMap<(String, Blend), Geo> = HashMap::new();
    for p in &sys.parts {
        let t = &p.tpl;
        let hx = t.size[0] + t.growth * p.age;
        let hy = t.size[1] + t.growth * p.age;
        if hx <= 0.0 || hy <= 0.0 {
            continue;
        }
        let mut c = t.colour;
        if let Some(Value::Colour(v)) = t.anim.as_deref().and_then(|a| table.animation(a)).and_then(|a| spr_anim::value(a, p.age, spr_anim::LOOP)) {
            c = v;
        }
        let mut off = [0.0f32; 2];
        for id in &t.sprite_anims {
            if let Some(a) = table.animation(id) {
                let o = spr_anim::offset_of(spr_anim::value(a, p.age, spr_anim::LOOP));
                off[0] += o[0];
                off[1] += o[1];
            }
        }
        let (u0, v0) = (t.uv[0] + off[0], t.uv[1] + off[1]);
        let (u1, v1) = (u0 + t.uv[2], v0 + t.uv[3]);
        let colour = linear(c, mult.get(&p.owner).copied().unwrap_or(1.0));
        let g = geo.entry((t.texture.to_ascii_lowercase(), t.blend)).or_default();
        let base = g.pos.len() as u32;
        let (r, u) = (right * hx, up * hy);
        for (corner, uv) in [
            (p.pos - r + u, [u0, v0]),
            (p.pos + r + u, [u1, v0]),
            (p.pos + r - u, [u1, v1]),
            (p.pos - r - u, [u0, v1]),
        ] {
            g.pos.push(corner.to_array());
            g.uv.push(uv);
            g.col.push(colour);
        }
        g.idx.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }

    // Empty out batches with no particles this frame.
    let keys: Vec<_> = sys.batches.keys().cloned().collect();
    for k in keys {
        geo.entry(k).or_default();
    }
    for (key, g) in geo {
        let handle = match sys.batches.get(&key) {
            Some((h, _)) => h.clone(),
            None => {
                let texture = loader.loader().colour(&key.0, false);
                let material = materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: texture,
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    alpha_mode: match key.1 {
                        Blend::Additive => AlphaMode::Add,
                        Blend::Translucent => AlphaMode::Blend,
                        Blend::Cutout => AlphaMode::Mask(0.5),
                    },
                    ..default()
                });
                let h = meshes.add(Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()));
                let e = commands
                    .spawn((Mesh3d(h.clone()), MeshMaterial3d(material), Transform::IDENTITY, bevy::camera::visibility::NoFrustumCulling, Visibility::Hidden))
                    .id();
                sys.batches.insert(key.clone(), (h.clone(), e));
                h
            }
        };
        let empty = g.idx.is_empty();
        if let Some((_, e)) = sys.batches.get(&key) {
            commands.entity(*e).insert(if empty { Visibility::Hidden } else { Visibility::Inherited });
        }
        if empty {
            continue;
        }
        if let Some(mut m) = meshes.get_mut(&handle) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, g.pos);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, g.uv);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, g.col);
            m.insert_indices(Indices::U32(g.idx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ClockMode;
    use crate::render::model_scene::{self, AnimKeys, NodeAnim};

    fn template(kind: EmitterType) -> Arc<Template> {
        Arc::new(Template {
            kind,
            speed: 3.0,
            growth: 0.0,
            life: 2.0,
            period: 0.05,
            spread: 0.4,
            count: 12,
            radius: 5.0,
            texture: String::new(),
            uv: [0.0, 0.0, 1.0, 1.0],
            blend: Blend::Additive,
            size: [1.0, 1.0],
            colour: [1.0; 3],
            anim: None,
            sprite_anims: Vec::new(),
        })
    }

    /// Run `frames` bench frames of an app with one spray and one cloud emitter
    /// plus a node animation; return every value that would reach the screen.
    fn run(frames: usize, step: f64) -> Vec<f32> {
        let mut app = App::new();
        app.insert_resource(AnimClock::new(ClockMode::Bench { step }, 99))
            .init_resource::<Particles>()
            .add_systems(Update, (simulate, model_scene::animate).chain());
        let owner = app.world_mut().spawn_empty().id();
        for (i, kind) in [EmitterType::Spray, EmitterType::Cloud].into_iter().enumerate() {
            app.world_mut().spawn((
                Emitter { tpl: template(kind), owner, timer: 0.0, rng: Default::default() },
                GlobalTransform::from(Transform::from_xyz(i as f32, 0.0, 0.0)),
                InheritedVisibility::VISIBLE,
            ));
        }
        let node = app
            .world_mut()
            .spawn((
                NodeAnim { period: 1.0, base: Transform::IDENTITY, keys: AnimKeys::Scale(vec![1.0, 2.0, 3.0, 4.0]) },
                Transform::IDENTITY,
            ))
            .id();
        let mut out = Vec::new();
        for _ in 0..frames {
            app.world_mut().resource_mut::<AnimClock>().advance_idle(0.123); // ignored in bench mode
            app.update();
            out.push(app.world().get::<Transform>(node).unwrap().scale.x);
        }
        let sys = app.world().resource::<Particles>();
        assert!(!sys.parts.is_empty());
        out.extend(sys.parts.iter().flat_map(|p| [p.pos.x, p.pos.y, p.pos.z, p.age]));
        out
    }

    #[test]
    fn same_bench_frames_give_identical_animation() {
        let a = run(240, 1.0 / 60.0);
        assert_eq!(a, run(240, 1.0 / 60.0));
        // The run is live: a different frame step gives a different result.
        assert_ne!(a, run(240, 1.0 / 50.0));
    }
}
