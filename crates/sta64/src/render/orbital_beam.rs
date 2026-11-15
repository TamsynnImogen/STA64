//! The orbital mining beam (SIM-65): a finished orbital station (class `p_mining`, A2 stock and
//! STA2 Classic's orbital mining and processing facilities) fires its mining weapon at its planet.
//!
//! Like a freighter's beam (RND-29, [`super::mining`]) this is a visual effect, not sim state: the
//! view says where the station and its planet are (the planet is the one whose orbit the station is
//! on, [`sta_sim::production::orbit::owner`]), and every `shotDelay` of the station's mining weapon
//! (`gOrbitalBeam`: 1.5 s, ordnance class `minebm`, `lifeSpan` 1.25 s) a [`MiningShot`] is sent for
//! [`weapons`](super::fx::weapons): the beam is on about five sixths of the time and follows the
//! station round the planet. No owner, crew or power is asked: the original's step tests none
//! (read from the executable, static). Which hardpoint it leaves from, where on the planet it ends
//! and whether a station that is disabled, derelict or captured keeps firing are not measured; the
//! beam runs from the model's origin to the planet's centre like the freighters'.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_content::ContentDb;
use sta_sim::production::orbit;
use sta_sim::view::UnitFlags;
use sta_sim::{EntityId, TICK_HZ, UnitView};

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::mining::{MiningShot, mining_weapon};
use crate::sim_link::{ContentRes, SimLink};

/// When each orbital station fires next, anim-clock seconds.
#[derive(Resource, Default)]
struct Beams(BTreeMap<EntityId, f64>);

pub struct OrbitalBeamPlugin;

impl Plugin for OrbitalBeamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Beams>().add_message::<MiningShot>().add_systems(
            Update,
            fire.after(crate::render::units::place)
                .before(crate::render::fx::weapons::spawn_fx)
                .in_set(GameSet::Interpolate)
                .distributive_run_if(resource_exists::<SimLink>)
                .distributive_run_if(resource_exists::<ContentRes>),
        );
    }
}

/// Whether `unit` is an orbital station that is standing and fires.
pub fn is_firing(db: &ContentDb, unit: &UnitView) -> bool {
    unit.progress.is_none()
        && !unit.flags.contains(UnitFlags::DYING)
        && db.unit(unit.def).is_some_and(|d| orbit::is_orbital(d) && mining_weapon(db, d).is_some())
}

