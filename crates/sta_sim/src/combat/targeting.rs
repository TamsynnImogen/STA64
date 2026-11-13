//! Automatic target choice and assist (SIM-36). Implements D5, D9.
//!
//! **Target choice** (`target_score`). A craft picks one target for all its
//! regular weapons: its attack order's, else the visible enemy within its
//! longest regular weapon range with the highest `attackPower / current
//! shields` (`attackPower` alone below one point of shields); each weapon then
//! fires at that one target when it is in that weapon's range and arc.
//! Read from the executable (stock A2 and the Roots engine, the same code,
//! static); measured on stock and Roots + STA2 Classic with
//! candidates differing in class, shields-hull and distance: a Galaxy and a
//! Defiant side by side, the Defiant (lower shields, same `attackPower`) is
//! chosen. Equal scores fall to the engine's spatial-grid walk, which depends on
//! map position: STA64 takes the nearer, then the lower id (not measured as a
//! rule; the measured picks between equal candidates moved with the scene's
//! position). The engine considers enemies out to its scan range and keeps the
//! best one even out of weapon range (read, not measured): not followed here.
//!
//! **Assist** (`idle_assist`). An idle craft with regular weapons, finding
//! within its sensor range (`rangeScan`) itself or an ally hit in the last
//! second, attacks that ally's last attacker; under `assist_on_ally_fire`
//! (stock) one that fired in the last second also draws it in, onto its own
//! best target in scan range. The craft itself counts as one of the allies
//! (as the engine reads it): an idle craft that fired takes an attack order on
//! its best target and closes in. The attack is an
//! ordinary attack order, so the approach rules apply (stock: `combatSpeed`).
//! Read from the executable (stock, static); measured on stock at every movement
//! autonomy (helper 600 from an attacked ally assists, 1500 does not) and on
//! Roots + STA2 Classic, where only high movement autonomy assists
//! (`assist_high_autonomy_only`) and a firing ally draws no one in (the Roots
//! engine's patch of the idle state, read statically). That also explains the
//! measured idle picks: a stock craft that is shot at turns on its attacker,
//! a Roots one at medium autonomy keeps its best-scoring target. Not modelled: alert status (the engine's
//! green alert never fires on its own; STA64 crafts are always at red), the
//! idle leash (a 300-unit dogfight radius around the hold point).
//!
//! The memory of who was hit by whom and who fired is kept for one second.

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, TeamId, TICK_HZ};

use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entity, EntityKind, Fnv, Table};
use crate::Sim;

/// How long a hit or a shot counts for an assist: one second (read from
/// the executable, static).
fn window() -> u64 {
    TICK_HZ as u64
}

/// The last hit taken (tick, attacker) and the last shot fired (tick), per
/// entity, for at most [`window`] ticks.
#[derive(Debug, Clone, Default)]
pub struct Engagement {
    hit: Table<(u64, EntityId)>,
    fired: Table<u64>,
}

/// [`Engagement`] as the snapshot stores it.
pub(crate) type EngagementRec = (Vec<(EntityId, (u64, EntityId))>, Vec<(EntityId, u64)>);

impl Engagement {
    pub(crate) fn is_empty(&self) -> bool {
        self.hit.is_empty() && self.fired.is_empty()
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.hit.remove(id);
        self.fired.remove(id);
    }

    /// Note this tick's hits and shots; forget what is older than the window.
    pub(crate) fn record(&mut self, tick: u64, events: &[SimEvent]) {
        for e in events {
            match *e {
                SimEvent::Hit { target, by, .. } if target != by => {
                    self.hit.insert(target, (tick, by));
                }
                SimEvent::Fired { shooter, .. } => {
                    self.fired.insert(shooter, tick);
                }
                _ => {}
            }
        }
        let old = |t: u64| tick >= t.saturating_add(window());
        let stale: Vec<EntityId> = self.hit.iter().filter(|(_, h)| old(h.0)).map(|(id, _)| id).collect();
        for id in stale {
            self.hit.remove(id);
        }
        let stale: Vec<EntityId> = self.fired.iter().filter(|(_, t)| old(**t)).map(|(id, _)| id).collect();
        for id in stale {
            self.fired.remove(id);
        }
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("engagement-v1");
        h.u64(self.hit.len() as u64);
        for (id, (t, by)) in self.hit.iter() {
            h.u64(id.0 as u64);
            h.u64(*t);
            h.u64(by.0 as u64);
        }
        h.u64(self.fired.len() as u64);
        for (id, t) in self.fired.iter() {
            h.u64(id.0 as u64);
            h.u64(*t);
        }
    }

    pub(crate) fn to_rec(&self) -> EngagementRec {
        (self.hit.iter().map(|(id, h)| (id, *h)).collect(), self.fired.iter().map(|(id, t)| (id, *t)).collect())
    }

    /// `None` when rows do not ascend, a tick is after `now`, or a row's
    /// entity is not `exists`.
    pub(crate) fn from_rec(r: EngagementRec, now: u64, exists: impl Fn(EntityId) -> bool) -> Option<Engagement> {
        if r.0.iter().any(|(id, (t, _))| *t > now || !exists(*id)) || r.1.iter().any(|(id, t)| *t > now || !exists(*id)) {
            return None;
        }
        Some(Engagement { hit: Table::from_rows(r.0)?, fired: Table::from_rows(r.1)? })
    }
}

/// `attackPower` when no ODF in the chain sets it: 0.5, the engine's base
/// class default (read from the executable, static).
fn default_attack_power() -> Fixed {
    Fixed::ONE / Fixed::from_int(2)
}

