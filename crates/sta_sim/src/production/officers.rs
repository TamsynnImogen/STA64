//! Officer reservation and admission (SIM-26).
//!
//! Officers are a cap, not a stock: a player's officer limit (`RTS_CFG.h`,
//! `SINGLE_PLAYER_OFFICER_LIMIT` / `MP_OFFICER_LIMIT`, held by
//! [`Economy::officer_limit`]) bounds the sum of the `officerCost` of every
//! live actor plus every build that has been admitted but has not yet become
//! an actor. That second part is the **reservation**.
//!
//! A reservation needs no state of its own. A build is admitted at the moment
//! its other costs are paid (when queued with `Features::pay_on_queue`, when
//! the item reaches the head of the queue and starts, or when a rig deploys,
//! otherwise), so "reserved" is exactly "paid" on a queue item or a pending
//! deploy. The snapshot already carries `paid`, the digest already hashes it,
//! and cancelling, destroying or transferring the builder drops the item and
//! with it the reservation. When the item finishes, the reservation ends in
//! the tick the finished actor appears, so the census never counts it twice.
//!
//! Admission (evidence: measured on the Armada II original, notes kept outside
//! this repository; the cap is taken when a build starts, tested against the
//! vacancies, and returned on cancel):
//!
//! - `cost = officerCost` of the product and `census = live + reserved`.
//! - Admitted when the player has no configured limit, when the cost is zero,
//!   or when `census + cost <= limit`.
//! - With the builder's `pay_on_queue`: an order that is not admitted is
//!   refused at once (nothing charged, nothing queued).
//! - Without it: the order is queued, and the head waits, unstarted and
//!   uncharged, until it is admitted and the stock covers it, like a short
//!   resource. A rig whose structure is not admitted on arrival drops its
//!   deploy, as when it cannot pay.

use sta_content::ContentDb;
use sta_core::{ContentKey, Fixed, PlayerId};

use super::Production;
use crate::economy::Economy;
use crate::view::UnitFlags;
use crate::world::Entities;

fn add(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(a.to_bits().saturating_add(b.to_bits()))
}

fn cost_of(db: &ContentDb, def: ContentKey) -> Fixed {
    db.unit(def).map_or(Fixed::ZERO, |d| d.build.officers.max(Fixed::ZERO))
}

/// Officers held by `player`'s live actors (a dying one has released its).
pub(crate) fn live(units: &Entities, db: &ContentDb, player: PlayerId) -> Fixed {
    units
        .iter()
        .filter(|u| u.owner == player && !u.flags.contains(UnitFlags::DYING))
        .fold(Fixed::ZERO, |sum, u| add(sum, cost_of(db, u.def)))
}

impl Production {
    /// Officers reserved by `player`'s admitted, unfinished builds.
    pub(crate) fn reserved_officers(&self, db: &ContentDb, player: PlayerId) -> Fixed {
        let queued = self.queues.iter().flat_map(|(_, q)| q.iter()).filter(|i| i.paid && i.owner == player);
        let deploys = self.deploys.iter().map(|(_, d)| d).filter(|d| d.paid && d.owner == player);
        let items = queued.map(|i| i.def).chain(deploys.map(|d| d.def));
        items.fold(Fixed::ZERO, |sum, def| add(sum, cost_of(db, def)))
    }

    /// The census the HUD shows and admission tests: live plus reserved.
    pub(crate) fn officer_census(&self, units: &Entities, db: &ContentDb, player: PlayerId) -> Fixed {
        add(live(units, db, player), self.reserved_officers(db, player))
    }

    /// Whether `player` may start building `what` now.
    pub(crate) fn officers_admit(
        &self,
        units: &Entities,
        db: &ContentDb,
        eco: &Economy,
        player: PlayerId,
        what: ContentKey,
    ) -> bool {
        let cost = cost_of(db, what);
        let Some(limit) = eco.officer_limit(player) else { return true };
        cost == Fixed::ZERO || add(self.officer_census(units, db, player), cost) <= limit
    }
}