fn fire(mut beams: ResMut<Beams>, link: Res<SimLink>, content: Res<ContentRes>, clock: Res<AnimClock>, mut shots: MessageWriter<MiningShot>) {
    let db = &content.0;
    let view = link.curr();
    let stations: Vec<&UnitView> = view.units.iter().filter(|u| is_firing(db, u)).collect();
    beams.0.retain(|id, _| stations.iter().any(|s| s.id == *id));
    if stations.is_empty() {
        return;
    }
    let (planets, _) = orbit::from_view(db, view);
    let now = clock.now();
    for s in stations {
        let Some(planet) = orbit::owner(&planets, s.pos) else { continue };
        let Some(wp) = db.unit(s.def).and_then(|d| mining_weapon(db, d)) else { continue };
        let next = beams.0.entry(s.id).or_insert(now);
        if now >= *next {
            shots.write(MiningShot { shooter: s.id, weapon: wp.key, node: planet.id });
            *next = now + wp.refire.max(1) as f64 / TICK_HZ as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::render::fx::weapons::{FxKind, WeaponFx, WeaponFxPlugin};
    use crate::render::units::to_bevy;
    use crate::sim_link::{SimEntities, SimEntity, SimLinkPlugin};
    use bevy::time::TimeUpdateStrategy;
    use sta_content::{ContentBuilder, DefRef, Fixed, Hardpoint, UnitDef, WeaponDef, WeaponKind};
    use sta_sim::PlayerId;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use std::time::Duration;

    fn db() -> ContentDb {
        let mut beam = WeaponDef::new("o_beam", "minebm", WeaponKind::Beam);
        beam.range = Fixed::from_int(180);
        beam.refire = 45;
        beam.duration = 37;
        let mut station = UnitDef::new("o_station", "p_mining");
        station.orbit_time = Some(Fixed::from_int(120));
        station.hardpoints = vec![Hardpoint { weapon: DefRef::new("o_beam"), node: "hp01".into(), arc: None }];
        let mut plain = UnitDef::new("o_plain", "starbase");
        plain.hardpoints = station.hardpoints.clone();
        let mut world = UnitDef::new("o_world", "planet");
        world.radius = Fixed::from_int(200);
        world.resource = Some(2);
        world.can_have_orbital_station = true;
        ContentBuilder::default().weapon(beam).unit(station).unit(plain).unit(world).build().unwrap()
    }

    /// A planet at the origin; an orbital station on its orbit (275 out) that moves a quarter
    /// turn between tick 0 and 300; an ordinary station beside it.
    fn app() -> (App, Entity) {
        let db = db();
        let text = "{ units: [
            { id: 1, def: \"o_world\", owner: 0, hull: 1, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
            { id: 2, def: \"o_station\", owner: 0, hull: 1, rails: [{ at: 0, pos: { x: 0, y: 0, z: 275 } }, { at: 300, pos: { x: 275, y: 0, z: 0 } }] },
            { id: 3, def: \"o_plain\", owner: 0, hull: 1, rails: [{ at: 0, pos: { x: 900, y: 0, z: 0 } }] },
        ], events: [] }";
        let src = ScriptedSource::from_json5(text, &db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / TICK_HZ as f64)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(ContentRes(std::sync::Arc::new(db)))
            .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_plugins(WeaponFxPlugin)
            .add_systems(Update, crate::render::units::place.in_set(GameSet::Interpolate));
        configure_sets(&mut app);
        let mut station = Entity::PLACEHOLDER;
        for (id, x, z) in [(1, 0.0, 0.0), (2, 0.0, 275.0), (3, 900.0, 0.0)] {
            let e = app.world_mut().spawn((SimEntity(EntityId(id)), Transform::from_translation(to_bevy([x, 0.0, z])))).id();
            app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(id), e);
            if id == 2 {
                station = e;
            }
        }
        (app, station)
    }

    /// The orbital station's beam is up in pulses all the time, from the station to the planet and
    /// along it while it circles; the ordinary station with the same weapon fires nothing.
    #[test]
    fn the_orbital_station_pulses_its_beam_at_its_planet_and_follows_it() {
        let (mut app, _) = app();
        let (mut up, mut shots, mut was) = (0, 0, false);
        let mut last_from = Vec3::ZERO;
        for frame in 0..290 {
            app.update();
            let mut q = app.world_mut().query::<&WeaponFx>();
            let beams: Vec<_> = q.iter(app.world()).filter(|f| f.kind == FxKind::Beam).collect();
            assert!(beams.iter().all(|f| f.shooter == EntityId(2) && f.target == sta_sim::command::Target::Entity(EntityId(1))), "only the orbital station fires, at its planet");
            if frame > 5 {
                up += usize::from(!beams.is_empty());
                shots += usize::from(!beams.is_empty() && !was);
            }
            was = !beams.is_empty();
            if let Some(b) = beams.first() {
                last_from = b.origin();
            }
        }
        // 1.5 s a shot (45 frames), on for about 1.25 s of it.
        assert!(up > 200, "{up} frames of 285 with a beam");
        assert!(shots >= 5, "{shots} separate pulses");
        // Late in the run the station has moved toward +X; the beam leaves from where it is.
        assert!(last_from.distance(to_bevy([275.0 * 0.98, 0.0, 275.0 * 0.2])) < 120.0, "{last_from:?}");
    }

    /// A station still being built, a dying one and an ordinary station do not fire.
    #[test]
    fn only_a_finished_standing_orbital_station_fires() {
        let db = db();
        let unit = |def: &str, progress: Option<Fixed>, flags: UnitFlags| UnitView {
            id: EntityId(9),
            def: db.key(def).unwrap(),
            owner: PlayerId(0),
            pos: sta_sim::Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO },
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            flags,
            progress,
            crew: None,
            energy: None,
            autonomy: None,
        };
        assert!(is_firing(&db, &unit("o_station", None, UnitFlags::default())));
        assert!(!is_firing(&db, &unit("o_station", Some(Fixed::ZERO), UnitFlags::default())));
        assert!(!is_firing(&db, &unit("o_station", None, UnitFlags::DYING)));
        assert!(!is_firing(&db, &unit("o_plain", None, UnitFlags::default())));
        assert!(!is_firing(&db, &unit("o_world", None, UnitFlags::default())));
    }
}
