//! Point defence (AI-04): the skirmish AI switches a ship's or station's PD
//! weapon on while it is under attack and off again once it has been quiet.
//!
//! Evidence (implements D5, D9): the PD weapon is a toggled special
//! (`toggle = 1`, read from the data of Classic and FO 4.0). The original
//! engine's special-weapon autonomy scores the weapon class with a utility
//! that is zero unless the craft is under attack (read from the executable,
//! static); the rule that consumes the score, and when the engine switches
//! PD off, were not read and are not measured. The AIP data has no PD
//! instruction. So this policy is STA64's own, built on that one fact:
//! under attack (an enemy fired at it, or hit it) -> on if the pool can pay
//! the weapon's cost; no attack for [`QUIET`] ticks -> off.
//!
//! The AI reads the unit's own energy from the view and learns the switch
//! from `SimEvent::PointDefenceChanged`; it sends `Order::Special` with
//! [`sta_sim::specials::point_defence_slot`] (the unit's first PD weapon).
//!
//! Limits, known: the view does not carry the PD switch, so a controller
//! built mid-game (snapshot load, replay seek) or handed a unit whose PD is
//! already on assumes it is off, and a script that toggles PD behind its
//! back is not seen; a UI-31 seam field would fix this. The policy does not
//! tell beam attacks (which PD cannot stop) from ordnance, and it asks for
//! twice the weapon's cost in the pool before switching on again, so an
//! emptied pool does not make it flap.

use sta_core::{EntityId, Fixed, TICK_HZ, Tick};
use sta_sim::command::{Command, Order, Target};
use sta_sim::specials::point_defence_slot;
use sta_sim::view::WorldView;
use sta_sim::SimEvent;
use sta_content::WeaponKind;

use super::SkirmishController;

/// Ticks without an attack before PD is switched off: three seconds.
pub(super) const QUIET: Tick = 3 * TICK_HZ as Tick;

impl SkirmishController {
    /// Note an attack on `target` at `tick`; the event stream is global, so
    /// only own units are looked up later.
    pub(super) fn note_attack(&mut self, tick: Tick, event: &SimEvent) {
        self.pd_attacked.retain(|_, (t, _)| tick.saturating_sub(*t) < QUIET);
        match event {
            SimEvent::Fired { shooter, target: Target::Entity(t), .. } | SimEvent::Hit { by: shooter, target: t, .. } => {
                self.pd_attacked.insert(*t, (tick, *shooter));
            }
            SimEvent::PointDefenceChanged { id, on } => {
                if *on {
                    self.pd_on.insert(*id);
                } else {
                    self.pd_on.remove(id);
                }
            }
            _ => {}
        }
    }

    /// Toggle PD on own units that have it.
    pub(super) fn point_defence(&mut self, view: &WorldView, me: &sta_sim::view::PlayerView, out: &mut Vec<Command>) {
        let db = std::sync::Arc::clone(&self.cfg.content);
        let seat = self.cfg.player;
        let tick = view.tick;
        let mut live: Vec<EntityId> = Vec::new();
        for u in view.units.iter().filter(|u| u.owner == seat) {
            live.push(u.id);
            let Some(def) = db.unit(u.def) else { continue };
            let Some(slot) = point_defence_slot(&db, def) else { continue };
            let cost = def
                .hardpoints
                .iter()
                .filter_map(|h| db.weapon(h.weapon.key))
                .find(|w| w.kind == WeaponKind::PointDefence)
                .map_or(Fixed::ZERO, |w| w.energy_cost);
            // An attack counts when its shooter is an enemy of this seat.
            let enemy = |shooter: EntityId| {
                view.units
                    .binary_search_by_key(&shooter, |s| s.id)
                    .ok()
                    .map(|i| view.units[i].owner)
                    .is_some_and(|o| me.relations.iter().any(|&(p, r)| p == o && r == sta_sim::view::Relation::Enemy))
            };
            let attacked = self
                .pd_attacked
                .get(&u.id)
                .is_some_and(|&(t, shooter)| tick.saturating_sub(t) < QUIET && enemy(shooter));
            let on = self.pd_on.contains(&u.id);
            let can_pay = u.energy.is_some_and(|[now, _]| now >= cost + cost && now > Fixed::ZERO);
            if (attacked && !on && can_pay) || (!attacked && on) {
                let order = Order::Special { units: vec![u.id], slot, target: Target::None };
                // No units in the record: a refused toggle must not clear the
                // unit's move, attack or mining state (`absorb`).
                self.issue(tick, out, Vec::new(), order, None, None);
            }
        }
        // Forget what no longer exists.
        self.pd_attacked.retain(|id, _| live.binary_search(id).is_ok());
        self.pd_on.retain(|id| live.binary_search(id).is_ok());
    }
}

#[cfg(test)]
#[path = "pd_tests.rs"]
mod tests;
