//! Armada I Federation support payloads. Targeting, energy, launch timing and
//! Gemini expiry belong to the special-weapon caller.

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed};

use crate::Sim;
use crate::command::Subsystem;
use crate::world::EntityKind;

impl Sim {
    /// Repair all damaged systems and restore at least the authored fraction of
    /// maximum health. A1's shield strength is its `maxHealth` hull pool.
    pub(in crate::specials) fn a1_engineering(
        &mut self,
        db: &ContentDb,
        target: EntityId,
        shields_returned: Fixed,
    ) {
        let Some(unit) = self.units.get(target) else {
            return;
        };
        if !matches!(unit.kind, EntityKind::Craft | EntityKind::Station)
            || unit.hull[0] <= Fixed::ZERO
        {
            return;
        }
        let Some(def) = db.unit(unit.def) else { return };
        let a1 = db.unit_features(def).a1_compat;

        let damage = self.subsystems.get(target).unwrap_or_default().damage;
        for (system, points) in damage.into_iter().enumerate() {
            if points > 0 {
                self.yard_repair_system(target, system, Fixed::from_bits(points));
            }
        }

        let Some(unit) = self.units.get_mut(target) else {
            return;
        };
        let fraction = shields_returned.max(Fixed::ZERO).min(Fixed::ONE);
        if a1 {
            unit.hull[0] = unit.hull[0].max(unit.hull[1] * fraction);
        } else {
            let threshold = unit.shields.max * fraction;
            let pools = if unit.shields.directional { 4 } else { 1 };
            for pool in &mut unit.shields.now[..pools] {
                *pool = (*pool).max(threshold);
            }
        }
    }

    /// Make a temporary, commandable copy of a live craft. The caller owns
    /// target eligibility and retires the returned id when its timer ends.
    pub(super) fn a1_gemini(
        &mut self,
        db: &ContentDb,
        caster: EntityId,
        target: EntityId,
    ) -> Option<EntityId> {
        let owner = self.units.get(caster)?.owner;
        let source = self.units.get(target)?.clone();
        if source.kind != EntityKind::Craft
            || source.hull[0] <= Fixed::ZERO
            || source.progress.is_some()
        {
            return None;
        }
        let def = db.unit(source.def)?;
        // Construction and research objects have lifecycle side effects that
        // must never be reproduced by a combat duplicate.
        if def.system_upgrade.is_some() || def.a1_officer_upgrade_gain.is_some() {
            return None;
        }

        let systems = [
            Subsystem::Engines,
            Subsystem::Weapons,
            Subsystem::Shields,
            Subsystem::LifeSupport,
            Subsystem::Sensors,
        ];
        let status = self.subsystems.get(target).unwrap_or_default();
        let race = self.attrs.race(target).map(str::to_owned);

        // This spawn emits Spawned and registers ordinary lifecycle hooks,
        // without creating a build queue or a research completion.
        let id = self.spawn_unqueued_def(source.def, owner, source.pos, source.heading)?;
        if let Some(copy) = self.units.get_mut(id) {
            copy.hull = source.hull;
            copy.crew = source.crew;
            copy.shields = source.shields;
            copy.stats = source.stats;
            copy.speed = source.speed;
            copy.omega = source.omega;
            copy.flags = copy
                .flags
                .with(
                    crate::view::UnitFlags::ENGINES_OFF,
                    source.flags.contains(crate::view::UnitFlags::ENGINES_OFF),
                )
                .with(
                    crate::view::UnitFlags::WEAPONS_OFF,
                    source.flags.contains(crate::view::UnitFlags::WEAPONS_OFF),
                );
        }
        for (n, system) in systems.into_iter().enumerate() {
            if status.damage[n] > 0 {
                self.edit_subsystem_damage(id, system, Fixed::from_bits(status.damage[n]));
            }
            if status.off & (1 << n) != 0 {
                self.edit_subsystem_switch(id, system, false);
            }
        }
        self.specials.set_energy(&self.units, db, id, Fixed::ZERO);
        if let Some(race) = race {
            self.attrs.set_race(id, &race);
        }
        Some(id)
    }
}
