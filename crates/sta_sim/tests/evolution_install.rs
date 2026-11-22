//! Install-gated regression for both Species 8472 evolution paths.
//!
//! Set `STA64_TEST_EVOLUTION_INSTALL` to an Armada II install. `STA64_TEST_A2_MOD`
//! selects a mod under `Data/Mods` (default: `STA2 Classic`).

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, OdfIndex};
use sta_core::{Fixed, PlayerId, TICK_HZ, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::EntityKind;
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

fn n(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn installed_db() -> Option<Arc<ContentDb>> {
    let root = std::env::var("STA64_TEST_EVOLUTION_INSTALL").ok()?;
    let active_mod = std::env::var("STA64_TEST_A2_MOD").unwrap_or_else(|_| "STA2 Classic".into());
    let active_mod = (!active_mod.is_empty()).then_some(active_mod);
    let vfs = Vfs::mount_install(Path::new(&root), active_mod.as_deref())
        .unwrap_or_else(|e| panic!("STA64_TEST_EVOLUTION_INSTALL {root:?}: {e}"));
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).expect("load installed ODF content");
    Some(Arc::new(db))
}

fn game(db: Arc<ContentDb>, embryo_odf: &str, embryo_at: Vec3) -> (Sim, sta_core::EntityId) {
    let map = MapDef {
        objects: vec![MapObject {
            odf: embryo_odf.into(),
            owner: Some(0),
            pos: embryo_at,
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        }],
        ..MapDef::empty()
    };
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "species8472".into(),
        control: Control::Human,
    };
    let sim = Sim::from_setup(&GameSetup {
        seed: 17,
        map: Arc::new(map),
        content: db,
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let embryo = sim
        .view()
        .units
        .iter()
        .find(|u| u.owner == PlayerId(0))
        .expect("placed embryo")
        .id;
    (sim, embryo)
}

fn run_evolution(
    db: Arc<ContentDb>,
    embryo_odf: &str,
    product_odf: &str,
    site: Option<Vec3>,
    authored_cost: i32,
    authored_seconds: i32,
) {
    let embryo_def = db.unit(db.key(embryo_odf).expect("embryo ODF")).unwrap();
    let product_def = db.unit(db.key(product_odf).expect("product ODF")).unwrap();
    assert!(
        embryo_def.is_evolver(),
        "{embryo_odf} must resolve as an evolver"
    );
    assert!(
        embryo_def
            .build
            .list
            .iter()
            .any(|item| item.key == product_def.key),
        "{embryo_odf} must author {product_odf} in its build list"
    );
    assert_eq!(
        product_def.build.cost.get(3),
        Some(&n(authored_cost)),
        "{product_odf} authored biomatter cost"
    );
    assert_eq!(
        product_def.build.ticks,
        u64::from(authored_seconds as u32 * TICK_HZ)
    );
    assert_eq!(
        EntityKind::build_requires_site(embryo_def, product_def),
        site.is_some()
    );

    let embryo_at = Vec3 {
        x: n(300),
        y: Fixed::ZERO,
        z: n(300),
    };
    let product_at = site.unwrap_or(embryo_at);
    let (mut sim, embryo) = game(db.clone(), embryo_odf, embryo_at);

    // Fund via a bench command; construction itself is admitted and advanced
    // through the same player Order path used by a game session.
    let initial_stock = authored_cost + 100;
    sim.step(&[Command {
        player: PlayerId::BENCH,
        seq: 0,
        kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 3, n(initial_stock))),
    }]);
    let before = sim.view().player(PlayerId(0)).unwrap().resources[3];
    let what = product_def.key;
    let start = sim.step(&[Command {
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::Order(Order::Build {
            builder: embryo,
            what,
            at: site,
        }),
    }]);
    assert!(
        !start
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. })),
        "{embryo_odf} -> {product_odf} build was rejected: {:?}",
        start.events
    );
    let starts = |events: &[SimEvent]| {
        events.iter().any(|event| matches!(
        event, SimEvent::BuildStarted { builder, what: item } if *builder == embryo && *item == what
    ))
    };
    if !starts(&start.events) {
        // A site away from the embryo needs travel before the cocoon starts.
        let mut reached = false;
        for _ in 0..(TICK_HZ * 60) {
            let out = sim.step(&[]);
            assert_eq!(
                out.view.units.len(),
                1,
                "product appeared before the cocoon"
            );
            if starts(&out.events) {
                reached = true;
                break;
            }
        }
        assert!(reached, "embryo never started evolving at the chosen site");
    }
    assert_eq!(
        sim.view().player(PlayerId(0)).unwrap().resources[3],
        before - n(authored_cost)
    );
    assert!(
        sim.unit(embryo).is_some(),
        "embryo must remain during construction"
    );
    assert_eq!(
        sim.view().units.len(),
        1,
        "product appeared before construction completed"
    );

    let mut replacements = Vec::new();
    for tick in 0..product_def.build.ticks {
        let out = sim.step(&[]);
        for event in out.events {
            if let SimEvent::Replaced {
                old,
                new,
                select: true,
            } = event
            {
                assert_eq!(old, embryo);
                replacements.push(new);
            }
        }
        if tick + 1 < product_def.build.ticks {
            assert!(
                sim.unit(embryo).is_some(),
                "embryo removed before authored build time elapsed"
            );
            assert_eq!(
                sim.view().units.len(),
                1,
                "product appeared before authored build time elapsed"
            );
        }
    }
    assert_eq!(replacements.len(), 1, "expected exactly one Replaced event");
    let product = sim
        .unit(replacements[0])
        .expect("replacement product is live");
    assert_eq!(product.def, what);
    assert_eq!(product.pos, product_at);
    assert!(
        sim.unit(embryo).is_none(),
        "replaced embryo must be removed"
    );
    assert_eq!(sim.view().units.len(), 1);
}

#[test]
fn installed_species_8472_evolution_uses_authored_builds_and_replaces_embryos() {
    let Some(db) = installed_db() else {
        eprintln!("STA64_TEST_EVOLUTION_INSTALL unset: installed evolution regression skipped");
        return;
    };

    // ODF evidence: active is 13 s / 40 biomatter; scout is 15 s / 55.
    run_evolution(db.clone(), "8472_active", "8472_scout", None, 55, 15);

    // ODF evidence: passive is 12 s and names the Fluidic Gate; its included
    // station definition is a starbase, and the gate costs 3000 biomatter over 180 s.
    let site = Vec3 {
        x: n(650),
        y: Fixed::ZERO,
        z: n(300),
    };
    run_evolution(
        db,
        "8472_passive",
        "8472_fluidicgate",
        Some(site),
        3000,
        180,
    );
}
