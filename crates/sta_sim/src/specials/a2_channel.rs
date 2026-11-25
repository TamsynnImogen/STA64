//! Sustained Armada II beams. The activation path owns their initial cost and
//! visual event; this path advances only the live transfer/boarding effect.

use serde::{Deserialize, Serialize};
use sta_content::{A2Effect, ContentDb};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TICK_HZ, Tick};

use super::{add_capped, target_ok, weapon_at};
use crate::Sim;
use crate::combat::boarding::{Party, race_strength};
use crate::command::Subsystem;
use crate::relations::Diplomacy;
use crate::view::UnitFlags;
use crate::world::Shields;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(super) struct Channel {
    pub source: EntityId,
    pub owner: PlayerId,
    pub target: EntityId,
    pub weapon: ContentKey,
    pub slot: u8,
    pub until: Tick,
    pub next_pulse: Tick,
    pub holding: bool,
}

impl Sim {
    /// Returns false when the beam must be removed. Target and source state is
    /// rechecked each tick because either can change while the beam is active.
    pub(super) fn step_a2_channel(&mut self, db: &ContentDb, channel: &mut Channel) -> bool {
        if self.tick >= channel.until || channel.source == channel.target {
            return false;
        }
        let Some(source) = self.units.get(channel.source).cloned() else {
            return false;
        };
        let Some(target) = self.units.get(channel.target).cloned() else {
            return false;
        };
        let Some(source_def) = db.unit(source.def) else {
            return false;
        };
        let Some((_, w, rule)) = weapon_at(db, source_def, channel.slot) else {
            return false;
        };
        if w.key != channel.weapon
            || channel.holding != matches!(rule.effect, A2Effect::HoldingBeam { .. })
            || source.owner != channel.owner
            || source.hull[0] <= Fixed::ZERO
            || source.is_unfinished()
            || source.flags.contains(UnitFlags::WEAPONS_OFF)
            || self.specials.stasis(channel.source, self.tick)
            || self.specials.stasis(channel.target, self.tick)
            || self
                .specials
                .a2_held(channel.source, Subsystem::Weapons, self.tick)
            || !target_ok(
                db,
                w,
                rule,
                &source,
                &target,
                Diplomacy::new(&self.players, &self.relations),
            )
        {
            return false;
        }

        let per_tick = Fixed::from_int(TICK_HZ as i32);
        match rule.effect {
            A2Effect::HoldingBeam {
                interval, party, ..
            } => {
                if !self.a2_channel_energy(channel.source, w.energy_cost / per_tick) {
                    return false;
                }
                if self.tick >= channel.next_pulse {
                    let available = source.crew.map_or(Fixed::ZERO, |c| {
                        (c[0] - Fixed::from_int(5)).max(Fixed::ZERO)
                    });
                    let size = party.max(Fixed::ZERO).min(available);
                    if size > Fixed::ZERO && target.crew.is_some_and(|c| c[1] > Fixed::ZERO) {
                        let race = self.unit_race(channel.source).unwrap_or("").to_owned();
                        let strength = race_strength(db, &race);
                        self.a2_lose_crew(channel.source, size);
                        // The holding beam delivers boarders directly. The
                        // normal transport shield gate is intentionally skipped.
                        self.resolve_party(
                            db,
                            Party {
                                arrive: self.tick,
                                seq: 0,
                                target: channel.target,
                                size,
                                strength,
                                race,
                                team: channel.owner,
                                source: channel.source,
                            },
                        );
                    }
                    channel.next_pulse = self.tick.saturating_add(interval.max(1));
                }
            }
            A2Effect::AutoAssimilator(rate) => {
                let amount = rate.max(Fixed::ZERO) / per_tick;
                let available = target.crew.map_or(Fixed::ZERO, |c| c[0]);
                if amount <= Fixed::ZERO
                    || available <= Fixed::ZERO
                    || !self.a2_channel_energy(channel.source, w.energy_cost / per_tick)
                {
                    return false;
                }
                let taken = amount.min(available);
                self.a2_lose_crew(channel.target, taken);
                let room = source
                    .crew
                    .map_or(Fixed::ZERO, |c| (c[1] - c[0]).max(Fixed::ZERO));
                let aboard = taken.min(room);
                if aboard > Fixed::ZERO
                    && let Some(u) = self.units.get_mut(channel.source)
                    && let Some([now, max]) = u.crew
                {
                    u.crew = Some([add_capped(now, aboard, max), max]);
                }
                let excess = taken - aboard;
                if excess > Fixed::ZERO {
                    let stock = self.economy.crew_of(channel.owner).unwrap_or(Fixed::ZERO);
                    self.economy.edit_team_crew(
                        channel.owner,
                        add_capped(stock, excess, Fixed::from_int(1_000_000_000)),
                    );
                }
            }
            A2Effect::ShieldProjector(rate) => {
                let amount = rate.max(Fixed::ZERO) / per_tick;
                let room = shield_room(&target.shields);
                let energy = self
                    .specials
                    .energy_of(channel.source)
                    .unwrap_or(Fixed::ZERO);
                let gain = amount.min(room).min(energy);
                if gain <= Fixed::ZERO {
                    return false;
                }
                if let Some(pool) = self.specials.energy_mut(channel.source) {
                    *pool = (*pool - gain).max(Fixed::ZERO);
                }
                if let Some(u) = self.units.get_mut(channel.target) {
                    give_shields(&mut u.shields, gain);
                }
            }
            A2Effect::ShieldInversion(rate) => {
                let amount = rate.max(Fixed::ZERO) / per_tick;
                let gain = amount.min(shield_total(&target.shields));
                if gain <= Fixed::ZERO
                    || !self.a2_channel_energy(channel.source, w.energy_cost / per_tick)
                {
                    return false;
                }
                if let Some(u) = self.units.get_mut(channel.target) {
                    take_shields(&mut u.shields, gain);
                }
                if let Some(u) = self.units.get_mut(channel.source) {
                    give_shields(&mut u.shields, gain);
                }
            }
            _ => return false,
        }
        true
    }

