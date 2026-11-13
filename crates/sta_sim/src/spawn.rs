//! Entities into and out of the world (SIM-01): the map at tick 0, and the
//! spawn and kill edits of the bench and missions.
//!
//! - [`Sim::from_setup`] spawns the map's `objects` in map order, resolving
//!   ODF names through the setup's [`ContentDb`]. Names that are not a unit
//!   definition are skipped with a [`Warning`], the same way on every peer.
//! - Start positions spawn nothing yet: what a seat starts with (a starbase,
//!   construction ships) depends on race and game options no package defines
//!   yet. They and the regions are kept for the systems that use them.
//! - `scenery` is never read.
//! - Every spawn emits [`SimEvent::Spawned`] and every removal
//!   [`SimEvent::Destroyed`], with the next step's output.

use std::sync::Arc;

use sta_content::{ContentDb, Region, StartPos, Warning};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3};

use crate::command::{Edit, Subsystem};
use crate::setup::GameSetup;
use crate::view::UnitFlags;
use crate::world::{Entity, NEUTRAL};
use crate::{Sim, SimEvent};

impl Sim {
    /// [`Sim::from_setup`], with the map objects it skipped.
    pub fn from_setup_with_warnings(s: &GameSetup) -> (Sim, Vec<Warning>) {
        let mut sim = Sim::new(s.seed);
        let mut players: Vec<_> = s.players.iter().map(|p| (p.id, p.team)).collect();
        players.sort();
        players.dedup_by_key(|p| p.0);
        sim.players = players;
        sim.appearance_factions = s.players.iter().filter_map(|p| {
            let name = p.race.trim().to_ascii_lowercase();
            (!name.is_empty()).then_some((p.id, name))
        }).collect();
        sim.appearance_factions.sort_by_key(|p| p.0);
        sim.appearance_factions.dedup_by_key(|p| p.0);
        sim.ai_seats = s
            .players
            .iter()
            .filter(|p| matches!(p.control, crate::setup::Control::Ai(_) | crate::setup::Control::RemoteAi(_)))
            .map(|p| p.id)
            .collect();
        sim.ai_seats.sort();
        sim.ai_seats.dedup();
        sim.content = Some(Arc::clone(&s.content));
        sim.starts = s.map.starts.clone();
        sim.regions = s.map.regions.clone();
        sim.bounds = Some(s.map.bounds);

        let (confinement, mut warnings) = crate::boundary::state::Confinement::prepare(Some(&s.content), sim.bounds);
        sim.confinement = confinement;
        for o in &s.map.objects {
            let Some(key) = s
                .content
                .key(&o.odf)
                .filter(|&k| s.content.unit(k).is_some())
            else {
                warnings.push(Warning::new(
                    &o.odf,
                    format!("{}: not a unit definition; not spawned", s.map.name),
                ));
                continue;
            };
            let owner = o.owner.map_or(NEUTRAL, PlayerId);
            let id = sim
                .spawn_def(key, owner, o.pos, o.heading)
                .expect("key resolved to a unit");
            sim.confinement.map_loaded(sim.units.get_mut(id).expect("spawned"));
            if let Some(label) = &o.label {
                sim.labels.insert(id, label.clone());
            }
        }
        (sim, warnings)
    }

    /// Spawn an entity at the definition's starting hull and shield strengths.
    /// `None` without content, or when `key` is not a unit.
    pub fn spawn_def(
        &mut self,
        key: ContentKey,
        owner: PlayerId,
        pos: Vec3,
        heading: (Fixed, Fixed),
    ) -> Option<EntityId> {
        let content = self.content.as_ref()?;
        let def = content.unit(key)?;
        let id = self.units.spawn(Entity::from_def(
            def,
            content.features(),
            owner,
            pos,
            heading,
        ));
        self.confinement.initialize(self.units.get_mut(id).expect("spawned"));
        self.events.push(SimEvent::Spawned { id });
        self.register_lifecycle(id);
        self.default_autonomy(id);
        Some(id)
    }

    /// Give a new craft the content's default autonomy (SIM-36, D9).
    pub(crate) fn default_autonomy(&mut self, id: EntityId) {
        let Some(db) = self.content.as_deref() else { return };
        let a = db.rules().autonomy();
        self.autonomy.spawned(&self.units, id, [a.movement, a.special_weapon]);
    }

    /// Take an entity out of the world, with everything kept about it.
    /// `by`: who destroyed it, if anyone. False when there is no such entity.
    ///
    /// Packages that keep a `Table` of their own drop its row here.
    pub fn despawn(&mut self, id: EntityId, by: Option<EntityId>) -> bool {
        self.remove_entity(id, Some(by))
    }

