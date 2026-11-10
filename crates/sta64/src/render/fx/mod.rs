//! Effects drawn from sim events, aligned with Armada II Refit: RND-09 is the
//! weapon set, RND-10 impacts, RND-11 specials. The shaders and recipes are ported
//! from Refit (`NOTICE.md`).

pub mod bees;
pub mod fire;
pub mod impacts;
pub mod quad;
pub mod recipe;
pub mod shield;
pub mod specials;
pub mod weapons;

use bevy::prelude::*;

/// The effects' shaders and materials. Left out of the windowless tests: the
/// effect modules attach visuals only when the material assets exist.
pub struct FxMaterialsPlugin;

impl Plugin for FxMaterialsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((quad::FxQuadPlugin, shield::ShieldPlugin, fire::FirePlugin)).add_systems(Startup, warm_up).add_systems(Update, end_warm_up);
    }
}

/// An effect pipeline's shader loads and compiles the first time something uses it;
/// an explosion that was the first would show a frame or two late, at a moment that
/// varies from run to run. These specks, drawn for a few frames at the start, take
/// that cost before any effect is due.
#[derive(Component)]
struct WarmUp(u8);

fn warm_up(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut quads: ResMut<Assets<quad::FxQuadMaterial>>,
    mut shields: ResMut<Assets<shield::ShieldMaterial>>,
    mut fires: ResMut<Assets<fire::FireMaterial>>,
) {
    let speck = Transform::from_scale(Vec3::splat(0.01));
    let rect = meshes.add(quad::rectangle());
    let ball = meshes.add(Sphere::new(1.0));
    commands.spawn((WarmUp(8), Mesh3d(rect), MeshMaterial3d(quads.add(quad::material(&quad::Look::new(quad::K_FLARE, Vec3::ZERO)))), speck));
    commands.spawn((WarmUp(8), Mesh3d(ball.clone()), MeshMaterial3d(shields.add(shield::ShieldMaterial { params: default() })), speck));
    commands.spawn((WarmUp(8), Mesh3d(ball.clone()), MeshMaterial3d(fires.add(fire::FireMaterial { params: default() })), speck));
    let ring = fire::FireParams { look: Vec4::new(0.0, 0.0, 1.0, 1.0), ..default() };
    commands.spawn((WarmUp(8), Mesh3d(ball), MeshMaterial3d(fires.add(fire::FireMaterial { params: ring })), speck));
}

fn end_warm_up(mut commands: Commands, mut q: Query<(Entity, &mut WarmUp)>) {
    for (e, mut w) in &mut q {
        w.0 -= 1;
        if w.0 == 0 {
            commands.entity(e).despawn();
        }
    }
}

/// Parse and validate a shader of this directory as Bevy would load it.
#[cfg(test)]
pub(crate) fn validate_wgsl(src: &str) {
    let src = src.replace("constants::MATERIAL_BIND_GROUP", "3");
    let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
}

#[cfg(test)]
mod tests {
    /// Each shader is embedded at the path its material names.
    #[test]
    fn shader_paths_match_embedded_registration() {
        for name in ["quad.wesl", "shield.wesl", "fire.wesl"] {
            let registered = match name {
                "quad.wesl" => bevy::asset::embedded_path!("src", "quad.wesl"),
                "shield.wesl" => bevy::asset::embedded_path!("src", "shield.wesl"),
                _ => bevy::asset::embedded_path!("src", "fire.wesl"),
            };
            assert!(registered.to_string_lossy().replace('\\', "/").ends_with(&format!("render/fx/{name}")), "{registered:?}");
        }
    }
}

#[cfg(test)]
mod despawn_tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::sim_link::{ContentRes, SimLink, SimLinkPlugin};
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::PlayerId;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use std::time::Duration;

    /// RND-19: about five minutes of scripted combat drives the weapon and impact
    /// effects with their mesh and material assets present, under an error handler
    /// that panics. A command on an effect that is already gone (an insert after a
    /// same-frame despawn, a second despawn from a repeated hit) would log
    /// `Entity despawned` in play; here it fails the test.
    #[test]
    fn five_minute_battle_never_commands_a_despawned_effect() {
        let db = sta_content::fake::db();
        let beam = db.key("fx_beam").unwrap().0;
        let torp = db.key("fx_torpedo").unwrap().0;
        let mut ev = String::new();
        let ticks = 5 * 60 * sta_sim::TICK_HZ as u64;
        for t in (10..ticks).step_by(23) {
            let w = if t % 2 == 0 { beam } else { torp };
            ev += &format!("{{ at: {t}, event: {{ Fired: {{ shooter: 1, weapon: {w}, hardpoint: 0, target: {{ Entity: 2 }} }} }} }},");
        }
        // Torpedo hits, some doubled on one tick, some with no shot in flight.
        for t in (30..ticks).step_by(37) {
            let hit = |facing: u64, shield: u64, hull: u64| {
                format!("{{ at: {t}, event: {{ Hit: {{ target: 2, by: 1, weapon: {torp}, facing: {facing}, shield: {shield}, hull: {hull}, at: {{ x: 0, y: 0, z: 590 }} }} }} }},")
            };
            ev += &hit(t % 4, 20 * (t % 2), 30);
            if t % 3 == 0 {
                ev += &hit(1, 0, 30);
            }
        }
        for t in (200..ticks).step_by(900) {
            ev += &format!("{{ at: {t}, event: {{ ShieldsDown: {{ id: 2, facing: 1 }} }} }},");
        }
        let src = format!(
            "{{ units: [
                {{ id: 1, def: \"fx_cruiser\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
                {{ id: 2, def: \"fx_frigate\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}] }},
            ], events: [{ev}] }}"
        );
        let src = ScriptedSource::from_json5(&src, &db).unwrap();
        let mut app = App::new();
        app.set_error_handler(bevy::ecs::error::panic)
            .add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<quad::FxQuadMaterial>>()
            .init_resource::<Assets<shield::ShieldMaterial>>()
            .init_resource::<Assets<fire::FireMaterial>>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 45.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(ContentRes(std::sync::Arc::new(db)))
            .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_plugins((weapons::WeaponFxPlugin, impacts::ImpactFxPlugin));
        configure_sets(&mut app);
        let mut spawned = 0;
        for _ in 0..ticks + 200 {
            app.update();
            spawned = spawned.max(app.world_mut().query::<&weapons::WeaponFx>().iter(app.world()).count());
        }
        assert!(spawned > 0);
    }
}
