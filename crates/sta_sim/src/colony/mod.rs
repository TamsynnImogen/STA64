//! Colonization and planet population (SIM-30). Armada II family only: the
//! rules are active when the content has the `RTS_CFG.h` population keys
//! (`sta_content::PopulationRules`) and a Producer planet with a `maxPopulation`
//! level; Armada I and the Fleet Ops line have none. Rules read from the
//! Armada II 1.1 executable (static) with the numbers from stock data; nothing
//! here was run on the bench. Implements D5 (data keys, no unit names).
//!
//! **Order.** `Order::Colonize` is accepted for a pod colony ship (class
//! `colony` whose first weapon's ordnance is a `ColonizerPod`) of the issuer,
//! or of anyone for a script (which needs the ship's special-forces flag) or
//! the bench; the target is a live Producer planet with `isColonizable`, with
//! no colonizer yet, unowned or owned by the ship's owner. The ship colonizes
//! `cfgPOP_SPARSE` colonists per order (not the ODF `colonists`). There is no
//! maximum order distance: out of range the ship travels until its centre
//! distance is at most `orbitRange + 2 x planet radius + ship radius - 15`
//! (the planet radius counts twice, as in the original), then it becomes the
//! planet's colonizer; a planet that already has another colonizer, or is
//! owned by someone else, stops the order.
//!
//! **Pods.** The first pod leaves at once, then one per `shotDelay` of the
//! launcher; its payload is `min(colonists left, partySize)`, taken from the
//! ship at launch. It lands at once (flight time: not measured) when the
//! planet is unowned or the ship's: population becomes
//! `clamp(population + payload, 0, cap)` with `cap` the `cfgPOP_` of the
//! planet's level. Done when population reaches the cap or fewer than one
//! colonist is left: the planet changes owner to the ship's owner if it
//! differs (the shared guarded transfer, so crew capacity follows) and the ship
//! is consumed (landing spiral duration: not measured, so it goes at once).
//! Not implemented, with the reason: the ship's crew moves to the planet
//! (planets have no crew pool in the sim); Borg colonizing beams
//! (`colonybm` ordnance, `cleansingWeaponIndex`): their order is rejected.
//!
//! **Interrupts.** Stop while colonizing: population becomes
//! `max(population at order start, population - colonists launched)` and the
//! colonizer is cleared; any other replacing order clears the colonizer and
//! leaves the landed colonists; a dying ship clears the colonizer, no rollback.
//!
//! **Growth.** A planet owned by a player with at least `cfgPOP_SPARSE` people
//! grows by `(4 P / cap + 1) x crewAccumulationRate(owner's race) x
//! civilianPopulationModifier x cfgPLANET_CIVILIAN_RATE_MODIFIER x 0.2` per
//! second, clamped to `[0, cap]`; a value above the cap is cut each tick. An
//! owned planet with population 0 or less becomes neutral on the next tick
//! (population kept). Weapon damage that reaches the hull of a planet with a
//! population row costs `0.1 x damage` people and leaves the hull alone.
//! Crew: each owned finished planet adds `P / cfgPOP_HEAVY x race rate x 4` crew
//! per second to its owner (`economy::crew`). A planet with no row has
//! population 0 and is not neutralised (the map-load path is not measured).
//!
//! State is in snapshots (format 27) and in the digest once any row exists.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, PlanetRole, PopulationRules};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Tick, Vec3};

use crate::command::Order;
use crate::view::UnitFlags;
use crate::world::{Entity, Fnv, NEUTRAL};
use crate::Sim;

/// The 0.2 of the growth rule and the 0.1 of the damage rule, as exact 32.32 values
/// (the bits `from_f64` gives; a test pins that, so no digest moves).
const GROWTH_FACTOR: Fixed = Fixed::from_bits(858_993_459);
const HIT_PEOPLE_FRACTION: Fixed = Fixed::from_bits(429_496_730);

/// Largest population a script may store (the original does not clamp).
const SCRIPT_LIMIT: i32 = 1_000_000_000;

