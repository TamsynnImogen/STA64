//! Armada II's additive warp sprite and cruise hull stretch. The entry flash's timing and projection follow the
//! original's rule (read from the executable, static): a point ahead of the craft on its velocity line, at the ramp's
//! length times the warp speed, drawn smaller the nearer the craft comes and removed once it has passed.
//!
//! The Stock look ([`Effect::Warp`], #1154) is what the notes establish of the original (read from the executable,
//! static; not seen in a frame, nothing measured about the view): the entry flash stays (it is the original's, in both
//! looks); **no exit pulse** (the original draws no flash on leaving, nor a trail or glow); the hull's stretch follows
//! the stored ramp the view's warp row carries: from 1 to the stretch factor over the entry ramp, held while at warp,
//! and back to 1 over the exit ramp, in ticks of the sim clock, not the craft's speed (under `warp_shape` the speed holds
//! and snaps, so it no longer says where the stretch is). INTERIM: the original sweeps a deformation plane through the
//! mesh, so the shape between the ends differs from a uniform stretch; both ends are the original's. INTERIM: on a
//! capped exit the original's factor is lower (inferred, not seen); the sim's factor for the craft's own speeds is used. Without the switch the row's ramp is a hint
//! derived from the old smooth ramp and is used the same way. Not built: the selection bubble and the shield caps
//! stretching with the hull, and the sounds at entry and exit (established, not yet drawn or played). The Remaster look
//! keeps its own (the exit pulse, the speed-driven stretch). Armada I has none of this: no hull part is attached to its
//! craft and its sprite table has no warp sprite.

use std::collections::BTreeSet;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_assets::spr_anim;
use sta_sim::{EntityId, Tick, UnitView, view::{UnitFlags, WarpPhase, WarpRow}};

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::map::MainCamera;
use crate::render::profile::{Effect, ProfileEffects, RenderProfile};
use crate::render::sprites::{self, SpriteTables};
use crate::render::textures::{Loader, TextureCache};
use crate::render::units::{LocalPlayer, cinematic_reveal, drawn_units, to_bevy};
use crate::sim_link::{ContentRes, SimEntity, SimLink};
use crate::ui::presentation::CineractiveState;
use crate::viewer::Mounted;

const SPRITE: &str = "warp_sprite";
const FLASH_RADII: f32 = 2.5;
const EXIT_PULSE_SECONDS: f64 = 0.45;

#[derive(Component)]
struct WarpFlash {
    ship: EntityId,
    anchor: Vec3,
    reach: f32,
    radius: f32,
    exit_started: Option<Tick>,
}

#[derive(Default)]
struct WarpHistory {
    tick: Option<Tick>,
    in_warp: BTreeSet<EntityId>,
}

/// Keeps the stretch on the ship model, off the selection and simulation root.
#[derive(Component)]
pub(crate) struct WarpHull {
    pub pivot_z: f32,
}

/// Whether the Stock warp look is the one drawn: under the Stock look with a stock implementation registered. Without
/// the profile resources (windowless tests of other effects) it is not.
pub(crate) fn stock_warp(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    match (profile, effects) {
        (Some(p), Some(e)) => e.implementation(*p, Effect::Warp) == RenderProfile::Stock,
        _ => false,
    }
}

/// How far the hull's stretch has come (0 none, 1 full) `now` ticks of the sim clock, from the craft's warp row: up over
/// the entry ramp, full while at warp, down over the exit ramp. A row with no ramp (an old snapshot) is at its end.
fn ramp_progress(row: &WarpRow, now: f64) -> f32 {
    let u = if row.ramp_len == 0 { 1.0 } else { ((now - row.ramp_start as f64) / row.ramp_len as f64).clamp(0.0, 1.0) as f32 };
    match row.phase {
        WarpPhase::Entering => u,
        WarpPhase::AtWarp => 1.0,
        WarpPhase::Leaving => 1.0 - u,
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<ProfileEffects>();
    app.world_mut().resource_mut::<ProfileEffects>().register_stock(Effect::Warp);
    app.add_systems(
        Update,
        (track, place, attach).chain().in_set(GameSet::Interpolate),
    )
    .add_systems(Update, stretch_hulls.in_set(GameSet::Render));
}

fn warp_speeds(link: &SimLink, unit: &UnitView, def: &sta_content::UnitDef) -> (f32, f32) {
    let impulse = link.impulse_cap(unit.id).unwrap_or(def.movement.max_speed);
    let rows = &link.real_curr().attrs.warp;
    let warp = rows
        .binary_search_by_key(&unit.id, |(id, _)| *id)
        .ok()
        .map_or(def.movement.warp_speed, |index| rows[index].1);
    (impulse.to_f32(), warp.to_f32())
}

/// The craft's stretch factor at full warp from its warp row: the sim's own (one source, the one its ramp is made from).
fn row_stretch(row: &WarpRow) -> f32 {
    row.stretch.to_f32().max(1.0)
}

/// What the hull's stretch needs, in one parameter: the one source of a craft's stretch now, for the hull, the
/// selection bubble and the shield cap.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct WarpStretch<'w> {
    profile: Option<Res<'w, RenderProfile>>,
    effects: Option<Res<'w, ProfileEffects>>,
    link: Option<Res<'w, SimLink>>,
    content: Option<Res<'w, ContentRes>>,
    clock: Option<Res<'w, AnimClock>>,
}

