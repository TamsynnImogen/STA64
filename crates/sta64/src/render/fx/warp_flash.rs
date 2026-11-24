//! Armada II's additive warp-entry sprite and cruise hull stretch. Timing and projection follow
//! Armada2.exe's mWarpCheck (0x004b3ce0, 0x004ad4a0), CreateWarpSpriteEffect
//! (0x0059c380), and WarpSpriteEffect::Render (0x0059ca40).

use std::collections::BTreeSet;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_assets::spr_anim;
use sta_sim::{EntityId, Tick, UnitView, view::UnitFlags};

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::map::MainCamera;
use crate::render::sprites::{self, SpriteTables};
use crate::render::textures::{Loader, TextureCache};
use crate::render::units::{LocalPlayer, cinematic_reveal, drawn_units, to_bevy};
use crate::sim_link::{ContentRes, SimEntity, SimLink};
use crate::ui::presentation::CineractiveState;
use crate::viewer::Mounted;

const SPRITE: &str = "warp_sprite";
const FLASH_RADII: f32 = 2.5;

#[derive(Component)]
struct WarpFlash {
    ship: EntityId,
    anchor: Vec3,
    reach: f32,
    radius: f32,
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

pub(super) fn register(app: &mut App) {
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

/// mWarpCheck's stock longitudinal stretch at full warp speed.
fn cruise_stretch(impulse: f32, warp: f32) -> f32 {
    (0.6 * warp / (impulse + 0.1)).clamp(1.5, 6.0)
}

fn stretch_hulls(
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    clock: Res<AnimClock>,
    ships: Query<&SimEntity>,
    mut models: Query<(&WarpHull, &ChildOf, &mut Transform)>,
) {
    let source = link.as_deref().zip(content.as_deref());
    for (model, parent, mut transform) in &mut models {
        let stretch = source
            .and_then(|(link, content)| {
                let id = ships.get(parent.parent()).ok()?.0;
                link.warp_phase(id)?;
                let unit = link.real_curr().unit(id)?;
                let def = content.0.unit(unit.def)?;
                let (impulse, warp) = warp_speeds(link, unit, def);
                if warp <= impulse || impulse < 0.0 {
                    return None;
                }
                let now = unit.speed.to_f32();
                let then = link
                    .prev()
                    .unit(id)
                    .map_or(now, |previous| previous.speed.to_f32());
                let speed = then + (now - then) * clock.alpha();
                // The original sweeps a deformation plane through the mesh. Use
                // the live speed ramp for the visual transform during the transition.
                let progress = ((speed - impulse) / (warp - impulse)).clamp(0.0, 1.0);
                Some(1.0 + (cruise_stretch(impulse, warp) - 1.0) * progress)
            })
            .unwrap_or(1.0);
        let scale = Vec3::new(1.0, 1.0, stretch);
        let translation = Vec3::Z * (model.pivot_z * (1.0 - stretch));
        if transform.scale != scale || transform.translation != translation {
            transform.scale = scale;
            transform.translation = translation;
        }
    }
}

/// mWarpCheck multiplies its clamped transition time by the warp speed.
/// Use the simulation's current speed limits and collision radius.
fn warp_reach(radius: f32, impulse: f32, warp: f32, speed: f32) -> Option<f32> {
    if radius <= 0.0 || impulse < 0.0 || warp <= impulse {
        return None;
    }
    let stretch = cruise_stretch(impulse, warp);
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
            let Some(reach) = warp_reach(radius, impulse, warp, speed) else {
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
            .filter(|_| link.warp_phase(id).is_some())
        else {
            commands.entity(entity).try_despawn();
            continue;
        };
        let (position, _) = link
            .interpolate(id, clock.alpha())
            .expect("the ship has a current view");
        let position = to_bevy(position);
        let forward = direction(&link, unit);
        let distance = (flash.anchor - position).dot(forward);
        let half_size = FLASH_RADII * flash.radius;
        if distance <= -half_size || distance > flash.reach + 0.01 {
            commands.entity(entity).try_despawn();
            continue;
        }
        // The native effect reprojects onto the ship's direction each frame.
        flash.anchor = position + forward * distance;
        let scale = (1.0 - distance.abs() / flash.reach).max(0.0) * (2.0 * half_size);
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
