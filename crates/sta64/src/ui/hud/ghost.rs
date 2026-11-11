//! The building ghost: while a build waits for its site ([`HudState::placing`])
//! a translucent copy of the building's model follows the cursor on the
//! ground at the builder's height, green where the site is clear and red where
//! it is not (stock Armada's blueprint, which the refit notes describe as green
//! when clear and red when blocked).
//!
//! Whether the site is clear is [`sta_sim::production::site::check_site`], the
//! rule the sim applies to the order, fed with the same view, so the ghost and
//! the sim agree; [`site_status`] is the one entry the click uses too.

use bevy::prelude::*;
use sta_sim::production::site::{self, SiteError};
use sta_sim::{ContentKey, Vec3 as SimVec3};

use super::model::HudFeed;
use super::{Armed, HudState, ground_point};
use crate::input::orders::Aim;
use crate::render::hulls::{HullMaterial, HullSource};
use crate::render::units::{NeedsModel, TeamColour, to_bevy};

/// The map's playable volume, for build sites (`None`: unbounded). Set when a
/// map loads, from the same bounds the sim was given.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct PlacementBounds(pub Option<(SimVec3, SimVec3)>);

/// Tint of a ghost over a clear site.
pub const CLEAR: Color = Color::srgba(0.35, 1.0, 0.4, 0.75);
/// Tint of a ghost over a blocked site.
pub const BLOCKED: Color = Color::srgba(1.0, 0.25, 0.2, 0.75);

/// The ghost entity; `status` is whether the site under the cursor is clear.
#[derive(Component, Debug)]
pub struct Ghost {
    pub def: ContentKey,
    pub status: Option<Result<(), SiteError>>,
    painted: Option<Result<(), SiteError>>,
    own: Vec<(Entity, Handle<StandardMaterial>)>,
}

/// Whether `armed` may be placed at `at` now: `check_site` over the feed's
/// view with the builder left out.
pub fn site_status(feed: &HudFeed, bounds: Option<&(SimVec3, SimVec3)>, armed: &Armed, at: SimVec3) -> Result<(), SiteError> {
    let Some(db) = feed.content.as_deref() else { return Ok(()) };
    site::check_site(db, bounds, armed.what, at, site::view_obstacles(db, &feed.view, Some(armed.builder)))
}

/// Whether the player can pay for `armed` now (the sim refuses a rig's order otherwise).
pub fn affordable(feed: &HudFeed, armed: &Armed) -> bool {
    let (Some(db), Some(p)) = (feed.content.as_deref(), feed.view.player(feed.player)) else { return true };
    let Some(def) = db.unit(armed.what) else { return true };
    def.build.cost.iter().enumerate().all(|(i, &c)| c <= sta_sim::Fixed::ZERO || p.resources.get(i).is_some_and(|&h| h >= c))
}

fn paint(color: Color) -> impl Fn(&mut StandardMaterial) {
    move |m| {
        // A dark diffuse map or its cut-out alpha must not attenuate the
        // blueprint. These changes are made only to the ghost's material copy.
        m.base_color_texture = None;
        m.normal_map_texture = None;
        m.metallic_roughness_texture = None;
        m.occlusion_texture = None;
        m.emissive_texture = None;
        m.base_color = color;
        m.alpha_mode = AlphaMode::Blend;
        m.unlit = true;
        m.emissive = LinearRgba::BLACK;
        m.metallic = 0.0;
        m.reflectance = 0.0;
        m.perceptual_roughness = 1.0;
    }
}

