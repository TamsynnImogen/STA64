//! Paused map-inspector edits. Validate the whole request before replacing the
//! simulation so a rejected field cannot leave a partly edited object.

use sta_core::{EntityId, Fixed};

use crate::Sim;
use crate::command::Subsystem;
use crate::source::Inspection;
use crate::view::UnitFlags;
use crate::world::{EntityKind, NEUTRAL};

const SYSTEMS: [Subsystem; 5] = [
    Subsystem::Engines,
    Subsystem::Weapons,
    Subsystem::Shields,
    Subsystem::LifeSupport,
    Subsystem::Sensors,
];

const MAX_VALUE: Fixed = Fixed::from_int(1_000_000_000);

impl Sim {
    pub(crate) fn inspect_placement(&mut self, inspection: Inspection) -> Option<EntityId> {
        let Inspection {
            id,
            owner,
            at,
            heading,
            label,
            name,
            properties,
        } = inspection;
        let db = self.content.as_deref()?;
        let unit = self.units.get(id)?;
        let def = db.unit(unit.def)?;
        if (owner != NEUTRAL && !owner.is_player())
            || unit.progress.is_some()
            || unit.flags.contains(UnitFlags::DYING)
            || unit.flags.contains(UnitFlags::DOCKED)
            || name
                .as_ref()
                .is_some_and(|name| name.len() > crate::switches::MAX_NAME_BYTES)
            || heading == (Fixed::ZERO, Fixed::ZERO)
            || heading.0 < -Fixed::ONE
            || heading.0 > Fixed::ONE
            || heading.1 < -Fixed::ONE
            || heading.1 > Fixed::ONE
        {
            return None;
        }
        let pair = |[now, max]: [Fixed; 2]| {
            now >= Fixed::ZERO && max >= Fixed::ZERO && now <= max && max <= MAX_VALUE
        };
        if properties.hull.is_some_and(|p| !pair(p))
            || properties
                .crew
                .is_some_and(|p| unit.crew.is_none() || !pair(p))
        {
            return None;
        }
        if let Some(shields) = properties.shields {
            let max = shields[0][1];
            if !matches!(unit.kind, EntityKind::Craft | EntityKind::Station)
                || shields.iter().any(|&p| !pair(p) || p[1] != max)
                || (!unit.shields.directional && shields.iter().any(|&p| p != shields[0]))
            {
                return None;
            }
        }
        if let Some(energy) = properties.energy {
            let cap = crate::specials::energy_def(def)?.max;
            if energy < Fixed::ZERO
                || energy > cap
                || properties.hull.unwrap_or(unit.hull)[0] <= Fixed::ZERO
            {
                return None;
            }
        }
        if let Some(health) = properties.systems
            && (!matches!(unit.kind, EntityKind::Craft | EntityKind::Station)
                || properties.hull.unwrap_or(unit.hull)[0] <= Fixed::ZERO
                || health.iter().enumerate().any(|(n, &f)| {
                    f < Fixed::ZERO
                        || f > Fixed::ONE
                        || (def.system_hit_points[n] <= Fixed::ZERO && f != Fixed::ONE)
                }))
        {
            return None;
        }
        if properties
            .perceived_owner
            .is_some_and(|owner| !crate::perception::representable(owner))
            || properties
                .systems_enabled
                .iter()
                .enumerate()
                .any(|(n, value)| {
                    value.is_some()
                        && (def.system_hit_points[n] <= Fixed::ZERO
                            || !matches!(unit.kind, EntityKind::Craft | EntityKind::Station))
                })
        {
            return None;
        }
        if let Some(amount) = properties.resource {
            let node = self.node_view().into_iter().find(|row| row.entity == id)?;
            if amount < Fixed::ZERO
                || amount > node.start
                || (node.infinite && amount != node.start)
            {
                return None;
            }
        }
        if let Some(population) = properties.population
            && (def.planet_role != Some(sta_content::PlanetRole::Producer)
                || properties.hull.unwrap_or(unit.hull)[0] <= Fixed::ZERO
                || population < Fixed::ZERO
                || population > MAX_VALUE)
        {
            return None;
        }

        // Editor-only ownership deliberately admits inactive authored seats.
        // All other failures were checked above, and this clone makes the
        // transaction indivisible if a module setter still declines it.
        let mut next = self.clone();
        let actor = next.units.get_mut(id)?;
        let transfer = actor.owner != owner;
        if transfer || actor.pos != at || actor.heading != heading {
            actor.owner = owner;
            actor.pos = at;
            actor.heading = crate::world::unit_heading(heading);
            actor.target = None;
            actor.speed = Fixed::ZERO;
            actor.omega = Fixed::ZERO;
            next.movement.clear(id);
        }
        if transfer {
            if next.perception.get(id) == Some(&owner) {
                next.perception.remove(id);
            }
            next.refresh_ship_upgrades();
        }
        if let Some(race) = properties.race.as_deref() {
            if race.is_empty() {
                next.attrs.remove(id);
            } else {
                let race = next
                    .known_race(race)
                    .filter(|race| race.len() <= crate::attrs::MAX_RACE_BYTES)?;
                next.attrs.set_race(id, &race);
            }
        }
        // Map authors can set an identity for an authored seat that is inactive in test play.
        if let Some(perceived) = properties.perceived_owner {
            if perceived == owner {
                next.perception.remove(id);
            } else {
                next.perception.insert(id, perceived);
            }
        }
        if let Some(on) = properties.immortal {
            let actor = next.units.get_mut(id)?;
            actor.flags = actor.flags.with(UnitFlags::IMMORTAL, on);
        }
        if let Some(label) = label {
            if label.is_empty() {
                next.labels.remove(id);
            } else {
                next.labels.insert(id, label);
            }
        }
        if let Some(name) = name {
            next.switches
                .edit(id, |row| row.name = (!name.is_empty()).then_some(name));
        }
        if let Some(hull) = properties.hull {
            next.units.get_mut(id)?.hull = hull;
        }
        if let Some(crew) = properties.crew {
            next.units.get_mut(id)?.crew = Some(crew);
        }
        // A shield switch may clear the pools. Set switches before applying
        // explicit pool values so loading a map reproduces its authored state.
        for (sub, enabled) in SYSTEMS.into_iter().zip(properties.systems_enabled) {
            if let Some(enabled) = enabled
                && !next.edit_subsystem_switch(id, sub, enabled)
            {
                return None;
            }
        }
        if let Some(shields) = properties.shields {
            let actor = next.units.get_mut(id)?;
            actor.shields.max = shields[0][1];
            if actor.shields.directional {
                actor.shields.now = shields.map(|p| p[0]);
            } else {
                actor.shields.now = [shields[0][0], Fixed::ZERO, Fixed::ZERO, Fixed::ZERO];
            }
            for pool in 0..4 {
                next.zero_shield_holds.clear_pool(id, pool);
            }
        }
        if let Some(energy) = properties.energy {
            let db = next.content.as_deref()?;
            if !next.specials.set_energy(&next.units, db, id, energy) {
                return None;
            }
        }
        if let Some(health) = properties.systems {
            for (n, sub) in SYSTEMS.into_iter().enumerate() {
                let cap = def.system_hit_points[n];
                if cap <= Fixed::ZERO {
                    continue;
                }
                let (_, old) = next.subsystem_state(id, sub)?;
                let desired = cap * (Fixed::ONE - health[n]);
                if !next.edit_subsystem_damage(id, sub, desired - old) {
                    return None;
                }
            }
        }
        if let Some(amount) = properties.resource {
            next.economy.edit_node_amount(id, amount);
        }
        if let Some(population) = properties.population
            && !next.edit_planet_population(id, population)
        {
            return None;
        }
        *self = next;
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::MapDef;

    use crate::WorldSource;
    use crate::setup::{GameOptions, GameSetup};
    use crate::source::{ObjectProperties, PlacementEdit};
    use sta_core::{PlayerId, Vec3};

    use super::*;

    #[test]
    fn invalid_inspector_field_leaves_all_simulation_state_untouched() {
        let content = Arc::new(sta_content::fake::db());
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 1,
            map: Arc::new(MapDef::empty()),
            content: content.clone(),
            players: Vec::new(),
            options: GameOptions::default(),
        });
        let id = sim
            .spawn_def(
                content.key("fx_cruiser").unwrap(),
                PlayerId(0),
                Vec3::ZERO,
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        let before = sim.digest();
        let mut properties = ObjectProperties {
            hull: Some([Fixed::from_int(50), Fixed::from_int(100)]),
            ..ObjectProperties::default()
        };
        properties.shields = Some([
            [Fixed::from_int(10), Fixed::from_int(20)],
            [Fixed::from_int(10), Fixed::from_int(21)],
            [Fixed::from_int(10), Fixed::from_int(20)],
            [Fixed::from_int(10), Fixed::from_int(20)],
        ]);
        assert_eq!(
            sim.edit_placement(PlacementEdit::Inspect(Box::new(Inspection {
                id,
                owner: PlayerId(7),
                at: Vec3::from_f32([10.0, 0.0, 0.0]),
                heading: (Fixed::ONE, Fixed::ZERO),
                label: Some("new label".into()),
                name: Some("New name".into()),
                properties,
            }))),
            None
        );
        assert_eq!(sim.digest(), before);
        assert_eq!(sim.tick, 0);
        assert_eq!(sim.units.get(id).unwrap().owner, PlayerId(0));
        assert!(sim.labels.get(id).is_none());

        sim.labels.insert(id, "existing label".into());
        assert_eq!(
            sim.edit_placement(PlacementEdit::Inspect(Box::new(Inspection {
                id,
                owner: PlayerId(7),
                at: Vec3::from_f32([10.0, 0.0, 0.0]),
                heading: (Fixed::ONE, Fixed::ZERO),
                label: None,
                name: None,
                properties: ObjectProperties::default(),
            }))),
            Some(id)
        );
        assert_eq!(sim.tick, 0);
        assert_eq!(sim.units.get(id).unwrap().owner, PlayerId(7));
        assert_eq!(
            sim.labels.get(id).map(String::as_str),
            Some("existing label")
        );
    }
}