/// One planet's colony state: a row exists once it was colonized or a script
/// set its population.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanetRow {
    /// People (`civilianPopulation`); the only population store.
    pub population: Fixed,
    /// The colony ship colonizing it now.
    pub colonizer: Option<EntityId>,
}

/// One colony ship's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShipProc {
    pub planet: EntityId,
    /// In range and the planet's colonizer: pods are launching.
    pub active: bool,
    /// Colonists still aboard.
    pub colonists_left: Fixed,
    pub colonists_at_start: Fixed,
    /// The planet's population when the order was given.
    pub population_at_start: Fixed,
    /// The tick the next pod leaves.
    pub next_launch: Tick,
    /// This order created the planet's population row (the planet had none when
    /// the ship started), so an order that ends with nobody landed removes it again.
    pub made_row: bool,
}

/// What a pod colony ship launches, from its definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pod {
    pub party: Fixed,
    pub orbit_range: Fixed,
    /// Ticks between pods.
    pub delay: Tick,
}

/// A pod ship's numbers: class `colony` whose first weapon's ordnance is a
/// `ColonizerPod` (read from data). Borg beam ships return `None`.
pub(crate) fn pod_of(db: &ContentDb, def: &sta_content::UnitDef) -> Option<Pod> {
    if def.class.0 != "colony" {
        return None;
    }
    let w = db.weapon(def.hardpoints.first()?.weapon.key)?;
    let c = w.colonize?;
    Some(Pod { party: c.party_size, orbit_range: c.orbit_range, delay: w.refire.max(1) })
}

/// All colony state, ascending id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Colonies {
    planets: BTreeMap<EntityId, PlanetRow>,
    ships: BTreeMap<EntityId, ShipProc>,
}

/// What the view shows of one planet row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColonyView {
    pub id: EntityId,
    /// People now.
    pub population: Fixed,
    /// The planet class's cap (`cfgPOP_` of its `maxPopulation` level); zero
    /// without the rules.
    pub max_population: Fixed,
    pub colonizer: Option<EntityId>,
}

/// One ship order as a snapshot row (see [`ColoniesRec::ships`]).
pub type ShipRow = (EntityId, EntityId, bool, i64, i64, i64, Tick, bool);

/// The snapshot form of [`Colonies`] (format 27).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColoniesRec {
    pub version: u8,
    /// `(planet, population bits, colonizer)`, ascending planet.
    pub planets: Vec<(EntityId, i64, Option<EntityId>)>,
    /// `(ship, planet, active, left bits, at start bits, population at start bits, next launch, made row)`, ascending ship.
    pub ships: Vec<ShipRow>,
}

// ---- small rules, each tested ----

/// Integer square root of a `u128` (Newton, exact floor).
fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = 1u128 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) >> 1;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// XZ distance in 128-bit arithmetic, so map-sized separations do not wrap the
/// 32.32 multiply (the review of this package found the naive form overflowing
/// above about 46 000 units).
fn flat(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (i128::from(a.x.to_bits()) - i128::from(b.x.to_bits()), i128::from(a.z.to_bits()) - i128::from(b.z.to_bits()));
    let d = isqrt((dx * dx + dz * dz) as u128);
    Fixed::from_bits(i64::try_from(d).unwrap_or(i64::MAX))
}

/// The ship is close enough to start: centre distance at most
/// `orbitRange + 2 x planet radius + ship radius - 15`.
pub(crate) fn in_colonize_range(dist: Fixed, orbit_range: Fixed, planet_radius: Fixed, ship_radius: Fixed) -> bool {
    dist <= orbit_range + planet_radius + planet_radius + ship_radius - Fixed::from_int(15)
}

