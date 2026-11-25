//! Persistent A1 special poses, reconciled with the simulation view each frame.
//! The view owns lifetimes; losing an effect removes its drawing immediately.

use std::collections::BTreeSet;

use bevy::prelude::*;
use sta_content::A1SpecialKind;
use sta_sim::ContentKey;

use super::quad::{self, FxQuadMaterial, Look};
use crate::app::{AppState, GameSet};
use crate::clock::AnimClock;
use crate::render::units::to_bevy;
use crate::render::{fog::FogState, units::LocalPlayer};
use crate::sim_link::{ContentRes, SimLink};

/// Specials whose lifetime and position are present in `SpecialView.effects`.
pub(super) fn tracked(kind: A1SpecialKind) -> bool {
    matches!(
        kind,
        A1SpecialKind::Mines
            | A1SpecialKind::ChainPulsar
            | A1SpecialKind::ShieldDisruptor
            | A1SpecialKind::Gemini
            | A1SpecialKind::Stasis
            | A1SpecialKind::Reflector
    )
}

#[derive(Component)]
pub struct A1Fx {
    pub id: u64,
    pub weapon: ContentKey,
    pub kind: A1SpecialKind,
}

pub struct A1FxPlugin;

impl Plugin for A1FxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            sync.in_set(GameSet::Interpolate)
                .run_if(resource_exists::<SimLink>)
                .run_if(resource_exists::<ContentRes>),
        )
        .add_systems(OnExit(AppState::InGame), clear);
    }
}

fn clear(mut commands: Commands, q: Query<Entity, With<A1Fx>>) {
    for entity in &q {
        commands.entity(entity).try_despawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn sync(
    mut commands: Commands,
    link: Res<SimLink>,
    content: Res<ContentRes>,
    clock: Res<AnimClock>,
    local: Option<Res<LocalPlayer>>,
    fog: Option<Res<FogState>>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mats: Option<ResMut<Assets<FxQuadMaterial>>>,
    q: Query<(Entity, &A1Fx, &MeshMaterial3d<FxQuadMaterial>)>,
    mut positions: Query<&mut Transform, With<A1Fx>>,
) {
    let (Some(mut meshes), Some(mut mats)) = (meshes, mats) else {
        return;
    };
    let eye = camera.single().ok().map(|c| c.translation());
    let mut matched = BTreeSet::new();
    let visible = |effect: &sta_sim::specials::EffectView| {
        local.as_ref().and_then(|local| local.0).is_none_or(|p| {
            p == effect.owner
                || fog.as_ref().is_none_or(|f| {
                    f.grid
                        .visible_at(effect.at.x.to_f32(), effect.at.z.to_f32())
                })
        })
    };
    let effects = &link.curr().a1_specials.effects;
    for (entity, fx, material) in &q {
        let Some(effect) = effects.iter().find(|e| e.id == fx.id && visible(e)) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        let Some(weapon) = content.0.weapon(effect.weapon) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        if fx.weapon != effect.weapon || weapon.a1_special.map(|s| s.kind) != Some(fx.kind) {
            commands.entity(entity).try_despawn();
            continue;
        }
        matched.insert(fx.id);
        let pos = to_bevy(effect.at.to_f32());
        let half = visual_radius(effect, weapon);
        if let Ok(mut tf) = positions.get_mut(entity) {
            *tf = quad::billboard(eye, pos, Vec2::splat(half));
        }
        let look = visual_look(weapon, fx.kind);
        quad::drive(
            &mut mats,
            &material.0,
            &look,
            1.0,
            quad::shader_clock(clock.now()),
        );
    }
    for effect in effects.iter().filter(|e| visible(e)) {
        if matched.contains(&effect.id) {
            continue;
        }
        let Some(weapon) = content.0.weapon(effect.weapon) else {
            continue;
        };
        let Some(kind) = weapon.a1_special.map(|s| s.kind).filter(|k| tracked(*k)) else {
            continue;
        };
        let pos = to_bevy(effect.at.to_f32());
        let half = visual_radius(effect, weapon);
        let look = visual_look(weapon, kind);
        commands.spawn((
            A1Fx {
                id: effect.id,
                weapon: effect.weapon,
                kind,
            },
            Mesh3d(meshes.add(quad::rectangle())),
            MeshMaterial3d(mats.add(quad::material(&look))),
            quad::billboard(eye, pos, Vec2::splat(half)),
        ));
    }
}

fn visual_radius(effect: &sta_sim::specials::EffectView, weapon: &sta_content::WeaponDef) -> f32 {
    effect
        .radius
        .map_or_else(|| weapon.look.radius.unwrap_or(4.0), |r| r.to_f32())
        .max(1.0)
}

fn visual_look(weapon: &sta_content::WeaponDef, kind: A1SpecialKind) -> Look {
    let color = weapon
        .look
        .light
        .as_ref()
        .map(|light| {
            let c = Vec3::from_array(light.color);
            c / if c.max_element() > 1.0 { 255.0 } else { 1.0 }
        })
        .unwrap_or(Vec3::new(0.45, 0.7, 1.0));
    match kind {
        A1SpecialKind::Stasis | A1SpecialKind::Reflector => Look {
            p3: Vec4::new(3.0, 0.6, 0.4, 0.0),
            ..Look::new(quad::K_CLOAK, color)
        },
        A1SpecialKind::Mines => Look::flare(color, 30.0, 4.0, 0.4, 18.0, 0.0, 0.0, 0.0),
        _ => Look {
            p3: Vec4::new(24.0, 4.0, 0.35, 0.0),
            col2: Vec3::ONE,
            ..Look::new(quad::K_BOLT, color)
        },
    }
}
