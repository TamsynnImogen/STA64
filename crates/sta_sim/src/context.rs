//! Context orders: what a player's right-click on a unit means for the
//! selection (UI-14). The client's input (`sta64::input`) and the bench's
//! `rclick` both ask [`context_order`], so the rule lives once, here.
//!
//! The rule is the originals' group vote (stock Armada II 1.1 and Fleet
//! Operations: Roots, which runs the same code; read from the executable and
//! measured on the bench with a player's right-click on mixed selections):
//!
//! - Every selected unit votes the order it would take on the target: a
//!   miner on a resource it can mine votes Mine, an armed unit on an enemy
//!   votes Attack (mining and tractor beams do not count), anything else
//!   votes nothing.
//! - The vote with the highest priority wins (Attack over Mine), and that one
//!   order goes to the whole selection. Units that cannot carry it out keep
//!   what they were doing: the sim's `Order::Mine` only takes miners and
//!   `Order::Attack` only armed units. Nobody is turned into a Move.
//! - When nobody votes, the right-click is a plain move to the point
//!   (`None` here): warships on a moon, unarmed units on an enemy.
//!
//! A right-click on a wormhole gate (class `wormhole_obj`) gives the mobile units a move onto the gate's
//! centre, which the sim takes as a go-through (`wormholes`): the originals give a gate its own action
//! (read from the executable, static), STA64 needs no new order for it.
//!
//! Stationary builders rally instead (STA64's rally order; the originals give
//! a yard's rally point through its own button).
//!
//! Resources are known by what the content says they are (the sim's own
//! [`EntityKind`] rule on class), whoever owns them; miners by the same test
//! the sim uses (class `freighter`, or `miner 1`). Not modelled: which
//! resource kinds a miner class can mine (every miner votes Mine on every
//! resource), a freighter's vote on its drop-off station, and the originals'
//! colonise, transport and trade votes.
//!
//! Repair-other (SIM-41): a repair ship votes Repair on a damaged ship (hull short) of its own
//! player or a confirmed ally, giving `Order::Repair` with the ship as target; the sim takes
//! only the repair ships (a ship short only of a system is judged by the sim, not voted on).

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed};

use crate::command::{Order, Target};
use crate::view::{Relation, UnitFlags, WorldView};
use crate::world::EntityKind;
use crate::PlayerId;

/// A resource node: a moon, asteroid or latinum field, by its definition.
fn is_resource(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(EntityKind::mineable_def)
}

fn is_miner(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| d.mines())
}

/// Armed for the vote, as the sim's `Order::Attack` takes it
/// ([`can_attack`](crate::combat::weapons::can_attack)).
fn is_armed(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| crate::combat::weapons::can_attack(db, d))
}

/// The units of `order` that carry it out (the sim drops it for the rest):
/// the miners of a Mine, the armed units of an Attack, everyone otherwise.
/// For the client's destination lines and acknowledgements.
pub fn takers(db: Option<&ContentDb>, view: &WorldView, order: &Order) -> Vec<EntityId> {
    let keep = |ids: &[EntityId], f: &dyn Fn(&ContentDb, EntityId) -> bool| match db {
        Some(db) => ids.iter().copied().filter(|&id| f(db, id)).collect(),
        None => ids.to_vec(),
    };
    match order {
        Order::Mine { units, .. } => keep(units, &|db, id| is_miner(db, view, id)),
        Order::Attack { units, .. } => keep(units, &|db, id| is_armed(db, view, id)),
        Order::Rally { builders, .. } => builders.clone(),
        Order::Repair { units, yard: Some(t) } if db.is_some_and(|db| view.unit(*t).and_then(|u| db.unit(u.def)).is_some_and(|d| EntityKind::of_def(d) == EntityKind::Craft)) => {
            keep(units, &|db, id| is_repair_ship(db, view, id))
        }
        Order::Move { units, .. } => units.clone(),
        _ => Vec::new(),
    }
}

fn is_repair_ship(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| d.class.0.eq_ignore_ascii_case("repairship"))
}