/// Where a ship out of range flies: on the line from the planet toward the
/// ship, at centre distance `2 x planet radius + ship radius + orbitRange - 100`.
pub(crate) fn approach_point(planet: Vec3, ship: Vec3, heading: (Fixed, Fixed), planet_radius: Fixed, ship_radius: Fixed, orbit_range: Fixed) -> Vec3 {
    let want = (planet_radius + planet_radius + ship_radius + orbit_range - Fixed::from_int(100)).max(Fixed::ZERO);
    let (dx, dz) = (ship.x - planet.x, ship.z - planet.z);
    let d = flat(ship, planet);
    let (ux, uz) = if d > Fixed::ZERO { (dx / d, dz / d) } else { (-heading.0, -heading.1) };
    Vec3 { x: planet.x + ux * want, y: ship.y, z: planet.z + uz * want }
}

/// The colonists one pod carries.
pub(crate) fn pod_payload(left: Fixed, party: Fixed) -> Fixed {
    left.min(party).max(Fixed::ZERO)
}

/// Population per second of an owned planet at or above the sparse level.
pub(crate) fn growth_per_second(pop: Fixed, cap: Fixed, race_rate: Fixed, civilian_modifier: Fixed, cfg_modifier: Fixed) -> Fixed {
    if cap <= Fixed::ZERO {
        return Fixed::ZERO;
    }
    // The population is clamped to the cap first and divided before multiplying, so
    // a script-set value cannot wrap the 32.32 product.
    let pop = pop.max(Fixed::ZERO).min(cap);
    (Fixed::from_int(4) * (pop / cap) + Fixed::ONE) * race_rate * civilian_modifier * cfg_modifier * GROWTH_FACTOR
}

/// Crew per second a planet of `pop` people gives its owner.
pub(crate) fn crew_per_second(pop: Fixed, heavy: Fixed, race_rate: Fixed) -> Fixed {
    if heavy <= Fixed::ZERO {
        return Fixed::ZERO;
    }
    pop.max(Fixed::ZERO) / heavy * race_rate * Fixed::from_int(4)
}

/// Population after a cancelled colonization.
pub(crate) fn cancelled_population(at_start: Fixed, now: Fixed, launched: Fixed) -> Fixed {
    at_start.max(now - launched)
}

impl Colonies {
    pub fn is_empty(&self) -> bool {
        self.planets.is_empty() && self.ships.is_empty()
    }

    pub fn row(&self, planet: EntityId) -> Option<&PlanetRow> {
        self.planets.get(&planet)
    }

    pub fn process(&self, ship: EntityId) -> Option<&ShipProc> {
        self.ships.get(&ship)
    }

    /// A ship's order ends. `cancel`: Stop, which takes back the colonists
    /// launched so far; other endings leave them landed.
    pub(crate) fn end_process(&mut self, ship: EntityId, cancel: bool) {
        let Some(p) = self.ships.remove(&ship) else { return };
        let Some(row) = self.planets.get_mut(&p.planet).filter(|r| r.colonizer == Some(ship)) else { return };
        row.colonizer = None;
        if cancel {
            row.population = cancelled_population(p.population_at_start, row.population, p.colonists_at_start - p.colonists_left);
        }
        // An order that made the row and ends with nobody landed leaves no row behind:
        // a row would keep the planet's hull from ever being damaged (the damage path
        // keys on the row) and would neutralise an owned planet that had none.
        if p.made_row && self.planets.get(&p.planet).is_some_and(|r| r.population <= Fixed::ZERO && r.colonizer.is_none()) {
            self.planets.remove(&p.planet);
        }
    }

