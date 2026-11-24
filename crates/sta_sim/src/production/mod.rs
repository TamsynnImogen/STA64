//! Construction (SIM-08): yards with build queues, and construction rigs that
//! deploy, weld a station and reveal it.
//!
//! - `Order::Build { builder, what, at }`:
//!   - A finished **station** (yard, starbase; [`EntityKind::Station`]) queues
//!     `what` (up to its resolved [`ContentDb::build_queue_limit`], including
//!     the active head; `at` is ignored). Each yard has its
//!     own queue and they run in parallel. The head item takes the def's
//!     `build.ticks`; a ship appears on the tick it finishes, at the yard's
//!     launch hardpoint facing out (`launch`, SIM-16; at the yard's centre
//!     when its def has none), then [`SimEvent::Built`] and `Spawned`; the
//!     next item starts
//!     in that same tick.
//!     A yard with a resolved `wormhole` opens that aperture after the build
//!     timer and releases the ship at its build hardpoint after opening.
//!   - A **craft** (construction rig) needs `at: Some(site)`. It steers there;
//!     within [`deploy_reach`] of the site (the rig's slow-turn orbit plus a margin, from its ODF movement stats) it deploys: the
//!     station entity appears at the site (`progress` 0,
//!     [`SimEvent::BuildStarted`]) and welds for `build.ticks`, `progress`
//!     rising each tick; on the last one `progress` becomes `None` (finished,
//!     revealed) and [`SimEvent::Built`] is emitted. The rig is held in place
//!     until the weld completes or is explicitly cancelled.
//!   - Timing: an item queued (or a rig deployed) in the step of tick `t`
//!     starts in that step and completes in the step of tick `t + ticks`.
//! - Payment: with the builder's resolved `Features::pay_on_queue` the cost leaves the player's stock
//!   when the order is accepted (rejected if it cannot be paid); otherwise a
//!   queue item waits at the head until the stock covers it and pays as it
//!   starts, and a rig pays on deploy (dropping the deploy if it cannot).
//! - Cancel: a `Stop` on a yard drops its whole queue, a non-queued `Move` or a
//!   `Stop` on a rig drops its pending deploy; whatever was paid is refunded in
//!   full (items that have started included). A yard or rig destroyed before
//!   its item or deploy completes refunds the same way. A weld in progress is
//!   not refundable. Stop cancels an active weld and removes its unfinished
//!   station; Move is refused while welding, including queued movement.
//!   `CancelBuild` removes one station queue job and refunds its payment,
//!   leaving the builder and the other jobs alone.
//! - Accepted: any def the content db has, if the builder's `build.list` is
//!   empty or names it. Tech prerequisites
//!   are checked before this, in `Sim::apply` ([`crate::tech`], SIM-09).
//! - Officers ([`officers`], SIM-26): a build's `officerCost` is reserved when
//!   it is paid (queued under `pay_on_queue`, started otherwise) and counted
//!   in the player's officer census with the live actors. Over the player's
//!   limit an order is refused (`pay_on_queue`), or its head waits unstarted.
//!   Cancel and loss of the builder release it.
//! - A mining station finished by a rig whose def names `freighterName` gets that unit
//!   free, at its dock node ([`free_unit`]).
//! - Not yet: cooperative multi-yard builds on one item,
//!   `Features::construction_rigs` gating.
//! - A rig's site must pass [`site::check_site`] (inside the map, clear of other
//!   objects) or the order is rejected. A station of class `p_mining` (SIM-62)
//!   instead snaps to a planet's orbit slot, or the order is rejected: [`orbit`].
//! - Persistent state is [`Production`]; the snapshot (`to_rec`/`from_rec`)
//!   must include it. It is in the digest once any exists.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3};

use crate::command::{BuildQueueId, Edit};
use crate::economy::Economy;
use crate::event::SimEvent;
use crate::world::{Entities, Entity, EntityKind, Fnv, Table};

pub mod afford;
mod free_unit;
#[cfg(test)]
mod afford_tests;
mod launch;
pub(crate) mod officers;
pub mod orbit;
pub(crate) mod research;
pub(crate) mod officer_upgrades;
pub mod rally;
pub mod site;
pub mod view;
mod transfer;

/// Legacy five-job fallback, retained for downstream source compatibility.
/// Station admission uses the builder's resolved content rule instead.
#[deprecated(note = "Use ContentDb::build_queue_limit for the producer's resolved capacity")]
pub const QUEUE_MAX: usize = 5;

