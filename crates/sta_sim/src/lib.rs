//! Deterministic lockstep simulation core.
//!
//! Rules for everything in this crate:
//! - No floats in simulation state. Positions, velocities and resources use
//!   [`Fixed`] so x86-64, ARM64 and Apple Silicon produce identical results.
//!   Floats only enter when loading data (`Fixed::from_f64` of parsed ODF values,
//!   which are correctly rounded everywhere) and leave only for rendering.
//! - No hash-map iteration order, wall-clock time, or thread scheduling may
//!   influence state. Entity iteration is by ascending id.
//! - All randomness comes from the seeded [`Rng`].
//! - No platform maths functions (`sin`, `atan2`, ...): steering uses dot and
//!   cross products, small-angle polynomials and the exact integer square root.
//! - Each tick ends with a [`Sim::digest`] that peers compare to detect desyncs.
//!
//! Space is the maps' own: Direct3D left-handed, +Y up. Units move in their
//! heading in the XZ plane and climb or sink toward a target height.
//!
//! The seams (docs/WORKPLAN.md) live in [`command`] (S2), [`view`] and
//! [`source`] (S3), [`event`] (S4) and [`setup`] (S7); [`fake`] has the
//! stand-ins other lanes build against. [`Sim`] is a [`WorldSource`] over
//! the entity model in [`world`], filled from the map by [`spawn`], with
//! today's movement.

pub mod autonomy;
pub mod boundary;
pub mod combat;
pub mod command;
pub mod economy;
pub mod event;
pub mod fake;
pub mod kinematics;
pub mod movement;
mod ownership;
mod perception;
#[cfg(test)]
mod perception_tests;
#[cfg(test)]
mod unit_status_tests;
pub mod present;
pub mod production;
mod avoid;
mod nav;
mod regions;
pub mod replay;
pub mod setup;
pub mod snapshot;
pub mod sensors;
pub mod source;
pub mod specials;
pub mod spatial;
pub mod spawn;
pub mod start;
pub mod tech;
pub mod victory;
pub mod view;
pub mod world;

/// The fixed-point type and its module, now in `sta_core`; re-exported so
/// `sta_sim::Fixed` and `sta_sim::fixed::Fixed` keep working.
pub use sta_core::fixed;
pub use sta_core::{ContentKey, EntityId, Fixed, PlayerId, SEAM_REV, TICK_HZ, TeamId, Tick, Vec3};
/// Movement limits now live with the rest of a unit's definition.
pub use sta_content::MoveStats;

pub use event::SimEvent;
pub use source::WorldSource;
pub use view::{TickOutput, UnitView, WorldView};

use std::sync::Arc;

use sta_content::{ContentDb, Region, StartPos};
use view::{PlayerView, Relation};
use world::{Entities, Entity, EntityKind, Fnv, Table};

pub(crate) use kinematics::{MIN_ALIGN, arrive_distance};

/// The entity, under the name movement and `sta64::units` know it by.
pub type Unit = Entity;