    /// An entity left the world.
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.end_process(id, false);
        if self.planets.remove(&id).is_some() {
            self.ships.retain(|_, p| p.planet != id);
        }
    }

    /// One pod, when it is time. True when the ship is done.
    pub(crate) fn fire(&mut self, ship: EntityId, tick: Tick, pod: Pod, cap: Fixed) -> bool {
        let Some(p) = self.ships.get_mut(&ship) else { return false };
        let Some(row) = self.planets.get_mut(&p.planet) else { return false };
        let done = |row: &PlanetRow, p: &ShipProc| row.population >= cap || p.colonists_left < Fixed::ONE;
        if done(row, p) {
            return true;
        }
        if tick < p.next_launch {
            return false;
        }
        let payload = pod_payload(p.colonists_left, pod.party);
        if payload <= Fixed::ZERO {
            return true;
        }
        p.colonists_left = p.colonists_left - payload;
        row.population = (row.population + payload).max(Fixed::ZERO).min(cap);
        p.next_launch = tick + pod.delay;
        done(row, p)
    }

    /// Whether the planet has a population row (its hull damage costs people).
    pub(crate) fn has_row(&self, planet: EntityId) -> bool {
        self.planets.contains_key(&planet)
    }

    /// A hit's hull damage reached the planet: `0.1 x damage` people are lost
    /// (read from the executable, static), never below zero.
    pub(crate) fn lose_population(&mut self, planet: EntityId, damage: Fixed) {
        if let Some(row) = self.planets.get_mut(&planet) {
            row.population = (row.population - HIT_PEOPLE_FRACTION * damage).max(Fixed::ZERO);
        }
    }

    pub(crate) fn to_rec(&self) -> ColoniesRec {
        ColoniesRec {
            version: 1,
            planets: self.planets.iter().map(|(id, r)| (*id, r.population.to_bits(), r.colonizer)).collect(),
            ships: self
                .ships
                .iter()
                .map(|(id, p)| (*id, p.planet, p.active, p.colonists_left.to_bits(), p.colonists_at_start.to_bits(), p.population_at_start.to_bits(), p.next_launch, p.made_row))
                .collect(),
        }
    }

    /// Rebuild from a snapshot record: ascending ids, entities that exist, planets
    /// that are Producer planets (the only ones the sim gives a row), and a colonizer link that agrees with its ship's order.
    pub(crate) fn from_rec(rec: ColoniesRec, exists: impl Fn(EntityId) -> bool, producer: impl Fn(EntityId) -> bool) -> Option<Colonies> {
        if rec.version != 1
            || !rec.planets.windows(2).all(|w| w[0].0 < w[1].0)
            || !rec.ships.windows(2).all(|w| w[0].0 < w[1].0)
        {
            return None;
        }
        const LIMIT: i64 = 1_000_000_000i64 << 32;
        let in_range = |bits: i64| (-LIMIT..=LIMIT).contains(&bits);
        if !rec.planets.iter().all(|r| in_range(r.1))
            || !rec.ships.iter().all(|r| (0..=LIMIT).contains(&r.3) && (0..=LIMIT).contains(&r.4) && r.3 <= r.4 && in_range(r.5))
        {
            return None;
        }
        let planets: BTreeMap<_, _> = rec
            .planets
            .into_iter()
            .map(|(id, pop, colonizer)| (id, PlanetRow { population: Fixed::from_bits(pop), colonizer }))
            .collect();
        let ships: BTreeMap<_, _> = rec
            .ships
            .into_iter()
            .map(|(id, planet, active, left, start, pop0, next_launch, made_row)| {
                (id, ShipProc {
                    planet,
                    active,
                    colonists_left: Fixed::from_bits(left),
                    colonists_at_start: Fixed::from_bits(start),
                    population_at_start: Fixed::from_bits(pop0),
                    next_launch,
                    made_row,
                })
            })
            .collect();
        let linked = |ship: EntityId, planet: EntityId| ships.get(&ship).is_some_and(|p| p.active && p.planet == planet);
        let ok = planets.keys().all(|&id| exists(id) && producer(id))
            && planets.iter().all(|(&id, r)| r.colonizer.is_none_or(|s| linked(s, id)))
            && ships.iter().all(|(&id, p)| {
                exists(id) && exists(p.planet) && producer(p.planet) && (!p.active || planets.get(&p.planet).is_some_and(|r| r.colonizer == Some(id)))
            });
        ok.then_some(Colonies { planets, ships })
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("colonies-v1");
        h.u64(self.planets.len() as u64);
        for (id, r) in &self.planets {
            h.u64(u64::from(id.0));
            h.fixed(r.population);
            h.u64(r.colonizer.map_or(0, |c| u64::from(c.0)));
        }
        h.u64(self.ships.len() as u64);
        for (id, p) in &self.ships {
            h.u64(u64::from(id.0));
            h.u64(u64::from(p.planet.0));
            h.u64(u64::from(p.active));
            h.fixed(p.colonists_left);
            h.fixed(p.colonists_at_start);
            h.fixed(p.population_at_start);
            h.u64(p.next_launch);
            h.u64(u64::from(p.made_row));
        }
    }
}