impl WarpStretch<'_> {
    /// The frame's place between the two views (1 without a clock).
    fn alpha(&self) -> f32 {
        self.clock.as_deref().map_or(1.0, AnimClock::alpha)
    }

    /// The longitudinal stretch of craft `id` now (1 when it is not in warp): the Stock look's by the stored ramp, the
    /// other look's by speed.
    pub(crate) fn of(&self, id: EntityId) -> f32 {
        let (Some(link), Some(content)) = (self.link.as_deref(), self.content.as_deref()) else { return 1.0 };
        let stock = stock_warp(self.profile.as_deref(), self.effects.as_deref());
        (|| {
            link.warp_phase(id)?;
            let unit = link.real_curr().unit(id)?;
            let def = content.0.unit(unit.def)?;
            let row = link.real_curr().warp_of(id)?;
            let (impulse, warp) = warp_speeds(link, unit, def);
            if warp <= impulse || impulse < 0.0 {
                return None;
            }
            if stock {
                // The stored ramp, on the sim clock: the view shows tick `tick - 1` at alpha 0 and `tick` at 1.
                let now = link.real_curr().tick as f64 - 1.0 + f64::from(self.alpha());
                return Some(1.0 + (row_stretch(row) - 1.0) * ramp_progress(row, now));
            }
            let now = unit.speed.to_f32();
            let then = link.prev().unit(id).map_or(now, |previous| previous.speed.to_f32());
            let speed = then + (now - then) * self.alpha();
            // The original sweeps a deformation plane through the mesh. Use
            // the live speed ramp for the visual transform during the transition.
            let progress = ((speed - impulse) / (warp - impulse)).clamp(0.0, 1.0);
            Some(1.0 + (row_stretch(row) - 1.0) * progress)
        })()
        .unwrap_or(1.0)
    }

    /// The stretch the selection bubble and the shield cap take (read from the executable, static: they follow the
    /// hull's warp progress): the Stock look's, for the stock Armada II craft that carry the hull stretch; 1 otherwise.
    /// INTERIM: the same uniform stretch as the hull, about the unit's origin.
    pub(crate) fn of_stock(&self, id: EntityId) -> f32 {
        let stock_content = self.content.as_deref().is_some_and(|c| c.0.profile() == sta_content::Profile::Sta2Classic);
        if stock_content && stock_warp(self.profile.as_deref(), self.effects.as_deref()) { self.of(id) } else { 1.0 }
    }
}

fn stretch_hulls(warp: WarpStretch, ships: Query<&SimEntity>, mut models: Query<(&WarpHull, &ChildOf, &mut Transform)>) {
    for (model, parent, mut transform) in &mut models {
        let stretch = ships.get(parent.parent()).map_or(1.0, |id| warp.of(id.0));
        let scale = Vec3::new(1.0, 1.0, stretch);
        let translation = Vec3::Z * (model.pivot_z * (1.0 - stretch));
        if transform.scale != scale || transform.translation != translation {
            transform.scale = scale;
            transform.translation = translation;
        }
    }
}

/// The original multiplies its clamped transition time by the warp speed (read from the executable, static).
/// Uses the simulation's current speed limits, collision radius and stretch factor.
fn warp_reach(radius: f32, impulse: f32, warp: f32, speed: f32, stretch: f32) -> Option<f32> {
    if radius <= 0.0 || impulse < 0.0 || warp <= impulse {
        return None;
    }
    let acceleration_span = warp - speed.clamp(0.0, impulse);
    let duration = ((stretch - 1.0) * (2.0 * radius) / acceleration_span).clamp(0.1, 3.0);
    Some(duration * warp)
}

