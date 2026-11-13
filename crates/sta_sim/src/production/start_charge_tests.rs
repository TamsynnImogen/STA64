//! SIM-33: a queued item is charged when it becomes the head and starts,
//! never at enqueue (measured on stock A2 1.1 and Roots; evidence grade
//! "measured"). Rule data: the builder's resolved `pay_on_queue`
//! (`queueInstantDeduct`). Implements D5, D9.
//!
//! Not covered here: TOOL-16 parity of the M3 scene (TOOL-16 is not merged).
use super::*;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, Features, Profile};
use sta_core::TeamId;
use std::sync::Arc;

const P0: PlayerId = PlayerId(0);
/// `fx_frigate`: cost 250 dilithium, 450 build ticks.
const COST: i32 = 250;
const TICKS: u64 = 450;

/// The fake content under `profile`'s default switches; the yard's own
/// `queueInstantDeduct` (resolved into its `Features`) as asked.
fn game(profile: Profile, yard_pays_on_queue: Option<bool>) -> Sim {
    let fake = sta_content::fake::db();
    let features: Features = profile.features();
    let mut b = ContentDb::builder().profile(profile).features(features.clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        if u.odf == "fx_yard"
            && let Some(on) = yard_pays_on_queue
        {
            let mut own = features.clone();
            own.pay_on_queue = on;
            u.features = Some(own);
        }
        b = b.unit(u);
    }
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(b.build().unwrap()),
        players: vec![PlayerSetup {
            id: P0,
            team: TeamId(0),
            race: "fx".into(),
            control: Control::Human,
        }],
        options: GameOptions::default(),
    })
}

fn cmd(player: PlayerId, kind: CommandKind) -> Command {
    Command {
        player,
        seq: 0,
        kind,
    }
}

fn stock(sim: &Sim) -> Fixed {
    sim.economy.stock_of(P0).first().copied().unwrap_or(Fixed::ZERO)
}

/// Three frigates queued at once on a funded yard. Returns the stock after
/// the enqueue step and, per tick after it, `(tick, stock, started?)`.
fn three_queued(sim: &mut Sim, ticks: u64) -> (Fixed, Vec<(u64, Fixed, bool)>) {
    let def = sim.content().unwrap().key("fx_yard").unwrap();
    let id = sim.units.next_id();
    let at = Vec3 {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    };
    sim.step(&[cmd(
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Spawn {
            odf: def,
            player: P0,
            pos: at,
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    )]);
    sim.step(&[cmd(
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Cargo(P0, 0, Fixed::from_int(3 * COST))),
    )]);
    let what = sim.content().unwrap().key("fx_frigate").unwrap();
    let orders: Vec<Command> = (0..3)
        .map(|_| {
            cmd(
                P0,
                CommandKind::Order(Order::Build {
                    builder: id,
                    what,
                    at: None,
                }),
            )
        })
        .collect();
    let out = sim.step(&orders);
    assert!(!out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert_eq!(sim.production.queue(id).len(), 3);
    let after_enqueue = stock(sim);
    let mut log = vec![];
    for _ in 0..ticks {
        let t = sim.tick;
        let out = sim.step(&[]);
        let started = out
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::BuildStarted { .. }));
        log.push((t, stock(sim), started));
    }
    (after_enqueue, log)
}

#[test]
fn queued_items_are_charged_one_by_one_when_they_start_not_at_enqueue() {
    for profile in [Profile::Sta2Classic, Profile::Fo4] {
        let mut sim = game(profile, None);
        assert!(!sim.content().unwrap().features().pay_on_queue, "{profile:?}");
        let (after_enqueue, log) = three_queued(&mut sim, 2 * TICKS + 20);
        // Measured: the first item is charged at the order (the queue was
        // empty, so it starts in that step); the other two are not.
        assert_eq!(
            after_enqueue,
            Fixed::from_int(2 * COST),
            "{profile:?}: only the head is charged at enqueue"
        );
        let starts: Vec<&(u64, Fixed, bool)> = log.iter().filter(|r| r.2).collect();
        assert_eq!(starts.len(), 2, "{profile:?}: items 2 and 3 start later");
        // Item 2 starts when item 1 finishes, item 3 a build time later, and
        // each start takes exactly one cost off the bank.
        assert_eq!(starts[1].0 - starts[0].0, TICKS, "{profile:?}");
        assert_eq!(starts[0].1, Fixed::from_int(COST), "{profile:?}");
        assert_eq!(starts[1].1, Fixed::ZERO, "{profile:?}");
        for (t, s, started) in &log {
            let expect = if *t < starts[0].0 {
                2 * COST
            } else if *t < starts[1].0 {
                COST
            } else {
                0
            };
            assert_eq!(*s, Fixed::from_int(expect), "{profile:?} tick {t} {started}");
        }
    }
}

#[test]
fn a_producer_with_queue_instant_deduct_and_the_native_profile_charge_at_enqueue() {
    // `queueInstantDeduct = 1` resolves into the producer's own `Features`.
    let mut sim = game(Profile::Fo4, Some(true));
    let (after_enqueue, log) = three_queued(&mut sim, 2 * TICKS + 20);
    assert_eq!(after_enqueue, Fixed::ZERO);
    assert!(log.iter().all(|r| r.1 == Fixed::ZERO));
    // The same switch turned off on a Native yard defers the charges again.
    let mut sim = game(Profile::Native, Some(false));
    let (after_enqueue, _) = three_queued(&mut sim, 1);
    assert_eq!(after_enqueue, Fixed::from_int(2 * COST));
    // Native's own default is to charge at enqueue.
    let mut sim = game(Profile::Native, None);
    assert!(sim.content().unwrap().features().pay_on_queue);
    let (after_enqueue, _) = three_queued(&mut sim, 1);
    assert_eq!(after_enqueue, Fixed::ZERO);
}