/// xorshift64* — small, fast and identical everywhere.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// The simulation state.
#[derive(Debug, Clone)]
pub struct Sim {
    pub tick: u64,
    pub rng: Rng,
    /// Every entity, ascending id (the name is the legacy one).
    pub units: Entities,
    /// Map labels of the entities that have one (missions find them by it).
    pub labels: Table<String>,
    /// Seats from the game setup, ascending id; in the digest.
    players: Vec<(PlayerId, TeamId)>,
    /// The seats whose setup said `Control::Ai`, ascending (SIM-09: the tech
    /// gate lets AI seats build the engine's unlisted tail slots). From the
    /// setup like `players`, so not in the digest.
    ai_seats: Vec<PlayerId>,
    /// What spawns resolve against; `None` for a sim made with [`Sim::new`].
    content: Option<Arc<ContentDb>>,
    /// The map's start positions and regions, kept for the systems that use
    /// them. Fixed by the setup, so not in the digest.
    starts: Vec<StartPos>,
    regions: Vec<Region>,
    /// The map's playable volume, for build sites; fixed by the setup, so
    /// not in the digest. `None` for a sim made without a map.
    bounds: Option<(Vec3, Vec3)>,
    /// Profile-backed immutable volumes and the saved mutable actor flags.
    confinement: boundary::state::Confinement,
    /// Canonical perceived-owner overrides; absent means real owner.
    perception: Table<PlayerId>,
    /// Events since the last step (spawns between ticks), emitted with it.
    events: Vec<SimEvent>,
    /// Weapon state (SIM-04); in the digest once any exists.
    weapons: combat::weapons::Weapons,
    /// Queued waypoints and formations (SIM-02).
    movement: movement::Movement,
    /// Stock, nodes and freighters (SIM-07); in the digest once any exists.
    economy: economy::Economy,
    /// Yard queues and rig deploys (SIM-08); in the digest once any exists.
    production: production::Production,
    /// Eliminated seats (SIM-13); in the digest once any exists.
    victory: victory::Victory,
    /// Special energy and cloaks (SIM-14); in the digest once any exists.
    specials: specials::Specials,
    /// Craft autonomy levels that were set (SIM-38); in the digest once any exists.
    autonomy: autonomy::Autonomy,
    /// Path planner grids and fields (SIM-23): a memo, not in the digest.
    nav: nav::Cache,
}

impl Sim {
    pub fn new(seed: u64) -> Self {
        Sim {
            tick: 0,
            rng: Rng::new(seed),
            units: Entities::new(),
            labels: Table::new(),
            players: Vec::new(),
            ai_seats: Vec::new(),
            content: None,
            starts: Vec::new(),
            regions: Vec::new(),
            bounds: None,
            confinement: Default::default(),
            perception: Table::new(),
            events: Vec::new(),
            weapons: Default::default(),
            movement: movement::Movement::default(),
            economy: Default::default(),
            production: Default::default(),
            victory: Default::default(),
            specials: Default::default(),
            autonomy: Default::default(),
            nav: Default::default(),
        }
    }

    /// Add a craft with no definition facing `heading` (x, z; normalised
    /// here). Content-made entities come from [`Sim::spawn_def`].
    pub fn spawn(&mut self, player: u8, pos: Vec3, heading: (Fixed, Fixed), stats: MoveStats) -> EntityId {
        let id = self.units.spawn(Entity::new(EntityKind::Craft, ContentKey::NONE, PlayerId(player), pos, heading, stats));
        self.events.push(SimEvent::Spawned { id });
        id
    }

    pub fn unit(&self, id: EntityId) -> Option<&Unit> {
        self.units.get(id)
    }

    /// Actual actor state; unavailable is distinct from a disabled flag.
    pub fn force_onto_map(&self, id: EntityId) -> Option<bool> {
        self.confinement.flag(self.units.get(id)?)
    }

    fn unit_mut(&mut self, id: EntityId) -> Option<&mut Unit> {
        self.units.get_mut(id)
    }

    fn set_target(&mut self, unit: EntityId, to: Option<Vec3>) {
        if let Some(u) = self.unit_mut(unit) {
            u.target = to;
        }
    }