    /// [`Sim::despawn`] without the [`SimEvent::Destroyed`]: a start helper
    /// that ends its lifecycle (expiry, replacement) did not die, so notices,
    /// music and effects must not react (SIM-28).
    pub(crate) fn retire(&mut self, id: EntityId) -> bool {
        self.remove_entity(id, None)
    }

    fn remove_entity(&mut self, id: EntityId, destroyed_by: Option<Option<EntityId>>) -> bool {
        if self.units.remove(id).is_none() {
            return false;
        }
        self.labels.remove(id);
        self.confinement.remove(id);
        self.perception.remove(id);
        self.autonomy.remove(id);
        self.approach_warp.remove(id);
        self.watchdog.remove(id);
        self.engagement.remove(id);
        self.production.remove_rally_entity(id);
        self.production.remove_weld_site(id);
        self.start_effects.remove(id);
        self.subsystems.remove(id);
        self.attrs.remove(id);
        self.scripted.remove(id);
        self.switches.remove(id);
        if let Some(by) = destroyed_by {
            self.events.push(SimEvent::Destroyed { id, by });
        }
        true
    }

    /// Apply an edit from `from`; false (rejected) when `from` may not edit
    /// or the edit does not apply. Only scripts and the bench may edit.
    pub(crate) fn apply_edit(&mut self, from: PlayerId, e: &Edit) -> bool {
        if from != PlayerId::SCRIPT && from != PlayerId::BENCH {
            return false;
        }
        if let Some(ok) = self.apply_switch_edit(e) {
            return ok;
        }
        match *e {
            Edit::Spawn {
                odf,
                player,
                pos,
                heading,
            } => self.spawn_def(odf, player, pos, heading).is_some(),
            Edit::SpawnRequest { request, odf, player, pos, heading } => {
                if from != PlayerId::SCRIPT {
                    return false;
                }
                let Some(id) = self.spawn_def(odf, player, pos, heading) else { return false };
                self.events.push(SimEvent::ScriptSpawned { request, id });
                true
            }
            Edit::Kill(id) => self.despawn(id, None),
            Edit::Team(id, to) => self.transfer_owner(from, id, to),
            Edit::PerceivedTeam(id, to) => self.set_perceived_owner(id, to),
            Edit::SetRelation { from, to, relation } => {
                let seat = |p: PlayerId| self.players.iter().any(|s| s.0 == p);
                if from == to || !seat(from) || !seat(to) {
                    return false;
                }
                let default = crate::relations::default_view(&self.players, from, to);
                let before = crate::relations::Diplomacy::new(&self.players, &self.relations).view(from, to);
                self.relations.set(from, to, relation, default);
                if before != relation {
                    // A commanded attack ends when the shooter's view of the
                    // target's perceived owner changes (read from the executable,
                    // static); auto fire re-checks the relation every tick anyway.
                    let dropped: Vec<EntityId> = self
                        .weapons
                        .attacks()
                        .into_iter()
                        .filter(|&(shooter, target)| {
                            self.units.get(shooter).is_some_and(|u| u.owner == from)
                                && self.units.get(target).is_some_and(|t| self.perception.get(t.id).copied().unwrap_or(t.owner) == to)
                        })
                        .map(|(shooter, _)| shooter)
                        .collect();
                    self.weapons.stop_attack(&self.units, PlayerId::SCRIPT, &dropped);
                    for id in dropped {
                        if let Some(u) = self.units.get_mut(id).filter(|u| u.kind == crate::world::EntityKind::Craft) {
                            u.target = None;
                        }
                        self.approach_warp.end(id);
                    }
                }
                true
            }
            Edit::ForceOntoMap { id, enabled } => self.units.get(id).is_some_and(|u| self.confinement.set(u, enabled)),
            Edit::Damage(id, amount) => {
                // Straight to the hull, past the shields; lethal unless immortal.
                let Some(u) = self.units.get_mut(id).filter(|u| u.hull[0] > Fixed::ZERO) else {
                    return false;
                };
                let floor = if u.flags.contains(UnitFlags::IMMORTAL) {
                    Fixed::ONE
                } else {
                    Fixed::ZERO
                };
                u.hull[0] = (u.hull[0] - amount).max(floor).min(u.hull[0]);
                if u.hull[0] <= Fixed::ZERO {
                    return self.despawn(id, None);
                }
                true
            }
            Edit::Hull(id, to) => {
                let Some(u) = self.units.get_mut(id).filter(|u| u.hull[0] > Fixed::ZERO) else {
                    return false;
                };
                let floor = if u.flags.contains(UnitFlags::IMMORTAL) {
                    Fixed::ONE
                } else {
                    Fixed::ZERO
                };
                u.hull[0] = to.max(floor).min(u.hull[1]);
                if u.hull[0] <= Fixed::ZERO {
                    return self.despawn(id, None);
                }
                true
            }
            Edit::Heal(id) => {
                let Some(u) = self.units.get_mut(id) else {
                    return false;
                };
                u.hull[0] = u.hull[1];
                u.shields = crate::world::Shields::full(u.shields.max, u.shields.directional);
                self.subsystems.repair(id);
                true
            }
            Edit::Immortal(id, on) => self.set_flag(id, UnitFlags::IMMORTAL, on),
            Edit::SetEnergy(id, to) => {
                let Some(db) = self.content.as_deref() else {
                    return false;
                };
                self.specials.set_energy(&self.units, db, id, to)
            }
            Edit::Engines(id, on) => {
                let Some(u) = self.units.get_mut(id) else {
                    return false;
                };
                if !on {
                    u.speed = Fixed::ZERO;
                    u.omega = Fixed::ZERO;
                    u.target = None;
                }
                u.flags = u.flags.with(UnitFlags::ENGINES_OFF, !on);
                true
            }
            Edit::Weapons(id, on) => self.set_flag(id, UnitFlags::WEAPONS_OFF, !on),
            Edit::Present(ref p) => {
                // Changes nothing; every client and replay sees the event.
                self.events.push(SimEvent::Present(p.clone()));
                true
            }
            Edit::Subsystem(id, Subsystem::Engines, on) => {
                self.apply_edit(from, &Edit::Engines(id, on))
            }
            Edit::Subsystem(id, Subsystem::Weapons, on) => {
                self.apply_edit(from, &Edit::Weapons(id, on))
            }
            Edit::Subsystem(id, sub, on) => self.edit_subsystem_switch(id, sub, on),
            Edit::SubsystemDamage(id, sub, points) => self.edit_subsystem_damage(id, sub, points),
            Edit::ShieldPercent(id, fraction) => self.edit_shield_percent(id, fraction),
            Edit::MaxShields(id, max) => self.edit_max_shields(id, max),
            Edit::MaxOfficers(player, value) => self.players.iter().any(|(id, _)| *id == player) && self.economy.edit_officer_limit(player, value),
            Edit::TeamCrew(player, value) => self.players.iter().any(|(id, _)| *id == player) && self.economy.edit_team_crew(player, value),
            Edit::UnitCrew(id, value) => {
                let Some(u) = self.units.get_mut(id).filter(|u| u.hull[0] > Fixed::ZERO && matches!(u.kind, crate::world::EntityKind::Craft | crate::world::EntityKind::Station)) else {
                    return false;
                };
                let Some([now, max]) = u.crew else { return false };
                // Cap: the current maximum crew (read from the executable, static).
                let _ = now;
                u.crew = Some([value.max(Fixed::ZERO).min(max), max]);
                true
            }
            Edit::MaxHull(id, v) => self.edit_max_hull(id, v),
            Edit::MaxCrew(id, v) => self.edit_max_crew(id, v),
            Edit::HullValue(id, v) => self.edit_hull_value(id, v),
            Edit::MaxShieldsKeep(id, v) => self.edit_max_shields_keep(id, v),
            Edit::UnitRace(id, ref race) => self.edit_unit_race(id, race),
            Edit::PlayerRace(p, ref race) => self.edit_player_race(p, race),
            Edit::Reserved44 => false,
            Edit::Position(id, to) => self.edit_position(id, to),
            Edit::Heading(id, dir) => self.edit_heading(id, dir),
            Edit::AngularVelocity(id, omega) => self.edit_angular_velocity(id, omega),
            // The rest land with the package that owns what they change.
            _ => false,
        }
    }