/// The engine's score of an enemy: `attackPower / current shields`, or
/// `attackPower` below one point of shields.
pub(crate) fn score(db: &ContentDb, t: &Entity) -> Fixed {
    let power = db.unit(t.def).and_then(|d| d.attack_power).unwrap_or_else(default_attack_power);
    let shields = t.shields.now.iter().fold(Fixed::ZERO, |a, s| a + *s);
    if shields >= Fixed::ONE { power / shields } else { power }
}

/// The best target of `u` among `candidates` within `reach` (centre
/// distance): highest score, then nearer, then lower id.
pub(crate) fn best<'a>(db: &ContentDb, u: &Entity, reach: Fixed, candidates: impl Iterator<Item = &'a Entity>) -> Option<&'a Entity> {
    let mut out: Option<(Fixed, Fixed, &Entity)> = None;
    for t in candidates {
        let (dx, dz) = (t.pos.x - u.pos.x, t.pos.z - u.pos.z);
        let d = (dx * dx + dz * dz).sqrt();
        if d > reach {
            continue;
        }
        let s = score(db, t);
        let better = match out {
            None => true,
            Some((bs, bd, _)) => s > bs || (s == bs && d < bd),
        };
        if better {
            out = Some((s, d, t));
        }
    }
    out.map(|o| o.2)
}

/// Can `t` be picked automatically by a craft of team `mine`: an enemy (by
/// perceived owner), alive, not cloaked to it, targetable, and seen.
#[allow(clippy::too_many_arguments)]
pub(crate) fn candidate(
    db: &ContentDb,
    u: &Entity,
    t: &Entity,
    mine: TeamId,
    team: &dyn Fn(PlayerId) -> Option<TeamId>,
    perception: &Table<PlayerId>,
    seen: Option<&[EntityId]>,
    // `Some` for an automatic pick: units the mission disallowed as a target
    // for this seat (SIM-58 `AllowAsTarget`) are not candidates. `None` for
    // answering an attacker, which the engine does not filter.
    switches: Option<&crate::switches::SwitchTable>,
) -> bool {
    let perceived = perception.get(t.id).copied().unwrap_or(t.owner);
    t.id != u.id
        && team(perceived).is_some_and(|x| x != mine)
        && t.hull[1] > Fixed::ZERO
        && !(t.flags.contains(UnitFlags::CLOAKED) && team(t.owner) != Some(mine))
        && seen.is_none_or(|ids| ids.binary_search(&t.id).is_ok())
        && !db.unit(t.def).is_some_and(|d| d.invalid_as_target)
        && !switches.is_some_and(|s| s.blocked_for(t.id, u.owner))
}

impl Sim {
    /// Idle crafts take up the fight near them (see the module docs). Runs
    /// before the attack approach.
    pub(crate) fn assist_idle(&mut self) {
        let Some(db) = self.content.clone() else { return };
        let f = db.features();
        if !f.idle_assist || self.engagement.is_empty() {
            return;
        }
        let players = self.players.clone();
        let team = move |p: PlayerId| players.iter().find(|x| x.0 == p).map(|x| x.1);
        let visibility = crate::sensors::compute(&self.units, Some(&db), &self.players);
        let mut orders: Vec<(EntityId, EntityId)> = Vec::new();
        for u in self.units.iter() {
            let Some(def) = db.unit(u.def) else { continue };
            let Some(range) = crate::movement::approach::attack_range(&db, def) else { continue };
            let idle = u.kind == EntityKind::Craft
                && u.target.is_none()
                && self.weapons.attack_of(u.id).is_none()
                && self.movement.course(u.id).is_none_or(|c| c.waypoints.is_empty())
                && !self.production.is_deploying(u.id)
                && !self.production.is_welding(u.id)
                && !self.economy.is_mining(u.id)
                && !u.flags.contains(UnitFlags::WEAPONS_OFF)
                && !u.flags.contains(UnitFlags::CLOAKED);
            if !idle || (f.assist_high_autonomy_only && self.autonomy.levels(u.id)[0] < 2) {
                continue;
            }
            let Some(mine) = team(u.owner) else { continue };
            let near = |a: &Entity| {
                let (dx, dz) = (a.pos.x - u.pos.x, a.pos.z - u.pos.z);
                (dx * dx + dz * dz).sqrt() <= def.sensor
            };
            let seen = visibility.iter().find(|(p, _)| *p == u.owner).map(|(_, ids)| ids.as_slice());
            let mut pick = None;
            for a in self.units.iter() {
                if !(a.id == u.id || (team(a.owner) == Some(mine) && a.hull[1] > Fixed::ZERO && near(a))) {
                    continue;
                }
                if let Some((_, by)) = self.engagement.hit.get(a.id)
                    && let Some(t) = self.units.get(*by).filter(|t| candidate(&db, u, t, mine, &team, &self.perception, seen, None))
                {
                    pick = Some(t.id);
                    break;
                }
                if f.assist_on_ally_fire && self.engagement.fired.get(a.id).is_some() {
                    let reach = def.sensor.max(range);
                    pick = best(&db, u, reach, self.units.iter().filter(|t| candidate(&db, u, t, mine, &team, &self.perception, seen, Some(&self.switches)))).map(|t| t.id);
                    if pick.is_some() {
                        break;
                    }
                }
            }
            if let Some(t) = pick {
                orders.push((u.id, t));
            }
        }
        for (u, t) in orders {
            self.weapons.set_attack(u, t);
        }
    }

    /// After the weapons stepped: note this tick's hits and shots.
    pub(crate) fn record_engagement(&mut self, from: usize) {
        if self.content.as_deref().is_some_and(|db| db.features().idle_assist) {
            let tick = self.tick;
            let events = std::mem::take(&mut self.events);
            self.engagement.record(tick, &events[from..]);
            self.events = events;
        }
    }
}