    /// Apply one seam command, or say it was rejected. Today moves, stops
    /// and the spawn and kill edits exist; everything else is rejected until
    /// its package lands.
    fn apply(&mut self, c: &command::Command) -> bool {
        use command::{CommandKind, Order};
        if self.victory.is_eliminated(c.player) {
            return false;
        }
        let order = match &c.kind {
            CommandKind::Order(o) => o,
            CommandKind::MissionText { text } => {
                if !c.player.is_player()
                    || !self.players.iter().any(|(id, _)| *id == c.player)
                    || text.len() > command::MAX_MISSION_TEXT_BYTES
                {
                    return false;
                }
                self.events.push(SimEvent::MissionText {
                    player: c.player,
                    seq: c.seq,
                    text: text.clone(),
                });
                return true;
            }
            CommandKind::Edit(e) => {
                return match self.economy.apply_edit(c.player, e) {
                    Some(ok) => ok,
                    None => self.apply_edit(c.player, e),
                };
            }
        };
        match order {
            Order::Move { units, to, queue } => {
                let unlocked: Vec<_> = units.iter().copied()
                    .filter(|id| !self.production.is_welding(*id)).collect();
                if unlocked.is_empty() { return false; }
                if !queue {
                    self.economy.cancel(&self.units, c.player, &unlocked);
                    if let Some(db) = self.content.clone() {
                        self.production.cancel(&db, &self.units, &mut self.economy, c.player, &unlocked);
                    }
                }
                self.apply_movement(c.player, &Order::Move { units: unlocked, to: *to, queue: *queue })
            }
            Order::Formation { .. } => self.apply_movement(c.player, order),
            Order::Mine { units, node } => {
                let Some(db) = self.content.clone() else { return false };
                let unlocked: Vec<_> = units.iter().copied()
                    .filter(|id| !self.production.is_welding(*id)).collect();
                self.economy.order_mine(&db, &self.units, c.player, &unlocked, *node)
            }
            Order::Stop { units } => {
                // Stop controls activities as well as movement. Select eligible
                // actors before cleanup, including stations that cannot move.
                let mut eligible: Vec<_> = units.iter().copied().filter(|&id| {
                    self.unit(id).is_some_and(|u| {
                        (matches!(u.kind, world::EntityKind::Craft | world::EntityKind::Station)
                            || self.content.as_deref().and_then(|db| db.unit(u.def))
                                .is_some_and(world::EntityKind::producer_def))
                            && (!c.player.is_player() || u.owner == c.player)
                    })
                }).collect();
                eligible.sort();
                eligible.dedup();
                if eligible.is_empty() { return false; }
                self.weapons.stop_attack(&self.units, c.player, &eligible);
                self.economy.cancel(&self.units, c.player, &eligible);
                if let Some(db) = self.content.clone() {
                    self.production.cancel(&db, &self.units, &mut self.economy, c.player, &eligible);
                }
                for site in self.production.cancel_welds(&eligible) {
                    self.despawn(site, None);
                }
                // Immobile actors have no course to stop; their accepted
                // cleanup must not be reported as a rejected movement order.
                self.apply_movement(c.player, &Order::Stop { units: eligible });
                true
            }
            Order::Build { builder, what, at } => {
                let Some(db) = self.content.clone() else { return false };
                if !tech::build_allowed(&db, &self.units, &self.players, &self.ai_seats, *builder, *what) {
                    return false;
                }
                self.production.order_build(&db, &mut self.units, &mut self.economy, c.player, *builder, *what, *at, self.bounds.as_ref())
            }
            Order::Attack { units, target } => {
                let Some(db) = self.content.clone() else { return false };
                self.weapons.order_attack(&db, &self.units, c.player, units, *target)
            }
            Order::Cloak { units, on } => {
                let Some(db) = self.content.clone() else { return false };
                self.specials.order_cloak(&db, &self.units, c.player, units, *on)
            }
            Order::Special { units, slot, .. } => {
                let Some(db) = self.content.clone() else { return false };
                self.specials.order_special(&db, &self.units, c.player, units, *slot)
            }
            Order::Autonomy { units, kind, level } => {
                self.autonomy.order(&self.units, c.player, units, *kind, *level)
            }
            Order::Rally { builders, at } => {
                let Some(db) = self.content.as_deref() else { return false };
                self.production.order_rally(db, &self.units, c.player, builders, *at)
            }
        }
    }

