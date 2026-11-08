//! Done-when for SIM-05 (TOOL-02's bench runner does not exist yet): the
//! `shields` and `explosion` curves of the `firing` fixture on the fake
//! content, against values worked out by hand.
//!
//! The cruiser (beam 20 every 45 ticks from tick 0; torpedo 60, fired every
//! 90 ticks from 0, arriving 27 ticks later) shoots the frigate (hull 300,
//! shields 150 per facing, recharge 0.25 per tick) from astern, so only the
//! aft pool and the hull move. Each tick recharges first, then takes hits.

use crate::command::{Command, CommandKind, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn fixture() -> (Sim, EntityId, EntityId) {
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control: Control::Idle,
    };
    let db = sta_content::fake::db();
    let (cruiser, frigate) = (db.key("fx_cruiser").unwrap(), db.key("fx_frigate").unwrap());
    let setup = GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let p = |x, z| Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    };
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let s = sim
        .spawn_def(cruiser, PlayerId(1), p(2000, 2000), ahead)
        .unwrap();
    let t = sim
        .spawn_def(frigate, PlayerId(2), p(2000, 2350), ahead)
        .unwrap();
    (sim, s, t)
}

/// The hand-worked model in quarter units: `(aft shield, hull)` after each
/// tick, and the tick the hull reaches zero.
fn model(ticks: u64) -> (Vec<(i64, i64)>, Option<u64>) {
    let (mut shield, mut hull) = (150 * 4, 300 * 4);
    let (mut out, mut dead) = (Vec::new(), None);
    for t in 0..ticks {
        shield = (shield + 1).min(600);
        let dmg = if t % 45 == 0 { 80 } else { 0 }
            + if t >= 27 && (t - 27) % 90 == 0 {
                240
            } else {
                0
            };
        let s = dmg.min(shield);
        shield -= s;
        hull = (hull - (dmg - s)).max(0);
        out.push((shield, hull));
        if hull == 0 && dead.is_none() {
            dead = Some(t);
            break;
        }
    }
    (out, dead)
}

fn q(f: Fixed) -> i64 {
    (f.to_f32() * 4.0).round() as i64
}

#[test]
fn shield_and_hull_curves_match_hand_values() {
    let (mut sim, s, t) = fixture();
    let (want, dead_at) = model(2000);
    let dead_at = dead_at.expect("the frigate dies in the run");
    let mut hits = Vec::new();
    let mut destroyed = None;
    let mut downs = Vec::new();
    for tick in 0..=dead_at + 5 {
        let cmds = if tick == 0 {
            vec![Command {
                player: PlayerId(1),
                seq: 0,
                kind: CommandKind::Order(Order::Attack {
                    units: vec![s],
                    target: t,
                }),
            }]
        } else {
            vec![]
        };
        let out = sim.step(&cmds);
        for e in &out.events {
            match e {
                SimEvent::Hit {
                    target,
                    by,
                    shield,
                    hull,
                    ..
                } if *target == t && *by == s => hits.push((tick, q(*shield), q(*hull))),
                SimEvent::ShieldsDown { id, facing } if *id == t => downs.push((tick, *facing)),
                SimEvent::Destroyed { id, by } if *id == t => destroyed = Some((tick, *by)),
                _ => {}
            }
        }
        if tick <= dead_at {
            let u = sim.unit(t);
            let (sh, hu) = match u {
                Some(u) => (q(u.shields.now[1]), q(u.hull[0])),
                None => (0, 0),
            };
            assert_eq!((sh, hu), want[tick as usize], "tick {tick}");
            // The other pools never move: they only recharge to full.
            if let Some(u) = u {
                assert_eq!(
                    [
                        q(u.shields.now[0]),
                        q(u.shields.now[2]),
                        q(u.shields.now[3])
                    ],
                    [600; 3]
                );
            }
        }
    }
    // Hand checks: tick 0 beam 20 -> 130; tick 27 torpedo -> 76.75; tick 45 beam -> 61.25.
    assert_eq!(want[0].0, 130 * 4);
    assert_eq!(want[27].0, 307);
    assert_eq!(want[45].0, 245);
    // The first hit is all shield; a Hit later splits across shield and hull.
    assert_eq!(hits[0], (0, 80, 0));
    assert!(
        hits.iter().any(|&(_, s, h)| s > 0 && h > 0),
        "a hit overflows the shield"
    );
    assert_eq!(downs.first().map(|d| d.1), Some(1));
    // Destroyed with the cruiser as killer, on the tick of the lethal hit,
    // and the entity is gone.
    assert_eq!(destroyed, Some((dead_at, Some(s))));
    assert!(sim.unit(t).is_none());
    // Hit damage adds up to the weapons' output: shield + hull never exceeds it.
    assert!(hits.iter().all(|&(_, s, h)| s + h <= 240));
}

#[test]
fn damage_run_digest_is_pinned() {
    let (mut sim, s, t) = fixture();
    for tick in 0..600 {
        let cmds = if tick == 0 {
            vec![Command {
                player: PlayerId(1),
                seq: 0,
                kind: CommandKind::Order(Order::Attack {
                    units: vec![s],
                    target: t,
                }),
            }]
        } else {
            vec![]
        };
        sim.step(&cmds);
    }
    assert_eq!(sim.digest(), 0xbaaa_8dfc_68d0_4cbe);
}
