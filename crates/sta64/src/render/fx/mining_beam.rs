//! A mining beam drawn as the ordnance's own model (`shotGeometry`).
//!
//! The rule STA64 follows: the ordnance of class `minebm` names a model (`shotGeometry`); the beam
//! is that model with its origin on the firing point and its +z axis toward the target, scaled by
//! `(radiusScale, radiusScale, length / lengthScale)` (ordnance keys), re-posed every frame. The
//! shape is the model's own, so a beam is a cone where the model is one (grades: the keys and the
//! model reference read from data; the scale rule read from the executable, static; the cone look
//! stated by a player).
//!
//! The beam's colour: a group's lighting-material `diffuse` multiplies its texture (`SpawnCtx::diffuse_tint`),
//! as the unlit additive path otherwise drops it. The numbers are gamma-space values, as on the lit hull path,
//! so the material carries their sRGB decode (exact against a gamma-space multiply for a pure power curve,
//! approximate for the sRGB curve and for the additive blend). For a model whose texture is colourless the diffuse is
//! the only colour the data gives (read from data); that the original shows it is stated by a player.
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

/// INTERIM (not established): a model that names no blend mode (the loader gives it `opaque`) is drawn
/// additive, as a beam is. A model that names a blend on any group keeps every group as loaded, its
/// unnamed groups too (opaque, unlit). Listed in TODO.md.
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

/// A beam is drawn unlit: that also keeps it out of the hull look, whose parts the shadow system turns
/// into shadow casters (a beam casts no shadow).
const BEAM_UNLIT: bool = true;

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

/// The model `name` (a file stem) from the mounted files, with the path it came from; a model that is
/// absent gives `None`, one that fails to load `None` with a warning naming `what`. Shared by the effects
/// that draw an ordnance's or an effect's own model (this beam, the stock shield cap).
pub(super) fn load_named_model(vfs: &sta_vfs::Vfs, name: &str, what: &str) -> Option<(String, Model)> {
    sta_assets::find_model(vfs, name).and_then(|(path, fmt)| match sta_assets::load_model(vfs, &path, fmt) {
        Ok(m) => Some((path, m)),
        Err(err) => {
            warn!("{what} {name}: {err}");
            None
        }
    })
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
                load_named_model(&vfs.0, &key, "mining beam model").map(|(path, mut m)| {
                    additive_if_plain(&mut m.nodes);
                    (path, Arc::new(m))
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
            // Its own cache entry: the tinted parts must not be shared with a plain spawn of the model.
            model_key: format!("{path}#beam"),
            unlit: BEAM_UNLIT,
            scale: 1.0,
            diffuse_tint: true,
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

    /// An authored one-triangle beam model whose colour lives in its lighting material (the texture is
    /// absent, so white), spawned as `spawn` says; returns the hull parts made and the material colours.
    fn spawn_authored(unlit: bool, diffuse_tint: bool, diffuse: [f32; 3]) -> (usize, Vec<LinearRgba>) {
        use crate::render::hulls::HullPart;
        use bevy::ecs::world::CommandQueue;
        use bevy::image::CompressedImageFormats;
        use sta_assets::model::{Group, LightingMaterial, Mesh as M, Node, NodeKind, SourceFormat};
        let model = Model {
            format: SourceFormat::M3d,
            version: 2.06,
            materials: vec![LightingMaterial { name: "m".into(), ambient: [0.5; 3], diffuse, specular: [1.0; 3], power: 50.0, model: 0, self_illuminating: false }],
            nodes: vec![Node {
                name: "beam".into(),
                parent: None,
                children: Vec::new(),
                transform: sta_assets::a2fmt::Matrix34::IDENTITY,
                kind: NodeKind::Mesh(M {
                    positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                    uvs: vec![[0.0, 0.0]; 3],
                    groups: vec![Group { material: "m".into(), blend: "additive".into(), maps: Default::default(), faces: vec![[(0, 0), (1, 1), (2, 2)]] }],
                    two_sided: false,
                }),
            }],
            channels: Vec::new(),
            flipbooks: Vec::new(),
            warnings: Vec::new(),
            pbr: None,
        };
        let table = sta_assets::a2fmt::spr::SpriteTable::parse(b"sprite_table\n@reference=64\n");
        let vfs = sta_vfs::Vfs::default();
        let (mut meshes, mut materials, mut images) = (Assets::<Mesh>::default(), Assets::<StandardMaterial>::default(), Assets::<Image>::default());
        let mut cache = TextureCache::default();
        let mut world = World::new();
        let root = world.spawn((Transform::IDENTITY, Visibility::default())).id();
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            let mut ctx = SpawnCtx {
                commands: &mut commands,
                meshes: &mut meshes,
                materials: &mut materials,
                textures: Loader { vfs: &vfs, cache: &mut cache, images: &mut images, formats: CompressedImageFormats::NONE },
                team: Color::WHITE,
                borg: false,
                sprites: &table,
                damaged: false,
                normal_maps: false,
                parts: None,
                model_key: "beam".into(),
                unlit,
                scale: 1.0,
                diffuse_tint,
            };
            model_scene::spawn(&mut ctx, &model, root);
        }
        queue.apply(&mut world);
        let hulls = world.query::<&HullPart>().iter(&world).count();
        (hulls, materials.iter().map(|(_, m)| m.base_color.to_linear()).collect())
    }

    /// A beam's model spawn makes no hull part, so the shadow system adds no caster for it; the same
    /// model spawned the way a unit is does (the test would otherwise prove nothing).
    #[test]
    fn a_beam_model_has_no_hull_part_and_so_casts_no_shadow() {
        assert_eq!(spawn_authored(BEAM_UNLIT, true, [0.2, 0.4, 1.0]).0, 0);
        assert!(spawn_authored(false, false, [0.2, 0.4, 1.0]).0 > 0, "a unit-style spawn of the same model has a hull part");
    }

    /// The colour of a beam whose model carries it in the lighting material reaches the drawn material:
    /// the diffuse, not white. A spawn without the tint stays white (every other model is unchanged).
    #[test]
    fn a_beam_takes_its_colour_from_the_lighting_material_diffuse() {
        let (_, tinted) = spawn_authored(BEAM_UNLIT, true, [0.2, 0.4, 1.0]);
        // The numbers are gamma-space, as on the lit hull path: the material carries their sRGB decode.
        assert_eq!(tinted, vec![Color::srgb(0.2, 0.4, 1.0).to_linear()]);
        assert_ne!(tinted, vec![LinearRgba::new(0.2, 0.4, 1.0, 1.0)], "not read as linear values");
        let (_, plain) = spawn_authored(BEAM_UNLIT, false, [0.2, 0.4, 1.0]);
        assert_eq!(plain, vec![LinearRgba::WHITE]);
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
