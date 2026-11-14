//! The building ghost: while a build waits for its site ([`HudState::placing`])
//! a translucent copy of the building's model follows the cursor on the
//! ground at the builder's height, green where the site is clear and red where
//! it is not (stock Armada's blueprint, which the refit notes describe as green
//! when clear and red when blocked).
//!
//! Whether the site is clear is [`sta_sim::production::site::check_site`], the
//! rule the sim applies to the order, fed with the same view, so the ghost and
//! the sim agree; [`site_status`] is the one entry the click uses too
//! (both go through [`crate::placement::ghost_site`]).

use bevy::prelude::*;
use sta_sim::production::site::SiteError;
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

/// Where `armed` would be built for a cursor at `at`, and whether the site is legal: the same
/// [`crate::placement::ghost_site`] rule the ghost draws, so the click and the ghost agree (an
/// orbital station snaps onto a planet's orbit). No content loaded: the cursor point, `Ok`.
pub fn site_status(feed: &HudFeed, bounds: Option<&(SimVec3, SimVec3)>, armed: &Armed, at: SimVec3) -> (SimVec3, Result<(), SiteError>) {
    let Some(db) = feed.content.as_deref() else { return (at, Ok(())) };
    let site = crate::placement::ghost_site(db, bounds, &feed.view, armed.what, armed.builder, at);
    (site.at, site.status)
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
                // The placement rule the sim applies: an orbital station snaps onto a planet's orbit.
                let (site_at, status) = site_status(&feed, bounds.as_deref().and_then(|b| b.0.as_ref()), armed, at);
                t.translation = to_bevy(site_at.to_f32());
                g.status = Some(status);
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

    /// An orbital station's ghost follows the sim's placement rule (it snaps onto the planet's
    /// orbit and is refused in open space), not the ordinary site rule.
    #[test]
    fn an_orbital_ghost_snaps_to_the_planet_and_is_refused_in_open_space() {
        use crate::input::select::AimState;
        use sta_content::{ContentDb, MoveStats, UnitDef};
        use sta_sim::view::UnitView;
        use sta_sim::view::UnitFlags;
        use sta_sim::{EntityId, Fixed, PlayerId, WorldView};
        use std::sync::Arc;

        let fake = sta_content::fake::db();
        let mut rig = UnitDef::new("fx_rig", "constructionrig");
        rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        let mut orbital = UnitDef::new("fx_orbital", "p_mining");
        orbital.orbit_time = Some(Fixed::from_int(120));
        let mut planet = UnitDef::new("fx_world", "planet");
        planet.radius = Fixed::from_int(200);
        planet.resource = Some(2);
        planet.can_have_orbital_station = true;
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone()).unit(rig).unit(orbital).unit(planet);
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            b = b.unit(u.clone());
        }
        let db = b.build().unwrap();
        let unit = |id, odf: &str, x: i32| UnitView {
            id: EntityId(id), def: db.key(odf).unwrap(), owner: PlayerId(0),
            pos: SimVec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::ZERO }, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], crew: None, energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let what = db.key("fx_orbital").unwrap();
        let view = WorldView { units: vec![unit(1, "fx_world", 1000), unit(2, "fx_rig", 0)], ..Default::default() };
        let aim_at = |x: f32, z: f32| Aim(Some(AimState {
            cursor: Vec2::ZERO,
            ray: Ray3d::new(Vec3::new(x, 100.0, -z), Dir3::NEG_Y),
            clip_from_world: Mat4::IDENTITY,
            size: Vec2::new(800.0, 600.0),
        }));

        let mut world = World::new();
        world.insert_resource(HudFeed { view: Arc::new(view), player: PlayerId(0), selected: Vec::new(), content: Some(Arc::new(db)) });
        let state = HudState { placing: Some(Armed { builder: EntityId(2), what, odf: "fx_orbital".into() }), ..Default::default() };
        world.insert_resource(state);
        let ghost = world.spawn((Ghost { def: what, status: None, painted: None, own: Vec::new() }, Transform::default(), Visibility::Hidden)).id();

        world.insert_resource(aim_at(1100.0, 0.0));
        world.run_system_once(update_ghost).unwrap();
        assert_eq!(world.get::<Ghost>(ghost).unwrap().status, Some(Ok(())));
        // Radius 200 + 75 from the planet's centre, on the cursor's bearing.
        assert_eq!(world.get::<Transform>(ghost).unwrap().translation.x, 1275.0);

        world.insert_resource(aim_at(3000.0, 3000.0));
        world.run_system_once(update_ghost).unwrap();
        assert_eq!(world.get::<Ghost>(ghost).unwrap().status, Some(Err(SiteError::Blocked)));
        assert_eq!(world.get::<Transform>(ghost).unwrap().translation.x, 3000.0);

        // The click asks the same rule: the site the ghost shows OK is accepted, open space refused.
        let feed = world.resource::<HudFeed>();
        let armed = world.resource::<HudState>().placing.clone().unwrap();
        let sv = |x: i32, z: i32| SimVec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) };
        let (snapped, ok) = site_status(feed, None, &armed, sv(1100, 0));
        assert_eq!((ok, snapped), (Ok(()), sv(1275, 0)));
        assert_eq!(site_status(feed, None, &armed, sv(3000, 3000)).1, Err(SiteError::Blocked));
    }
}

#[cfg(test)]
#[path = "ghost_gpu.rs"]
mod gpu_tests;