/// The clicking player or a confirmed ally of theirs (a stated Ally relation, else the same team).
fn is_friend(view: &WorldView, me: PlayerId, owner: PlayerId) -> bool {
    if owner == me {
        return true;
    }
    if !owner.is_player() {
        return false;
    }
    if let Some(relation) = view.player(me).and_then(|p| p.relations.iter().find(|(id, _)| *id == owner).map(|(_, r)| *r)) {
        return relation == Relation::Ally;
    }
    matches!((view.player(me), view.player(owner)), (Some(a), Some(b)) if a.team == b.team)
}

fn is_enemy(view: &WorldView, me: PlayerId, owner: PlayerId) -> bool {
    if owner == me || !owner.is_player() {
        return false;
    }
    if let Some(relation) = view.player(me).and_then(|p| p.relations.iter().find(|(id, _)| *id == owner).map(|(_, r)| *r)) {
        return relation == Relation::Enemy;
    }
    match (view.player(me), view.player(owner)) {
        (Some(a), Some(b)) => a.team != b.team,
        _ => true, // Legacy/minimal views may omit player metadata.
    }
}

/// The whole selected group must consist of finished, own, stationary
/// builders: producer stations (by class label, whatever physics file they
/// name: Armada I stations carry a positive `impulseSpeed`, read from data)
/// and builders that cannot move. Mobile construction rigs keep their
/// right-click Move behavior.
pub fn rally_builders(db: Option<&ContentDb>, view: &WorldView, me: PlayerId, own: &[EntityId]) -> bool {
    let Some(db) = db else { return false };
    !own.is_empty() && own.iter().all(|&id| {
        view.unit(id).is_some_and(|u| {
            u.owner == me
                && !u.flags.contains(UnitFlags::DYING)
                && u.progress.is_none_or(|p| p >= Fixed::ONE)
                && db.unit(u.def).is_some_and(|d| {
                    !d.build.list.is_empty()
                        && (EntityKind::producer_def(d)
                            || (d.movement.max_speed == Fixed::ZERO
                                && matches!(EntityKind::of_def(d), EntityKind::Station | EntityKind::Craft)))
                })
        })
    })
}

/// One unit's vote, with the originals' priority (higher wins; their table
/// also ranks colonise 5, repair-other 6 and special attack 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Vote {
    Mine = 4,
    Repair = 6,
    Attack = 7,
}

/// The order a right-click on `target` gives `own`, or `None` for a plain
/// move. `me` is the clicking player.
pub fn context_order(db: Option<&ContentDb>, view: &WorldView, me: PlayerId, own: &[EntityId], target: EntityId) -> Option<Order> {
    let t = view.unit(target)?;
    if t.flags.contains(UnitFlags::DYING) {
        return None;
    }
    if rally_builders(db, view, me, own) {
        return Some(Order::Rally { builders: own.to_vec(), at: Target::Entity(target) });
    }
    if let Some(db) = db
        && db.unit(t.def).is_some_and(|d| d.class.0.eq_ignore_ascii_case("wormhole_obj"))
    {
        let movers: Vec<EntityId> = own
            .iter()
            .copied()
            .filter(|&id| view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| d.movement.max_speed > Fixed::ZERO))
            .collect();
        return (!movers.is_empty()).then_some(Order::Move { units: movers, to: t.pos, queue: false });
    }
    let enemy = is_enemy(view, me, t.owner);
    let Some(db) = db else {
        // No content to vote with: an enemy is attacked, as before content.
        return enemy.then(|| Order::Attack { units: own.to_vec(), target });
    };
    let resource = is_resource(db, view, target);
    let damaged_friend = is_friend(view, me, t.owner)
        && t.hull[0] < t.hull[1]
        && db.unit(t.def).is_some_and(|d| EntityKind::of_def(d) == EntityKind::Craft);
    let vote = own
        .iter()
        .filter_map(|&id| {
            if resource && is_miner(db, view, id) {
                Some(Vote::Mine)
            } else if enemy && id != target && is_armed(db, view, id) {
                Some(Vote::Attack)
            } else if damaged_friend && id != target && is_repair_ship(db, view, id) {
                Some(Vote::Repair)
            } else {
                None
            }
        })
        .max()?;
    Some(match vote {
        Vote::Mine => Order::Mine { units: own.to_vec(), node: target },
        Vote::Attack => Order::Attack { units: own.to_vec(), target },
        Vote::Repair => Order::Repair { units: own.to_vec(), yard: Some(target) },
    })
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
