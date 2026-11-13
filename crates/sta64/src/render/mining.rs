//! Mining ships animate while they mine (RND-29, issue #345).
//!
//! The sim moves a freighter to within reach of its resource, mines in place, carries the
//! load to the nearest station and unloads in place ([`SimEvent::MinerPhase`] announces
//! each phase). What the player sees is drawn on top of that, from the game files:
//!
//! - **Orbit.** While mining, the freighter circles its resource at the *range of its
//!   mining weapon* (ODF `range` of its weapon whose ordnance is a mining beam, class
//!   label `minebm`), at its own top speed (`maxSpeed`), blending in from where the sim
//!   parked it. The weapons' `validTargets` lists (which moon each beam works) are lines
//!   the ODF reader does not keep, so the first mining weapon serves every resource; in
//!   the shipped files the dilithium and tritanium beams have the same range and delay. The sim position is not touched: the circle is a
//!   visual offset on the model ([`VisualOffsets`]), faded out when mining ends.
//! - **Beam.** The same weapon fires at the resource every `shotDelay` (its `refire`)
//!   while mining: a [`MiningShot`] for [`weapons`](super::fx::weapons), which draws it
//!   with the weapon's recipe (by ordnance class) for the shot's flight time at
//!   `shotSpeed`: the ordnance `lifespan` is only kept for beam-class ordnance, and the mining
//!   ordnance is not one. It is a message of its own, not a `SimEvent::Fired`, so combat
//!   music, sounds and weapon animations (which listen for real shots) do not take it for a
//!   fight. The beam leaves the orbiting model.
//! - **Dock.** While unloading the freighter glides to the station's position (the
//!   station that the sim named in the event). The files name no berth: no ODF key and no
//!   SOD hardpoint of the Armada, Armada II or Fleet Operations stations names a dock
//!   point (searched, not measured against the original), so the fallback is the
//!   station's origin, "in or on top of the station" as the issue says.
//!
//! Evidence grade: orbit radius, speed, beam cadence and lifespan read from data; the
//! blend time and the orbit direction are cosmetic choices; that the original engine
//! orbits at the weapon's range is not measured.
//!
//! A phase the sim announced before the link existed (a loaded save mid-cycle) shows
//! from the next change. A miner the sim moves off the spot it started working at (a stop,
//! a move or a new order do not announce themselves) stops being drawn at its work.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_content::{ContentDb, UnitDef};
use sta_sim::{ContentKey, EntityId, MinerPhase, SimEvent, TICK_HZ, Tick};

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::units::to_bevy;
use crate::sim_link::{ContentRes, SimEntities, SimLink, SimMsg};

/// Seconds a visual offset takes to blend in (into the orbit, onto the berth) or out: a
/// choice, the files say nothing.
const BLEND: f64 = 1.0;
/// How far the sim may move a miner from where its work began before the work is over.
const MOVED: f32 = 5.0;

/// A mining shot to draw: a freighter's mining weapon fires at its resource.
#[derive(Message, Debug, Clone, Copy)]
pub struct MiningShot {
    pub shooter: EntityId,
    pub weapon: ContentKey,
    pub node: EntityId,
}

/// How the renderer sees one freighter's mining weapon, from data.
#[derive(Debug, Clone, PartialEq)]
pub struct MiningWeapon {
    pub key: ContentKey,
    pub hardpoint: u8,
    /// Weapon `range`: the orbit radius.
    pub range: f32,
    /// Weapon `shotDelay` in ticks: the beam's cadence.
    pub refire: Tick,
}

/// The ordnance class label of a mining beam (`classLabel = "minebm"` of the ordnance ODF).
pub const MINING_BEAM_CLASS: &str = "minebm";

/// The weapon of `unit` that mines: the first hardpoint whose weapon's ordnance is a mining beam.
pub fn mining_weapon(db: &ContentDb, unit: &UnitDef) -> Option<MiningWeapon> {
    unit.hardpoints.iter().enumerate().find_map(|(i, hp)| {
        let w = db.weapon(hp.weapon.key)?;
        (w.class.0 == MINING_BEAM_CLASS).then(|| MiningWeapon { key: hp.weapon.key, hardpoint: i as u8, range: w.range.to_f32(), refire: w.refire })
    })
}

