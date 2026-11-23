//! Ship research effects derive from live completed pods. The stock shield
//! and weapon tiers feed different stats. Extended tiers replace the tier-3
//! value for that system, using the highest live level.

use std::collections::{BTreeMap, BTreeSet};

use sta_content::{ContentDb, UpgradeSystem};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId};

use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entities, EntityKind};
use crate::Sim;

#[derive(Clone, Copy)]
pub(crate) enum Effect {
    ShieldAbsorption,
    ShieldCapacity,
    Engines,
    WeaponDamage,
    WeaponReload,
    LifeSupport,
    Sensors,
}

#[derive(Default, Debug, Clone)]
pub struct ShipUpgrades(BTreeMap<PlayerId, [Option<(u32, Fixed)>; 7]>);

impl ShipUpgrades {
    pub(crate) fn collect(db: &ContentDb, units: &Entities) -> Self {
        Self::from_defs(db, units.iter().filter(|u| u.hull[0] > Fixed::ZERO
            && !u.flags.contains(UnitFlags::DYING) && !u.is_unfinished()).map(|u| (u.owner, u.def)))
    }

    pub fn from_defs(db: &ContentDb, units: impl Iterator<Item = (PlayerId, ContentKey)>) -> Self {
        let mut upgrades = Self::default();
        let max = db.rules().upgrade_pod_maximum_tier();
        for (owner, def) in units {
            let Some(pod) = db.unit(def).and_then(|d| d.system_upgrade) else { continue };
            let tier = if pod.tier > max { 3 } else { pod.tier };
            let effect = match pod.system {
                UpgradeSystem::Shields if tier == 2 => Effect::ShieldAbsorption,
                UpgradeSystem::Shields => Effect::ShieldCapacity,
                UpgradeSystem::Engines => Effect::Engines,
                UpgradeSystem::Weapons if tier == 2 => Effect::WeaponDamage,
                UpgradeSystem::Weapons => Effect::WeaponReload,
                UpgradeSystem::LifeSupport => Effect::LifeSupport,
                UpgradeSystem::Sensors => Effect::Sensors,
            };
            upgrades.set(owner, effect, tier, pod.multiplier);
            // The native replacement keeps the first shield/weapon upgrade's
            // separate stat while the later pod in that chain remains held.
            let earlier_effect = match effect {
                Effect::ShieldCapacity => Some(Effect::ShieldAbsorption),
                Effect::WeaponReload => Some(Effect::WeaponDamage),
                _ => None,
            };
            if let Some(effect) = earlier_effect && let Some(primary) = db.research_primary(def) {
                upgrades.set(owner, effect, primary.tier, primary.multiplier);
            }
        }
        upgrades
    }

    fn set(&mut self, owner: PlayerId, effect: Effect, tier: u32, multiplier: Fixed) {
        let value = &mut self.0.entry(owner).or_default()[effect as usize];
        // Stock applies the latest constructed pod at a tier. Some Fleet
        // Ops chains intentionally improve a modifier without changing tier.
        if value.is_none_or(|(current, _)| tier >= current) {
            *value = Some((tier, multiplier));
        }
    }

    pub(crate) fn multiplier(&self, owner: PlayerId, effect: Effect) -> Fixed {
        self.0.get(&owner).and_then(|row| row[effect as usize]).map_or(Fixed::ONE, |(_, factor)| factor)
    }

    pub fn sensor_multiplier(&self, owner: PlayerId) -> Fixed {
        self.multiplier(owner, Effect::Sensors)
    }

    fn ship_stats(&self, owner: PlayerId) -> Stats {
        Stats {
            shields: self.multiplier(owner, Effect::ShieldCapacity),
            engines: self.multiplier(owner, Effect::Engines),
        }
    }
}

