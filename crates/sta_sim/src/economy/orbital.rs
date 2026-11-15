//! Orbital mining income (SIM-66, part of #748): a finished orbital station on its
//! planet's ring whose mining beam is alive takes from the planet's node into its owner's
//! stock, with no freighter and no dock.
//!
//! Rule (read from the executable, static; the key names read from data; see the CON-51
//! content fields): per tick of a living beam the station yields
//! `miningRate x miningMultiplier x race miningRateModifier / TICK_HZ`, limited by what
//! remains, into the owner's stock of the node's resource; the resource is the node's
//! (`UnitDef::resource`, the first slot with `amt<slot>` above zero) when the station's
//! `resourcesCanHandle` names it. At or below `trickleAmount` the node holds there and
//! yields `trickleMiningRateFraction` of that rate; an infinite node never falls.
//!
//! The beam: the sim keeps no beam state (the beam is a client effect). It is alive for
//! `duration` ticks of every `refire` ticks of the station's mining weapon (the first
//! hardpoint whose weapon is class `minebm`), counted from tick 0: a function of the tick,
//! so nothing is saved. Its phase against the client's drawn pulse is not synchronised
//! (not measured). A station still being built, dying, or owned by nobody earns nothing.
//!
//! Choices, labelled: a race without `miningRateModifier` counts 1 (the engine's default
//! was not read); the single-player difficulty modifier is not applied (not graded); only
//! the node's own resource is mined, because the saved node table holds one amount per
//! node; a station without `miningMultiplier` earns nothing (its default is copied from a
//! template class in the engine, not read).

use sta_content::ContentDb;
use sta_core::{Fixed, PlayerId};

use super::{Economy, RESOURCES, node_resource, per_tick, planet_trickle, is_infinite, start_amount};
use crate::production::orbit;
use crate::world::{Entities, NEUTRAL};

/// The class of the mining beam's ordnance, as for freighters.
const BEAM_CLASS: &str = "minebm";

/// Whether the station's beam is alive on `tick`: the first mining weapon's pulse.
fn beam_alive(db: &ContentDb, def: &sta_content::UnitDef, tick: u64) -> bool {
    let Some(w) = def.hardpoints.iter().find_map(|h| db.weapon(h.weapon.key).filter(|w| w.class.0 == BEAM_CLASS)) else {
        return false;
    };
    tick % w.refire.max(1) < w.duration
}

impl Economy {
    /// One tick of every orbital station's income. `race_modifiers` is each seat's
    /// `miningRateModifier` (`None`: its race authors none).
    pub(crate) fn orbital_income(&mut self, db: &ContentDb, units: &Entities, tick: u64, race_modifiers: &[(PlayerId, Option<Fixed>)]) {
        let (planets, orbiters) = orbit::from_entities(db, units);
        if planets.is_empty() || orbiters.is_empty() {
            return;
        }
        for e in units.iter() {
            if e.progress.is_some() || e.owner == NEUTRAL || e.flags.contains(crate::view::UnitFlags::DYING) {
                continue;
            }
            let Some(d) = db.unit(e.def).filter(|d| orbit::is_orbital(d)) else { continue };
            let Some(station) = d.station_mining else { continue };
            let (Some(mult), Some(handles)) = (station.multiplier, station.can_handle) else { continue };
            if !beam_alive(db, d, tick) {
                continue;
            }
            let Some(planet) = orbit::on_ring(&planets, e.pos) else { continue };
            let Some(pd) = units.get(planet.id).and_then(|p| db.unit(p.def)) else { continue };
            let Some(node) = pd.node_income else { continue };
            let Some(rate) = node.mining_rate else { continue };
            let kind = usize::from(node_resource(db, units.get(planet.id).map_or(e.def, |p| p.def))).min(RESOURCES - 1);
            if !handles[kind] {
                continue;
            }
            let race = race_modifiers.iter().find(|(p, _)| *p == e.owner).and_then(|(_, m)| *m).unwrap_or(Fixed::ONE);
            let per = per_tick(rate * mult * race);
            let def = units.get(planet.id).map_or(e.def, |p| p.def);
            let left = self.nodes.get(planet.id).copied().unwrap_or_else(|| start_amount(db, def));
            let trickle = planet_trickle(db, def);
            let amount = if is_infinite(db, def) {
                per
            } else if let Some((_, fraction)) = trickle.filter(|(floor, _)| left <= *floor) {
                // Held at the floor.
                per * fraction
            } else {
                let take = per.min(left);
                // Taking the node below its trickle amount leaves it at the amount.
                let rest = trickle.map_or(left - take, |(floor, _)| (left - take).max(floor));
                self.nodes.insert(planet.id, rest);
                take
            };
            if amount > Fixed::ZERO {
                self.slot(e.owner)[kind] = self.slot(e.owner)[kind] + amount;
            }
        }
    }
}