fn direction(link: &SimLink, unit: &UnitView) -> Vec3 {
    let forward =
        Vec3::new(unit.heading.0.to_f32(), 0.0, -unit.heading.1.to_f32()).normalize_or(Vec3::NEG_Z);
    link.interpolate(unit.id, 0.0)
        .map_or(forward, |(previous, _)| {
            (to_bevy(unit.pos.to_f32()) - to_bevy(previous)).normalize_or(forward)
        })
}

fn track(
    mut commands: Commands,
    profile: Option<Res<RenderProfile>>,
    effects: Option<Res<ProfileEffects>>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    mut history: Local<WarpHistory>,
    flashes: Query<(Entity, &WarpFlash)>,
) {
    let (Some(link), Some(content)) = (link, content) else {
        if history.tick.take().is_some() {
            history.in_warp.clear();
            for (entity, _) in &flashes {
                commands.entity(entity).try_despawn();
            }
        }
        return;
    };
    let stock = stock_warp(profile.as_deref(), effects.as_deref());
    let view = link.real_curr();
    let reset = link.is_added() || history.tick.is_none_or(|previous| previous > view.tick);
    if !reset && history.tick == Some(view.tick) {
        return;
    }
    history.tick = Some(view.tick);
    let in_warp: BTreeSet<_> = view
        .units
        .iter()
        .filter(|unit| link.warp_phase(unit.id) == Some(true))
        .map(|unit| unit.id)
        .collect();
    if reset {
        // Loading an already-warping ship must not create a fresh entry flash.
        for (entity, _) in &flashes {
            commands.entity(entity).try_despawn();
        }
    } else {
        for &id in in_warp.difference(&history.in_warp) {
            let Some(unit) = view.unit(id) else { continue };
            let Some(def) = content.0.unit(unit.def) else {
                continue;
            };
            let radius = def.radius.to_f32();
            let (impulse, warp) = warp_speeds(&link, unit, def);
            let speed = link.prev().unit(id).unwrap_or(unit).speed.to_f32();
            let Some(row) = view.warp_of(id) else { continue };
            let Some(reach) = warp_reach(radius, impulse, warp, speed, row_stretch(row)) else {
                continue;
            };
            let Some((start, _)) = link.interpolate(id, 0.0) else {
                continue;
            };
            let anchor = to_bevy(start) + direction(&link, unit) * reach;
            // Armada II keeps one effect record per ship.
            for (entity, _) in flashes.iter().filter(|(_, flash)| flash.ship == id) {
                commands.entity(entity).try_despawn();
            }
            commands.spawn((
                WarpFlash {
                    ship: id,
                    anchor,
                    reach,
                    radius,
                    exit_started: None,
                },
                Transform::from_translation(anchor),
                Visibility::Hidden,
            ));
        }
        // A warp-out may retain a live unit while its phase becomes false, or the
        // source may clear the phase entirely. Only a still-present unit can own
        // an exit pulse; removed ships must not leave a ghost behind.
        // The Stock look draws none on leaving (the original has no flash there).
        for &id in history.in_warp.difference(&in_warp).filter(|_| !stock) {
            let Some(unit) = view
                .unit(id)
                .filter(|unit| !unit.flags.contains(UnitFlags::DYING))
            else {
                continue;
            };
            let Some(def) = content.0.unit(unit.def) else {
                continue;
            };
            let Some((position, _)) = link.interpolate(id, 0.0) else {
                continue;
            };
            let radius = def.radius.to_f32();
            if radius <= 0.0 {
                continue;
            }
            let forward = direction(&link, unit);
            let anchor = to_bevy(position) + forward * radius;
            for (entity, _) in flashes.iter().filter(|(_, flash)| flash.ship == id) {
                commands.entity(entity).try_despawn();
            }
            commands.spawn((
                WarpFlash {
                    ship: id,
                    anchor,
                    reach: 0.0,
                    radius,
                    exit_started: Some(view.tick),
                },
                Transform::from_translation(anchor),
                Visibility::Hidden,
            ));
        }
    }
    history.in_warp = in_warp;
}