/// Keep one ghost while a placement is armed: spawn it, move it to the
/// cursor, set its status, remove it when the placement ends.
#[allow(clippy::too_many_arguments)]
pub fn update_ghost(
    mut commands: Commands,
    state: Res<HudState>,
    feed: Res<HudFeed>,
    aim: Option<Res<Aim>>,
    bounds: Option<Res<PlacementBounds>>,
    mut ghosts: Query<(Entity, &mut Ghost, &mut Transform, &mut Visibility)>,
) {
    let armed = state.placing.as_ref().filter(|a| feed.view.unit(a.builder).is_some());
    let Some(armed) = armed else {
        for (e, ..) in &ghosts {
            commands.entity(e).despawn();
        }
        return;
    };
    let mut have = false;
    for (e, mut g, mut t, mut vis) in &mut ghosts {
        if have || g.def != armed.what {
            commands.entity(e).despawn();
            continue;
        }
        have = true;
        let y = feed.view.unit(armed.builder).map_or(0.0, |b| b.pos.y.to_f32());
        let at = aim.as_ref().and_then(|a| a.0).and_then(|a| ground_point(a.ray, y));
        match at {
            Some(at) => {
                t.translation = to_bevy(at.to_f32());
                g.status = Some(site_status(&feed, bounds.as_deref().and_then(|b| b.0.as_ref()), armed, at));
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    if !have {
        commands.spawn((
            Name::new("building ghost"),
            Ghost { def: armed.what, status: None, painted: None, own: Vec::new() },
            NeedsModel(armed.what),
            TeamColour(Color::WHITE),
            // Faces the way the finished building will (heading +z).
            Transform::IDENTITY.looking_to(Vec3::NEG_Z, Vec3::Y),
            Visibility::Hidden,
        ));
    }
}

/// Give the ghost's model translucent materials of its own (the model's parts
/// share theirs with real units) and keep them tinted by the site's status.
pub fn tint_ghost(
    mut commands: Commands,
    mut ghosts: Query<(Entity, &mut Ghost)>,
    children: Query<&Children>,
    mut handles: Query<&mut MeshMaterial3d<StandardMaterial>>,
    hull_sources: Query<&HullSource>,
    assets: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let Some(mut assets) = assets else { return };
    for (root, mut g) in &mut ghosts {
        let Some(status) = g.status else { continue };
        let mut new_parts = false;
        for d in children.iter_descendants(root) {
            if let Some((_, own)) = g.own.iter().find(|(entity, _)| *entity == d) {
                if hull_sources.get(d).is_ok() {
                    commands.entity(d)
                        .remove::<HullSource>()
                        .remove::<MeshMaterial3d<HullMaterial>>()
                        .insert(MeshMaterial3d(own.clone()));
                }
                continue;
            }
            let source = hull_sources.get(d).ok().map(|h| h.source.clone());
            let original = source.clone().or_else(|| handles.get(d).ok().map(|h| h.0.clone()));
            let Some(copy) = original.and_then(|h| assets.get(&h).cloned()) else { continue };
            let own = assets.add(copy);
            if source.is_some() {
                commands.entity(d)
                    .remove::<HullSource>()
                    .remove::<MeshMaterial3d<HullMaterial>>()
                    .insert(MeshMaterial3d(own.clone()));
            } else if let Ok(mut h) = handles.get_mut(d) {
                h.0 = own.clone();
            }
            g.own.push((d, own));
            new_parts = true;
        }
        if g.painted == Some(status) && !new_parts && !g.own.is_empty() {
            continue;
        }
        let color = if status.is_ok() { CLEAR } else { BLOCKED };
        let apply = paint(color);
        for (_, h) in &g.own {
            if let Some(mut m) = assets.get_mut(h) {
                apply(&mut m);
            }
        }
        if !g.own.is_empty() {
            g.painted = Some(status);
        }
    }
}

/// Why a click on a blocked site is refused, as the notice says it.
pub fn refusal(e: SiteError) -> &'static str {
    match e {
        SiteError::OffMap => "Cannot build here: off the map",
        SiteError::Blocked => "Cannot build here: the site is blocked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn blueprint_copy_ignores_dark_transparent_source_and_preserves_original() {
        let source = StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.0),
            base_color_texture: Some(Handle::default()),
            normal_map_texture: Some(Handle::default()),
            metallic_roughness_texture: Some(Handle::default()),
            occlusion_texture: Some(Handle::default()),
            emissive_texture: Some(Handle::default()),
            ..default()
        };
        let mut world = World::new();
        let mut materials = Assets::<StandardMaterial>::default();
        let original = materials.add(source);
        world.insert_resource(materials);
        let child = world.spawn(MeshMaterial3d(original.clone())).id();
        let converted = world.spawn((
            HullSource::test_source(original.clone()),
            MeshMaterial3d::<HullMaterial>(Handle::default()),
        )).id();
        let root = world.spawn(Ghost {
            def: ContentKey::NONE,
            status: Some(Ok(())),
            painted: None,
            own: Vec::new(),
        }).add_children(&[child, converted]).id();
        world.run_system_once(tint_ghost).unwrap();
        assert!(world.get::<HullSource>(converted).is_none());
        assert!(world.get::<MeshMaterial3d<HullMaterial>>(converted).is_none());
        let converted_material = world.get::<MeshMaterial3d<StandardMaterial>>(converted).unwrap().0.clone();
        assert_ne!(converted_material, original);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().get(&converted_material).unwrap().base_color, CLEAR);
        let own = world.get::<MeshMaterial3d<StandardMaterial>>(child).unwrap().0.clone();
        assert_ne!(own, original);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().get(&own).unwrap().base_color, CLEAR);
        world.get_mut::<Ghost>(root).unwrap().status = Some(Err(SiteError::Blocked));
        world.run_system_once(tint_ghost).unwrap();
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(child).unwrap().0, own);

        let materials = world.resource::<Assets<StandardMaterial>>();
        let source = materials.get(&original).unwrap();
        let blocked = materials.get(&own).unwrap();
        assert_eq!(source.base_color, Color::srgba(0.0, 0.0, 0.0, 0.0));
        assert!(source.base_color_texture.is_some());
        assert_eq!(blocked.base_color, BLOCKED);
        assert!(blocked.unlit);
        assert!(matches!(blocked.alpha_mode, AlphaMode::Blend));
        assert!(blocked.base_color_texture.is_none());
        assert!(blocked.normal_map_texture.is_none());
        assert!(blocked.metallic_roughness_texture.is_none());
        assert!(blocked.occlusion_texture.is_none());
        assert!(blocked.emissive_texture.is_none());
        assert_eq!(blocked.emissive, LinearRgba::BLACK);
        assert_ne!(source.base_color, blocked.base_color);
    }
}
