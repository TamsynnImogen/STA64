//! CON-51: a planet's node follows its data (`amt<slot>`, `trickleAmount`,
//! `trickleMiningRateFraction`, `infinite`) for a miner too. Authored numbers.

use super::tests::{game_with_content, mine, spawn, v};
use super::*;
use crate::world::NEUTRAL;
use crate::WorldSource;
use sta_content::{NodeIncome, UnitDef};

fn world(income: Option<NodeIncome>) -> sta_content::ContentDb {
    let base = sta_content::fake::db();
    let mut builder = sta_content::ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for unit in base.units() {
        builder = builder.unit(unit.clone());
    }
    for weapon in base.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    let mut planet = UnitDef::new("fx_world", "planet");
    planet.resource = Some(0);
    planet.node_income = income;
    builder.unit(planet).build().unwrap()
}

fn amounts(dilithium: i32) -> [Option<Fixed>; RESOURCES] {
    [Some(Fixed::from_int(dilithium)), None, None, None]
}

fn left(sim: &crate::Sim, node: EntityId) -> Option<Fixed> {
    sim.economy.nodes.get(node).copied()
}

fn run_mining(income: Option<NodeIncome>, ticks: u32) -> (crate::Sim, EntityId) {
    let mut sim = game_with_content(world(income));
    let node = spawn(&mut sim, "fx_world", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, node));
    for _ in 0..ticks {
        sim.step(&[]);
    }
    (sim, node)
}

#[test]
fn a_planet_node_starts_with_its_data_amount_and_runs_dry() {
    let income = NodeIncome { amounts: amounts(200), ..NodeIncome::default() };
    let (sim, node) = run_mining(Some(income), 3000);
    assert_eq!(left(&sim, node), Some(Fixed::ZERO));
    let delivered = sim.view().players.iter().find(|p| p.id == PlayerId(0)).unwrap().resources[0];
    assert_eq!(delivered, Fixed::from_int(200));
}

#[test]
fn a_planet_without_data_holds_the_fixed_default() {
    let (sim, node) = run_mining(None, 300);
    assert!(left(&sim, node).unwrap() > Fixed::from_int(RICHNESS - 1000));
}

#[test]
fn a_node_at_its_trickle_amount_holds_there_and_still_yields() {
    let income = NodeIncome {
        amounts: amounts(200),
        trickle_amount: Some(Fixed::from_int(80)),
        trickle_fraction: Some(Fixed::from_f64(0.5)),
        ..NodeIncome::default()
    };
    let (sim, node) = run_mining(Some(income), 6000);
    assert_eq!(left(&sim, node), Some(Fixed::from_int(80)));
    let got = sim.view().players.iter().find(|p| p.id == PlayerId(0)).unwrap().resources[0];
    assert!(got > Fixed::from_int(120), "{got:?}");
}

#[test]
fn an_infinite_node_never_falls() {
    let income = NodeIncome { amounts: amounts(200), infinite: Some(true), ..NodeIncome::default() };
    let (sim, node) = run_mining(Some(income), 3000);
    assert!(left(&sim, node).is_none_or(|l| l == Fixed::from_int(200)));
    let got = sim.view().players.iter().find(|p| p.id == PlayerId(0)).unwrap().resources[0];
    assert!(got > Fixed::from_int(200));
}