    /// Move every unit one tick and close the tick with its view.
    fn advance(&mut self) -> TickOutput {
        self.hold_welding_builders();
        self.constrain_targets();
        let routed = self.route_orders();
        let held = self.steer_around();
        self.step_kinematics();
        self.restore_targets(held);
        self.restore_targets(routed);
        self.separate_units();
        self.push_out_of_fields();
        if let Some(db) = self.content.clone() {
            combat::damage::recharge(&mut self.units, &db);
            self.specials.step(&mut self.units, &db, &mut self.events);
            let from = self.events.len();
            self.weapons.step_with_perception(self.tick, &self.units, &db, &self.players, &self.perception, &mut self.events);
            let mut lost = Vec::new();
            for (id, by) in combat::damage::apply(&mut self.units, &mut self.events, from) {
                lost.extend(self.units.get(id).map(|u| u.owner));
                self.despawn(id, by);
            }
            self.specials.react(&mut self.units, &db, &mut self.events, from);
            self.victory.step(&self.units, &mut lost, &mut self.events);
        }
        if let Some(db) = &self.content {
            self.economy.step(&mut self.units, db, &mut self.events);
            self.production.step(&mut self.units, db, &mut self.economy, &mut self.events, &mut self.confinement);
        }
        self.hold_welding_builders();
        self.advance_courses();
        self.constrain_targets();
        self.tick += 1;
        TickOutput { view: Arc::new(self.build_view()), events: std::mem::take(&mut self.events) }
    }

    /// Live actors only; queued reservations/admission are not implemented.
    fn officer_census(&self, player: PlayerId) -> Fixed {
        let Some(db) = self.content.as_deref() else { return Fixed::ZERO };
        self.units.iter().filter(|u| u.owner == player && !u.flags.contains(view::UnitFlags::DYING))
            .filter_map(|u| db.unit(u.def)).fold(Fixed::ZERO, |sum, def| {
                Fixed::from_bits(sum.to_bits().saturating_add(def.build.officers.max(Fixed::ZERO).to_bits()))
            })
    }

    fn build_view(&self) -> WorldView {
        // `hidden = 1` objects (start pods, faction objects, map markers) stay in
        // `units` for scripts, but no player sees or picks them (`EntityKind::hidden`).
        // Only the view changes: state and digest do not.
        let mut hidden: Vec<EntityId> = Vec::new();
        let units = self.units.iter().map(|entity| {
            let mut view = entity.view();
            if self.content.as_deref().and_then(|db| db.unit(entity.def)).is_some_and(EntityKind::hidden) {
                hidden.push(entity.id);
                view.flags = view.flags.with(view::UnitFlags::SELECTABLE, false);
            }
            view.flags = view.flags.with(view::UnitFlags::BUILDING, self.production.is_welding(entity.id));
            if matches!(entity.kind, EntityKind::Craft) {
                view.autonomy = Some(self.autonomy.levels(entity.id));
            }
            view.energy = self.specials.energy_of(entity.id).and_then(|now| {
                self.content.as_deref()
                    .and_then(|db| db.unit(entity.def))
                    .and_then(specials::energy_def)
                    .map(|def| [now, def.max])
            });
            view
        }).collect();
        let players = self
            .players
            .iter()
            .map(|&(id, team)| PlayerView {
                id,
                team,
                resources: self.economy.stock_of(id),
                officers: self.economy.officer_limit(id).map_or([Fixed::ZERO; 2], |limit| [self.officer_census(id), limit]),
                crew: self.economy.crew_of(id),
                eliminated: self.victory.is_eliminated(id),
                relations: self
                    .players
                    .iter()
                    .filter(|o| o.0 != id)
                    .map(|&(o, t)| (o, if t == team { Relation::Ally } else { Relation::Enemy }))
                    .collect(),
            })
            .collect();
        let mut visibility = sensors::compute(&self.units, self.content.as_deref(), &self.players);
        if !hidden.is_empty() {
            // Both lists ascend by id.
            for (_, seen) in &mut visibility {
                seen.retain(|id| hidden.binary_search(id).is_err());
            }
        }
        let perceived_owners = self.perception.iter().map(|(id, &owner)| (id, owner)).collect();
        WorldView { tick: self.tick, digest: self.digest(), units, players, visibility, perceived_owners }
    }