/// Where a miner's model is drawn off its sim position, per sim entity, in render space.
/// Read by the weapon effects so a beam leaves the model.
#[derive(Resource, Default, Debug)]
pub struct VisualOffsets(pub BTreeMap<EntityId, Vec3>);

#[derive(Debug, Clone)]
struct Work {
    phase: MinerPhase,
    node: EntityId,
    /// The station's place, render space, when unloading.
    dock: Option<Vec3>,
    /// Where the sim had the miner when the phase began: it does not move while it works.
    sim0: Vec3,
    /// Anim-clock time the phase began.
    since: f64,
    weapon: Option<MiningWeapon>,
    speed: f32,
    /// Where the orbit began: bearing and distance from the resource, and the offset the
    /// model had.
    angle0: f32,
    radius0: f32,
    from_offset: Vec3,
    next_shot: f64,
}

#[derive(Resource, Default)]
struct Miners(BTreeMap<EntityId, Work>);

pub struct MiningPlugin;

impl Plugin for MiningPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Miners>().init_resource::<VisualOffsets>().add_message::<SimMsg>().add_message::<MiningShot>().add_systems(
            Update,
            (track, animate)
                .chain()
                .after(crate::render::units::place)
                .before(crate::render::fx::weapons::spawn_fx)
                .in_set(GameSet::Interpolate)
                .distributive_run_if(resource_exists::<SimLink>)
                .distributive_run_if(resource_exists::<ContentRes>),
        );
    }
}