/// Multipliers already reflected in the live ship stats. The snapshot stores
/// the stats themselves and the pod entities, so this memo is derived on load.
#[derive(Default, Debug, Clone)]
pub(crate) struct Applied(BTreeMap<EntityId, Stats>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stats {
    shields: Fixed,
    engines: Fixed,
}

impl Default for Stats {
    fn default() -> Self {
        Self { shields: Fixed::ONE, engines: Fixed::ONE }
    }
}

impl Sim {
    pub(crate) fn refresh_ship_upgrades(&mut self) {
        let Some(db) = self.content.as_deref() else { return };
        let upgrades = ShipUpgrades::collect(db, &self.units);
        if upgrades.0.is_empty() && self.ship_upgrades.0.is_empty() {
            return;
        }
        let spawned: BTreeSet<_> = self.events.iter().filter_map(|e| match e {
            SimEvent::Spawned { id } => Some(*id),
            _ => None,
        }).collect();
        self.ship_upgrades.0.retain(|id, _| self.units.contains(*id));
        for ship in self.units.iter_mut().filter(|u| matches!(u.kind, EntityKind::Craft | EntityKind::Station)) {
            let Some(def) = db.unit(ship.def) else { continue };
            let old = self.ship_upgrades.0.get(&ship.id).copied().unwrap_or_default();
            let new = upgrades.ship_stats(ship.owner);
            if old.engines != new.engines && ship.kind == EntityKind::Craft {
                if ship.stats.max_speed == def.movement.max_speed * old.engines {
                    ship.stats.max_speed = def.movement.max_speed * new.engines;
                }
                ship.stats.combat_speed = def.movement.combat_speed * new.engines;
            }
            if old.shields != new.shields {
                let max = def.shields.max * new.shields;
                let initial = spawned.contains(&ship.id) && def.initial_shields.is_none() && old == Stats::default();
                for now in &mut ship.shields.now[..if ship.shields.directional { 4 } else { 1 }] {
                    *now = if initial && *now == ship.shields.max { max } else { (*now).min(max) };
                }
                ship.shields.max = max;
            }
            if new != Stats::default() {
                self.ship_upgrades.0.insert(ship.id, new);
            } else if old != Stats::default() {
                self.ship_upgrades.0.remove(&ship.id);
            }
        }
    }

    pub(crate) fn restore_ship_upgrade_memo(&mut self) {
        let Some(db) = self.content.as_deref() else { return };
        let upgrades = ShipUpgrades::collect(db, &self.units);
        self.ship_upgrades.0 = self.units.iter()
            .filter(|u| matches!(u.kind, EntityKind::Craft | EntityKind::Station))
            .filter_map(|u| {
                let stats = upgrades.ship_stats(u.owner);
                (stats != Stats::default()).then_some((u.id, stats))
            })
            .collect();
    }

    pub(crate) fn engine_upgrade(&self, owner: PlayerId) -> Fixed {
        self.content.as_deref().map_or(Fixed::ONE, |db| ShipUpgrades::collect(db, &self.units).multiplier(owner, Effect::Engines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::{SystemUpgrade, UnitDef};

    #[test]
    fn later_pods_at_the_same_fleet_ops_tier_take_effect() {
        let pod = |name, multiplier| {
            let mut def = UnitDef::new(name, "pod");
            def.system_upgrade = Some(SystemUpgrade { system: UpgradeSystem::Weapons, tier: 2, multiplier });
            def
        };
        let first = Fixed::from_f64(1.1);
        let second = Fixed::from_f64(1.2);
        let db = ContentDb::builder().unit(pod("first", first)).unit(pod("second", second)).build().unwrap();
        let owner = PlayerId(0);
        let one = (owner, db.key("first").unwrap());
        let two = (owner, db.key("second").unwrap());
        assert_eq!(ShipUpgrades::from_defs(&db, [one, two].into_iter()).multiplier(owner, Effect::WeaponDamage), second);
        assert_eq!(ShipUpgrades::from_defs(&db, [one].into_iter()).multiplier(owner, Effect::WeaponDamage), first);
    }
}
