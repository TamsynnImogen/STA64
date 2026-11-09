//! Construction (SIM-08): yards with build queues, and construction rigs that
//! deploy, weld a station and reveal it.
//!
//! - `Order::Build { builder, what, at }`:
//!   - A finished **station** (yard, starbase; [`EntityKind::Station`]) queues
//!     `what` (up to [`QUEUE_MAX`] items; `at` is ignored). Each yard has its
//!     own queue and they run in parallel. The head item takes the def's
//!     `build.ticks`; a ship appears at the yard's position on the tick it
//!     finishes ([`SimEvent::Built`] then `Spawned`), and the next item starts
//!     in that same tick.
//!   - A **craft** (construction rig) needs `at: Some(site)`. It steers there;
//!     within [`deploy_reach`] of the site (the rig's slow-turn orbit plus a margin, from its ODF movement stats) it deploys: the
//!     station entity appears at the site (`progress` 0,
//!     [`SimEvent::BuildStarted`]) and welds for `build.ticks`, `progress`
//!     rising each tick; on the last one `progress` becomes `None` (finished,
//!     revealed) and [`SimEvent::Built`] is emitted. The rig is free once
//!     deployed; the weld goes on without it.
//!   - Timing: an item queued (or a rig deployed) in the step of tick `t`
//!     starts in that step and completes in the step of tick `t + ticks`.
//! - Payment: with `Features::pay_on_queue` the cost leaves the player's stock
//!   when the order is accepted (rejected if it cannot be paid); otherwise a
//!   queue item waits at the head until the stock covers it and pays as it
//!   starts, and a rig pays on deploy (dropping the deploy if it cannot).
//! - Cancel: a `Stop` on a yard drops its whole queue, a non-queued `Move` or a
//!   `Stop` on a rig drops its pending deploy; whatever was paid is refunded in
//!   full (items that have started included). A yard or rig destroyed before
//!   its item or deploy completes refunds the same way. A weld in progress is
//!   not refundable.
//! - Accepted: any def the content db has, if the builder's `build.list` is
//!   empty or names it. Tech prerequisites
//!   are checked before this, in `Sim::apply` ([`crate::tech`], SIM-09).
//! - Not yet: officers (no officer stock exists on the player), cooperative
//!   multi-yard builds on one item, rally points,
//!   `Features::construction_rigs` gating.
//! - Persistent state is [`Production`]; the snapshot (`to_rec`/`from_rec`)
//!   must include it. It is in the digest once any exists.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3};

use crate::command::Edit;
use crate::economy::Economy;
use crate::event::SimEvent;
use crate::world::{Entities, Entity, EntityKind, Fnv, Table};

/// Longest build queue of one yard.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Item {
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
    /// By yard.
    queues: Table<Vec<Item>>,
    /// By rig.
    deploys: Table<Deploy>,
    /// By the station being welded.
    welds: Table<Weld>,
}

/// `(def, owner, done, started, paid)`.
pub type ItemRec = (u32, u8, u64, bool, bool);

/// Plain form of [`Production`] for snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionRec {
    /// `(yard, [(def, owner, done, started, paid)])`.
    pub queues: Vec<(u32, Vec<ItemRec>)>,
    /// `(rig, def, owner, [x, y, z] bits, paid)`.
    pub deploys: Vec<(u32, u32, u8, [i64; 3], bool)>,
    /// `(station, builder, done, ticks)`.
    pub welds: Vec<(u32, u32, u64, u64)>,
}

fn finished(e: &Entity) -> bool {
    e.progress.is_none_or(|p| p >= Fixed::ONE)
}

fn ticks_of(db: &ContentDb, def: ContentKey) -> u64 {
    db.unit(def).map_or(1, |d| d.build.ticks.max(1))
}

/// Take the whole cost of `def` from `p`, or nothing.
fn pay(eco: &mut Economy, db: &ContentDb, p: PlayerId, def: ContentKey) -> bool {
    let Some(d) = db.unit(def) else { return false };
    let have = eco.stock_of(p);
    let short = d
        .build
        .cost
        .iter()
        .enumerate()
        .any(|(i, &c)| c > Fixed::ZERO && have.get(i).is_none_or(|&h| h < c));
    if short {
        return false;
    }
    for (i, &c) in d.build.cost.iter().enumerate() {
        if c > Fixed::ZERO {
            let ok = eco.spend(p, i as u8, c);
            debug_assert!(ok);
        }
    }
    true
}