fn smooth(x: f64) -> f32 {
    let t = x.clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

fn point(link: &SimLink, id: EntityId, alpha: f32) -> Option<Vec3> {
    link.interpolate(id, alpha).map(|(p, _)| to_bevy(p))
}

/// Reads the sim's phase announcements.
fn track(
    mut msgs: MessageReader<SimMsg>,
    mut miners: ResMut<Miners>,
    offsets: Res<VisualOffsets>,
    link: Res<SimLink>,
    content: Res<ContentRes>,
    clock: Res<AnimClock>,
) {
    let now = clock.now();
    let alpha = clock.alpha();
    for SimMsg(ev) in msgs.read() {
        let SimEvent::MinerPhase { id, phase, node, dock } = ev else { continue };
        let db = &content.0;
        let unit = link.curr().unit(*id).and_then(|u| db.unit(u.def));
        let weapon = unit.and_then(|u| mining_weapon(db, u));
        let (here, there) = (point(&link, *id, alpha), point(&link, *node, alpha));
        let (angle0, radius0) = match (here, there) {
            (Some(h), Some(n)) => ((h.z - n.z).atan2(h.x - n.x), Vec2::new(h.x - n.x, h.z - n.z).length()),
            _ => (0.0, 0.0),
        };
        let from_offset = offsets.0.get(id).copied().unwrap_or(Vec3::ZERO);
        let speed = unit.map_or(0.0, |u| u.movement.max_speed.to_f32());
        miners.0.insert(
            *id,
            Work {
                phase: *phase,
                node: *node,
                dock: dock.map(|d| to_bevy(d.to_f32())),
                sim0: here.unwrap_or(Vec3::ZERO),
                since: now,
                weapon,
                speed,
                angle0,
                radius0,
                from_offset,
                next_shot: now,
            },
        );
    }
}

/// Poses the miners: the orbit, the berth, the fade, and the beam's shots.
#[allow(clippy::too_many_arguments)]
fn animate(
    mut miners: ResMut<Miners>,
    mut offsets: ResMut<VisualOffsets>,
    link: Res<SimLink>,
    ents: Res<SimEntities>,
    clock: Res<AnimClock>,
    mut q: Query<&mut Transform, With<crate::sim_link::SimEntity>>,
    mut shots: MessageWriter<MiningShot>,
) {
    let now = clock.now();
    let alpha = clock.alpha();
    let offsets_prev = std::mem::take(&mut offsets.0);
    miners.0.retain(|id, w| {
        let Some(sim) = point(&link, *id, alpha) else { return false };
        // The sim does not move a miner while it works, nor lets the resource go and the
        // miner stay: either way the work is over (a stop, a move, a new order, a mined-out
        // node), and the model fades back to where the sim has it.
        let node_at = point(&link, w.node, alpha);
        let working = matches!(w.phase, MinerPhase::Mining | MinerPhase::Unloading);
        if working && (sim.distance(w.sim0) > MOVED || (w.phase == MinerPhase::Mining && node_at.is_none())) {
            w.phase = MinerPhase::Idle;
            w.from_offset = offsets_prev.get(id).copied().unwrap_or(Vec3::ZERO);
            w.since = now;
        }
        let t = now - w.since;
        let blend = smooth(t / BLEND);
        let mut face = None;
        let offset = match (w.phase, node_at, w.weapon.as_ref()) {
            (MinerPhase::Mining, Some(node), Some(wp)) if wp.range > 1.0 && w.speed > 0.0 => {
                let radius = w.radius0 + (wp.range - w.radius0) * blend;
                let angle = w.angle0 + (w.speed / wp.range) * t as f32;
                let at = Vec3::new(node.x + radius * angle.cos(), sim.y, node.z + radius * angle.sin());
                // Round in the render space's +angle direction (the z mirror makes that the
                // clockwise one in the game's); a choice, the files do not say. Nose along the circle.
                face = Some(Vec3::new(-angle.sin(), 0.0, angle.cos()));
                if now >= w.next_shot {
                    shots.write(MiningShot { shooter: *id, weapon: wp.key, node: w.node });
                    w.next_shot = now + wp.refire.max(1) as f64 / TICK_HZ as f64;
                }
                (at - sim) * blend + w.from_offset * (1.0 - blend)
            }
            (MinerPhase::Unloading, _, _) if w.dock.is_some() => (w.dock.unwrap_or(sim) - sim) * blend + w.from_offset * (1.0 - blend),
            _ => w.from_offset * (1.0 - blend),
        };
        // Mining and unloading hold their offset; the rest fade and go.
        let holds = matches!(w.phase, MinerPhase::Mining | MinerPhase::Unloading) && (w.weapon.is_some() || w.dock.is_some());
        if !holds && t >= BLEND {
            return false;
        }
        if let Some(e) = ents.0.get(id)
            && let Ok(mut tf) = q.get_mut(*e)
        {
            tf.translation += offset;
            if let Some(f) = face {
                tf.rotation = Transform::IDENTITY.looking_to(f, Vec3::Y).rotation;
            }
        }
        offsets.0.insert(*id, offset);
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::render::fx::weapons::{FxKind, WeaponFx, WeaponFxPlugin};
    use crate::sim_link::{SimEntity, SimLinkPlugin};
    use bevy::time::TimeUpdateStrategy;
    use sta_content::{ContentBuilder, DefRef, Hardpoint, MoveStats, WeaponDef, WeaponKind};
    use sta_sim::PlayerId;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use std::time::Duration;

    const RANGE: f32 = 180.0;

    /// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`): every miner of the install, the
    /// mining weapon it has and where its orbit radius
    /// (weapon `range`), speed (`maxSpeed`) and beam (`shotDelay`, ordnance `lifespan`)
    /// come from; the berth is the stated fallback (no key names one). Run with
    /// `--nocapture` for the listing.
    #[test]
    fn install_lists_each_miners_orbit_beam_and_berth() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (mut miners, mut armed) = (0, 0);
        for u in db.units().filter(|u| u.miner) {
            miners += 1;
            let found: Vec<MiningWeapon> = mining_weapon(&db, u).into_iter().collect();
            armed += usize::from(!found.is_empty());
            let line: Vec<String> = found
                .iter()
                .map(|w| {
                    let d = db.weapon(w.key).unwrap();
                    format!("{} (range {} -> orbit radius, shotDelay {} ticks, lifespan {} ticks)", d.odf, w.range, w.refire, d.duration)
                })
                .collect();
            println!(
                "{}: speed {:.0} (maxSpeed); mining weapons: {}; berth: station origin (fallback, no key names one)",
                u.odf,
                u.movement.max_speed.to_f32(),
                if line.is_empty() { "none: no orbit, no beam".to_owned() } else { line.join(", ") }
            );
        }
        println!("{miners} miners, {armed} with a mining beam");
        assert!(miners == 0 || armed > 0, "no miner has a weapon whose ordnance is a mining beam");
    }

    /// A freighter with a mining beam and a phaser, a moon and a station.
    fn db() -> ContentDb {
        let mut mine = WeaponDef::new("m_mine", "minebm", WeaponKind::Beam);
        mine.range = sta_sim::Fixed::from_int(RANGE as i32);
        mine.refire = 45;
        mine.duration = 37;
        let mut other = WeaponDef::new("m_other", "phaser", WeaponKind::Beam);
        other.range = sta_sim::Fixed::from_int(500);
        let mut miner = UnitDef::new("m_miner", "freighter");
        miner.movement = MoveStats::from_f32(60.0, 1.0, 1.0, 1.0, 1.0);
        miner.hardpoints = vec![
            Hardpoint { weapon: DefRef::new("m_other"), node: "hp01".into(), arc: None },
            Hardpoint { weapon: DefRef::new("m_mine"), node: "hp02".into(), arc: None },
        ];
        ContentBuilder::default()
            .weapon(mine)
            .weapon(other)
            .unit(miner)
            .unit(UnitDef::new("m_moon", "planet"))
            .unit(UnitDef::new("m_station", "starbase"))
            .build()
            .unwrap()
    }

    #[test]
    fn the_mining_weapon_is_the_hardpoint_with_a_mining_beam() {
        let db = db();
        let miner = db.unit(db.key("m_miner").unwrap()).unwrap();
        let w = mining_weapon(&db, miner).unwrap();
        assert_eq!((w.hardpoint, w.range, w.refire), (1, RANGE, 45));
        assert_eq!(db.weapon(w.key).unwrap().odf, "m_mine");
        let moon = db.unit(db.key("m_moon").unwrap()).unwrap();
        assert!(mining_weapon(&db, moon).is_none(), "no mining beam: no orbit");
    }

    /// Scripted cycle: mining from tick 10, the load carried off at tick 200, unloading at
    /// the station from tick 230.
    fn cycle(miner_rails: &str) -> (App, Entity) {
        let db = db();
        let text = format!(
            "{{ units: [
                {{ id: 1, def: \"m_moon\", owner: 0, hull: 1, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
                {{ id: 2, def: \"m_miner\", owner: 0, hull: 1, rails: [{miner_rails}] }},
                {{ id: 3, def: \"m_station\", owner: 0, hull: 1, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}] }},
            ], events: [
                {{ at: 10, event: {{ MinerPhase: {{ id: 2, phase: \"Mining\", node: 1, dock: null }} }} }},
                {{ at: 200, event: {{ MinerPhase: {{ id: 2, phase: \"ToDock\", node: 1, dock: null }} }} }},
                {{ at: 230, event: {{ MinerPhase: {{ id: 2, phase: \"Unloading\", node: 1, dock: {{ x: 0, y: 0, z: 600 }} }} }} }},
                {{ at: 300, event: {{ MinerPhase: {{ id: 2, phase: \"ToNode\", node: 1, dock: null }} }} }},
            ] }}"
        );
        let src = ScriptedSource::from_json5(&text, &db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / TICK_HZ as f64)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(ContentRes(std::sync::Arc::new(db)))
            .insert_resource(SimLink::new(Box::new(src), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_plugins(WeaponFxPlugin)
            .add_systems(Update, crate::render::units::place.in_set(GameSet::Interpolate));
        configure_sets(&mut app);
        let e = app.world_mut().spawn((SimEntity(EntityId(2)), Transform::IDENTITY)).id();
        app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(2), e);
        for (id, x, z) in [(1, 0.0, 0.0), (3, 0.0, 600.0)] {
            let t = app.world_mut().spawn((SimEntity(EntityId(id)), Transform::from_translation(to_bevy([x, 0.0, z])))).id();
            app.world_mut().resource_mut::<SimEntities>().0.insert(EntityId(id), t);
        }
        (app, e)
    }

    fn xz(app: &App, e: Entity) -> Vec2 {
        let t = app.world().get::<Transform>(e).unwrap().translation;
        Vec2::new(t.x, t.z)
    }

    /// A move, a stop or a new order end mining without announcing it: the sim takes the miner
    /// off its spot, and the model goes with it instead of circling the old resource.
    #[test]
    fn a_miner_the_sim_moves_away_stops_orbiting() {
        // Still until tick 100, then 250 units away at tick 101; mining announced at tick 10.
        let (mut app, e) = cycle("{ at: 0, pos: { x: 150, y: 0, z: 0 } }, { at: 100, pos: { x: 150, y: 0, z: 0 } }, { at: 101, pos: { x: 400, y: 0, z: 0 } }");
        let mut orbited = false;
        for frame in 0..160 {
            app.update();
            if frame == 90 {
                orbited = (xz(&app, e).length() - RANGE).abs() < 1.5;
            }
        }
        assert!(orbited, "it was orbiting before the move");
        // Faded back (a second) to the sim's place, x = 400, not still on the old circle.
        let p = xz(&app, e);
        assert!(p.distance(Vec2::new(400.0, 0.0)) < 3.0, "at {p:?}");
        let mut q = app.world_mut().query::<&WeaponFx>();
        let late: Vec<_> = q.iter(app.world()).filter(|f| f.kind == FxKind::Beam).collect();
        assert!(late.is_empty(), "{} beams still at the old resource", late.len());
    }

    #[test]
    fn a_miner_orbits_at_its_weapons_range_beams_the_resource_and_berths_at_the_station() {
        let (mut app, e) = cycle("{ at: 0, pos: { x: 150, y: 0, z: 0 } }");
        let mut radii = Vec::new();
        let mut angles = Vec::new();
        let mut beam_from_model = 0;
        let mut beams = 0;
        for frame in 0..420 {
            app.update();
            if (70..190).contains(&frame) {
                let p = xz(&app, e);
                radii.push(p.length());
                angles.push(p.to_angle());
                // The beam leaves the model where it is drawn (the shooter's end of the ribbon).
                let mut q = app.world_mut().query::<&WeaponFx>();
                for f in q.iter(app.world()).filter(|f| f.kind == FxKind::Beam && f.shooter == EntityId(2)) {
                    beams += 1;
                    beam_from_model += usize::from(f.origin().distance(Vec3::new(p.x, 0.0, p.y)) < 1.0);
                }
            }
            if frame == 260 {
                // Unloading: at the station (render space mirrors z).
                let p = xz(&app, e);
                assert!(p.distance(Vec2::new(0.0, -600.0)) < 5.0, "berthed at {p:?}");
            }
        }
        // On the circle of the weapon's range, going round.
        assert!(radii.iter().all(|r| (r - RANGE).abs() < 1.5), "radii {:?}", &radii[..5]);
        assert!(angles.windows(2).filter(|w| (w[1] - w[0]).abs() > 1e-3).count() > 100, "it did not move");
        // A beam of the mining weapon, from the model.
        assert!(beams > 0 && beam_from_model == beams, "{beam_from_model} of {beams} beams left the model");
        // Mining over, ToNode: back on its sim place, 150 from the moon.
        let p = xz(&app, e);
        assert!((p.length() - 150.0).abs() < 1.0, "back at {p:?}");
    }
}
