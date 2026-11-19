//! A mining beam drawn as the ordnance's own model (`shotGeometry`).
//!
//! The rule STA64 follows: the ordnance of class `minebm` names a model (`shotGeometry`); the beam
//! is that model with its origin on the firing point and its +z axis toward the target, scaled by
//! `(radiusScale, radiusScale, length / lengthScale)` (ordnance keys), re-posed every frame. The
//! shape is the model's own, so a beam is a cone where the model is one (grades: the keys and the
//! model reference read from data; the scale rule read from the executable, static; the cone look
//! stated by a player).
//!
//! The model is loaded and drawn by the same code as a unit's. A weapon whose ordnance has no
//! such model (not installed, or a mod's own) keeps the recipe-drawn ribbon of [`super::weapons`].

use std::collections::HashMap;
use std::sync::Arc;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_assets::model::Model;
use sta_content::weapon::MINING_BEAM_CLASS;

use super::quad::FxQuadMaterial;
use super::weapons::{FxKind, WeaponFx};
use crate::render::model_scene::{self, PartCache, SpawnCtx};
use crate::render::sprites::SpriteTables;
use crate::render::textures::{Loader, TextureCache};
use crate::sim_link::ContentRes;
use crate::viewer::Mounted;

/// A beam entity drawn as its ordnance's model: `radius` is `radiusScale`, `length` is `lengthScale`.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ModelBeam {
    pub radius: f32,
    pub length: f32,
}

/// The transform that puts a beam model's origin at `from`, its +z axis (render space mirrors the
/// model's z, so the entity's -z) toward `to`, scaled `(radius, radius, distance / length)`.
pub fn beam_transform(from: Vec3, to: Vec3, radius: f32, length: f32) -> Transform {
    let d = to - from;
    let dist = d.length().max(1e-3);
    let up = if (d / dist).dot(Vec3::Y).abs() > 0.999 { Vec3::X } else { Vec3::Y };
    let mut tf = Transform::from_translation(from).looking_to(d / dist, up);
    tf.scale = Vec3::new(radius, radius, dist / length.max(1e-3));
    tf
}

/// INTERIM (not established): a model of a file version before 1.7 names no blend mode, so the loader
/// draws it opaque and lit, which no beam can be. Its lighting material is named `add_...` and the
/// same beam models of the later versions declare `additive`; until the pre-1.7 texture-material modes
/// are read, such a beam is drawn additive. A model that names a blend mode keeps it. Listed in TODO.md.
fn additive_if_plain(nodes: &mut [sta_assets::model::Node]) {
    use sta_assets::model::NodeKind;
    let named = nodes.iter().any(|n| matches!(&n.kind, NodeKind::Mesh(m) if m.groups.iter().any(|g| g.blend != "opaque")));
    if named {
        return;
    }
    for n in nodes.iter_mut() {
        if let NodeKind::Mesh(m) = &mut n.kind {
            for g in &mut m.groups {
                g.blend = "additive".to_owned();
            }
        }
    }
}

/// The render assets the model beams need exist (not in windowless tests).
pub(super) fn can_attach(
    vfs: Option<Res<Mounted>>,
    tables: Option<Res<SpriteTables>>,
    cache: Option<Res<TextureCache>>,
    parts: Option<Res<PartCache>>,
    meshes: Option<Res<Assets<Mesh>>>,
    materials: Option<Res<Assets<StandardMaterial>>>,
    images: Option<Res<Assets<Image>>>,
) -> bool {
    vfs.is_some() && tables.is_some() && cache.is_some() && parts.is_some() && meshes.is_some() && materials.is_some() && images.is_some()
}

