//! Immutable cargo presentation derived from live economy records.

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed};

use crate::world::{Entities, EntityKind};

use super::{Economy, cfg_of};

/// A freighter's carried amount and resolved hold capacity.
/// `resource` is known only while actual cargo is aboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CargoView {
    pub entity: EntityId,
    pub amount: Fixed,
    pub capacity: Fixed,
    pub resource: Option<u8>,
}

/// A resource node's amount left and what it started with (UI-48: the HUD's dilithium and
/// planet-metal bars). `start` is the amount a fresh node holds; an infinite node is never
/// reduced, so its amount stays at `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeView {
    pub entity: EntityId,
    pub resource: u8,
    pub amount: Fixed,
    pub start: Fixed,
    pub infinite: bool,
}

pub(super) fn node_rows(economy: &Economy, db: &ContentDb, units: &Entities) -> Vec<NodeView> {
    units
        .iter()
        .filter(|u| db.unit(u.def).is_some_and(EntityKind::mineable_def))
        .map(|u| {
            let start = super::start_amount(db, u.def);
            NodeView {
                entity: u.id,
                resource: super::node_resource(db, u.def),
                amount: economy.nodes.get(u.id).copied().unwrap_or(start),
                start,
                infinite: super::is_infinite(db, u.def),
            }
        })
        .collect()
}

