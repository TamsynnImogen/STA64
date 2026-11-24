//! Read-only presentation data (RND-33). Faction labels do not change rules or
//! digests. Subsystem health is derived from the existing saved damage state.

use serde::{Deserialize, Serialize};
use sta_core::{EntityId, Fixed, PlayerId};

use crate::{Sim, command::Subsystem, world::EntityKind};

/// Engines, weapons, shields, life support, sensors; no inferred hull state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemAppearance {
    pub enabled: bool,
    pub health: [Fixed; 2],
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceView {
    /// Owning-seat faction from the resolved setup. Missing means unknown.
    pub factions: Vec<(PlayerId, String)>,
    /// Only definition-backed live craft/stations. A zero-HP system is unknown.
    pub systems: Vec<(EntityId, [Option<SystemAppearance>; 5])>,
}

impl AppearanceView {
    pub fn faction(&self, owner: PlayerId) -> Option<&str> {
        self.factions
            .binary_search_by_key(&owner, |r| r.0)
            .ok()
            .map(|i| self.factions[i].1.as_str())
    }

    pub fn systems(&self, id: EntityId) -> Option<&[Option<SystemAppearance>; 5]> {
        self.systems
            .binary_search_by_key(&id, |r| r.0)
            .ok()
            .map(|i| &self.systems[i].1)
    }
}

impl Sim {
    pub(crate) fn appearance_view(&self) -> AppearanceView {
        let systems = self
            .units
            .iter()
            .filter_map(|u| {
                if !matches!(u.kind, EntityKind::Craft | EntityKind::Station) {
                    return None;
                }
                let def = self.content.as_ref()?.unit(u.def)?;
                let subs = [
                    Subsystem::Engines,
                    Subsystem::Weapons,
                    Subsystem::Shields,
                    Subsystem::LifeSupport,
                    Subsystem::Sensors,
                ];
                let health = std::array::from_fn(|i| {
                    let max = def.system_hit_points[i];
                    if max <= Fixed::ZERO {
                        return None;
                    }
                    let (enabled, damage) = self.subsystem_state(u.id, subs[i])?;
                    Some(SystemAppearance {
                        enabled,
                        health: [(max - damage).max(Fixed::ZERO), max],
                    })
                });
                Some((u.id, health))
            })
            .collect();
        AppearanceView {
            factions: self.appearance_factions.clone(),
            systems,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        WorldSource,
        command::{Command, CommandKind, Edit, Order},
        setup::{Control, GameSetup, PlayerSetup},
    };
    use sta_content::{ContentDb, MapDef, MoveStats, UnitDef};
    use sta_core::{TeamId, Vec3};
    use std::sync::Arc;

    fn setup(faction: &str) -> GameSetup {
        let mut def = UnitDef::new("visual_test", "craft");
        def.hull = Fixed::from_int(100);
        def.system_hit_points = [Fixed::from_int(20); 5];
        GameSetup {
            seed: 7,
            map: Arc::new(MapDef::empty()),
            content: Arc::new(ContentDb::builder().unit(def).build().unwrap()),
            players: vec![PlayerSetup {
                id: PlayerId(0),
                team: TeamId(0),
                race: faction.into(),
                control: Control::Human,
            }],
            options: Default::default(),
        }
    }

    #[test]
    fn labels_are_cosmetic_and_survive_snapshot_and_view_log() {
        let setup = setup("Klingon");
        let mut sim = Sim::from_setup(&setup);
        assert_eq!(sim.view().appearance.faction(PlayerId(0)), Some("klingon"));
        assert_eq!(
            sim.digest(),
            Sim::from_setup(&self::setup("federation")).digest()
        );
        let restored = Sim::from_snapshot(&sim.snapshot(), Some(setup.content.clone())).unwrap();
        assert_eq!(sim.view().appearance, restored.view().appearance);
        let log = crate::fake::ViewLog::record(&mut sim, [Vec::new()]);
        assert_eq!(
            crate::fake::ViewLog::from_bytes(&log.to_bytes())
                .unwrap()
                .view()
                .appearance
                .faction(PlayerId(0)),
            Some("klingon")
        );
    }

    #[test]
    fn actual_subsystem_health_repairs_and_transfers_without_guessing() {
        let setup = setup("federation");
        let mut sim = Sim::from_setup(&setup);
        let id = sim
            .spawn_def(
                setup.content.key("visual_test").unwrap(),
                PlayerId(0),
                Vec3::ZERO,
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        let state = |sim: &Sim| sim.view().appearance.systems(id).unwrap()[0].unwrap();
        assert_eq!(state(&sim).health, [Fixed::from_int(20); 2]);
        assert!(sim.apply_edit(
            PlayerId::SCRIPT,
            &Edit::SubsystemDamage(id, Subsystem::Engines, Fixed::from_int(20))
        ));
        assert_eq!(state(&sim).health[0], Fixed::ZERO);
        assert!(sim.apply_edit(
            PlayerId::SCRIPT,
            &Edit::SubsystemDamage(id, Subsystem::Engines, Fixed::from_int(-5))
        ));
        assert_eq!(state(&sim).health[0], Fixed::from_int(5));
        let restored = Sim::from_snapshot(&sim.snapshot(), Some(setup.content.clone())).unwrap();
        assert_eq!(state(&sim), state(&restored));
        sim.players.push((PlayerId(1), TeamId(1)));
        assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(id, PlayerId(1))));
        assert_eq!(sim.view().unit(id).unwrap().owner, PlayerId(1));
        assert_eq!(sim.view().appearance.faction(PlayerId(1)), None);
        sim.despawn(id, None);
        assert!(sim.view().appearance.systems(id).is_none());
    }

    #[test]
    fn scripted_warp_exposes_live_entry_and_exit_phases() {
        let mut setup = setup("federation");
        let mut def = UnitDef::new("motion_test", "craft");
        def.hull = Fixed::from_int(100);
        def.movement = MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 1.0);
        def.movement.warp_speed = Fixed::from_int(100);
        setup.content = Arc::new(ContentDb::builder().unit(def).build().unwrap());
        let mut sim = Sim::from_setup(&setup);
        let id = sim.spawn_def(
            setup.content.key("motion_test").unwrap(), PlayerId(0), Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
        ).unwrap();
        assert_eq!(sim.impulse_caps(), vec![(id, Fixed::from_int(20))]);
        let goal = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::from_int(1200) };
        sim.step(&[Command { player: PlayerId(0), seq: 0,
            kind: CommandKind::Order(Order::Move { units: vec![id], to: goal, queue: false }),
        }]);
        for _ in 0..10 { sim.step(&[]); }
        assert!(sim.unit(id).unwrap().speed > Fixed::ZERO, "plain move is at impulse");
        assert!(sim.warp_phases().is_empty(), "plain move has no warp phase");
        sim.step(&[Command { player: PlayerId(0), seq: 1,
            kind: CommandKind::Order(Order::MoveWarp { units: vec![id], to: goal, queue: false }),
        }]);
        assert!(sim.warp_phases().is_empty(), "accelerating at impulse before warp entry");
        let mut entered = false;
        let mut exited = false;
        for _ in 0..1500 {
            sim.step(&[]);
            match sim.warp_phases().as_slice() {
                [(_, true)] => entered = true,
                [(_, false)] if entered => { exited = true; break; }
                _ => {}
            }
        }
        assert!(entered, "scripted move entered live warp");
        assert!(exited, "scripted move exposed warp-out while slowing");
    }
}