#[allow(clippy::too_many_arguments)]
fn place(
    mut commands: Commands,
    link: Option<Res<SimLink>>,
    clock: Res<AnimClock>,
    local: Option<Res<LocalPlayer>>,
    cine: Option<Res<CineractiveState>>,
    cameras: Query<&GlobalTransform, MainCamera>,
    mut flashes: Query<(Entity, &mut WarpFlash, &mut Transform, &mut Visibility)>,
) {
    let Some(link) = link else { return };
    if flashes.is_empty() {
        return;
    }
    let visible = drawn_units(
        link.curr(),
        local.and_then(|local| local.0),
        cinematic_reveal(cine.as_deref()),
    );
    let camera_rotation = cameras
        .single()
        .map_or(Quat::IDENTITY, |camera| camera.compute_transform().rotation);
    for (entity, mut flash, mut transform, mut visibility) in &mut flashes {
        let id = flash.ship;
        let Some(unit) = link
            .real_curr()
            .unit(id)
            .filter(|unit| !unit.flags.contains(UnitFlags::DYING))
        else {
            commands.entity(entity).try_despawn();
            continue;
        };
        let half_size = FLASH_RADII * flash.radius;
        let scale = if let Some(started) = flash.exit_started {
            // One pulse at the exit point. It expires on the sim clock even
            // if deceleration takes longer or the source skipped warp-out.
            let ticks =
                link.real_curr().tick.saturating_sub(started) as f64 + f64::from(clock.alpha());
            let elapsed = ticks / f64::from(sta_sim::TICK_HZ);
            if elapsed >= EXIT_PULSE_SECONDS {
                commands.entity(entity).try_despawn();
                continue;
            }
            let t = (elapsed / EXIT_PULSE_SECONDS) as f32;
            let pulse = if t < 0.35 { t / 0.35 } else { (1.0 - t) / 0.65 };
            (2.0 * half_size) * pulse
        } else {
            if link.warp_phase(id).is_none() {
                commands.entity(entity).try_despawn();
                continue;
            }
            let (position, _) = link
                .interpolate(id, clock.alpha())
                .expect("the ship has a current view");
            let position = to_bevy(position);
            let forward = direction(&link, unit);
            let distance = (flash.anchor - position).dot(forward);
            if distance <= -half_size || distance > flash.reach + 0.01 {
                commands.entity(entity).try_despawn();
                continue;
            }
            // The native entry sprite reprojects onto the ship's direction.
            flash.anchor = position + forward * distance;
            (1.0 - distance.abs() / flash.reach).max(0.0) * (2.0 * half_size)
        };
        *transform = Transform::from_translation(flash.anchor)
            .with_rotation(camera_rotation)
            .with_scale(Vec3::splat(scale));
        *visibility = if scale > 0.0 && visible.binary_search_by_key(&id, |unit| unit.id).is_ok() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

#[derive(Default)]
struct Art {
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
}

#[allow(clippy::too_many_arguments)]
fn attach(
    mut commands: Commands,
    mounted: Option<Res<Mounted>>,
    tables: Option<Res<SpriteTables>>,
    device: Option<Res<RenderDevice>>,
    cache: Option<ResMut<TextureCache>>,
    images: Option<ResMut<Assets<Image>>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut art: Local<Art>,
    flashes: Query<Entity, (With<WarpFlash>, Without<Mesh3d>)>,
) {
    let (Some(mounted), Some(tables)) = (mounted, tables) else {
        art.material = None;
        return;
    };
    if mounted.is_changed() || tables.is_changed() {
        art.material = None;
    }
    if flashes.is_empty() {
        return;
    }
    let (Some(mut meshes), Some(mut materials), Some(mut images), Some(mut cache)) =
        (meshes, materials, images, cache)
    else {
        return;
    };
    if art.material.is_none() {
        let Some(sprite) = tables.0.sprite(SPRITE) else {
            return;
        };
        let formats = device
            .as_ref()
            .map_or(CompressedImageFormats::BC, |device| {
                CompressedImageFormats::from_features(device.features())
            });
        let Some(texture) = (Loader {
            vfs: &mounted.0,
            cache: &mut cache,
            images: &mut images,
            formats,
        })
        .colour(&sprite.texture, false) else {
            return;
        };
        art.material = Some(materials.add(StandardMaterial {
            base_color_texture: Some(texture),
            uv_transform: sprites::uv_affine(spr_anim::sprite_uv(&tables.0, sprite, 0.0)),
            alpha_mode: sprites::alpha_mode(spr_anim::sprite_blend(&sprite.material)),
            unlit: true,
            cull_mode: None,
            ..default()
        }));
    }
    let mesh = art
        .mesh
        .get_or_insert_with(|| meshes.add(sprites::unit_quad()))
        .clone();
    let material = art.material.as_ref().unwrap();
    for entity in &flashes {
        commands
            .entity(entity)
            .try_insert((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
    }
}

#[cfg(test)]
#[path = "warp_flash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "warp_flash_gpu.rs"]
mod gpu;
