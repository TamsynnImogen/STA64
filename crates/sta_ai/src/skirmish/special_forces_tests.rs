//! LUA-28: the skirmish AI leaves special-forces craft alone. Fake content, authored scene.

use std::collections::BTreeSet;
use std::sync::Arc;

use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::Registry;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn units_of(order: &Order) -> &[EntityId] {
    match order {
        Order::Move { units, .. }
        | Order::Attack { units, .. }
        | Order::AttackAny { units }
        | Order::Mine { units, .. }
        | Order::Stop { units }
        | Order::Patrol { units, .. }
        | Order::Special { units, .. } => units,
        _ => &[],
    }
}

/// Run the AI for `minutes`; returns every unit of seat 0 it ordered, with `flag` of the six
/// cruisers flagged first.
fn ordered(flag: bool, minutes: u64) -> (BTreeSet<EntityId>, Vec<EntityId>) {
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    let setup = GameSetup {
        seed: 5,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![seat(0, Control::Ai(super::SKIRMISH.into())), seat(1, Control::Idle)],
        options: GameOptions::default(),
    };
    let db = Arc::clone(&setup.content);
    let mut sim = Sim::from_setup(&setup);
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let mut seq = 0;
    let mut bench = |e: Edit| {
        seq += 1;
        Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(e) }
    };
    let mut batch = Vec::new();
    for (i, s) in setup.map.starts.iter().enumerate() {
        let p = PlayerId(i as u8);
        for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
            let pos = Vec3 { x: s.pos.x, y: s.pos.y, z: s.pos.z + Fixed::from_int(dz) };
            batch.push(bench(Edit::Spawn { odf: db.key(odf).unwrap(), player: p, pos, heading: s.heading }));
        }
        batch.push(bench(Edit::Cargo(p, 0, Fixed::from_int(1000))));
    }
    sim.step(&batch);
    let cruisers: Vec<EntityId> = (0..6)
        .map(|n| sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(0), v(1500 + 40 * n, 1500), ahead).unwrap())
        .collect();
    if flag {
        let edits: Vec<Command> = cruisers
            .iter()
            .enumerate()
            .map(|(n, id)| Command { player: PlayerId::SCRIPT, seq: n as u32 + 1, kind: CommandKind::Edit(Edit::SpecialForces(*id, true)) })
            .collect();
        sim.step(&edits);
        assert!(cruisers.iter().all(|id| sim.view().is_special_forces(*id)), "the view lists the flagged craft");
    }
    let mut ctl = Registry::builtin().create_for_seat(super::SKIRMISH, &setup, PlayerId(0)).unwrap();
    let (mut events, mut seen) = (Vec::<SimEvent>::new(), BTreeSet::new());
    for _ in 0..minutes * 60 * u64::from(TICK_HZ) {
        let view = sim.view();
        let mut out = Vec::new();
        ctl.think(&view, &events, &mut out);
        for c in &out {
            if let CommandKind::Order(o) = &c.kind {
                seen.extend(units_of(o).iter().copied());
            }
        }
        events = sim.step(&out).events;
    }
    (seen, cruisers)
}

#[test]
fn flagged_craft_get_no_orders_from_the_strategic_ai_and_unflagged_ones_do() {
    let (seen, cruisers) = ordered(false, 6);
    assert!(cruisers.iter().any(|c| seen.contains(c)), "control: the AI uses unflagged warships");
    let (seen, cruisers) = ordered(true, 6);
    assert!(cruisers.iter().all(|c| !seen.contains(c)), "no order named a flagged craft: {seen:?}");
}

/// Plan once from a hand-built view: its own yard plus `crafts` (def name, flagged), the AIP
/// list asking for `count` of `want`; returns what the yard was told to build.
fn builds(tree: Option<&str>, want: &str, count: i32, crafts: &[(&str, bool)]) -> Vec<sta_core::ContentKey> {
    use sta_sim::view::{UnitFlags, UnitView, WorldView};
    let db = Arc::new(super::tests::rebuilt_for_flags(tree));
    let key = |n: &str| db.key(n).unwrap();
    let mut cfg = super::SkirmishConfig {
        player: PlayerId(0),
        race: "fx".into(),
        content: Arc::clone(&db),
        home: Vec3::ZERO,
        enemy_starts: Vec::new(),
        nodes: Vec::new(),
        seed: 1,
        params: super::Params::default(),
    };
    cfg.params.build_list = vec![sta_content::BuildItem {
        unit: want.into(),
        count: Fixed::from_int(count),
        base: None,
        max: None,
        tech_level: None,
    }];
    let me = sta_sim::view::PlayerView {
        crew: None,
        id: PlayerId(0),
        team: TeamId(0),
        resources: vec![Fixed::from_int(100_000); 4],
        officers: [Fixed::ZERO; 2],
        eliminated: false,
        relations: Vec::new(),
    };
    let unit = |id: u32, def: &str| UnitView {
        id: EntityId(id),
        def: key(def),
        owner: PlayerId(0),
        pos: Vec3::ZERO,
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2],
        shields: [[Fixed::ZERO; 2]; 4],
        crew: None,
        energy: None,
        autonomy: None,
        flags: UnitFlags::SELECTABLE,
        progress: None,
    };
    let mut units = vec![unit(1, "fx_yard"), unit(2, "fx_station")];
    let mut flagged = Vec::new();
    for (n, (def, on)) in crafts.iter().enumerate() {
        let id = 10 + n as u32;
        units.push(unit(id, def));
        if *on {
            flagged.push(EntityId(id));
        }
    }
    let mut view = WorldView { units, ..WorldView::default() };
    view.attrs.special_forces = flagged;
    let mut ai = super::SkirmishController::new(cfg);
    let mut out = Vec::new();
    ai.plan(&view, &me, &mut out);
    out.iter()
        .filter_map(|c| match &c.kind {
            CommandKind::Order(Order::Build { what, .. }) => Some(*what),
            _ => None,
        })
        .collect()
}

// With the list satisfied the yard still fills the army with the dearest warship (not the
// frigate), so these tests look for the listed class among what it builds.
#[test]
fn build_totals_leave_flagged_craft_out_so_the_ai_builds_replacements() {
    let frigate = sta_content::fake::db().key("fx_frigate").unwrap();
    // Control: two unflagged frigates meet a list asking for two frigates.
    let b = builds(None, "fx_frigate", 2, &[("fx_frigate", false), ("fx_frigate", false)]);
    assert!(!b.contains(&frigate), "{b:?}");
    // One of them flagged: the total is one, so it builds a replacement.
    let b = builds(None, "fx_frigate", 2, &[("fx_frigate", false), ("fx_frigate", true)]);
    assert_eq!(b.first(), Some(&frigate), "{b:?}");
}

#[test]
fn a_flagged_craft_still_counts_as_a_tech_prerequisite() {
    // The cruiser needs a frigate. Prerequisites count flagged craft (the rule names build totals).
    let tree = "fx_frigate 0\nfx_miner 0\nfx_yard 0\nfx_station 0\nfx_cruiser 1 fx_frigate\n";
    let cruiser = sta_content::fake::db().key("fx_cruiser").unwrap();
    let b = builds(Some(tree), "fx_cruiser", 1, &[]);
    assert!(!b.contains(&cruiser), "control: no frigate, no cruiser: {b:?}");
    for flagged in [false, true] {
        let b = builds(Some(tree), "fx_cruiser", 1, &[("fx_frigate", flagged)]);
        assert_eq!(b.first(), Some(&cruiser), "frigate flagged={flagged}: {b:?}");
    }
}