    fn a2_channel_energy(&mut self, source: EntityId, cost: Fixed) -> bool {
        if cost <= Fixed::ZERO {
            return true;
        }
        let Some(pool) = self.specials.energy_mut(source) else {
            return false;
        };
        if *pool < cost {
            return false;
        }
        *pool = *pool - cost;
        true
    }
}

fn count(shields: &Shields) -> usize {
    if shields.directional { 4 } else { 1 }
}

fn shield_total(shields: &Shields) -> Fixed {
    shields.now[..count(shields)]
        .iter()
        .copied()
        .fold(Fixed::ZERO, |sum, n| sum + n.max(Fixed::ZERO))
}

fn shield_room(shields: &Shields) -> Fixed {
    shields.now[..count(shields)]
        .iter()
        .copied()
        .fold(Fixed::ZERO, |sum, n| {
            sum + (shields.max - n).max(Fixed::ZERO)
        })
}

// A channel moves a total amount, rather than one full rate per facing.
// Ascending facing order makes directional transfers deterministic.
fn take_shields(shields: &mut Shields, mut amount: Fixed) {
    let n = count(shields);
    for pool in &mut shields.now[..n] {
        let taken = amount.min(*pool);
        *pool = *pool - taken;
        amount = amount - taken;
        if amount <= Fixed::ZERO {
            break;
        }
    }
}

fn give_shields(shields: &mut Shields, mut amount: Fixed) {
    let n = count(shields);
    for pool in &mut shields.now[..n] {
        let gain = amount.min((shields.max - *pool).max(Fixed::ZERO));
        *pool = add_capped(*pool, gain, shields.max);
        amount = amount - gain;
        if amount <= Fixed::ZERO {
            break;
        }
    }
}
