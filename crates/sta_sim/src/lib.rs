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
pub mod appearance;
pub mod boundary;
pub mod combat;
pub mod colony;
pub mod command;
pub mod context;
pub mod digest;
pub mod economy;
pub mod event;
pub mod fake;
pub mod kinematics;
pub mod move_mode;
pub mod movement;
mod ownership;
pub mod speed_override;
pub mod warp_override;
pub mod alert;
#[cfg(test)]
mod alert_tests;
pub mod mission_switches;
#[cfg(test)]
mod mission_switches_tests;
#[cfg(test)]
mod warp_override_tests;
#[cfg(test)]
mod speed_override_tests;
mod perception;
pub mod relations;
#[cfg(test)]
mod perception_tests;
#[cfg(test)]
mod unit_status_tests;
pub mod present;
pub mod production;
mod avoid;
mod nav;
mod nebula;
mod regions;
mod watchdog;
pub mod orders;
#[cfg(test)]
#[path = "orders/tests.rs"]
mod orders_tests;
pub mod switches;
pub mod replay;
pub mod repair;
pub mod recrew;
#[cfg(test)]
mod recrew_tests;
pub mod save;
pub mod setup;
pub mod snapshot;
pub mod sensors;
pub mod ship_upgrades;
pub mod source;
mod inspector_edit;
pub mod specials;
pub mod spatial;
pub mod spawn;
pub mod start;
mod start_effects;
mod replacement;
pub mod attrs;
pub mod team_pool;
#[cfg(test)]
mod rerace_tests;
#[cfg(test)]
mod special_forces_tests;
#[cfg(test)]
mod cheat_grant_tests;
pub use start_effects::unsupported as unsupported_start_lifecycle;
pub mod tech;
pub mod victory;
pub mod view;
pub mod world;
mod wormholes;

/// The fixed-point type and its module, now in `sta_core`; re-exported so
/// `sta_sim::Fixed` and `sta_sim::fixed::Fixed` keep working.
pub use sta_core::fixed;
pub use sta_core::{ContentKey, EntityId, Fixed, PlayerId, SEAM_REV, TICK_HZ, TeamId, Tick, Vec3};
/// Movement limits now live with the rest of a unit's definition.
pub use sta_content::MoveStats;

pub use event::{MinerPhase, SimEvent};
pub use source::WorldSource;
pub use view::{TickOutput, UnitView, WorldView};

use std::sync::Arc;