/// How close a rig must come to deploy, from its own movement data. A craft
/// that is not facing its target slows to [`MIN_ALIGN`](crate::MIN_ALIGN) of
/// cruise and turns at the rate that speed allows, so one that keeps missing
/// a point settles into an orbit of radius `speed / omega` at that speed.
/// Reach is that orbit plus a quarter as a margin and the kinematics' own
/// arrival distance: a nimble rig must nearly arrive, a sluggish one deploys
/// from further out, and none can circle its site for ever. Pure movement data
/// (`impulseSpeed`, `turnOmega`, `turnOmegaFractionAtRest`), so it holds for
/// any game or mod.
pub fn deploy_reach(stats: &crate::MoveStats) -> Fixed {
    let a = Fixed::from_f64(crate::MIN_ALIGN);
    let omega = stats.turn_omega * (stats.turn_at_rest + (Fixed::ONE - stats.turn_at_rest) * a);
    let orbit = if omega > Fixed::ZERO {
        stats.max_speed * a / omega
    } else {
        Fixed::ZERO
    };
    orbit + orbit / Fixed::from_int(4) + crate::arrive_distance()
}

/// Gap kept between a rig and the footprint of the structure it builds, units.
pub const STAND_GAP: i32 = 10;

/// How far from a site's centre its rig parks: the structure's footprint, the
/// rig's own body and a small gap (SIM-21).
pub fn stand_ring(db: &ContentDb, units: &Entities, what: ContentKey, rig: EntityId) -> Fixed {
    let body = |def: ContentKey| db.unit(def).map_or(Fixed::ZERO, |d| d.radius * Fixed::from_int(3) / Fixed::from_int(4));
    let rig_body = units.get(rig).map_or(Fixed::ZERO, |r| body(r.def));
    site::footprint(db, what).max(body(what)) + rig_body + Fixed::from_int(STAND_GAP)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Item {
    id: BuildQueueId,
    def: ContentKey,
    owner: PlayerId,
    /// Ticks built so far; only counts once `started`.
    done: u64,
    started: bool,
    paid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Deploy {
    def: ContentKey,
    owner: PlayerId,
    at: Vec3,
    paid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Weld {
    builder: EntityId,
    done: u64,
    ticks: u64,
}

/// Build queues, pending rig deploys and stations being welded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Production {
    /// Per-builder destinations (SIM-17), independent of a build queue.
    rallies: rally::Rallies,
    /// By yard.
    queues: Table<Vec<Item>>,
    /// A job id is never reused, even after the queue becomes empty.
    last_job_id: u64,
    /// By rig.
    deploys: Table<Deploy>,
    /// By the station being welded.
    welds: Table<Weld>,
    research: Table<research::Attachment>,
    officer_upgrades: Table<u32>,
}

#[derive(Default)]
pub(crate) struct Completions {
    pub evolved: Vec<(EntityId, EntityId)>,
    pub gate_rallies: Vec<(EntityId, Vec3)>,
    pub replaced_pods: Vec<EntityId>,
}

/// `(def, owner, done, started, paid, job id)`.
pub type ItemRec = (u32, u8, u64, bool, bool, u64);

/// Plain form of [`Production`] for snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionRec {
    /// `(builder, original owner, point or entity destination)` in id order.
    pub rallies: rally::RallyRec,
    /// `(yard, [(def, owner, done, started, paid, job id)])`.
    pub queues: Vec<(u32, Vec<ItemRec>)>,
    /// `(rig, def, owner, [x, y, z] bits, paid)`.
    pub deploys: Vec<(u32, u32, u8, [i64; 3], bool)>,
    /// `(station, builder, done, ticks)`.
    pub welds: Vec<(u32, u32, u64, u64)>,
    /// Last assigned queue job id. Zero before the first admission.
    pub last_job_id: u64,
}

fn finished(e: &Entity) -> bool {
    e.progress.is_none_or(|p| p >= Fixed::ONE)
}

/// Resolved, non-null `pushBuildItem0..9` in authored slot order. Gaps and
/// unresolved class names are skipped; repeated class names remain repeated.
fn push_build_defs(db: &ContentDb, units: &Entities, id: EntityId) -> Vec<ContentKey> {
    let Some(yard) = units.get(id) else { return Vec::new() };
    let Some(def) = db.unit(yard.def) else { return Vec::new() };
    if !finished(yard) || !EntityKind::producer_def(def) {
        return Vec::new();
    }
    def.stats.push_build_items
        .iter()
        .filter_map(|name| db.key(name).filter(|&key| db.unit(key).is_some()))
        .collect()
}

impl crate::Sim {
    /// Enqueue an already finished producer's ODF push-build items once.
    /// Call on creation or replacement, never on routine per-tick updates.
    pub(crate) fn push_build_items(&mut self, id: EntityId) -> usize {
        let Some(db) = self.content.as_deref() else { return 0 };
        let defs = push_build_defs(db, &self.units, id);
        defs.into_iter().filter(|&what| {
            self.production.push_build_item(db, &self.units, &mut self.economy, id, what)
        }).count()
    }
}

fn ticks_of(db: &ContentDb, def: ContentKey) -> u64 {
    db.unit(def).map_or(1, |d| d.build.ticks.max(1))
}

/// Take the whole cost of `def` from `p`, or nothing.
fn can_pay(eco: &Economy, db: &ContentDb, p: PlayerId, def: ContentKey) -> bool {
    let Some(d) = db.unit(def) else { return false };
    let crew = officer_upgrades::crew_cost(d);
    afford::first_short(&eco.stock_of(p), &eco.scaled_cost(p, &d.build.cost)).is_none()
        && (crew == Fixed::ZERO || eco.crew_of(p).is_some_and(|pool| pool >= crew))
}

fn pay(eco: &mut Economy, db: &ContentDb, p: PlayerId, def: ContentKey) -> bool {
    let Some(d) = db.unit(def) else { return false };
    if !can_pay(eco, db, p, def) {
        return false;
    }
    for (i, c) in eco.scaled_cost(p, &d.build.cost).into_iter().enumerate() {
        if c > Fixed::ZERO {
            let ok = eco.spend(p, i as u8, c);
            debug_assert!(ok);
        }
    }
    let crew = officer_upgrades::crew_cost(d);
    if crew > Fixed::ZERO {
        let paid = eco.withdraw_crew(p, crew);
        debug_assert_eq!(paid, crew);
    }
    true
}

fn refund(eco: &mut Economy, db: &ContentDb, p: PlayerId, def: ContentKey) {
    let Some(d) = db.unit(def) else { return };
    for (i, c) in eco.scaled_cost(p, &d.build.cost).into_iter().enumerate() {
        if c > Fixed::ZERO {
            let now = eco.stock_of(p).get(i).copied().unwrap_or(Fixed::ZERO);
            eco.apply_edit(PlayerId::BENCH, &Edit::Cargo(p, i as u8, now + c));
        }
    }
    officer_upgrades::credit_crew(eco, p, officer_upgrades::crew_cost(d));
}

impl Production {
    /// Admit one ODF push-build item through the yard's ordinary queue rules.
    /// The callback's items need not appear in the player's build list.
    fn push_build_item(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        eco: &mut Economy,
        yard: EntityId,
        what: ContentKey,
    ) -> bool {
        let Some(y) = units.get(yard) else { return false };
        let Some(bd) = db.unit(y.def) else { return false };
        let Some(product) = db.unit(what) else { return false; };
        if !finished(y) || !EntityKind::producer_def(bd) || !self.admits_officer_upgrade(y, bd, product) {
            return false;
        }
        if self.research_block(db, units, yard, bd, product, false).is_some() {
            return false;
        }
        self.queue_build(db, units, eco, y, bd, what)
    }

    fn queue_build(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        eco: &mut Economy,
        builder: &Entity,
        builder_def: &sta_content::UnitDef,
        what: ContentKey,
    ) -> bool {
        let limit = db.build_queue_limit(builder_def) as usize;
        let queued = self.queues.get(builder.id).map_or(0, Vec::len);
        let Some(id) = self.last_job_id.checked_add(1) else { return false; };
        if queued >= limit {
            return false;
        }
        let owner = builder.owner;
        let paid = db.unit_features(builder_def).pay_on_queue;
        if paid && !(self.officers_admit(units, db, eco, owner, what) && pay(eco, db, owner, what)) {
            return false;
        }
        self.last_job_id = id;
        let item = Item { id: BuildQueueId(id), def: what, owner, done: 0, started: false, paid };
        match self.queues.get_mut(builder.id) {
            Some(q) => q.push(item),
            None => { self.queues.insert(builder.id, vec![item]); }
        }
        true
    }

    pub(crate) fn has_digest_state(&self) -> bool {
        self.last_job_id != 0 || !self.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.queues.is_empty() && self.deploys.is_empty() && self.welds.is_empty()
            && self.rallies.is_empty() && self.research.is_empty()
            && self.officer_upgrades.is_empty()
    }

    pub(crate) fn order_rally(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        player: PlayerId,
        builders: &[EntityId],
        at: crate::command::Target,
    ) -> bool {
        self.rallies.order(db, units, player, builders, at)
    }

    pub(crate) fn remove_rally_entity(&mut self, id: EntityId) {
        self.rallies.remove_entity(id);
    }

    pub(crate) fn clear_builder_rally(&mut self, id: EntityId) {
        self.rallies.remove_builder(id);
    }

    /// Items queued at `yard`, in order, as `(def, ticks done)`.
    pub fn queue(&self, yard: EntityId) -> Vec<(ContentKey, u64)> {
        self.queues
            .get(yard)
            .map(|q| q.iter().map(|i| (i.def, i.done)).collect())
            .unwrap_or_default()
    }

    /// Whether `rig` has a deploy pending.
    pub fn is_deploying(&self, rig: EntityId) -> bool {
        self.deploys.get(rig).is_some()
    }

    /// Derived from the existing persisted weld records: no separate lock state.
    pub fn is_welding(&self, rig: EntityId) -> bool {
        self.welds.iter().any(|(_, weld)| weld.builder == rig)
    }

    pub(crate) fn welding_builders(&self) -> Vec<EntityId> {
        let mut ids: Vec<_> = self.welds.iter().map(|(_, weld)| weld.builder).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub(crate) fn stationary_evolvers(&self, db: &ContentDb, units: &Entities) -> impl Iterator<Item = EntityId> {
        self.queues.iter().filter_map(|(id, queue)| {
            let head = queue.first().filter(|job| job.started)?;
            let builder = db.unit(units.get(id)?.def)?;
            (builder.is_evolver() && db.unit(head.def).is_some_and(|d| d.is_starbase)).then_some(id)
        })
    }

    /// Builders flying to a build site or building there, ascending
    /// (SIM-36: stock construction ships hold fire meanwhile).
    pub(crate) fn builders_at_work(&self) -> Vec<EntityId> {
        let mut ids = self.welding_builders();
        ids.extend(self.deploys.iter().map(|(id, _)| id));
        ids.sort();
        ids.dedup();
        ids
    }

    /// Explicit Stop preserves the existing non-refundable active-weld rule.
    /// The caller removes these unfinished actors through ordinary despawn.
    pub(crate) fn cancel_welds(&mut self, ids: &[EntityId]) -> Vec<EntityId> {
        let sites: Vec<_> = self.welds.iter()
            .filter(|(site, weld)| ids.contains(site) || ids.contains(&weld.builder))
            .map(|(site, _)| site).collect();
        for &site in &sites { self.welds.remove(site); }
        sites
    }

    pub(crate) fn remove_weld_site(&mut self, site: EntityId) {
        self.welds.remove(site);
    }

    /// `Order::Build`; false when rejected.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn order_build(
        &mut self,
        db: &ContentDb,
        units: &mut Entities,
        eco: &mut Economy,
        player: PlayerId,
        builder: EntityId,
        what: ContentKey,
        at: Option<Vec3>,
        bounds: Option<&(Vec3, Vec3)>,
        regions: &[sta_content::Region],
    ) -> bool {
        let Some(b) = units.get(builder) else {
            return false;
        };
        if (player.is_player() && b.owner != player) || !finished(b) || self.is_welding(builder) {
            return false;
        }
        let (kind, owner, bdef) = (b.kind, b.owner, b.def);
        let (Some(bd), Some(product)) = (db.unit(bdef), db.unit(what)) else {
            return false;
        };
        if bd.research_build.is_empty() && !bd.build.list.is_empty() && !bd.build.list.iter().any(|r| r.key == what) {
            return false;
        }
        if !self.admits_officer_upgrade(b, bd, product)
            || self.research_block(db, units, builder, bd, product, true).is_some()
        {
            return false;
        }
        let evolves = bd.is_evolver();
        if evolves && (bd.build.list.is_empty() || self.queues.get(builder).is_some() || self.is_deploying(builder)) {
            return false;
        }
        let kind = if EntityKind::producer_def(bd) || (evolves && !EntityKind::build_requires_site(bd, product)) {
            EntityKind::Station
        } else {
            kind
        };
        let pay_now = db.unit_features(bd).pay_on_queue;
        match kind {
            EntityKind::Station => self.queue_build(db, units, eco, b, bd, what),
            EntityKind::Craft => {
                let Some(at) = at else { return false };
                // The same rule the placement ghost shows (see [`site`], [`orbit`]).
                // An orbital station replaces the site rule with the orbit's:
                // the sim snaps the cursor point to the planet's slot.
                let mut outward = None;
                let mut parent_planet = None;
                let at = if db.unit(what).is_some_and(orbit::is_orbital) {
                    let (planets, mut orbiters) = orbit::from_entities(db, units);
                    orbiters.extend(
                        self.deploys
                            .iter()
                            .filter(|(id, d)| {
                                *id != builder && db.unit(d.def).is_some_and(orbit::is_orbital)
                            })
                            .map(|(_, d)| d.at),
                    );
                    orbiters.extend(self.stationary_evolvers(db, units).filter_map(|id| {
                        let job = self.queues.get(id)?.first()?;
                        orbit::is_orbital(db.unit(job.def)?).then_some(units.get(id)?.pos)
                    }));
                    match orbit::snap(&planets, &orbiters, at) {
                        Ok(slot) => {
                            outward = Some(slot.heading);
                            parent_planet = Some(slot.planet);
                            slot.at
                        }
                        Err(_) => return false,
                    }
                } else {
                    at
                };
                let obstacles = units
                    .iter()
                    .filter(|u| {
                        u.id != builder
                            && Some(u.id) != parent_planet
                            && u.kind != EntityKind::Projectile
                    })
                    .filter(|u| db.unit(u.def).is_some())
                    .map(|u| site::unit_bounds(db, u.def, u.pos));
                let reserved = self
                    .deploys
                    .iter()
                    .filter(|(id, _)| *id != builder)
                    .map(|(_, d)| site::unit_bounds(db, d.def, d.at));
                let cocoons = self.stationary_evolvers(db, units).filter_map(|id| {
                    let job = self.queues.get(id)?.first()?;
                    Some(site::unit_bounds(db, job.def, units.get(id)?.pos))
                });
                // An orbital station must overlap its attachment planet. It
                // still cannot encroach on another object, region or site.
                let limits = if parent_planet.is_some() {
                    None
                } else {
                    bounds
                };
                if site::check_bounds(
                    limits,
                    site::unit_bounds(db, what, at),
                    obstacles
                        .chain(reserved)
                        .chain(cocoons)
                        .chain(regions.iter().filter_map(site::region_bounds)),
                )
                .is_err()
                {
                    return false;
                }
                // Invalid geometry leaves an existing reservation untouched.
                self.drop_deploy(db, eco, builder);
                if pay_now
                    && !(self.officers_admit(units, db, eco, owner, what)
                        && pay(eco, db, owner, what))
                {
                    return false;
                }
                self.deploys.insert(
                    builder,
                    Deploy {
                        def: what,
                        owner,
                        at,
                        paid: pay_now,
                    },
                );
                // The rig parks beside the site, not on it: the structure
                // goes up at `at` and the rig stops at its edge.
                let ring = if evolves { Fixed::ZERO } else { stand_ring(db, units, what, builder) };
                if let Some(r) = units.get_mut(builder) {
                    let (mut dx, mut dz) = (r.pos.x - at.x, r.pos.z - at.z);
                    let mut d = (dx * dx + dz * dz).sqrt();
                    // An orbital station's rig waits outside the orbit, on the planet's far side
                    // from the centre, never inside the planet.
                    if let Some(h) = outward {
                        (dx, dz, d) = (h.0, h.1, Fixed::ONE);
                    }
                    if d == Fixed::ZERO {
                        (dx, dz, d) = (-r.heading.0, -r.heading.1, Fixed::ONE);
                    }
                    r.target = Some(Vec3 { x: at.x + dx / d * ring, y: at.y, z: at.z + dz / d * ring });
                }
                true
            }
            _ => false,
        }
    }

    fn drop_deploy(&mut self, db: &ContentDb, eco: &mut Economy, rig: EntityId) {
        if let Some(d) = self.deploys.remove(rig)
            && d.paid
        {
            refund(eco, db, d.owner, d.def);
        }
    }

    fn drop_queue(&mut self, db: &ContentDb, eco: &mut Economy, yard: EntityId) {
        for i in self.queues.remove(yard).unwrap_or_default() {
            if i.paid {
                refund(eco, db, i.owner, i.def);
            }
        }
    }

    /// Remove just the selected job. Armada II's producer removes either
    /// the active build or a waiting list entry (static executable evidence).
    /// Its paid active build credits the class costs and discharges officers.
    /// For preload and other profiles, refunding exactly the paid jobs is
    /// STA64's interim rule and matches its existing queue-wide cancellation.
    /// The normal production step admits and starts the next job in this tick.
    pub(crate) fn cancel_build(
        &mut self,
        db: &ContentDb,
        eco: &mut Economy,
        builder: &Entity,
        slot: u32,
        job: BuildQueueId,
    ) -> bool {
        let Some(queue) = self.queues.get_mut(builder.id) else { return false; };
        let slot = slot as usize;
        if !queue.get(slot).is_some_and(|item| item.id == job && item.owner == builder.owner) {
            return false;
        }
        let item = queue.remove(slot);
        if queue.is_empty() {
            self.queues.remove(builder.id);
        }
        if item.paid {
            refund(eco, db, item.owner, item.def);
        }
        true
    }

    /// `Stop` or a non-queued `Move`: drop the queues and deploys of the
    /// player's `ids`, refunding what was paid. A pending rig deploy leaves
    /// the rig's movement to the caller.
    pub(crate) fn cancel(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        eco: &mut Economy,
        player: PlayerId,
        ids: &[EntityId],
    ) {
        for &id in ids {
            if units
                .get(id)
                .is_some_and(|u| !player.is_player() || u.owner == player)
            {
                self.drop_queue(db, eco, id);
                self.drop_deploy(db, eco, id);
            }
        }
    }

    /// One tick: finish welds and queue heads, then start queue heads and
    /// deploy rigs that have arrived. Ascending ids throughout.
    pub(crate) fn step(
        &mut self,
        units: &mut Entities,
        db: &ContentDb,
        eco: &mut Economy,
        events: &mut Vec<SimEvent>,
        confinement: &mut crate::boundary::state::Confinement,
        bounds: Option<(sta_core::Vec3, sta_core::Vec3)>,
    ) -> Completions {
        let mut completed = Completions::default();
        if self.is_empty() {
            return completed;
        }
        self.rallies.prune(units);
        let mut newly_finished = Vec::new();
        let welds: Vec<EntityId> = self.welds.iter().map(|(id, _)| id).collect();
        for id in welds {
            if !units.contains(id) {
                self.welds.remove(id);
                continue;
            }
            let w = self.welds.get_mut(id).expect("listed");
            w.done += 1;
            let (done, ticks, builder) = (w.done, w.ticks, w.builder);
            let Some(s) = units.get_mut(id) else { continue };
            if done >= ticks {
                s.progress = None;
                confinement.initialize(s);
                let (owner, mobile) = (s.owner, s.orderable());
                self.welds.remove(id);
                if mobile {
                    let target = self.rallies.destination(builder, owner, units);
                    units.get_mut(id).expect("completed weld").target = target;
                }
                events.push(SimEvent::Built { builder, id });
                free_unit::spawn(units, db, eco, id, confinement, events);
                newly_finished.push(id);
            } else {
                s.progress = Some(Fixed::from_int(done as i32) / Fixed::from_int(ticks as i32));
            }
        }

        let yards: Vec<EntityId> = self.queues.iter().map(|(id, _)| id).collect();
        for yard in yards {
            let Some(y) = units.get(yard) else {
                self.drop_queue(db, eco, yard);
                continue;
            };
            let (pos, heading, yard_def) = (y.pos, y.heading, y.def);
            let builder = db.unit(yard_def);
            let evolves = builder.is_some_and(sta_content::UnitDef::is_evolver);
            let portal = builder.and_then(|d| db.launch_wormhole(d));
            let q = self.queues.get_mut(yard).expect("listed");
            if let Some(head) = q.first_mut()
                && head.started
            {
                head.done = head.done.saturating_add(1);
                let delay = portal.map_or(0, sta_content::UnitDef::wormhole_open_ticks);
                if head.done >= ticks_of(db, head.def).saturating_add(delay) {
                    let Item { def, owner, .. } = q.remove(0);
                    let product = db.unit(def);
                    if let Some(upgrade) = product.filter(|d| d.a1_officer_upgrade_gain.is_some()) {
                        if !builder.is_some_and(|base| self.finish_officer_upgrade(y, base, upgrade)) {
                            refund(eco, db, owner, def);
                        }
                    } else if let Some(d) = product {
                        let mut attachment = None;
                        // External research pods finish directly in an attachment slot.
                        let out = match builder {
                            Some(b) if research::external_pod(b, d) => {
                                let Some(pod) = self.completed_slot(db, units, yard, b, d) else {
                                    refund(eco, db, owner, def);
                                    if self.queues.get(yard).is_some_and(Vec::is_empty) {
                                        self.queues.remove(yard);
                                    }
                                    continue;
                                };
                                let (pos, heading, _) = research::pose(y, b, pod.slot).expect("usable slot");
                                if let Some(old) = pod.replacing {
                                    self.research.remove(old);
                                    completed.replaced_pods.push(old);
                                }
                                attachment = Some(research::Attachment { station: yard, slot: pod.slot });
                                launch::Launch { pos, heading, exit: None }
                            }
                            Some(b) if portal.is_some() => launch::through_wormhole(b, pos, heading),
                            Some(b) if !evolves => launch::place(b, pos, heading),
                            _ => launch::Launch { pos, heading, exit: None },
                        };
                        let mut unit = Entity::from_def(d, db.features(), owner, out.pos, out.heading);
                        let mut follow = None;
                        if attachment.is_some() {
                            unit.stats.max_speed = Fixed::ZERO;
                        }
                        if unit.orderable() && !evolves {
                            let rally = self.rallies.destination(yard, owner, units).or_else(|| {
                                let a2 = matches!(db.profile(), sta_content::Profile::Sta2Classic | sta_content::Profile::Fo3 | sta_content::Profile::Fo4);
                                builder.filter(|_| a2)
                                    .and_then(|b| launch::default_rally(b, pos, out.heading, bounds))
                            });
                            if let Some(exit) = out.exit.filter(|_| portal.is_some()) {
                                unit.target = Some(exit);
                                follow = rally.filter(|&rally| rally != exit);
                            } else {
                                unit.target = rally.or(out.exit);
                            }
                        }
                        let id = units.spawn(unit);
                        if let Some(link) = attachment {
                            self.research.insert(id, link);
                        }
                        if let Some(rally) = follow {
                            completed.gate_rallies.push((id, rally));
                        }
                        confinement.initialize(units.get_mut(id).expect("spawned"));
                        events.push(SimEvent::Built { builder: yard, id });
                        events.push(SimEvent::Spawned { id });
                        newly_finished.push(id);
                        if evolves {
                            completed.evolved.push((yard, id));
                        }
                    }
                }
            }
            // SIM-26: a waiting head starts only once its officers fit.
            let admitted = match self.queues.get(yard).and_then(|q| q.first()) {
                Some(h) if !h.started && !h.paid => self.officers_admit(units, db, eco, h.owner, h.def),
                _ => true,
            };
            let q = self.queues.get_mut(yard).expect("listed");
            if let Some(head) = q.first_mut()
                && !head.started
                && admitted
                && (head.paid || pay(eco, db, head.owner, head.def))
            {
                head.paid = true;
                head.started = true;
                head.done = 0;
                events.push(SimEvent::BuildStarted {
                    builder: yard,
                    what: head.def,
                });
            }
            if q.is_empty() {
                self.queues.remove(yard);
            }
        }

        // A welded or yard-built producer first receives its push-build items
        // when construction completes. The spawn path handles initial actors.
        for id in newly_finished {
            for what in push_build_defs(db, units, id) {
                self.push_build_item(db, units, eco, id, what);
            }
        }

        let rigs: Vec<EntityId> = self.deploys.iter().map(|(id, _)| id).collect();
        for rig in rigs {
            let d = *self.deploys.get(rig).expect("listed");
            let Some(r) = units.get(rig) else {
                self.drop_deploy(db, eco, rig);
                continue;
            };
            let evolves = db.unit(r.def).is_some_and(sta_content::UnitDef::is_evolver);
            let (dx, dz) = (d.at.x - r.pos.x, d.at.z - r.pos.z);
            // The smooth model (SIM-15) stops a craft anywhere within
            // `tooCloseToTurn` of its point, so a rig there must be in reach.
            let reach = match crate::kinematics::Model::for_content(db) {
                crate::kinematics::Model::Smooth | crate::kinematics::Model::A1 => deploy_reach(&r.stats).max(r.stats.too_close_to_turn + crate::arrive_distance()),
                crate::kinematics::Model::Native => deploy_reach(&r.stats),
            };
            // The rig stops beside the structure's footprint (SIM-21).
            let reach = if evolves {
                r.stats.too_close_to_turn + crate::arrive_distance()
            } else {
                reach + stand_ring(db, units, d.def, rig)
            };
            if (dx * dx + dz * dz).sqrt() > reach {
                continue;
            }
            let next_job = self.last_job_id.checked_add(1).map(BuildQueueId);
            if evolves && next_job.is_none() {
                self.drop_deploy(db, eco, rig);
                continue;
            }
            self.deploys.remove(rig);
            if !d.paid && !(self.officers_admit(units, db, eco, d.owner, d.def) && pay(eco, db, d.owner, d.def)) {
                continue;
            }
            let Some(def) = db.unit(d.def) else { continue };
            if let Some(r) = units.get_mut(rig) {
                r.target = None;
                if evolves {
                    r.pos = d.at;
                    r.speed = Fixed::ZERO;
                }
            }
            if let Some(id) = next_job.filter(|_| evolves) {
                // Evolver::StartBuild turns the embryo at the site into a
                // cocoon. The product is constructed only at FinishBuild.
                self.last_job_id = id.0;
                self.queues.insert(rig, vec![Item { id, def: d.def, owner: d.owner, done: 0, started: true, paid: true }]);
                events.push(SimEvent::BuildStarted { builder: rig, what: d.def });
                continue;
            }
            // An orbital station faces along its planet's outward radial.
            let heading = if orbit::is_orbital(def) {
                let (planets, _) = orbit::from_entities(db, units);
                orbit::owner(&planets, d.at).map_or((Fixed::ZERO, Fixed::ONE), |p| orbit::slot(&p, d.at).heading)
            } else {
                (Fixed::ZERO, Fixed::ONE)
            };
            let mut s = Entity::from_def(def, db.features(), d.owner, d.at, heading);
            s.progress = Some(Fixed::ZERO);
            let id = units.spawn(s);
            confinement.initialize(units.get_mut(id).expect("spawned"));
            self.welds.insert(
                id,
                Weld {
                    builder: rig,
                    done: 0,
                    ticks: ticks_of(db, d.def),
                },
            );
            events.push(SimEvent::BuildStarted {
                builder: rig,
                what: d.def,
            });
            events.push(SimEvent::Spawned { id });
        }
        completed
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.queues.len() as u64);
        for (id, q) in self.queues.iter() {
            h.u64(id.0 as u64);
            h.u64(q.len() as u64);
            for i in q {
                h.u64(i.def.0 as u64);
                h.u64(i.owner.0 as u64);
                h.u64(i.done);
                h.u64(i.started as u64 | (i.paid as u64) << 1);
                h.u64(i.id.0);
            }
        }
        h.u64(self.deploys.len() as u64);
        for (id, d) in self.deploys.iter() {
            h.u64(id.0 as u64);
            h.u64(d.def.0 as u64);
            h.u64(d.owner.0 as u64);
            for f in [d.at.x, d.at.y, d.at.z] {
                h.fixed(f);
            }
            h.u64(d.paid as u64);
        }
        h.u64(self.welds.len() as u64);
        for (id, w) in self.welds.iter() {
            h.u64(id.0 as u64);
            h.u64(w.builder.0 as u64);
            h.u64(w.done);
            h.u64(w.ticks);
        }
        self.rallies.hash_into(h);
        if !self.research.is_empty() {
            h.u64(self.research.iter().count() as u64);
            for (id, link) in self.research.iter() {
                h.u64(u64::from(id.0));
                h.u64(u64::from(link.station.0));
                h.u64(link.slot as u64);
            }
        }
        self.hash_officer_upgrades(h);
        if self.last_job_id != 0 {
            h.u64(self.last_job_id);
        }
    }

    #[allow(dead_code)] // the snapshot calls it
    pub(crate) fn to_rec(&self) -> ProductionRec {
        ProductionRec {
            rallies: self.rallies.to_rec(),
            last_job_id: self.last_job_id,
            queues: self
                .queues
                .iter()
                .map(|(id, q)| {
                    (
                        id.0,
                        q.iter()
                            .map(|i| (i.def.0, i.owner.0, i.done, i.started, i.paid, i.id.0))
                            .collect(),
                    )
                })
                .collect(),
            deploys: self
                .deploys
                .iter()
                .map(|(id, d)| {
                    (
                        id.0,
                        d.def.0,
                        d.owner.0,
                        [d.at.x.to_bits(), d.at.y.to_bits(), d.at.z.to_bits()],
                        d.paid,
                    )
                })
                .collect(),
            welds: self
                .welds
                .iter()
                .map(|(id, w)| (id.0, w.builder.0, w.done, w.ticks))
                .collect(),
        }
    }

    #[allow(dead_code)] // the snapshot calls it
    pub(crate) fn from_rec(r: &ProductionRec) -> Option<Production> {
        let mut p = Production {
            rallies: rally::Rallies::from_rec(&r.rallies)?,
            last_job_id: r.last_job_id,
            ..Production::default()
        };
        let mut job_ids = std::collections::BTreeSet::new();
        for (id, q) in &r.queues {
            let mut items = Vec::with_capacity(q.len());
            for &(def, owner, done, started, paid, job) in q {
                if job == 0 || job > r.last_job_id || !job_ids.insert(job) {
                    return None;
                }
                items.push(Item {
                    id: BuildQueueId(job),
                    def: ContentKey(def),
                    owner: PlayerId(owner),
                    done,
                    started,
                    paid,
                });
            }
            p.queues.insert(EntityId(*id), items);
        }
        for &(id, def, owner, at, paid) in &r.deploys {
            let at = Vec3 {
                x: Fixed::from_bits(at[0]),
                y: Fixed::from_bits(at[1]),
                z: Fixed::from_bits(at[2]),
            };
            p.deploys.insert(
                EntityId(id),
                Deploy {
                    def: ContentKey(def),
                    owner: PlayerId(owner),
                    at,
                    paid,
                },
            );
        }
        for &(id, builder, done, ticks) in &r.welds {
            p.welds.insert(
                EntityId(id),
                Weld {
                    builder: EntityId(builder),
                    done,
                    ticks,
                },
            );
        }
        Some(p)
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod charge_tests;

#[cfg(test)]
mod queue_limit_tests;

#[cfg(test)]
mod push_build_tests;

#[cfg(test)]
mod start_charge_tests;

#[cfg(test)]
mod officer_tests;

#[cfg(test)]
mod orbit_tests;