pub(super) fn rows(economy: &Economy, db: &ContentDb, units: &Entities) -> Vec<CargoView> {
    units
        .iter()
        .filter(|unit| unit.kind == EntityKind::Craft)
        .filter_map(|unit| {
            if let Some(cargo) = economy.trade_cargo(db, unit) {
                return Some(cargo);
            }
            let cfg = cfg_of(db, unit.def)?;
            let miner = economy.miners.get(unit.id);
            let amount = miner.map_or(Fixed::ZERO, |m| m.cargo);
            Some(CargoView {
                entity: unit.id,
                amount,
                capacity: cfg.capacity,
                resource: miner.filter(|_| amount > Fixed::ZERO).map(|m| m.res),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::MapDef;
    use sta_core::{PlayerId, TeamId, Vec3};

    use crate::command::{Command, CommandKind, Edit, Order};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use crate::world::NEUTRAL;
    use crate::{Sim, WorldSource};

    use super::*;

    fn game() -> Sim {
        let base = sta_content::fake::db();
        let mut builder = ContentDb::builder()
            .profile(base.profile())
            .features(base.features().clone());
        for weapon in base.weapons() {
            builder = builder.weapon(weapon.clone());
        }
        for unit in base.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_miner" {
                unit.max_dilithium = Fixed::from_int(90);
            }
            builder = builder.unit(unit);
        }
        let db = Arc::new(builder.build().unwrap());
        let seat = |id| PlayerSetup {
            id: PlayerId(id),
            team: TeamId(id),
            race: "fx".into(),
            control: Control::Human,
        };
        Sim::from_setup(&GameSetup {
            seed: 3,
            map: Arc::new(MapDef::empty()),
            content: db,
            players: vec![seat(0), seat(1)],
            options: GameOptions::default(),
        })
    }

    fn step(sim: &mut Sim, player: PlayerId, kind: CommandKind) {
        sim.step(&[Command {
            player,
            seq: 0,
            kind,
        }]);
    }

    fn spawn_at(sim: &mut Sim, odf: &str, owner: PlayerId, pos: Vec3) -> EntityId {
        let id = sim
            .view()
            .units
            .last()
            .map_or(EntityId(1), |u| EntityId(u.id.0 + 1));
        let def = sim.content().unwrap().key(odf).unwrap();
        step(
            sim,
            PlayerId::BENCH,
            CommandKind::Edit(Edit::Spawn {
                odf: def,
                player: owner,
                pos,
                heading: (Fixed::ZERO, Fixed::ONE),
            }),
        );
        assert!(sim.unit(id).is_some());
        id
    }

    fn spawn(sim: &mut Sim, odf: &str, owner: PlayerId) -> EntityId {
        spawn_at(sim, odf, owner, Vec3::ZERO)
    }

    fn read(sim: &Sim) -> Vec<CargoView> {
        let digest = sim.digest();
        let snapshot = sim.snapshot();
        let rows = sim.cargo_view();
        assert_eq!(sim.digest(), digest);
        assert_eq!(sim.snapshot(), snapshot);
        rows
    }

    #[test]
    fn live_cargo_view_tracks_idle_partial_full_transfer_unload_and_loss() {
        let mut sim = game();
        let moon = spawn(&mut sim, "fx_moon", NEUTRAL);
        let miner = spawn(&mut sim, "fx_miner", PlayerId(0));
        // Keep the second owner's craft outside the first owner's cruiser's
        // weapons; otherwise combat can eliminate that seat before transfer.
        let other = spawn_at(
            &mut sim,
            "fx_miner",
            PlayerId(1),
            Vec3::from_f32([10_000.0, 0.0, 0.0]),
        );
        spawn(&mut sim, "fx_cruiser", PlayerId(0));
        let empty = read(&sim);
        assert_eq!(
            empty.iter().map(|row| row.entity).collect::<Vec<_>>(),
            vec![miner, other]
        );
        assert!(empty.iter().all(|row| row.amount == Fixed::ZERO
            && row.capacity == Fixed::from_int(90)
            && row.resource.is_none()));

        step(
            &mut sim,
            PlayerId(0),
            CommandKind::Order(Order::Mine {
                units: vec![miner],
                node: moon,
            }),
        );
        for _ in 0..10 {
            sim.step(&[]);
        }
        let partial = read(&sim)[0];
        assert!(partial.amount > Fixed::ZERO && partial.amount < partial.capacity);
        assert_eq!(partial.resource, Some(0));
        for _ in 0..400 {
            sim.step(&[]);
        }
        let full = read(&sim)[0];
        assert_eq!(full.amount, Fixed::from_int(90));
        assert_eq!(full.capacity, Fixed::from_int(90));
        assert_eq!(full.resource, Some(0));
        assert!(sim.unit(other).is_some());
        assert!(!sim.is_eliminated(PlayerId(1)));

        let transfer = sim.step(&[Command {
            player: PlayerId::BENCH,
            seq: 0,
            kind: CommandKind::Edit(Edit::Team(miner, PlayerId(1))),
        }]);
        assert!(
            !transfer
                .events
                .iter()
                .any(|event| matches!(event, crate::SimEvent::Rejected { .. }))
        );
        assert_eq!(sim.unit(miner).unwrap().owner, PlayerId(1));
        assert_eq!(read(&sim)[0].amount, full.amount);
        spawn(&mut sim, "fx_station", PlayerId(1));
        step(
            &mut sim,
            PlayerId(1),
            CommandKind::Order(Order::Mine {
                units: vec![miner],
                node: moon,
            }),
        );
        let mut saw_unloading = false;
        for _ in 0..300 {
            sim.step(&[]);
            let now = read(&sim)[0];
            if now.amount < full.amount && now.amount > Fixed::ZERO {
                saw_unloading = true;
            }
            if saw_unloading && now.amount == Fixed::ZERO {
                assert_eq!(now.resource, None);
                break;
            }
        }
        assert!(saw_unloading);
        assert_eq!(read(&sim)[0].amount, Fixed::ZERO);
        step(
            &mut sim,
            PlayerId::BENCH,
            CommandKind::Edit(Edit::Kill(miner)),
        );
        assert_eq!(
            read(&sim).iter().map(|row| row.entity).collect::<Vec<_>>(),
            vec![other]
        );
    }

    #[test]
    fn scripted_source_has_no_authoritative_cargo() {
        let db = sta_content::fake::db();
        let source = crate::fake::ScriptedSource::from_json5("{units:[]}", &db).unwrap();
        assert!(source.cargo_view().is_empty());
    }

    /// UI-48: a node's view starts at its start amount and falls as it is mined.
    #[test]
    fn the_node_view_reports_what_is_left_of_each_node() {
        let db = std::sync::Arc::new(sta_content::fake::db());
        let mut sim = crate::Sim::from_setup(&crate::setup::GameSetup {
            seed: 1,
            map: std::sync::Arc::new(sta_content::MapDef::empty()),
            content: db.clone(),
            players: vec![crate::setup::PlayerSetup { id: sta_core::PlayerId(0), team: sta_core::TeamId(0), race: "fx".into(), control: crate::setup::Control::Idle }],
            options: Default::default(),
        });
        let moon = sim.spawn_def(db.key("fx_moon").unwrap(), crate::world::NEUTRAL, sta_core::Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
        let rows = sim.node_view();
        let row = rows.iter().find(|r| r.entity == moon).expect("the moon is a node");
        assert_eq!(row.amount, row.start);
        assert!(row.start > Fixed::ZERO);
        let miner = sim.spawn_def(db.key("fx_miner").unwrap(), sta_core::PlayerId(0), sta_core::Vec3 { x: Fixed::from_int(100), y: Fixed::ZERO, z: Fixed::ZERO }, (Fixed::ZERO, Fixed::ONE)).unwrap();
        let order = crate::command::Command { player: sta_core::PlayerId(0), seq: 0, kind: crate::command::CommandKind::Order(crate::command::Order::Mine { units: vec![miner], node: moon }) };
        crate::WorldSource::step(&mut sim, &[order]);
        for _ in 0..3000 {
            crate::WorldSource::step(&mut sim, &[]);
        }
        let left = sim.node_view().into_iter().find(|r| r.entity == moon).unwrap();
        assert!(left.amount < left.start, "{left:?}");
    }
}