    /// FNV-1a over all simulation state, for desync detection. The content
    /// hash goes in first, so tick 0's digest already tells peers with
    /// different content apart (S5), and every later one carries it.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick);
        h.u64(self.rng.0);
        h.u64(self.content.as_ref().map_or(0, |c| c.hash()));
        h.u64(self.players.len() as u64);
        for &(p, t) in &self.players {
            h.u64(p.0 as u64);
            h.u64(t.0 as u64);
        }
        self.units.hash_into(&mut h);
        self.movement.hash_into(&mut h);
        h.u64(self.labels.len() as u64);
        for (id, label) in self.labels.iter() {
            h.u64(id.0 as u64);
            h.str(label);
        }
        if !self.weapons.is_empty() {
            self.weapons.hash_into(&mut h);
        }
        if !self.economy.is_empty() {
            self.economy.hash_into(&mut h);
        }
        if !self.production.is_empty() {
            self.production.hash_into(&mut h);
        }
        if !self.victory.is_empty() {
            self.victory.hash_into(&mut h);
        }
        if !self.specials.is_empty() {
            self.specials.hash_into(&mut h);
        }
        self.confinement.hash_into(&mut h);
        if !self.autonomy.is_empty() {
            self.autonomy.hash_into(&mut h);
        }
        if !self.perception.is_empty() {
            h.str("perceived-team-v1");
            h.u64(self.perception.len() as u64);
            for (id, owner) in self.perception.iter() {
                h.u64(id.0 as u64);
                h.u64(owner.0 as u64);
            }
        }
        let crew_count = self.units.iter().filter(|unit| unit.crew.is_some()).count();
        if crew_count != 0 {
            h.str("unit-crew-v1");
            h.u64(crew_count as u64);
            for unit in self.units.iter() {
                if let Some([now, max]) = unit.crew {
                    h.u64(unit.id.0 as u64);
                    h.fixed(now);
                    h.fixed(max);
                }
            }
        }
        h.0
    }
}

impl WorldSource for Sim {
    /// Applies `commands` sorted by `(player, seq)` (sorted again here, so a
    /// caller's order cannot matter), then advances. Rejected commands come
    /// out as [`SimEvent::Rejected`], in batch order.
    fn step(&mut self, commands: &[command::Command]) -> TickOutput {
        let mut batch: Vec<&command::Command> = commands.iter().collect();
        batch.sort_by_key(|c| c.sort_key());
        let mut captured = Vec::new();
        for c in batch {
            let previous = match &c.kind {
                command::CommandKind::Edit(command::Edit::Team(id, to)) => self.units.get(*id)
                    .filter(|u| u.owner != *to && self.players.iter().any(|(p, _)| *p == u.owner)
                        && matches!(u.kind, world::EntityKind::Craft | world::EntityKind::Station))
                    .map(|u| u.owner),
                _ => None,
            };
            if !self.apply(c) {
                self.events.push(SimEvent::Rejected { player: c.player, seq: c.seq });
            } else if let Some(owner) = previous {
                captured.push(owner);
            }
        }
        // Resolve only after the complete ordered batch: another transfer in
        // this batch can restore an asset. No transient candidate is persisted.
        self.victory.step(&self.units, &mut captured, &mut self.events);
        self.advance()
    }

