//! SIM-17: persistent destinations for a builder's newly produced craft.
//!
//! Setting/clearing a rally does not move the builder or touch its queue.
//! Entity destinations are sampled at each production completion, not followed
//! continuously. Missing targets and destroyed/captured builders lose the rally.
//! This is the deterministic bridge policy, not measured A1/A2/FO parity.
//! Launch hardpoints/clearance remain SIM-16's responsibility.

use sta_content::ContentDb;
use sta_core::{EntityId, PlayerId, Vec3};

use crate::command::Target;
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, EntityKind, Fnv, Table};

/// Canonical builder-id order; stored with production in snapshots.
pub type RallyRec = Vec<(EntityId, PlayerId, Target)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rally {
    owner: PlayerId,
    at: Target,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Rallies {
    targets: Table<Rally>,
}

fn live(u: &Entity) -> bool {
    !u.flags.contains(UnitFlags::DYING)
}

fn builder(u: &Entity, db: &ContentDb) -> bool {
    live(u)
        && u.owner.is_player()
        && matches!(u.kind, EntityKind::Station | EntityKind::Craft)
        && u.progress.is_none_or(|p| p >= sta_core::Fixed::ONE)
        && db.unit(u.def).is_some_and(|d| !d.build.list.is_empty())
}

fn valid_target(at: Target, units: &Entities) -> bool {
    match at {
        Target::None | Target::Point(_) => true,
        Target::Entity(id) => units.get(id).is_some_and(live),
    }
}

impl Rallies {
    pub(super) fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub(super) fn order(
        &mut self,
        db: &ContentDb,
        units: &Entities,
        from: PlayerId,
        ids: &[EntityId],
        at: Target,
    ) -> bool {
        if !(from.is_player() || from == PlayerId::SCRIPT || from == PlayerId::BENCH) || !valid_target(at, units) {
            return false;
        }
        let mut ids: Vec<_> = ids
            .iter()
            .copied()
            .filter(|&id| {
                units
                    .get(id)
                    .is_some_and(|u| builder(u, db) && (!from.is_player() || from == u.owner))
            })
            .collect();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return false;
        }
        for id in ids {
            if at == Target::None {
                self.targets.remove(id);
            } else {
                self.targets.insert(
                    id,
                    Rally {
                        owner: units.get(id).expect("validated builder").owner,
                        at,
                    },
                );
            }
        }
        true
    }

    pub(super) fn prune(&mut self, units: &Entities) {
        let stale: Vec<_> = self
            .targets
            .iter()
            .filter_map(|(id, r)| {
                let owned = units.get(id).is_some_and(|u| live(u) && u.owner == r.owner);
                (!owned || !valid_target(r.at, units)).then_some(id)
            })
            .collect();
        for id in stale {
            self.targets.remove(id);
        }
    }

    pub(super) fn destination(&self, id: EntityId, owner: PlayerId, units: &Entities) -> Option<Vec3> {
        let r = self.targets.get(id)?;
        if r.owner != owner || units.get(id)?.owner != owner {
            return None;
        }
        match r.at {
            Target::Point(p) => Some(p),
            Target::Entity(id) => units.get(id).filter(|u| live(u)).map(|u| u.pos),
            Target::None => None,
        }
    }

    pub(super) fn remove_entity(&mut self, id: EntityId) {
        self.targets.remove(id);
        let refs: Vec<_> = self
            .targets
            .iter()
            .filter_map(|(builder, r)| (r.at == Target::Entity(id)).then_some(builder))
            .collect();
        for builder in refs {
            self.targets.remove(builder);
        }
    }

    pub(super) fn hash_into(&self, h: &mut Fnv) {
        // Preserve every existing no-rally digest, including active production.
        if self.is_empty() {
            return;
        }
        h.str("rally");
        h.u64(self.targets.len() as u64);
        for (id, r) in self.targets.iter() {
            h.u64(id.0 as u64);
            h.u64(r.owner.0 as u64);
            match r.at {
                Target::None => unreachable!("clear removes its row"),
                Target::Entity(id) => {
                    h.u64(1);
                    h.u64(id.0 as u64);
                }
                Target::Point(p) => {
                    h.u64(2);
                    for v in [p.x, p.y, p.z] {
                        h.fixed(v);
                    }
                }
            }
        }
    }

    pub(super) fn to_rec(&self) -> RallyRec {
        self.targets.iter().map(|(id, r)| (id, r.owner, r.at)).collect()
    }

    pub(super) fn from_rec(rec: &RallyRec) -> Option<Self> {
        if rec.iter().any(|(id, owner, at)| {
            id.0 == 0 || !owner.is_player() || matches!(at, Target::None | Target::Entity(EntityId(0)))
        }) {
            return None;
        }
        Some(Self {
            targets: Table::from_rows(rec.iter().map(|&(id, owner, at)| (id, Rally { owner, at })).collect())?,
        })
    }
}

#[cfg(test)]
mod tests;