use sta_content::{ContentDb, Region, StartPos};
use view::PlayerView;
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
    /// The seats whose setup said `Control::Ai` or `Control::RemoteAi`,
    /// ascending (SIM-09: the tech gate lets AI seats build the engine's
    /// unlisted tail slots; passive economy and start helpers read it too).
    /// Session state, identical on every peer; in the digest once any
    /// exists. Which machine runs the AI is not state and is not kept here.
    ai_seats: Vec<PlayerId>,
    /// What spawns resolve against; `None` for a sim made with [`Sim::new`].
    content: Option<Arc<ContentDb>>,
    /// A1 match rule for using dogfight movement in combat.
    directors_cut: bool,
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
    ship_upgrades: ship_upgrades::Applied,
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
    /// Crafts in warp on an attack approach (SIM-36); in the digest once any is.
    approach_warp: movement::approach::Warp,
    /// Recent hits and shots for assists (SIM-36); in the digest once any is.
    engagement: combat::targeting::Engagement,
    /// Alert levels, last enemy hits and ally attack counters (LUA-32); in the digest once any exists.
    alerts: alert::Alerts,
    /// Pending start-helper lifecycle steps (SIM-28); in the digest once any exist.
    start_effects: start_effects::StartEffects,
    replacements: replacement::Replacements,
    /// Move orders that make no progress (stuck detector): in snapshots, not in the digest.
    watchdog: watchdog::Watchdog,
    /// Path planner grids and fields (SIM-23): a memo, not in the digest.
    nav: nav::Cache,
    /// Subsystem switches and damage set by missions (SIM-55); in the digest
    /// once any exists.
    subsystems: combat::damage::Subsystems,
    /// Stock A2 shield pools held at zero after damage; saved and hashed while active.
    zero_shield_holds: combat::damage::ZeroShieldHolds,
    /// Mission-set unit names and races and seat races (SIM-54); in the digest once any exists.
    attrs: attrs::Attrs,
    /// Scripted orders (SIM-56); in the digest once any exists.
    scripted: orders::Scripted,
    /// Per-craft switches set by missions (SIM-58); in the digest once any exists.
    switches: switches::SwitchTable,
    /// Mission-set relations between seats (SIM-63); in the digest once any exists.
    relations: relations::Relations,
    /// Units whose team just changed (SIM-29): `activateOnTeamChange` weapons
    /// engage on the tick's weapon pass. Filled by the tick's commands and
    /// drained by the same tick, so it is empty between ticks and not saved.
    team_changes: Vec<EntityId>,
    /// Cosmetic owning-seat faction labels, recorded from resolved setup.
    appearance_factions: Vec<(PlayerId, String)>,
    // (Also the setup race of each seat: the base of `Sim::seat_race`, which picks the officer cap
    // and latinum rules; fixed by the setup, so peers agree.)
    /// Planet population and colony ship orders (SIM-30); in the digest once any exists.
    colonies: colony::Colonies,
    /// Transporter orders, parties in flight and shield holds (SIM-46); in the digest once any exists.
    transports: combat::boarding::Transports,
    /// Station recovery activation and damage holds (SIM-70), saved and hashed.
    recrew: recrew::Recrews,
    /// Yard repair queues (SIM-40); in the digest and the snapshot once any exists.
    repairs: repair::Repairs,
    /// Units being decommissioned (SIM-45); in the digest once any exists.
    recycles: economy::recycle::Recycles,
    /// Shields a shield-disabling nebula took and gives back (SIM-75 slice 2); in the digest once any exists.
    nebula_shields: nebula::NebulaShields,
    /// Mission-wide switches (LUA-41), saved; hashed only when not the default.
    mission_switches: mission_switches::MissionSwitches,
    /// The move mode each unit's last move order named (SIM-76); saved, and in the
    /// digest once any exists. No rule reads it yet.
    move_modes: move_mode::MoveModes,
    /// Units in their dying phase (#1100, `dying_phase`); saved, and in the digest once any is.
    dying: combat::dying::Dying,
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
            directors_cut: false,
            starts: Vec::new(),
            regions: Vec::new(),
            bounds: None,
            confinement: Default::default(),
            perception: Table::new(),
            events: Vec::new(),
            weapons: Default::default(),
            movement: movement::Movement::default(),
            ship_upgrades: Default::default(),
            economy: Default::default(),
            production: Default::default(),
            victory: Default::default(),
            specials: Default::default(),
            autonomy: Default::default(),
            approach_warp: Default::default(),
            engagement: Default::default(),
            alerts: Default::default(),
            start_effects: Default::default(),
            replacements: Default::default(),
            watchdog: Default::default(),
            nav: Default::default(),
            subsystems: Default::default(),
            zero_shield_holds: Default::default(),
            attrs: Default::default(),
            scripted: Default::default(),
            switches: Default::default(),
            relations: Default::default(),
            team_changes: Vec::new(),
            appearance_factions: Vec::new(),
            colonies: Default::default(),
            transports: Default::default(),
            recrew: Default::default(),
            repairs: Default::default(),
            recycles: Default::default(),
            nebula_shields: Default::default(),
            mission_switches: Default::default(),
            move_modes: Default::default(),
            dying: Default::default(),
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
        // A dying unit takes no orders (#1100): they apply to the others it was given to.
        let living;
        let c = match &c.kind {
            command::CommandKind::Order(o) => match self.order_for_the_living(o) {
                combat::dying::Living::All => c,
                combat::dying::Living::None => return false,
                combat::dying::Living::Some(o) => {
                    living = command::Command { player: c.player, seq: c.seq, kind: command::CommandKind::Order(o) };
                    &living
                }
            },
            _ => c,
        };
        let ok = self.apply_command(c);
        if ok && let command::CommandKind::Order(o) = &c.kind {
            self.colonies_after_order(c.player, o);
            self.transports_after_order(c.player, o);
            self.repairs_after_order(c.player, o);
            self.recycles_after_order(c.player, o);
        }
        ok
    }

    fn apply_command(&mut self, c: &command::Command) -> bool {
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
            Order::Move { units, to, queue, mode } => {
                let unlocked: Vec<_> = units.iter().copied()
                    .filter(|id| !self.production.is_welding(*id)).collect();
                if unlocked.is_empty() { return false; }
                // A queued leg does not extend a patrol (it never ends): it replaces it.
                let patrols: Vec<_> = unlocked.iter().copied().filter(|id| matches!(self.scripted.job(*id), Some(orders::Job::Patrol { .. }))).collect();
                self.clear_scripted(c.player, &patrols);
                if !queue {
                    self.clear_scripted(c.player, &unlocked);
                    // A move replaces an attack (SIM-36: the approach would
                    // otherwise steer the craft back to its target).
                    self.weapons.stop_attack(&self.units, c.player, &unlocked);
                    self.economy.cancel(&self.units, c.player, &unlocked);
                    if let Some(db) = self.content.clone() {
                        let cancel: Vec<_> = unlocked.iter().copied()
                            .filter(|&id| !self.units.get(id).is_some_and(|u| u.mobile_station))
                            .collect();
                        self.production.cancel(&db, &self.units, &mut self.economy, c.player, &cancel);
                    }
                }
                let taken = self.apply_movement(c.player, &Order::Move { units: unlocked.clone(), to: *to, queue: *queue, mode: *mode });
                if taken {
                    self.record_move_mode(c.player, &unlocked, *mode);
                    self.auto_warp_moves(c.player, &unlocked);
                }
                taken
            }
            Order::Formation { .. } => self.apply_movement(c.player, order),
            Order::Mine { units, node } => {
                let Some(db) = self.content.clone() else { return false };
                let unlocked: Vec<_> = units.iter().copied()
                    .filter(|id| !self.production.is_welding(*id)).collect();
                self.clear_scripted(c.player, &unlocked);
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
                for &id in &eligible {
                    self.recrew.cancel(id, self.tick);
                }
                self.clear_scripted(c.player, &eligible);
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
                self.clear_move_modes(&eligible);
                self.apply_movement(c.player, &Order::Stop { units: eligible });
                true
            }
            Order::Build { builder, what, at } => {
                let Some(db) = self.content.clone() else { return false };
                if self.specials.is_gemini(*builder) || !tech::build_allowed(&db, &self.units, &self.players, &self.ai_seats, *builder, *what) {
                    return false;
                }
                self.clear_scripted(c.player, &[*builder]);
                let ok = self.production.order_build(&db, &mut self.units, &mut self.economy, c.player, *builder, *what, *at, self.bounds.as_ref(), &self.regions);
                if ok && self.production.is_deploying(*builder) {
                    // A craft sent to build drops its attack (SIM-36: the
                    // approach would otherwise steer it off its site).
                    self.weapons.stop_attack(&self.units, c.player, &[*builder]);
                }
                ok
            }
            Order::Attack { units, target } => {
                let Some(db) = self.content.clone() else { return false };
                self.clear_scripted(c.player, units);
                let ok = self.weapons.order_attack(&db, &self.units, c.player, units, *target);
                if ok {
                    let takers: Vec<EntityId> = units.iter().copied().filter(|&id| combat::weapons::takes_attack(&db, &self.units, c.player, id, *target)).collect();
                    self.count_ally_attacks(&takers, *target);
                }
                // The attack replaces queued legs, mining and a pending build
                // site of the crafts that took it (SIM-36); the approach sets
                // the course.
                let took: Vec<EntityId> = units.iter().copied()
                    .filter(|&id| self.weapons.attack_of(id) == Some(*target) && self.unit(id).is_some_and(|u| u.orderable()))
                    .collect();
                for &id in &took {
                    self.movement.clear_waypoints(id);
                }
                if !took.is_empty() {
                    self.economy.cancel(&self.units, c.player, &took);
                    self.production.cancel(&db, &self.units, &mut self.economy, c.player, &took);
                }
                ok
            }
            Order::Cloak { units, on } => {
                let Some(db) = self.content.clone() else { return false };
                let eligible: Vec<_> = units.iter().copied().filter(|&id| {
                    !*on || self.phase_cloak_researched(&db, c.player, id, None)
                }).collect();
                self.specials.order_cloak(&db, &self.units, c.player, &eligible, *on)
            }
            Order::Special { units, slot, target } => {
                let Some(db) = self.content.clone() else { return false };
                let mut legacy = Vec::new();
                let mut accepted = false;
                let mut seen = std::collections::BTreeSet::new();
                for &id in units.iter().filter(|&&id| seen.insert(id)) {
                    match self.order_a2_special(&db, c.player, id, *slot, *target)
                        .or_else(|| self.order_a1_special(&db, c.player, id, *slot, *target))
                    {
                        Some(took) => accepted |= took,
                        None if self.phase_cloak_researched(&db, c.player, id, Some(*slot)) => legacy.push(id),
                        None => {}
                    }
                }
                if legacy.is_empty() {
                    return accepted;
                }
                let toggled = self.weapons.order_special(self.tick, &db, &self.units, &mut self.specials, &mut self.rng, &mut self.events, c.player, &legacy, *slot);
                self.order_replacements(&db, c.player, &legacy, *slot, *target) || toggled || accepted
            }
            Order::Autonomy { units, kind, level } => {
                self.autonomy.order(&self.units, c.player, units, *kind, *level)
            }
            Order::Patrol { .. }
            | Order::SearchAndDestroy { .. }
            | Order::Scout { .. }
            | Order::AttackAny { .. }
            | Order::AttackPoint { .. }
            | Order::MoveWarp { .. }
            | Order::Guard { .. } => self.apply_scripted(c.player, order),
            Order::Colonize { units, planet } => self.apply_colonize(c.player, units, *planet),
            Order::Transport { units, target, attack } => self.apply_transport(c.player, units, *target, *attack),
            Order::Rally { builders, at } => {
                let Some(db) = self.content.as_deref() else { return false };
                self.production.order_rally(db, &self.units, c.player, builders, *at)
            }
            Order::Repair { units, yard } => self.apply_repair(c.player, units, *yard),
            Order::Decommission { units, yard } => self.apply_decommission(c.player, units, *yard),
            Order::Recrew { units, on } => self.order_recrew(c.player, units, *on),
            Order::Alert { units, level } => self.order_alert(c.player, units, *level),
            Order::Trade { units, action } => self.order_trade(c.player, units, *action),
            Order::CancelBuild { builder, slot, job } => {
                let Some(db) = self.content.as_deref() else { return false; };
                let Some(builder) = self.units.get(*builder).filter(|unit|
                    c.player.is_player() && unit.owner == c.player && !unit.flags.contains(view::UnitFlags::DYING)
                ) else { return false; };
                self.production.cancel_build(db, &mut self.economy, builder, *slot, *job)
            }
        }
    }

    /// Move every unit one tick and close the tick with its view.
    fn advance(&mut self) -> TickOutput {
        // #1100: units whose dying phase ends on this tick leave first, before anything moves
        // or fires; their owners are checked for elimination with this tick's losses.
        let died = self.remove_dead();
        if self.content.is_none() {
            self.team_changes.clear();
        }
        if let Some(db) = self.content.clone() {
            self.step_a1_timers(&db);
        }
        self.step_start_effects();
        self.refresh_ship_upgrades();
        self.hold_welding_builders();
        self.constrain_targets();
        self.watch_orders();
        self.assist_idle();
        self.step_scripted();
        let gravity_wells = self.gravity_wells();
        let mut plans = self.steer_attackers(&gravity_wells);
        // SIM-75: who is in which nebula, from the positions at the tick's start.
        let nebulae = self.nebula_contents();
        let speed_caps = if nebulae.is_empty() { Vec::new() } else { self.nebula_speed_caps(&nebulae) };
        // Before routing and avoidance replace targets with detours.
        let gate_goers = self.wormhole_goers();
        let routed = self.route_orders();
        let held = self.steer_around();
        self.brake_for_attack_detours(&mut plans);
        // Warp eligibility follows the aim this tick will actually use, including
        // temporary navigation and avoidance targets.
        let warps = self.steer_warp_moves(&gravity_wells);
        if !warps.is_empty() {
            plans.extend(warps);
            plans.sort_by_key(|p| p.0);
        }
        self.step_kinematics(&plans, &speed_caps);
        self.drift_dying();
        self.restore_targets(held);
        self.restore_targets(routed);
        self.step_wormholes(&gate_goers);
        self.separate_units();
        self.push_out_of_fields();
        if let Some(db) = self.content.clone() {
            production::orbit::step(&db, &mut self.units);
            let race_modifiers: Vec<_> = self.players.iter().map(|p| (p.0, db.rules().race_mining_rate_modifier(self.seat_race(p.0).unwrap_or("")))).collect();
            self.economy.orbital_income(&db, &self.units, self.tick, &race_modifiers);
            let returned = self.transports.expire(self.tick);
            self.restore_carrier_shields(&returned);
            let shields_off = if nebulae.is_empty() { Vec::new() } else { self.nebula_shields_enter(&db, &nebulae) };
            combat::damage::recharge(&mut self.units, &db, &self.switches, &self.subsystems, &self.transports, &mut self.zero_shield_holds, self.tick, &self.specials);
            let nebula_holds = if nebulae.is_empty() { nebula::Holds::default() } else { self.nebula_systems(&db, &nebulae) };
            self.nebula_shields_tail(&shields_off);
            self.activate_team_change_weapons(&db);
            let cloak_from = self.events.len();
            self.specials.step(&mut self.units, &db, &mut self.events);
            self.forget_hits_of_cloakers(cloak_from);
            let from = self.events.len();
            let mut lost = self.step_a2_specials(&db);
            lost.extend(died);
            self.step_a1_ordnance(&db);
            let mut held = if db.features().builder_hold_fire { self.production.builders_at_work() } else { Vec::new() };
            held.extend(self.scripted.holding_fire());
            held.extend(nebula_holds.held.iter().copied());
            self.weapons.hold_fire(held);
            let crew_delay = self.crew_weapon_delays(&db);
            self.weapons.set_crew_delay(crew_delay);
            self.weapons.step_with_perception(self.tick, &self.units, &db, relations::Diplomacy::new(&self.players, &self.relations), &self.perception, &mut self.specials, &mut self.rng, &self.switches, &self.alerts, &mut self.events);
            self.reflect_special_hits(&db, from);
            self.record_engagement(from);
            self.record_enemy_hits(from);
            // An attack whose target is gone leaves no course behind (SIM-36).
            for id in self.weapons.take_ended() {
                if let Some(u) = self.units.get_mut(id).filter(|u| u.kind == EntityKind::Craft) {
                    u.target = None;
                }
                self.approach_warp.end(id);
            }
            // Pause station recovery for incoming positive hits, including
            // casualty hits whose final hull-damage event may become zero.
            self.recrew.note_hits(&self.units, &db, &self.events[from..], self.tick);
            let upgrades = ship_upgrades::ShipUpgrades::collect(&db, &self.units);
            let mut casualties = combat::casualty::Ctx { upgrades: &upgrades, subsystems: &mut self.subsystems, rng: &mut self.rng, switches: &self.switches, colonies: &mut self.colonies };
            let mut killed = combat::damage::apply(&mut self.units, &db, &mut self.events, from, &mut casualties, &mut self.zero_shield_holds, self.tick, &self.specials);
            if !nebulae.is_empty() {
                killed.extend(self.nebula_damage(&db, &nebulae));
            }
            // #1100: hits on units already dying shorten their phase; this tick's kills start
            // theirs under `dying_phase` and are removed when it ends, the others now.
            self.shorten_dying(from);
            let killed = self.start_dying(&db, killed);
            // Pulses and arrivals after the hits, before the latch upkeep (SIM-46).
            self.step_transports();
            self.step_recrew();
            self.auto_repair();
            self.step_repairs();
            self.step_recycles();
            let mut casualties = combat::casualty::Ctx { upgrades: &upgrades, subsystems: &mut self.subsystems, rng: &mut self.rng, switches: &self.switches, colonies: &mut self.colonies };
            combat::casualty::step(&mut self.units, &db, &mut casualties);
            let mut life_support_off = nebula_holds.life_support_off;
            life_support_off.extend(self.specials.a2_disabled_units().filter(|&id| {
                self.specials.a2_held(id, command::Subsystem::LifeSupport, self.tick)
            }));
            life_support_off.sort_unstable();
            life_support_off.dedup();
            combat::casualty::nebula_life_support(&mut self.units, &db, &mut casualties, &life_support_off);
            for (id, by) in killed {
                lost.extend(self.units.get(id).map(|u| u.owner));
                self.despawn(id, by);
            }
            self.specials.react(&mut self.units, &db, &mut self.events, from);
            self.victory.step(&self.units, &mut lost, &mut self.events);
        }
        self.step_colonies();
        let colony_crew = self.content.as_deref().map(|db| self.colony_crew_income(db)).unwrap_or_default();
        if let Some(db) = &self.content {
            self.economy.step(&mut self.units, db, self.tick, &self.confinement, &mut self.events);
            self.economy.step_trade(&mut self.units, economy::trade::TradeRules(db, &self.attrs), self.tick, &mut self.rng);
            self.economy.step_passive(&self.units, db, &self.players, &self.ai_seats, self.tick);
            self.economy.step_crew(&self.units, db, &self.attrs, self.tick, &colony_crew);
            let from = self.events.len();
            self.production.update_research_poses(db, &mut self.units);
            let completed = self.production.step(&mut self.units, db, &mut self.economy, &mut self.events, &mut self.confinement, self.bounds, |id| self.specials.stasis(id, self.tick) || self.dying.contains(id));
            for old in completed.replaced_pods {
                self.retire(old);
            }
            self.inherit_builder_races(from);
            self.inherit_builder_alert(from);
            let built: Vec<EntityId> = self.events[from..].iter().filter_map(|e| match e { SimEvent::Spawned { id } => Some(*id), _ => None }).collect();
            for id in built {
                self.default_autonomy(id);
            }
            for (unit, rally) in completed.gate_rallies {
                self.movement.set_waypoints(unit, vec![rally]);
            }
            for (embryo, product) in completed.evolved {
                self.autonomy.copy_to(embryo, product);
                if self.units.get(product).is_some_and(|unit| unit.kind == world::EntityKind::Craft) {
                    self.weapons.retarget_replacement_shots(embryo, product);
                }
                self.retire(embryo);
                self.events.push(SimEvent::Replaced { old: embryo, new: product, select: true });
            }
        }
        self.hold_welding_builders();
        self.advance_courses();
        self.prune_move_modes();
        self.constrain_targets();
        self.refresh_ship_upgrades();
        self.tick += 1;
        TickOutput { view: Arc::new(self.build_view()), events: std::mem::take(&mut self.events) }
    }

    /// Live actors plus the officers reserved by admitted, unfinished builds
    /// (SIM-26, [`production::officers`]).
    fn officer_census(&self, player: PlayerId) -> Fixed {
        let Some(db) = self.content.as_deref() else { return Fixed::ZERO };
        let census = self.production.officer_census(&self.units, db, player);
        Fixed::from_bits(census.to_bits().saturating_add(self.economy.officer_adjust(player).to_bits()))
    }

    fn officer_limit(&self, player: PlayerId) -> Option<Fixed> {
        match self.content.as_deref() {
            Some(db) => self.production.officer_limit(&self.units, db, &self.economy, player),
            None => self.economy.officer_limit(player),
        }
    }

    /// Production presentation at the current tick, derived from saved state.
    pub fn production_view(&self) -> Vec<production::view::ProductionView> {
        self.content.as_deref().map_or_else(Vec::new, |db| self.production.view(db, &self.units))
    }

    /// Read-only amounts left in the resource nodes (UI-48).
    pub fn node_view(&self) -> Vec<economy::view::NodeView> {
        self.content.as_deref().map_or_else(Vec::new, |db| self.economy.node_view(db, &self.units))
    }

    /// Read-only cargo and hold capacity for live mining craft.
    pub fn cargo_view(&self) -> Vec<economy::view::CargoView> {
        self.content.as_deref().map_or_else(Vec::new, |db| self.economy.cargo_view(db, &self.units))
    }

    fn build_view(&self) -> WorldView {
        // `hidden = 1` objects (start pods, faction objects, map markers) stay in
        // `units` for scripts, but no player sees or picks them (`EntityKind::hidden`).
        // Only the view changes: state and digest do not.
        let mut hidden: Vec<EntityId> = Vec::new();
        let docked = self.economy.docked_stations(&self.units);
        let units = self.units.iter().map(|entity| {
            let mut view = entity.view();
            if self.content.as_deref().and_then(|db| db.unit(entity.def)).is_some_and(EntityKind::hidden) {
                hidden.push(entity.id);
                view.flags = view.flags.with(view::UnitFlags::SELECTABLE, false);
            }
            // #1100: a dying unit is deselected and cannot be picked (measured, once).
            if self.dying.contains(entity.id) {
                view.flags = view.flags.with(view::UnitFlags::SELECTABLE, false);
            }
            view.flags = view.flags.with(view::UnitFlags::BUILDING, self.production.is_welding(entity.id));
            view.flags = view.flags.with(view::UnitFlags::DOCKED, docked.contains(&entity.id));
            if self.specials.shields_held(entity.id, self.tick) {
                for pool in &mut view.shields { pool[0] = Fixed::ZERO; }
            }
            if self.specials.engines_held(entity.id, self.tick) {
                view.flags = view.flags | view::UnitFlags::ENGINES_OFF;
            }
            if self.specials.a2_held(entity.id, command::Subsystem::Weapons, self.tick) {
                view.flags = view.flags | view::UnitFlags::WEAPONS_OFF;
            }
            if matches!(entity.kind, EntityKind::Craft | EntityKind::Station) {
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
                officers: self.officer_limit(id).map_or([Fixed::ZERO; 2], |limit| [self.officer_census(id), limit]),
                crew: self.economy.crew_of(id),
                eliminated: self.victory.is_eliminated(id),
                relations: self
                    .players
                    .iter()
                    .filter(|o| o.0 != id)
                    .map(|&(o, _)| (o, relations::Diplomacy::new(&self.players, &self.relations).view(id, o)))
                    .collect(),
            })
            .collect();
        let mut visibility = sensors::compute(&self.units, self.content.as_deref(), relations::Diplomacy::new(&self.players, &self.relations), &self.specials, self.tick);
        if !hidden.is_empty() {
            // Both lists ascend by id.
            for (_, seen) in &mut visibility {
                seen.retain(|id| hidden.binary_search(id).is_err());
            }
        }
        let perceived_owners = self.perception.iter().map(|(id, &owner)| (id, owner)).collect();
        let active_point_defence = self.content.as_deref().map_or_else(Vec::new, |db| self.weapons.active_point_defence(db, &self.units));
        WorldView { tick: self.tick, digest: self.digest(), units, players, visibility, perceived_owners, appearance: self.appearance_view(), colonies: self.colony_views(), attrs: crate::attrs::AttrsView { names: self.switches.names(), seats: self.seat_views(), impulse: self.impulse_overrides(), warp: self.warp_overrides(), alerts: self.alerts.levels(), enemy_hit: self.alerts.hits(), ally_attacks: self.alerts.ally_rows().to_vec(), special_forces: self.switches.special_forces(), mission_switches: self.mission_switches, move_modes: self.move_mode_rows(), ..self.attrs.view() }, active_point_defence, a1_specials: self.specials.effect_view(self.tick, &self.units, self.content.as_deref()) }
    }

    /// FNV-1a over all simulation state, for desync detection. The content
    /// hash goes in first, so tick 0's digest already tells peers with
    /// different content apart (S5), and every later one carries it.
    pub fn digest(&self) -> u64 {
        self.digest_with_content_hash(self.content.as_ref().map_or(0, |c| c.hash()))
    }

    /// [`Sim::digest`] with `content_hash` in place of the content's: lets
    /// tests compare state against goldens recorded under an older content
    /// hash scheme (CON-35 changed every content hash, not the state).
    pub(crate) fn digest_with_content_hash(&self, content_hash: u64) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick);
        h.u64(self.rng.0);
        h.u64(content_hash);
        h.u64(self.players.len() as u64);
        for &(p, t) in &self.players {
            h.u64(p.0 as u64);
            h.u64(t.0 as u64);
        }
        if !self.ai_seats.is_empty() {
            h.str("ai_seats");
            h.u64(self.ai_seats.len() as u64);
            for p in &self.ai_seats {
                h.u64(p.0 as u64);
            }
        }
        self.units.hash_into(&mut h);
        self.movement.hash_into(&mut h);
        self.hash_move_modes(&mut h);
        self.hash_directors_cut(&mut h);
        if !self.approach_warp.is_empty() {
            self.approach_warp.hash_into(&mut h);
        }
        if !self.engagement.is_empty() {
            self.engagement.hash_into(&mut h);
        }
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
        if self.production.has_digest_state() {
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
        if !self.start_effects.is_empty() {
            self.start_effects.hash_into(&mut h);
        }
        if !self.replacements.is_empty() {
            self.replacements.hash_into(&mut h);
        }
        if !self.subsystems.is_empty() {
            self.subsystems.hash_into(&mut h);
        }
        if !self.zero_shield_holds.is_empty() {
            self.zero_shield_holds.hash_into(&mut h);
        }
        // #1100: the dying phases (nothing is hashed without any).
        if !self.dying.is_empty() {
            self.dying.hash_into(&mut h);
        }
        if !self.scripted.is_empty() {
            self.scripted.hash_into(&mut h);
        }
        if !self.switches.is_empty() {
            self.switches.hash_into(&mut h);
        }
        if !self.relations.is_empty() {
            self.relations.hash_into(&mut h);
        }
        if !self.attrs.is_empty() {
            self.attrs.hash_into(&mut h);
        }
        // SIM-30: population and colonizing orders (nothing is hashed without colony state).
        if !self.colonies.is_empty() {
            self.colonies.hash_into(&mut h);
        }
        // LUA-31: scripted warp speeds (nothing is hashed without an override).
        self.hash_warp_overrides(&mut h);
        // LUA-41: the mission switches (nothing is hashed at the default).
        self.mission_switches.hash_into(&mut h);
        // SIM-46: transporter orders, parties in flight, shield holds (nothing is hashed without them).
        if !self.transports.is_empty() {
            self.transports.hash_into(&mut h);
        }
        // LUA-32: alert levels, enemy hits (while a level exists) and ally attack counters.
        self.alerts.hash_into(&mut h);
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
    fn supports_placement_edits(&self) -> bool {
        self.content.is_some()
    }

    fn edit_placement(&mut self, edit: source::PlacementEdit) -> Option<EntityId> {
        use source::PlacementEdit;

        let id = match edit {
            PlacementEdit::Place { def, owner, at } => {
                let defn = self.content.as_ref()?.unit(def)?;
                if !source::editable_placement(defn) {
                    return None;
                }
                let id = self.spawn_unqueued_def(def, owner, at, (Fixed::ZERO, Fixed::ONE))?;
                self.refresh_ship_upgrades();
                // Map placements enter at rest with no construction callback or spawn effect.
                self.events.retain(|event| !matches!(event, SimEvent::Spawned { id: spawned } if *spawned == id));
                id
            }
            PlacementEdit::Transform { id, at, quarter_turns } => {
                let unit = self.units.get_mut(id)?;
                if !self.content.as_ref()?.unit(unit.def).is_some_and(source::editable_placement)
                    || unit.progress.is_some()
                    || unit.flags.contains(view::UnitFlags::DYING)
                    || unit.flags.contains(view::UnitFlags::DOCKED)
                {
                    return None;
                }
                unit.pos = at;
                let (x, z) = unit.heading;
                unit.heading = match quarter_turns.rem_euclid(4) {
                    1 => (z, -x),
                    2 => (-x, -z),
                    3 => (-z, x),
                    _ => (x, z),
                };
                unit.target = None;
                unit.speed = Fixed::ZERO;
                unit.omega = Fixed::ZERO;
                self.movement.clear(id);
                id
            }
            PlacementEdit::Inspect(inspection) => {
                return self.inspect_placement(*inspection);
            }
            PlacementEdit::Remove(id) => {
                let unit = self.units.get(id)?;
                if !self.content.as_ref()?.unit(unit.def).is_some_and(source::editable_placement)
                    || unit.progress.is_some()
                    || unit.flags.contains(view::UnitFlags::DYING)
                    || unit.flags.contains(view::UnitFlags::DOCKED)
                    || !self.retire(id)
                {
                    return None;
                }
                id
            }
        };
        self.refresh_ship_upgrades();
        Some(id)
    }

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

    fn snapshot(&self) -> Option<Vec<u8>> {
        Some(Sim::snapshot(self))
    }

    fn production_view(&self) -> Vec<production::view::ProductionView> {
        Sim::production_view(self)
    }

    fn cargo_view(&self) -> Vec<economy::view::CargoView> {
        Sim::cargo_view(self)
    }

    fn node_view(&self) -> Vec<economy::view::NodeView> {
        Sim::node_view(self)
    }

    fn system_view(&self) -> Vec<combat::damage::SystemView> {
        Sim::system_view(self)
    }

    fn warp_phases(&self) -> Vec<(EntityId, bool)> {
        self.approach_warp.to_rec().into_iter()
            .map(|(id, phase)| (id, phase == movement::approach::IN_WARP))
            .collect()
    }

    fn impulse_caps(&self) -> Vec<(EntityId, Fixed)> {
        self.units.iter().filter(|unit| unit.kind == world::EntityKind::Craft)
            .map(|unit| (unit.id, unit.stats.max_speed)).collect()
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
        order(player, 0, command::Order::Move { units: vec![id], to, queue: false, mode: None })
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
                1 => vec![order(0, 0, command::Order::Move { units: vec![a], to: v(500, 30, -300), queue: false, mode: None })],
                10 => vec![order(1, 0, command::Order::Move { units: vec![b], to: v(0, 0, 900), queue: false, mode: None })],
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
            order(1, 0, command::Order::Move { units: vec![a], to: v(100, 0, 0), queue: false, mode: None }),
            // Not implemented yet.
            order(0, 0, command::Order::Cloak { units: vec![a], on: true }),
            // The bench may order anything.
            order(255, 0, command::Order::Move { units: vec![a], to: v(100, 0, 0), queue: false, mode: None }),
        ];
        let out = WorldSource::step(&mut sim, &cmds);
        let rejected = [SimEvent::Rejected { player: PlayerId(0), seq: 0 }, SimEvent::Rejected { player: PlayerId(1), seq: 0 }];
        assert_eq!(out.events, rejected);
        assert_eq!(sim.unit(a).unwrap().target, Some(v(100, 0, 0)));
    }

    #[test]
    fn recrew_is_rejected_without_changing_the_existing_order() {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        let destination = v(100, 0, 0);
        sim.step(&[order(0, 0, command::Order::Move { units: vec![a], to: destination, queue: false, mode: None })]);
        for (seq, on) in [(1, None), (2, Some(true)), (3, Some(false))] {
            let mut baseline = sim.clone();
            baseline.step(&[]);
            let out = sim.step(&[order(0, seq, command::Order::Recrew { units: vec![a], on })]);
            assert!(out.events.contains(&SimEvent::Rejected { player: PlayerId(0), seq }));
            assert_eq!(sim.unit(a).unwrap().target, Some(destination));
            assert_eq!(sim.digest(), baseline.digest());
        }
    }
}