fn refund(eco: &mut Economy, db: &ContentDb, p: PlayerId, def: ContentKey) {
    let Some(d) = db.unit(def) else { return };
    for (i, &c) in d.build.cost.iter().enumerate() {
        if c > Fixed::ZERO {
            let now = eco.stock_of(p).get(i).copied().unwrap_or(Fixed::ZERO);
            eco.apply_edit(PlayerId::BENCH, &Edit::Cargo(p, i as u8, now + c));
        }
    }
}

impl Production {
    pub fn is_empty(&self) -> bool {
        self.queues.is_empty() && self.deploys.is_empty() && self.welds.is_empty()
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
    ) -> bool {
        let Some(b) = units.get(builder) else {
            return false;
        };
        if (player.is_player() && b.owner != player) || !finished(b) {
            return false;
        }
        let (kind, owner, bdef) = (b.kind, b.owner, b.def);
        let (Some(bd), Some(_)) = (db.unit(bdef), db.unit(what)) else {
            return false;
        };
        if !bd.build.list.is_empty() && !bd.build.list.iter().any(|r| r.key == what) {
            return false;
        }
        let pay_now = db.features().pay_on_queue;
        match kind {
            EntityKind::Station => {
                if self
                    .queues
                    .get(builder)
                    .is_some_and(|q| q.len() >= QUEUE_MAX)
                {
                    return false;
                }
                if pay_now && !pay(eco, db, owner, what) {
                    return false;
                }
                let item = Item {
                    def: what,
                    owner,
                    done: 0,
                    started: false,
                    paid: pay_now,
                };
                match self.queues.get_mut(builder) {
                    Some(q) => q.push(item),
                    None => {
                        self.queues.insert(builder, vec![item]);
                    }
                }
                true
            }
            EntityKind::Craft => {
                let Some(at) = at else { return false };
                // A new order replaces a pending deploy.
                self.drop_deploy(db, eco, builder);
                if pay_now && !pay(eco, db, owner, what) {
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
                if let Some(r) = units.get_mut(builder) {
                    r.target = Some(at);
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
    ) {
        if self.is_empty() {
            return;
        }
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
                self.welds.remove(id);
                events.push(SimEvent::Built { builder, id });
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
            let (pos, heading) = (y.pos, y.heading);
            let q = self.queues.get_mut(yard).expect("listed");
            if let Some(head) = q.first_mut()
                && head.started
            {
                head.done += 1;
                if head.done >= ticks_of(db, head.def) {
                    let Item { def, owner, .. } = q.remove(0);
                    if let Some(d) = db.unit(def) {
                        let id =
                            units.spawn(Entity::from_def(d, db.features(), owner, pos, heading));
                        events.push(SimEvent::Built { builder: yard, id });
                        events.push(SimEvent::Spawned { id });
                    }
                }
            }
            let q = self.queues.get_mut(yard).expect("listed");
            if let Some(head) = q.first_mut()
                && !head.started
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

        let rigs: Vec<EntityId> = self.deploys.iter().map(|(id, _)| id).collect();
        for rig in rigs {
            let d = *self.deploys.get(rig).expect("listed");
            let Some(r) = units.get(rig) else {
                self.drop_deploy(db, eco, rig);
                continue;
            };
            let (dx, dz) = (d.at.x - r.pos.x, d.at.z - r.pos.z);
            if (dx * dx + dz * dz).sqrt() > deploy_reach(&r.stats) {
                continue;
            }
            self.deploys.remove(rig);
            if !d.paid && !pay(eco, db, d.owner, d.def) {
                continue;
            }
            let Some(def) = db.unit(d.def) else { continue };
            if let Some(r) = units.get_mut(rig) {
                r.target = None;
            }
            let mut s =
                Entity::from_def(def, db.features(), d.owner, d.at, (Fixed::ZERO, Fixed::ONE));
            s.progress = Some(Fixed::ZERO);
            let id = units.spawn(s);
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
    }

    #[allow(dead_code)] // the snapshot calls it
    pub(crate) fn to_rec(&self) -> ProductionRec {
        ProductionRec {
            queues: self
                .queues
                .iter()
                .map(|(id, q)| {
                    (
                        id.0,
                        q.iter()
                            .map(|i| (i.def.0, i.owner.0, i.done, i.started, i.paid))
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
    pub(crate) fn from_rec(r: &ProductionRec) -> Production {
        let mut p = Production::default();
        for (id, q) in &r.queues {
            let items = q
                .iter()
                .map(|&(def, owner, done, started, paid)| Item {
                    def: ContentKey(def),
                    owner: PlayerId(owner),
                    done,
                    started,
                    paid,
                })
                .collect();
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
        p
    }
}

#[cfg(test)]
pub(crate) mod tests;