fn alive(e: &Entity) -> bool {
    e.hull[0] > Fixed::ZERO && !e.flags.contains(UnitFlags::DYING)
}

impl Sim {
    /// People on a planet, when it has a population row.
    pub fn planet_population(&self, planet: EntityId) -> Option<Fixed> {
        self.colonies.row(planet).map(|r| r.population)
    }

    pub fn colonies(&self) -> &Colonies {
        &self.colonies
    }

    /// `crewAccumulationRate` of the seat's race; zero when unknown.
    fn seat_crew_rate(&self, db: &ContentDb, p: PlayerId) -> Fixed {
        let Some(race) = self.seat_race(p).map(|r| r.trim().to_owned()) else { return Fixed::ZERO };
        db.rules()
            .crew()
            .and_then(|c| c.races.iter().find(|(k, _)| k.eq_ignore_ascii_case(&race)))
            .map_or(Fixed::ZERO, |(_, r)| *r)
    }

    /// `Order::Colonize`: see the module docs. False when no ship could take it.
    pub(crate) fn apply_colonize(&mut self, player: PlayerId, ships: &[EntityId], planet: EntityId) -> bool {
        let Some(db) = self.content.clone() else { return false };
        let Some(rules) = db.rules().population().copied() else { return false };
        let Some(pl) = self.units.get(planet).filter(|p| alive(p)) else { return false };
        if !db.unit(pl.def).is_some_and(|d| d.planet_role == Some(PlanetRole::Producer) && d.planet_colonizable && rules.cap(d.planet_max_population) > Fixed::ZERO) {
            return false;
        }
        if self.colonies.row(planet).is_some_and(|r| r.colonizer.is_some()) {
            return false;
        }
        let (planet_owner, pop0) = (pl.owner, self.colonies.row(planet).map_or(Fixed::ZERO, |r| r.population));
        let mut ids: Vec<EntityId> = ships
            .iter()
            .copied()
            .filter(|&id| {
                !self.production.is_welding(id)
                    && self.units.get(id).filter(|u| alive(u)).is_some_and(|u| {
                        u.orderable()
                            && (!player.is_player() || u.owner == player)
                            // A mission's Colonize needs the special-forces flag (read from the executable, static).
                            && (player != PlayerId::SCRIPT || self.switches.get(id).special_forces)
                            && (planet_owner == NEUTRAL || planet_owner == u.owner)
                            && db.unit(u.def).and_then(|d| pod_of(&db, d)).is_some()
                    })
            })
            .collect();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return false;
        }
        // Like every order that replaces what a craft is doing.
        self.weapons.stop_attack(&self.units, player, &ids);
        self.economy.cancel(&self.units, player, &ids);
        self.production.cancel(&db, &self.units, &mut self.economy, player, &ids);
        self.clear_scripted(player, &ids);
        for &id in &ids {
            self.watchdog.remove(id);
            self.movement.clear_waypoints(id);
            self.set_target(id, None);
            self.colonies.end_process(id, false);
            // The colonists of an order: `cfgPOP_SPARSE`, not the ODF's `colonists`.
            let colonists = rules.cap(1);
            self.colonies.ships.insert(
                id,
                ShipProc { planet, active: false, colonists_left: colonists, colonists_at_start: colonists, population_at_start: pop0, next_launch: 0, made_row: false },
            );
        }
        true
    }

    /// What other orders do to a colonizing ship, after the order was taken.
    /// Stop cancels; a replacing order just ends the colonization.
    pub(crate) fn colonies_after_order(&mut self, player: PlayerId, order: &Order) {
        if self.colonies.ships.is_empty() {
            return;
        }
        let (units, cancel): (&[EntityId], bool) = match order {
            Order::Stop { units } => (units, true),
            Order::Move { units, queue: false, .. }
            | Order::MoveWarp { units, .. }
            | Order::Attack { units, .. }
            | Order::Mine { units, .. }
            | Order::Patrol { units, .. }
            | Order::SearchAndDestroy { units }
            | Order::Scout { units }
            | Order::AttackAny { units }
            | Order::AttackPoint { units, .. } => (units, false),
            Order::Build { builder, .. } => (std::slice::from_ref(builder), false),
            _ => return,
        };
        for &id in units {
            if self.units.get(id).is_some_and(|u| !player.is_player() || u.owner == player) {
                self.colonies.end_process(id, cancel);
            }
        }
    }

    /// `Edit::PlanetPopulation` (mission `SetPlanetCivPop`): the population is
    /// set as given within +-1e9; growth cuts it to the cap on the next tick.
    /// STA64 policy: only a live Producer planet (the original has no type check).
    pub(crate) fn edit_planet_population(&mut self, id: EntityId, people: Fixed) -> bool {
        let Some(db) = self.content.clone() else { return false };
        let ok = self.units.get(id).filter(|u| alive(u)).is_some_and(|u| db.unit(u.def).is_some_and(|d| d.planet_role == Some(PlanetRole::Producer)));
        if !ok {
            return false;
        }
        let limit = Fixed::from_int(SCRIPT_LIMIT);
        let people = people.max(-limit).min(limit);
        self.colonies.planets.entry(id).or_insert(PlanetRow { population: Fixed::ZERO, colonizer: None }).population = people;
        true
    }

    /// A craft changed owner: its colonization ends.
    pub(crate) fn colonies_craft_transferred(&mut self, id: EntityId) {
        self.colonies.end_process(id, false);
    }

    /// One tick of every colony ship, then every planet.
    pub(crate) fn step_colonies(&mut self) {
        if self.colonies.is_empty() {
            return;
        }
        let Some(db) = self.content.clone() else { return };
        let Some(rules) = db.rules().population().copied() else { return };
        self.step_colony_ships(&db, &rules);
        self.step_colony_planets(&db, &rules);
    }

    fn step_colony_ships(&mut self, db: &ContentDb, rules: &PopulationRules) {
        let ids: Vec<EntityId> = self.colonies.ships.keys().copied().collect();
        for id in ids {
            let Some(proc) = self.colonies.ships.get(&id).copied() else { continue };
            let seen = (|| {
                let (ship, planet) = (self.units.get(id).filter(|u| alive(u))?, self.units.get(proc.planet).filter(|u| alive(u))?);
                let (sdef, pdef) = (db.unit(ship.def)?, db.unit(planet.def)?);
                Some((ship.pos, ship.heading, ship.owner, ship.target.is_some(), planet.pos, planet.owner, sdef.radius, pdef.radius, pdef.planet_max_population, pod_of(db, sdef)?))
            })();
            let Some((spos, heading, owner, has_target, ppos, powner, sr, pr, level, pod)) = seen else {
                self.colonies.end_process(id, false);
                continue;
            };
            if !in_colonize_range(flat(spos, ppos), pod.orbit_range, pr, sr) {
                if !has_target {
                    let to = approach_point(ppos, spos, heading, pr, sr, pod.orbit_range);
                    self.set_target(id, Some(to));
                    self.movement.clear_waypoints(id);
                }
                continue;
            }
            if powner != NEUTRAL && powner != owner {
                self.colonies.end_process(id, false);
                self.set_target(id, None);
                continue;
            }
            match (proc.active, self.colonies.row(proc.planet).and_then(|r| r.colonizer)) {
                (_, Some(other)) if other != id => {
                    self.colonies.end_process(id, false);
                    self.set_target(id, None);
                    continue;
                }
                (false, _) => {
                    let made_row = !self.colonies.planets.contains_key(&proc.planet);
                    self.colonies.planets.entry(proc.planet).or_insert(PlanetRow { population: Fixed::ZERO, colonizer: None }).colonizer = Some(id);
                    if let Some(p) = self.colonies.ships.get_mut(&id) {
                        p.made_row = made_row;
                        p.active = true;
                        p.next_launch = self.tick;
                    }
                    self.set_target(id, None);
                    self.movement.clear_waypoints(id);
                }
                _ => {}
            }
            if self.colonies.fire(id, self.tick, pod, rules.cap(level)) {
                self.finish_colonization(id, proc.planet, owner, powner);
            }
        }
    }

    /// The pods are done: the planet is the ship owner's and the ship is consumed.
    fn finish_colonization(&mut self, ship: EntityId, planet: EntityId, owner: PlayerId, planet_owner: PlayerId) {
        self.colonies.end_process(ship, false);
        // A refused transfer (eliminated seat, unrepresentable owner) keeps the ship.
        if planet_owner != owner && !self.transfer_owner(PlayerId::SCRIPT, planet, owner) {
            return;
        }
        self.retire(ship);
    }

    fn step_colony_planets(&mut self, db: &ContentDb, rules: &PopulationRules) {
        let ids: Vec<EntityId> = self.colonies.planets.keys().copied().collect();
        for id in ids {
            let Some(pl) = self.units.get(id).filter(|u| u.hull[0] > Fixed::ZERO) else {
                continue;
            };
            let Some(pdef) = db.unit(pl.def) else { continue };
            let (cap, civ, owner) = (rules.cap(pdef.planet_max_population), pdef.planet_civilian_population_modifier, pl.owner);
            let Some(pop) = self.colonies.row(id).map(|r| r.population) else { continue };
            // Max crew 0 neutralizes an owned planet on the next tick; population is kept.
            let owner = if owner.is_player() && pop <= Fixed::ZERO && self.transfer_owner(PlayerId::SCRIPT, id, NEUTRAL) { NEUTRAL } else { owner };
            let step = if owner.is_player() && pop >= rules.cap(1) {
                growth_per_second(pop, cap, self.seat_crew_rate(db, owner), civ, rules.civilian_rate_modifier) / Fixed::from_int(TICK_HZ as i32)
            } else {
                Fixed::ZERO
            };
            if let Some(r) = self.colonies.planets.get_mut(&id) {
                r.population = (pop + step).max(Fixed::ZERO).min(cap);
            }
        }
    }

    /// Crew per second each owned finished planet with people gives its owner.
    pub(crate) fn colony_crew_income(&self, db: &ContentDb) -> Vec<(PlayerId, Fixed)> {
        let Some(rules) = db.rules().population() else { return Vec::new() };
        let heavy = rules.cap(4);
        self.colonies
            .planets
            .iter()
            .filter_map(|(&id, row)| {
                let u = self.units.get(id).filter(|u| alive(u) && u.owner.is_player() && u.progress.is_none_or(|p| p >= Fixed::ONE))?;
                Some((u.owner, crew_per_second(row.population, heavy, self.seat_crew_rate(db, u.owner))))
            })
            .filter(|(_, r)| *r > Fixed::ZERO)
            .collect()
    }

    /// The planets with a population row, for the view.
    pub(crate) fn colony_views(&self) -> Vec<ColonyView> {
        let rules = self.content.as_deref().and_then(|db| db.rules().population());
        self.colonies
            .planets
            .iter()
            .map(|(&id, r)| {
                let level = self.content.as_deref().and_then(|db| db.unit(self.units.get(id)?.def)).map_or(0, |d| d.planet_max_population);
                ColonyView { id, population: r.population, max_population: rules.map_or(Fixed::ZERO, |p| p.cap(level)), colonizer: r.colonizer }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