/// Models by file stem, loaded once.
#[derive(Default)]
pub(super) struct BeamModels(HashMap<String, Option<(String, Arc<Model>)>>);

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn attach_models(
    mut commands: Commands,
    new: Query<(Entity, &WeaponFx), Added<WeaponFx>>,
    all: Query<(Entity, &WeaponFx)>,
    vfs: Res<Mounted>,
    content: Res<ContentRes>,
    tables: Res<SpriteTables>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<TextureCache>,
    mut parts: ResMut<PartCache>,
    mut models: Local<BeamModels>,
    device: Option<Res<RenderDevice>>,
) {
    let formats = device.map(|d| CompressedImageFormats::from_features(d.features())).unwrap_or(CompressedImageFormats::BC);
    for (e, fx) in new.iter().filter(|(_, f)| f.kind == FxKind::Beam) {
        let Some(def) = content.0.weapon(fx.weapon) else { continue };
        let (true, Some(scale), Some(name)) = (def.class.0 == MINING_BEAM_CLASS, def.shot_scale, def.fx.as_deref()) else { continue };
        let key = name.to_ascii_lowercase().trim_end_matches(".sod").to_owned();
        let found = models
            .0
            .entry(key.clone())
            .or_insert_with(|| {
                sta_assets::find_model(&vfs.0, &key).and_then(|(path, fmt)| match sta_assets::load_model(&vfs.0, &path, fmt) {
                    Ok(mut m) => {
                        additive_if_plain(&mut m.nodes);
                        Some((path, Arc::new(m)))
                    }
                    Err(err) => {
                        warn!("mining beam model {key}: {err}");
                        None
                    }
                })
            })
            .clone();
        let Some((path, model)) = found else { continue };
        let mut ctx = SpawnCtx {
            commands: &mut commands,
            meshes: &mut meshes,
            materials: &mut materials,
            textures: Loader { vfs: &vfs.0, cache: &mut cache, images: &mut images, formats },
            team: Color::WHITE,
            borg: false,
            sprites: &tables.0,
            damaged: false,
            normal_maps: false,
            parts: Some(&mut parts),
            model_key: path,
            unlit: false,
            scale: 1.0,
        };
        model_scene::spawn(&mut ctx, &model, e);
        commands
            .entity(e)
            .insert((ModelBeam { radius: scale.radius.to_f32(), length: scale.length.to_f32() }, Visibility::default()))
            .remove::<(Mesh3d, MeshMaterial3d<FxQuadMaterial>)>();
        // The recipe's flares at the ends are not part of the model: the same shot's go.
        for (other, o) in &all {
            if matches!(o.kind, FxKind::Flash | FxKind::Impact) && o.shooter == fx.shooter && o.weapon == fx.weapon && o.fired == fx.fired {
                commands.entity(other).try_despawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model point at distance `z` along +z lands on the line to the target, scaled.
    #[test]
    fn the_model_runs_from_the_firing_point_to_the_target_scaled() {
        let (from, to) = (Vec3::new(10.0, 4.0, -20.0), Vec3::new(10.0, 4.0, -220.0));
        let tf = beam_transform(from, to, 2.5, 5.0);
        // Render space mirrors the model's z: model (0, 0, 5) is (0, 0, -5).
        let far = tf.transform_point(Vec3::new(0.0, 0.0, -5.0));
        assert!((far - to).length() < 1e-3, "{far}");
        // The cross-section scales by `radiusScale`.
        let side = tf.transform_point(Vec3::new(1.0, 0.0, -5.0)) - far;
        assert!((side.length() - 2.5).abs() < 1e-3, "{side}");
        assert!(tf.transform_point(Vec3::ZERO).distance(from) < 1e-4);
    }

    #[test]
    fn a_plain_model_is_drawn_additive_and_one_that_names_a_blend_keeps_it() {
        use sta_assets::model::{Group, Maps, Mesh as M, Node, NodeKind};
        let node = |blend: &str| Node {
            name: "n".into(),
            parent: None,
            children: vec![],
            transform: sta_assets::a2fmt::Matrix34::IDENTITY,
            kind: NodeKind::Mesh(M {
                positions: vec![],
                uvs: vec![],
                groups: vec![Group { material: "m".into(), blend: blend.into(), maps: Maps::default(), faces: vec![] }],
                two_sided: false,
            }),
        };
        let blends = |ns: &[Node]| ns.iter().filter_map(|n| match &n.kind { NodeKind::Mesh(m) => Some(m.groups[0].blend.clone()), _ => None }).collect::<Vec<_>>();
        let mut plain = vec![node("opaque"), node("opaque")];
        additive_if_plain(&mut plain);
        assert_eq!(blends(&plain), ["additive", "additive"]);
        let mut named = vec![node("opaque"), node("alpha")];
        additive_if_plain(&mut named);
        assert_eq!(blends(&named), ["opaque", "alpha"]);
    }

    #[test]
    fn any_direction_and_a_vertical_one_keep_the_end_on_the_target() {
        for to in [Vec3::new(30.0, -40.0, 70.0), Vec3::new(0.0, 90.0, 0.0), Vec3::new(0.0, -90.0, 0.0)] {
            let from = Vec3::new(1.0, 2.0, 3.0);
            let tf = beam_transform(from, to, 7.0, 5.0);
            assert!((tf.transform_point(Vec3::new(0.0, 0.0, -5.0)) - to).length() < 1e-3, "{to}");
        }
    }
}
