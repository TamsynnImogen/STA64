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
//! Stationary builders rally instead (STA64's rally order; the originals give
//! a yard's rally point through its own button).
//!
//! Resources are known by what the content says they are (the sim's own
//! [`EntityKind`] rule on class), whoever owns them; miners by the same test
//! the sim uses (class `freighter`, or `miner 1`). Not modelled: which
//! resource kinds a miner class can mine (every miner votes Mine on every
//! resource), a freighter's vote on its drop-off station, and the originals'
//! colonise, transport, trade and repair-other votes (STA64 has no such
//! orders yet).

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

/// A weapon that attacks: not point defence or a special, and not a tool.
/// Measured on stock A2 and Roots + Classic: a freighter (its mining beams,
/// ordnance class `minebm`) and a construction ship (its tractor beam, a
/// `toggleweapon`) take no part in an attack vote; what the engine's own
/// "can attack" test reads is not known.
fn attacks(w: &sta_content::WeaponDef) -> bool {
    let tool = w.class.0 == "minebm" || w.raw.str("classLabel").is_some_and(|c| c.eq_ignore_ascii_case("toggleweapon"));
    w.kind != sta_content::WeaponKind::PointDefence && !crate::specials::is_special(w) && !tool
}

/// Armed for the vote: a hardpoint with a weapon that [`attacks`].
fn is_armed(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id)
        .and_then(|u| db.unit(u.def))
        .is_some_and(|d| d.hardpoints.iter().any(|hp| db.weapon(hp.weapon.key).is_some_and(attacks)))
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
/// builders. Mobile construction rigs keep their right-click Move behavior.
pub fn rally_builders(db: Option<&ContentDb>, view: &WorldView, me: PlayerId, own: &[EntityId]) -> bool {
    let Some(db) = db else { return false };
    !own.is_empty() && own.iter().all(|&id| {
        view.unit(id).is_some_and(|u| {
            u.owner == me
                && !u.flags.contains(UnitFlags::DYING)
                && u.progress.is_none_or(|p| p >= Fixed::ONE)
                && db.unit(u.def).is_some_and(|d| {
                    !d.build.list.is_empty()
                        && d.movement.max_speed == Fixed::ZERO
                        && (matches!(EntityKind::of_def(d), EntityKind::Station | EntityKind::Craft)
                            || EntityKind::producer_def(d))
                })
        })
    })
}

/// One unit's vote, with the originals' priority (higher wins; their table
/// also ranks colonise 5, repair-other 6 and special attack 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Vote {
    Mine = 4,
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
    let enemy = is_enemy(view, me, t.owner);
    let Some(db) = db else {
        // No content to vote with: an enemy is attacked, as before content.
        return enemy.then(|| Order::Attack { units: own.to_vec(), target });
    };
    let resource = is_resource(db, view, target);
    let vote = own
        .iter()
        .filter_map(|&id| {
            if resource && is_miner(db, view, id) {
                Some(Vote::Mine)
            } else if enemy && id != target && is_armed(db, view, id) {
                Some(Vote::Attack)
            } else {
                None
            }
        })
        .max()?;
    Some(match vote {
        Vote::Mine => Order::Mine { units: own.to_vec(), node: target },
        Vote::Attack => Order::Attack { units: own.to_vec(), target },
    })
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
