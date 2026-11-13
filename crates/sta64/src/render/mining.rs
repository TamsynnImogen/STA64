//! Mining ships while they mine (RND-29, issue #345).
//!
//! The sim moves a freighter for real: it circles its resource while it works (`economy`,
//! the point on the circle of its mining weapon's range, as the original steers) and
//! announces each phase ([`SimEvent::MinerPhase`], also when a stop, a move or a new order
//! ends the work). What is drawn on top, from the game files:
//!
//! - **Beam.** For as long as the sim says the freighter is in its `Mining` phase, its mining
//!   weapon fires at the resource every `shotDelay` (the weapon's `refire`): a [`MiningShot`] for
//!   [`weapons`](super::fx::weapons), drawn with the weapon's recipe (by ordnance class) for the
//!   ordnance `lifeSpan` (1.25 s in the installs, so the beam is on about five sixths of the
//!   time, as in the original). It is a message of its own, not a `SimEvent::Fired`, so combat
//!   music and weapon animations do not take it for a fight.
//! - **Dock.** While unloading the freighter glides to the station's position (the station the
//!   sim named in the event). The original docks at the station's `dockingHardpoint` node
//!   (default SOD node `dock`); that node is not read yet, the fallback is the station origin.
//!
//! Evidence grade: the orbit, the pulse and the dock node were read from the executable and the
//! ODFs (static); the on-screen orbit direction, speed and which hardpoint fires are not
//! measured. The blend time is a cosmetic choice.

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
    /// Weapon `range`.
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
    /// Anim-clock time the phase began.
    since: f64,
    weapon: Option<MiningWeapon>,
    /// The offset the model had when the phase began.
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
    for SimMsg(ev) in msgs.read() {
        let SimEvent::MinerPhase { id, phase, node, dock } = ev else { continue };
        let db = &content.0;
        let unit = link.curr().unit(*id).and_then(|u| db.unit(u.def));
        let weapon = unit.and_then(|u| mining_weapon(db, u));
        let from_offset = offsets.0.get(id).copied().unwrap_or(Vec3::ZERO);
        miners.0.insert(
            *id,
            Work {
                phase: *phase,
                node: *node,
                dock: dock.map(|d| to_bevy(d.to_f32())),
                since: now,
                weapon,
                from_offset,
                next_shot: now,
            },
        );
    }
}

/// Poses the miners: the beam's shots while mining, the berth while unloading, the fade.
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
        // A resource that is gone ends the beam; the sim announces the next phase itself.
        let node_there = point(&link, w.node, alpha).is_some();
        if w.phase == MinerPhase::Mining && !node_there {
            w.phase = MinerPhase::Idle;
            w.from_offset = offsets_prev.get(id).copied().unwrap_or(Vec3::ZERO);
            w.since = now;
        }
        let t = now - w.since;
        let blend = smooth(t / BLEND);
        if let (MinerPhase::Mining, Some(wp)) = (w.phase, w.weapon.as_ref())
            && now >= w.next_shot
        {
            shots.write(MiningShot { shooter: *id, weapon: wp.key, node: w.node });
            w.next_shot = now + wp.refire.max(1) as f64 / TICK_HZ as f64;
        }
        let offset = match (w.phase, w.dock) {
            (MinerPhase::Unloading, Some(dock)) => (dock - sim) * blend + w.from_offset * (1.0 - blend),
            _ => w.from_offset * (1.0 - blend),
        };
        // Mining holds no offset; unloading holds the berth; the rest fade and go.
        let holds = (w.phase == MinerPhase::Unloading && w.dock.is_some()) || (w.phase == MinerPhase::Mining && w.weapon.is_some());
        if !holds && t >= BLEND {
            return false;
        }
        if let Some(e) = ents.0.get(id)
            && let Ok(mut tf) = q.get_mut(*e)
        {
            tf.translation += offset;
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
                    format!("{} (range {}, shotDelay {} ticks, lifespan {} ticks)", d.odf, w.range, w.refire, d.duration)
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

    /// The beam is up for as long as the sim says the freighter mines (shot after shot, not
    /// once), starts at the drawn model, and is gone soon after the phase ends; the freighter
    /// is drawn where the sim has it while it mines.
    #[test]
    fn the_beam_is_up_while_the_sim_says_mining_and_gone_otherwise() {
        let (mut app, e) = cycle("{ at: 0, pos: { x: 150, y: 0, z: 0 } }");
        let (mut frames_with_beam, mut shots_seen, mut was) = (0, 0, false);
        let mut from_model = 0;
        for frame in 0..420 {
            app.update();
            let mut q = app.world_mut().query::<&WeaponFx>();
            let p = xz(&app, e);
            let beams: Vec<_> = q.iter(app.world()).filter(|f| f.kind == FxKind::Beam && f.shooter == EntityId(2)).collect();
            if (30..190).contains(&frame) {
                // Mining from tick 10 to 200 (30 Hz, one frame a tick): pulsed, never over for long.
                frames_with_beam += usize::from(!beams.is_empty());
                shots_seen += usize::from(!beams.is_empty() && !was);
                from_model += beams.iter().filter(|f| f.origin().distance(Vec3::new(p.x, 0.0, p.y)) < 1.0).count();
                // The sim keeps the miner; nothing is drawn off it while it mines.
                assert!((p.length() - 150.0).abs() < 1.0, "drawn at {p:?}");
            }
            was = !beams.is_empty();
            if frame == 300 {
                assert!(beams.is_empty(), "beam after the mining ended");
            }
            if frame == 260 {
                assert!(p.distance(Vec2::new(0.0, -600.0)) < 5.0, "berthed at {p:?}");
            }
        }
        assert!(frames_with_beam > 100, "{frames_with_beam} of 160 frames with a beam");
        assert!(shots_seen >= 3, "{shots_seen} separate shots");
        assert!(from_model > 0);
        let p = xz(&app, e);
        assert!((p.length() - 150.0).abs() < 1.0, "back at {p:?}");
    }
}