    fn view(&self) -> Arc<WorldView> {
        Arc::new(self.build_view())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::from_int(z),
        }
    }

    fn stats() -> MoveStats {
        MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2)
    }

    /// A single order from `player` to one of its units.
    fn go(player: u8, id: EntityId, to: Vec3) -> command::Command {
        order(player, 0, command::Order::Move { units: vec![id], to, queue: false })
    }

    fn halt(player: u8, id: EntityId) -> command::Command {
        order(player, 0, command::Order::Stop { units: vec![id] })
    }

    fn run() -> u64 {
        run_sim().digest()
    }

    fn run_sim() -> Sim {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        let b = sim.spawn(1, v(1000, 0, 0), (-Fixed::ONE, Fixed::ZERO), stats());
        for t in 0..600 {
            let cmds = match t {
                0 => vec![go(0, a, v(500, 30, -300))],
                10 => vec![go(1, b, v(0, 0, 900))],
                200 => vec![halt(0, a)],
                _ => vec![],
            };
            sim.step(&cmds);
            sim.rng.next_u64();
        }
        sim
    }

    /// The digest before the entity model: the fields movement had, hashed
    /// the old way.
    fn movement_digest(sim: &Sim) -> u64 {
        let mut h = Fnv::new();
        h.u64(sim.tick);
        h.u64(sim.rng.0);
        for u in &sim.units {
            h.u64(u.id.0 as u64);
            h.u64(u.owner.0 as u64);
            for f in [u.pos.x, u.pos.y, u.pos.z, u.heading.0, u.heading.1, u.speed, u.omega] {
                h.fixed(f);
            }
            match u.target {
                Some(t) => {
                    h.u64(1);
                    for f in [t.x, t.y, t.z] {
                        h.fixed(f);
                    }
                }
                None => h.u64(0),
            }
        }
        h.0
    }

    #[test]
    fn replay_is_deterministic() {
        assert_eq!(run(), run());
    }

    /// The exact digest pins the arithmetic: any platform or compiler change
    /// that alters a single bit of the simulation fails here (CI runs it on
    /// x86-64, ARM64 Linux, Windows and Apple Silicon).
    ///
    /// The entity model (SIM-01) added kind, definition, stats, hull,
    /// shields, flags, players and the next id to the digest, so its value
    /// moved from `0xa4c0_7121_6986_d7dc`; the motion did not, which the old
    /// formula over the same run still shows.
    #[test]
    fn digest_is_pinned() {
        assert_eq!(run(), 0x2e54_3b95_7f83_da45);
        assert_eq!(movement_digest(&run_sim()), 0xa4c0_7121_6986_d7dc);
    }

    #[test]
    fn unit_turns_around_and_arrives() {
        let mut sim = Sim::new(1);
        // Facing +Z, target behind at -Z and off to the side.
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        sim.step(&[go(0, a, v(300, 50, -800))]);
        let mut arrived = None;
        for t in 0..30 * 60 {
            sim.step(&[]);
            if sim.unit(a).unwrap().target.is_none() {
                arrived = Some(t);
                break;
            }
        }
        let u = sim.unit(a).unwrap();
        assert!(arrived.is_some(), "never arrived: {:?}", u.pos.to_f32());
        assert_eq!(u.pos, v(300, 50, -800));
        assert_eq!(u.speed, Fixed::ZERO);
        // Heading stays unit length.
        let l = (u.heading.0 * u.heading.0 + u.heading.1 * u.heading.1).to_f32();
        assert!((l - 1.0).abs() < 1e-6, "{l}");
    }

    /// Once a unit has turned onto its course it must not weave across it
    /// (the earlier proportional controller crossed it 806 times).
    #[test]
    fn no_weaving_on_course() {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ONE, Fixed::ZERO), stats());
        sim.step(&[go(0, a, v(0, 0, 4000))]);
        let mut crossings = 0;
        let mut worst = 0.0f32;
        let mut last: Option<bool> = None;
        for _ in 0..30 * 40 {
            sim.step(&[]);
            let u = sim.unit(a).unwrap();
            let Some(t) = u.target else { break };
            let (dx, dz) = (t.x - u.pos.x, t.z - u.pos.z);
            let d = (dx * dx + dz * dz).sqrt();
            // Course error as the sine of the angle; ignore below a milliradian.
            let side = (u.heading.0 * dz - u.heading.1 * dx) / d;
            if side.abs() < Fixed::from_f64(1e-3) {
                continue;
            }
            let s = side > Fixed::ZERO;
            if last.is_some_and(|l| l != s) {
                crossings += 1;
                worst = worst.max(side.abs().to_f32());
            }
            last = Some(s);
        }
        // Settling after the turn may cross the (moving) course line once or
        // twice by a fraction of a degree; weaving means many or large swings.
        assert!(crossings <= 2, "weaved across the course {crossings} times");
        assert!(worst < 0.01, "overshoot of {worst} rad");
        assert!(sim.unit(a).unwrap().target.is_none(), "did not arrive");
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // platform trig as the reference only
    fn small_rotation_matches_trig() {
        let h = kinematics::rotate((Fixed::ONE, Fixed::ZERO), Fixed::from_f64(0.05));
        assert!((h.0.to_f32() - 0.05f32.cos()).abs() < 1e-6);
        assert!((h.1.to_f32() - 0.05f32.sin()).abs() < 1e-6);
    }

    fn order(player: u8, seq: u32, o: command::Order) -> command::Command {
        command::Command { player: PlayerId(player), seq, kind: command::CommandKind::Order(o) }
    }

    /// The pinned run again, through the seam: same digest as setting targets directly.
    #[test]
    fn world_source_matches_direct_targets() {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        let b = sim.spawn(1, v(1000, 0, 0), (-Fixed::ONE, Fixed::ZERO), stats());
        let first = WorldSource::step(&mut sim, &[]);
        assert_eq!(first.events, [SimEvent::Spawned { id: a }, SimEvent::Spawned { id: b }]);
        for t in 1..600 {
            let cmds = match t {
                1 => vec![order(0, 0, command::Order::Move { units: vec![a], to: v(500, 30, -300), queue: false })],
                10 => vec![order(1, 0, command::Order::Move { units: vec![b], to: v(0, 0, 900), queue: false })],
                200 => vec![order(0, 1, command::Order::Stop { units: vec![a] })],
                _ => vec![],
            };
            let out = WorldSource::step(&mut sim, &cmds);
            assert_eq!(out.view.tick, sim.tick);
            assert_eq!(out.view.digest, sim.digest());
            sim.rng.next_u64();
        }
        // The same run with targets set directly, no commands.
        let mut old = Sim::new(1);
        old.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        old.spawn(1, v(1000, 0, 0), (-Fixed::ONE, Fixed::ZERO), stats());
        old.advance();
        for t in 1..600 {
            match t {
                1 => old.set_target(a, Some(v(500, 30, -300))),
                10 => old.set_target(b, Some(v(0, 0, 900))),
                200 => old.set_target(a, None),
                _ => {}
            }
            old.advance();
            old.rng.next_u64();
        }
        assert_eq!(sim.digest(), old.digest());
        assert_eq!(sim.view().units.len(), 2);
    }

    #[test]
    fn foreign_and_unknown_commands_are_rejected() {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        sim.step(&[]);
        let cmds = [
            // Player 1 may not order player 0's unit.
            order(1, 0, command::Order::Move { units: vec![a], to: v(100, 0, 0), queue: false }),
            // Not implemented yet.
            order(0, 0, command::Order::Cloak { units: vec![a], on: true }),
            // The bench may order anything.
            order(255, 0, command::Order::Move { units: vec![a], to: v(100, 0, 0), queue: false }),
        ];
        let out = WorldSource::step(&mut sim, &cmds);
        let rejected = [SimEvent::Rejected { player: PlayerId(0), seq: 0 }, SimEvent::Rejected { player: PlayerId(1), seq: 0 }];
        assert_eq!(out.events, rejected);
        assert_eq!(sim.unit(a).unwrap().target, Some(v(100, 0, 0)));
    }
}