    fn set_flag(&mut self, id: EntityId, flag: UnitFlags, on: bool) -> bool {
        let Some(u) = self.units.get_mut(id) else {
            return false;
        };
        u.flags = u.flags.with(flag, on);
        true
    }

    /// The content this sim resolves spawns against.
    pub fn content(&self) -> Option<&Arc<ContentDb>> {
        self.content.as_ref()
    }

    /// The map's start positions, as the setup gave them.
    pub fn starts(&self) -> &[StartPos] {
        &self.starts
    }

    /// The map's regions, as the setup gave them.
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
}

#[cfg(test)]
#[path = "spawn/request_tests.rs"]
mod request_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldSource;
    use crate::command::{Command, CommandKind, Order};
    use crate::setup::{Control, GameOptions, PlayerSetup};
    use crate::view::UnitFlags;
    use crate::world::EntityKind;
    use sta_content::{MapDef, MapObject};
    use sta_core::TeamId;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::from_int(z),
        }
    }

    fn setup(map: MapDef) -> GameSetup {
        let seat = |id, control| PlayerSetup {
            id: PlayerId(id),
            team: TeamId(id),
            race: "fx".into(),
            control,
        };
        GameSetup {
            seed: 11,
            map: Arc::new(map),
            content: Arc::new(sta_content::fake::db()),
            players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
            options: GameOptions::default(),
        }
    }

    fn object(odf: &str, owner: Option<u8>, pos: Vec3) -> MapObject {
        MapObject {
            odf: odf.into(),
            owner,
            pos,
            heading: (Fixed::ONE, Fixed::ZERO),
            label: None,
        }
    }

    fn cmd(player: PlayerId, seq: u32, kind: CommandKind) -> Command {
        Command { player, seq, kind }
    }

    fn spawn(seq: u32, db: &ContentDb, odf: &str, player: u8, pos: Vec3) -> Command {
        let e = Edit::Spawn {
            odf: db.key(odf).unwrap(),
            player: PlayerId(player),
            pos,
            heading: (Fixed::ZERO, Fixed::ONE),
        };
        cmd(PlayerId::BENCH, seq, CommandKind::Edit(e))
    }

    fn mv(player: u8, seq: u32, units: &[u32], to: Vec3) -> Command {
        let units = units.iter().map(|&i| EntityId(i)).collect();
        cmd(
            PlayerId(player),
            seq,
            CommandKind::Order(Order::Move {
                units,
                to,
                queue: false,
            }),
        )
    }

    /// SIM-18: a resource placed with a team (or spawned for a seat) is neutral.
    #[test]
    fn resources_spawn_neutral() {
        let mut map = sta_content::fake::two_bases();
        for o in &mut map.objects {
            o.owner = Some(0);
        }
        let s = setup(map);
        let mut sim = Sim::from_setup(&s);
        let moon = s.content.key("fx_moon").unwrap();
        sim.spawn_def(moon, PlayerId(1), v(5, 0, 5), (Fixed::ONE, Fixed::ZERO))
            .unwrap();
        let view = sim.view();
        let moons: Vec<_> = view.units.iter().filter(|u| u.def == moon).collect();
        assert_eq!(moons.len(), 2);
        assert!(moons.iter().all(|u| u.owner == NEUTRAL));
    }

    /// `two_bases` places one object, the moon; the starts spawn nothing.
    #[test]
    fn two_bases_spawns_its_objects() {
        let s = setup(sta_content::fake::two_bases());
        let (sim, warnings) = Sim::from_setup_with_warnings(&s);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(sim.units.len(), 1);
        let moon = sim.unit(EntityId(1)).unwrap();
        assert_eq!(moon.kind, EntityKind::Resource);
        assert_eq!(moon.def, s.content.key("fx_moon").unwrap());
        assert_eq!(moon.owner, NEUTRAL);
        assert_eq!(moon.pos, Vec3::ZERO);
        assert!(!moon.mobile() && !moon.orderable());
        assert_eq!(sim.labels.get(moon.id).map(String::as_str), Some("moon"));
        assert_eq!(sim.starts().len(), 2);
        assert_eq!(sim.regions().len(), 2);
        assert_eq!(sim.content().unwrap().hash(), s.content.hash());
        assert_eq!(Sim::from_setup(&s).digest(), sim.digest());
    }

    /// Owned objects of every kind, in map order; unknown names and
    /// non-units skipped with a warning each.
    #[test]
    fn map_objects_get_kinds_owners_and_defs() {
        let mut map = sta_content::fake::two_bases();
        map.objects.extend([
            object("fx_station", Some(0), v(-4000, 0, 0)),
            object("FX_Yard", Some(1), v(4000, 0, 0)),
            object("fx_nothing", Some(0), v(0, 0, 0)),
            object("fx_cruiser", Some(0), v(-3800, 0, 200)),
            object("fx_beam", None, v(0, 0, 0)),
            object("fx_miner", Some(1), v(3800, 0, -200)),
        ]);
        let s = setup(map);
        let db = &s.content;
        let (sim, warnings) = Sim::from_setup_with_warnings(&s);
        let skipped: Vec<_> = warnings.iter().map(|w| w.file.as_str()).collect();
        assert_eq!(skipped, ["fx_nothing", "fx_beam"]);
        let got: Vec<_> = sim
            .units
            .iter()
            .map(|e| (e.id.0, e.kind, e.def, e.owner))
            .collect();
        let want = [
            (1, EntityKind::Resource, "fx_moon", NEUTRAL),
            (2, EntityKind::Station, "fx_station", PlayerId(0)),
            (3, EntityKind::Station, "fx_yard", PlayerId(1)),
            (4, EntityKind::Craft, "fx_cruiser", PlayerId(0)),
            (5, EntityKind::Craft, "fx_miner", PlayerId(1)),
        ]
        .map(|(id, kind, odf, owner)| (id, kind, db.key(odf).unwrap(), owner));
        assert_eq!(got, want);
        let yard = sim.unit(EntityId(3)).unwrap();
        assert_eq!(yard.hull, [Fixed::from_int(2000); 2]);
        assert_eq!(yard.shields.view(), [[Fixed::from_int(800); 2]; 4]);
    }

    /// Scenery is render-only: changing it changes nothing in the sim.
    #[test]
    fn scenery_never_enters_the_sim() {
        let plain = setup(sta_content::fake::two_bases());
        let mut map = sta_content::fake::two_bases();
        map.scenery = Default::default();
        let bare = setup(map);
        let (mut a, mut b) = (Sim::from_setup(&plain), Sim::from_setup(&bare));
        assert_eq!(a.view(), b.view());
        for _ in 0..10 {
            assert_eq!(
                WorldSource::step(&mut a, &[]),
                WorldSource::step(&mut b, &[])
            );
        }
    }

    /// Peers with different content differ from tick 0.
    #[test]
    fn content_hash_is_in_the_first_digest() {
        let a = setup(MapDef::empty());
        let mut b = a.clone();
        b.content = Arc::new(ContentDb::builder().build().unwrap());
        assert_ne!(a.content.hash(), b.content.hash());
        assert_ne!(Sim::from_setup(&a).digest(), Sim::from_setup(&b).digest());
    }

    /// A scripted game on `two_bases`: the bench sets up both bases, the
    /// players fly, a ship is added mid-game and one is killed. Returns the
    /// digest at tick 300 and every event with its tick.
    fn scripted_game() -> (u64, Vec<(u64, SimEvent)>) {
        let s = setup(sta_content::fake::two_bases());
        let db = Arc::clone(&s.content);
        let mut sim = Sim::from_setup(&s);
        let mut log = Vec::new();
        for t in 0..300 {
            // Ids: 1 the moon, then in spawn order.
            let cmds = match t {
                0 => vec![
                    spawn(0, &db, "fx_station", 0, v(-4000, 0, 0)),    // 2
                    spawn(1, &db, "fx_cruiser", 0, v(-3800, 0, 200)),  // 3
                    spawn(2, &db, "fx_frigate", 0, v(-3800, 0, -200)), // 4
                    spawn(3, &db, "fx_yard", 1, v(4000, 0, 0)),        // 5
                    spawn(4, &db, "fx_cruiser", 1, v(3800, 0, 0)),     // 6
                ],
                1 => vec![
                    mv(0, 0, &[3, 4], v(-1000, 20, 500)),
                    mv(1, 0, &[6], v(1000, -20, -500)),
                ],
                90 => vec![
                    spawn(5, &db, "fx_miner", 1, v(3900, 0, 300)),
                    mv(0, 1, &[4], v(0, 0, 1500)),
                ], // 7
                120 => vec![mv(1, 1, &[7], v(500, 0, 0))],
                150 => vec![cmd(
                    PlayerId::BENCH,
                    6,
                    CommandKind::Edit(Edit::Kill(EntityId(3))),
                )],
                200 => vec![cmd(
                    PlayerId(0),
                    2,
                    CommandKind::Order(Order::Stop {
                        units: vec![EntityId(4)],
                    }),
                )],
                _ => vec![],
            };
            let out = WorldSource::step(&mut sim, &cmds);
            assert_eq!(out.view.digest, sim.digest());
            log.extend(out.events.into_iter().map(|e| (out.view.tick, e)));
        }
        assert_eq!(sim.tick, 300);
        assert_eq!(sim.units.len(), 6);
        assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0x4098_02e0_f981_8a21);
        (sim.digest(), log)
    }

    /// SIM-01's done-when: the same game twice gives the same digest at tick
    /// 300, pinned so any change to the arithmetic or the model shows here.
    #[test]
    fn golden_digest_at_tick_300() {
        let (digest, log) = scripted_game();
        let (again, log2) = scripted_game();
        assert_eq!(digest, again);
        assert_eq!(log, log2);
        let spawned: Vec<_> = log
            .iter()
            .filter_map(|(t, e)| matches!(e, SimEvent::Spawned { .. }).then_some(*t))
            .collect();
        // The moon comes out with the first step, with the five bench spawns;
        // the miner at tick 91.
        assert_eq!(spawned, [1, 1, 1, 1, 1, 1, 91]);
        assert!(log.contains(&(
            151,
            SimEvent::Destroyed {
                id: EntityId(3),
                by: None
            }
        )));
        assert!(
            !log.iter()
                .any(|(_, e)| matches!(e, SimEvent::Rejected { .. })),
            "{log:?}"
        );
        assert_eq!(digest, 0x9620_de0c_9c45_52e4);
    }

    #[test]
    fn ids_are_not_reused_after_a_kill() {
        let s = setup(MapDef::empty());
        let db = Arc::clone(&s.content);
        let mut sim = Sim::from_setup(&s);
        WorldSource::step(
            &mut sim,
            &[
                spawn(0, &db, "fx_cruiser", 0, v(0, 0, 0)),
                spawn(1, &db, "fx_frigate", 1, v(100, 0, 0)),
            ],
        );
        let kill = |seq, id| {
            cmd(
                PlayerId::SCRIPT,
                seq,
                CommandKind::Edit(Edit::Kill(EntityId(id))),
            )
        };
        let out = WorldSource::step(
            &mut sim,
            &[kill(0, 2), spawn(1, &db, "fx_miner", 0, v(0, 0, 100))],
        );
        assert_eq!(
            out.events,
            [
                SimEvent::Destroyed {
                    id: EntityId(2),
                    by: None
                },
                SimEvent::Spawned { id: EntityId(3) }
            ]
        );
        let out = WorldSource::step(
            &mut sim,
            &[kill(0, 1), spawn(1, &db, "fx_miner", 0, v(0, 0, 200))],
        );
        assert_eq!(
            out.events,
            [
                SimEvent::Destroyed {
                    id: EntityId(1),
                    by: None
                },
                SimEvent::Spawned { id: EntityId(4) }
            ]
        );
        let ids: Vec<_> = out.view.units.iter().map(|u| u.id.0).collect();
        assert_eq!(ids, [3, 4]);
    }

    #[test]
    fn bench_edits_damage_heal_immortal_switches() {
        let s = setup(MapDef::empty());
        let db = Arc::clone(&s.content);
        let mut sim = Sim::from_setup(&s);
        WorldSource::step(&mut sim, &[spawn(0, &db, "fx_frigate", 1, v(0, 0, 0))]);
        let id = EntityId(1);
        let e = |seq, e| cmd(PlayerId::BENCH, seq, CommandKind::Edit(e));
        let hull = |sim: &Sim| sim.unit(id).map(|u| u.hull[0].to_f32());
        let out = WorldSource::step(&mut sim, &[e(0, Edit::Damage(id, Fixed::from_int(100)))]);
        assert!(
            !out.events
                .iter()
                .any(|x| matches!(x, SimEvent::Rejected { .. }))
        );
        assert_eq!(hull(&sim), Some(200.0));
        WorldSource::step(&mut sim, &[e(0, Edit::Heal(id))]);
        assert_eq!(hull(&sim), Some(300.0));
        // Immortal: lethal damage leaves one hull; off again, it kills.
        WorldSource::step(
            &mut sim,
            &[
                e(0, Edit::Immortal(id, true)),
                e(1, Edit::Damage(id, Fixed::from_int(999))),
            ],
        );
        assert_eq!(hull(&sim), Some(1.0));
        WorldSource::step(
            &mut sim,
            &[
                e(0, Edit::Weapons(id, false)),
                e(1, Edit::Weapons(id, true)),
            ],
        );
        assert!(!sim.unit(id).unwrap().flags.contains(UnitFlags::WEAPONS_OFF));
        WorldSource::step(
            &mut sim,
            &[
                e(0, Edit::Immortal(id, false)),
                e(1, Edit::Damage(id, Fixed::from_int(5))),
            ],
        );
        assert!(sim.unit(id).is_none());
        let out = WorldSource::step(&mut sim, &[e(0, Edit::Heal(id))]);
        assert!(
            out.events
                .iter()
                .any(|x| matches!(x, SimEvent::Rejected { .. })),
            "no such unit"
        );
    }

    #[test]
    fn absolute_hull_preserves_shield_pools_and_legacy_heal() {
        for directional in [false, true] {
            let s = setup(MapDef::empty());
            let mut sim = Sim::from_setup(&s);
            let id = sim.spawn_def(
                s.content.key("fx_frigate").unwrap(), PlayerId(0), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE),
            ).unwrap();
            let u = sim.units.get_mut(id).unwrap();
            u.hull[0] = Fixed::from_int(100);
            u.shields = crate::world::Shields {
                now: [37, 23, 11, 7].map(Fixed::from_int),
                max: Fixed::from_int(80),
                directional,
            };
            let shields = u.shields;
            // Direct application isolates the edit from normal regeneration.
            for (to, expected) in [(150, 150), (75, 75), (999, 300)] {
                assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Hull(id, Fixed::from_int(to))));
                let u = sim.units.get(id).unwrap();
                assert_eq!(u.hull[0], Fixed::from_int(expected));
                assert_eq!(u.shields, shields);
            }
            assert!(sim.apply_edit(PlayerId::BENCH, &Edit::Heal(id)));
            let u = sim.units.get(id).unwrap();
            assert_eq!(u.hull[0], u.hull[1]);
            assert_eq!(u.shields, crate::world::Shields::full(shields.max, directional));
        }
    }

    #[test]
    fn absolute_hull_caps_immortal_floor_and_rejects_nonlive_targets() {
        let s = setup(MapDef::empty());
        let mut sim = Sim::from_setup(&s);
        let id = sim.spawn_def(
            s.content.key("fx_frigate").unwrap(), PlayerId(0), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE),
        ).unwrap();
        let half = Fixed::ONE / Fixed::from_int(2);
        let u = sim.units.get_mut(id).unwrap();
        u.hull = [half, half];
        u.flags = u.flags.with(UnitFlags::IMMORTAL, true);
        assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Hull(id, Fixed::from_bits(i64::MIN))));
        assert_eq!(sim.units.get(id).unwrap().hull, [half, half]);
        // Even a retained zero-hull row cannot be resurrected by the edit.
        sim.units.get_mut(id).unwrap().hull[0] = Fixed::ZERO;
        assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::Hull(id, Fixed::ONE)));
        assert_eq!(sim.units.get(id).unwrap().hull[0], Fixed::ZERO);
    }

    #[test]
    fn present_passes_through_and_mission_edits() {
        use crate::command::AlertState;
        use crate::present::{Presentation, Spot};
        let s = setup(MapDef::empty());
        let db = Arc::clone(&s.content);
        let mut sim = Sim::from_setup(&s);
        WorldSource::step(&mut sim, &[spawn(0, &db, "fx_frigate", 1, v(0, 0, 0))]);
        let id = EntityId(1);
        let script = |seq, e| cmd(PlayerId::SCRIPT, seq, CommandKind::Edit(e));
        let shot = Presentation::Shot {
            from: Spot::Entity(id),
            at: Spot::Path("intro".into()),
            seconds: Fixed::from_int(3),
        };
        let out = WorldSource::step(
            &mut sim,
            &[
                script(0, Edit::Present(shot.clone())),
                script(1, Edit::Subsystem(id, Subsystem::Engines, false)),
                script(2, Edit::Alert(id, AlertState::Red)),
            ],
        );
        assert!(
            out.events.contains(&SimEvent::Present(shot)),
            "{:?}",
            out.events
        );
        assert!(
            sim.unit(id).unwrap().flags.contains(UnitFlags::ENGINES_OFF),
            "Subsystem(Engines) acts as Engines"
        );
        assert!(
            out.events.contains(&SimEvent::Rejected {
                player: PlayerId::SCRIPT,
                seq: 2
            }),
            "Alert waits for its package"
        );
        WorldSource::step(
            &mut sim,
            &[script(0, Edit::Subsystem(id, Subsystem::Weapons, false))],
        );
        assert!(sim.unit(id).unwrap().flags.contains(UnitFlags::WEAPONS_OFF));
        // Edits are for scripts and the bench: a player cannot present.
        let out = WorldSource::step(
            &mut sim,
            &[cmd(
                PlayerId(0),
                0,
                CommandKind::Edit(Edit::Present(Presentation::StopMusic)),
            )],
        );
        assert!(!out.events.iter().any(|x| matches!(x, SimEvent::Present(_))));
        assert!(out.events.contains(&SimEvent::Rejected {
            player: PlayerId(0),
            seq: 0
        }));
    }

    #[test]
    fn bad_commands_are_rejected() {
        let s = setup(sta_content::fake::two_bases());
        let db = Arc::clone(&s.content);
        let mut sim = Sim::from_setup(&s);
        // 2: player 0's cruiser, 3: player 1's frigate, killed at once.
        WorldSource::step(
            &mut sim,
            &[
                spawn(0, &db, "fx_cruiser", 0, v(0, 0, 0)),
                spawn(1, &db, "fx_frigate", 1, v(100, 0, 0)),
            ],
        );
        WorldSource::step(
            &mut sim,
            &[cmd(
                PlayerId::BENCH,
                2,
                CommandKind::Edit(Edit::Kill(EntityId(3))),
            )],
        );
        let to = v(500, 0, 500);
        let nothing = Edit::Spawn {
            odf: ContentKey::NONE,
            player: PlayerId(0),
            pos: to,
            heading: (Fixed::ZERO, Fixed::ONE),
        };
        let cmds = [
            mv(0, 0, &[3], to),                                              // dead
            mv(0, 1, &[1], to), // the moon: neutral, cannot move
            mv(1, 0, &[2], to), // foreign
            cmd(PlayerId(0), 2, CommandKind::Edit(Edit::Kill(EntityId(2)))), // players may not edit
            cmd(
                PlayerId::BENCH,
                0,
                CommandKind::Edit(Edit::Kill(EntityId(3))),
            ), // already dead
            cmd(
                PlayerId::BENCH,
                1,
                CommandKind::Edit(Edit::Kill(EntityId(99))),
            ), // never was
            spawn(2, &db, "fx_beam", 0, to), // a weapon, not a unit
            cmd(PlayerId::BENCH, 3, CommandKind::Edit(nothing)), // no definition
            mv(0, 3, &[3, 2], to), // one dead, one fine: applies
        ];
        let out = WorldSource::step(&mut sim, &cmds);
        let rejected: Vec<_> = out
            .events
            .iter()
            .filter_map(|e| match e {
                SimEvent::Rejected { player, seq } => Some((player.0, *seq)),
                _ => None,
            })
            .collect();
        assert_eq!(
            rejected,
            [
                (0, 0),
                (0, 1),
                (0, 2),
                (1, 0),
                (255, 0),
                (255, 1),
                (255, 2),
                (255, 3)
            ]
        );
        assert_eq!(out.events.len(), rejected.len());
        assert_eq!(sim.unit(EntityId(2)).unwrap().target, Some(to));
        // A sim without content cannot spawn from it.
        let mut bare = Sim::new(1);
        let out = WorldSource::step(&mut bare, &[spawn(0, &db, "fx_cruiser", 0, to)]);
        assert_eq!(
            out.events,
            [SimEvent::Rejected {
                player: PlayerId::BENCH,
                seq: 0
            }]
        );
    }

    /// Views show the entities as they are, and spawned craft fly on their
    /// definition's movement.
    #[test]
    fn views_match_the_entities() {
        let s = setup(sta_content::fake::two_bases());
        let db = Arc::clone(&s.content);
        let mut sim = Sim::from_setup(&s);
        let first = WorldSource::step(&mut sim, &[spawn(0, &db, "fx_cruiser", 1, v(0, 0, -1000))]);
        assert_eq!(
            first.events,
            [
                SimEvent::Spawned { id: EntityId(1) },
                SimEvent::Spawned { id: EntityId(2) }
            ]
        );
        let to = v(0, 0, 600);
        WorldSource::step(&mut sim, &[mv(1, 0, &[2], to)]);
        let mut arrived = false;
        for t in 0..30 * 30 {
            let out = WorldSource::step(&mut sim, &[]);
            assert_eq!(
                // Autonomy joins the view in the sim, not on the entity.
                out.view.units.iter().cloned().map(|mut u| { u.autonomy = None; u }).collect::<Vec<_>>(),
                sim.units.iter().map(Entity::view).collect::<Vec<_>>()
            );
            let c = out.view.unit(EntityId(2)).unwrap();
            if t == 30 {
                // A second in: under way, no faster than its definition allows.
                assert!(
                    c.speed > Fixed::ZERO && c.speed <= Fixed::from_int(140),
                    "{:?}",
                    c.speed
                );
            }
            if sim.unit(EntityId(2)).unwrap().target.is_none() {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "never arrived");
        let view = sim.view();
        let c = view.unit(EntityId(2)).unwrap();
        assert_eq!(
            (c.def, c.owner, c.pos, c.speed),
            (db.key("fx_cruiser").unwrap(), PlayerId(1), to, Fixed::ZERO)
        );
        assert_eq!(c.hull, [Fixed::from_int(800); 2]);
        assert_eq!(c.shields, [[Fixed::from_int(400); 2]; 4]);
        assert_eq!(c.flags, UnitFlags::SELECTABLE);
        assert_eq!(c.progress, None);
        let moon = view.unit(EntityId(1)).unwrap();
        assert_eq!(
            (moon.owner, moon.hull, moon.speed),
            (NEUTRAL, [Fixed::ZERO; 2], Fixed::ZERO)
        );
    }
}
