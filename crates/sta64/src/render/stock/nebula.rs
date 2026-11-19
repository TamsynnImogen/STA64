//! Stock nebulae: the games' own cloud sprites (the nebula ODF's SOD of billboard
//! puffs, see `render::nebula::NebulaBoards`), not the volumetric march.

use crate::render::profile::{Effect, ProfileEffects, RenderProfile};
use bevy::prelude::*;
use sta_surface::nebula::NebulaRecipe;

/// Both representations belong to the map root, so replacing the map removes them together.
#[derive(Component)]
pub(crate) struct MapNebulae {
    pub gas: Vec<(NebulaRecipe, f32, Vec<[f32; 3]>)>,
    pub boards: Vec<Entity>,
    pub fields: Vec<Entity>,
    pub volumetric: bool,
}

/// Switch only map visuals. The simulation and the selectable object entities stay put.
#[allow(clippy::too_many_arguments)]
pub(crate) fn switch(
    mut commands: Commands,
    profile: Option<Res<RenderProfile>>,
    effects: Option<Res<ProfileEffects>>,
    mut maps: Query<(Entity, &mut MapNebulae)>,
    mut visibility: Query<&mut Visibility>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::render::nebula::NebulaMaterial>>,
    mut images: ResMut<Assets<Image>>,
    noise: Res<crate::render::nebula::NoiseVolume>,
) {
    let want = volumetric(profile.as_deref(), effects.as_deref());
    for (root, mut map) in &mut maps {
        if map.volumetric == want { continue; }
        for &entity in &map.boards {
            if let Ok(mut vis) = visibility.get_mut(entity) {
                *vis = if want { Visibility::Hidden } else { Visibility::Inherited };
            }
        }
        if want && map.fields.is_empty() {
            // No field assets are built for an initially Stock map until requested.
            let fields: Vec<_> = map.gas.iter().map(|(recipe, radius, pos)| {
                crate::render::nebula::spawn_field(&mut commands, &mut meshes, &mut materials, &mut images, &noise, recipe.clone(), *radius, pos, root)
            }).collect();
            map.fields.extend(fields);
        }
        for &entity in &map.fields {
            commands.entity(entity).insert(if want { Visibility::Inherited } else { Visibility::Hidden });
        }
        map.volumetric = want;
    }
}

/// Whether a nebula class with a gas recipe is drawn as volumetric gas. True only for
/// the Refit implementation of the scenery effect; Stock draws the class's own
/// billboard model. With no profile resource (tests), the Refit look as before.
pub fn volumetric(profile: Option<&RenderProfile>, effects: Option<&ProfileEffects>) -> bool {
    match (profile, effects) {
        (Some(p), Some(e)) => e.implementation(*p, Effect::Scenery) == RenderProfile::Refit,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_draws_billboards_refit_draws_gas() {
        let mut fx = ProfileEffects::default();
        fx.register_stock(Effect::Scenery);
        assert!(!volumetric(Some(&RenderProfile::Stock), Some(&fx)));
        assert!(volumetric(Some(&RenderProfile::Refit), Some(&fx)));
        assert!(volumetric(Some(&RenderProfile::Stock), Some(&ProfileEffects::default())));
        assert!(volumetric(None, None));
    }

    #[test]
    fn live_map_switch_keeps_one_field_and_cleans_up_with_root() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RenderProfile::Stock)
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<Image>::default())
            .insert_resource(Assets::<crate::render::nebula::NebulaMaterial>::default())
            .insert_resource(crate::render::nebula::NoiseVolume::default())
            .add_systems(Update, switch);
        let mut effects = ProfileEffects::default();
        effects.register_stock(Effect::Scenery);
        app.insert_resource(effects);
        let root = app.world_mut().spawn((Transform::default(), Visibility::Inherited)).id();
        let board = app.world_mut().spawn((Transform::default(), Visibility::Inherited, ChildOf(root))).id();
        app.world_mut().entity_mut(root).insert(MapNebulae {
            gas: vec![(NebulaRecipe::parse(include_str!("../../../../sta_surface/nebulae/latinum.ini")), 100.0, vec![[0.0, 0.0, 0.0]])],
            boards: vec![board], fields: Vec::new(), volumetric: false,
        });
        let fallback = app.world_mut().spawn((Transform::default(), Visibility::Inherited)).id();
        let fallback_board = app.world_mut().spawn((Transform::default(), Visibility::Inherited, ChildOf(fallback))).id();
        app.world_mut().entity_mut(fallback).insert(MapNebulae { gas: Vec::new(), boards: Vec::new(), fields: Vec::new(), volumetric: false });
        app.update();
        assert!(app.world().get::<MapNebulae>(root).unwrap().fields.is_empty());
        assert_eq!(app.world().get::<Visibility>(board), Some(&Visibility::Inherited));
        *app.world_mut().resource_mut::<RenderProfile>() = RenderProfile::Refit;
        app.update();
        let field = app.world().get::<MapNebulae>(root).unwrap().fields[0];
        assert_eq!(app.world().get::<Visibility>(board), Some(&Visibility::Hidden));
        assert_eq!(app.world().get::<Visibility>(field), Some(&Visibility::Inherited));
        assert!(app.world().get::<MapNebulae>(fallback).unwrap().fields.is_empty());
        assert_eq!(app.world().get::<Visibility>(fallback_board), Some(&Visibility::Inherited));
        *app.world_mut().resource_mut::<RenderProfile>() = RenderProfile::Stock;
        app.update();
        assert_eq!(app.world().get::<Visibility>(board), Some(&Visibility::Inherited));
        assert_eq!(app.world().get::<Visibility>(field), Some(&Visibility::Hidden));
        *app.world_mut().resource_mut::<RenderProfile>() = RenderProfile::Refit;
        app.update();
        assert_eq!(app.world().get::<MapNebulae>(root).unwrap().fields, vec![field]);
        app.world_mut().entity_mut(root).despawn();
        app.update();
        assert!(app.world().get_entity(field).is_err());
        assert!(app.world().get_entity(board).is_err());
    }
}
