//! Public production-lifecycle checks for map-confinement state.

use std::sync::Arc;

use sta_content::{ConfinementOverrides, ContentDb, DefRef, MapDef, MoveStats, Profile, UnitDef};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order, Target};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn p(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: n(x),
        y: Fixed::ZERO,
        z: n(z),
    }
}

fn mobile() -> MoveStats {
    MoveStats::from_f32(120.0, 2.0, 2.0, 2.0, 1.0)
}

fn unit(name: &str, class: &str) -> UnitDef {
    let mut u = UnitDef::new(name, class);
    u.hull = n(100);
    u.build.ticks = 3;
    u
}

fn sim(profile: Profile, defs: Vec<UnitDef>) -> Sim {
    let mut builder = ContentDb::builder().profile(profile);
    for def in defs {
        builder = builder.unit(def);
    }
    let db = Arc::new(builder.build().unwrap());
    let mut map = MapDef::empty();
    map.bounds = (p(-500, -500), p(500, 500));
    Sim::from_setup(&GameSetup {
        seed: 83,
        map: Arc::new(map),
        content: db,
        players: [0, 1]
            .into_iter()
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "synthetic".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    })
}

fn key(sim: &Sim, name: &str) -> ContentKey {
    sim.content().unwrap().key(name).unwrap()
}

fn spawn(sim: &mut Sim, name: &str, pos: Vec3) -> EntityId {
    let k = key(sim, name);
    sim.spawn_def(k, PlayerId(0), pos, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

fn order(seq: u32, value: Order) -> Command {
    Command {
        player: PlayerId(0),
        seq,
        kind: CommandKind::Order(value),
    }
}

fn set(seq: u32, id: EntityId, enabled: bool) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::ForceOntoMap { id, enabled }),
    }
}

fn accepted(events: &[SimEvent]) {
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

fn built(events: &[SimEvent]) -> Option<EntityId> {
    events.iter().find_map(|event| match event {
        SimEvent::Built { id, .. } => Some(*id),
        _ => None,
    })
}

fn wait_built(sim: &mut Sim, steps: usize) -> EntityId {
    for _ in 0..steps {
        if let Some(id) = built(&sim.step(&[]).events) {
            return id;
        }
    }
    panic!("production did not complete in {steps} steps");
}

#[test]
fn yard_birth_uses_product_flag_for_shared_raw_rally() {
    let mut yard = unit("yard", "shipyard");
    yard.build.list = vec![DefRef::new("enabled"), DefRef::new("disabled")];
    let mut enabled = unit("enabled", "craft");
    enabled.movement = mobile();
    let mut disabled = unit("disabled", "craft");
    disabled.movement = mobile();
    disabled.confinement = ConfinementOverrides {
        capability: Some(false),
        ..Default::default()
    };
    let mut sim = sim(Profile::Fo4, vec![yard, enabled, disabled]);
    let yard = spawn(&mut sim, "yard", p(0, 0));
    let rally = p(467, 0);
    accepted(
        &sim.step(&[order(
            1,
            Order::Rally {
                builders: vec![yard],
                at: Target::Point(rally),
            },
        )])
        .events,
    );

    let enabled_key = key(&sim, "enabled");
    accepted(
        &sim.step(&[order(
            2,
            Order::Build {
                builder: yard,
                what: enabled_key,
                at: None,
            },
        )])
        .events,
    );
    let first = wait_built(&mut sim, 8);
    assert_eq!(sim.unit(first).unwrap().def, key(&sim, "enabled"));
    assert_eq!(sim.force_onto_map(first), Some(true));
    assert_eq!(
        sim.unit(first).unwrap().target.unwrap().x,
        n(400) - Fixed::from_bits(429496736)
    );

    let disabled_key = key(&sim, "disabled");
    accepted(
        &sim.step(&[order(
            3,
            Order::Build {
                builder: yard,
                what: disabled_key,
                at: None,
            },
        )])
        .events,
    );
    let second = wait_built(&mut sim, 8);
    assert_eq!(sim.unit(second).unwrap().def, key(&sim, "disabled"));
    assert_eq!(sim.force_onto_map(second), None);
    assert_eq!(sim.unit(second).unwrap().target, Some(rally));
    assert_eq!(
        sim.unit(first).unwrap().owner,
        sim.unit(second).unwrap().owner
    );
}

#[test]
fn a1_terrain_rig_weld_uses_plain_placeholder_then_resets_finished_flag() {
    let mut rig = unit("rig", "constructionrig");
    rig.movement = mobile();
    rig.build.list = vec![DefRef::new("terrain")];
    let terrain = unit("terrain", "nebula_obj");
    let mut sim = sim(Profile::Sta1Classic, vec![rig, terrain]);
    let rig = spawn(&mut sim, "rig", p(440, 0));
    let site = p(450, 0); // inside raw XZ, outside A1's padded XZ
    let terrain_key = key(&sim, "terrain");
    let out = sim.step(&[order(
        1,
        Order::Build {
            builder: rig,
            what: terrain_key,
            at: Some(site),
        },
    )]);
    accepted(&out.events);
    let placeholder = EntityId(rig.0 + 1);
    assert_eq!(sim.unit(placeholder).unwrap().progress, Some(Fixed::ZERO));
    assert_eq!(sim.force_onto_map(placeholder), Some(false));
    assert_eq!(sim.unit(placeholder).unwrap().pos, site);
    let completed = wait_built(&mut sim, 6);
    assert_eq!(completed, placeholder, "weld must retain placeholder ID");
    assert_eq!(sim.unit(completed).unwrap().progress, None);
    assert_eq!(sim.force_onto_map(completed), Some(true));
    assert_eq!(
        sim.unit(completed).unwrap().pos,
        site,
        "terrain bypasses ordinary projection"
    );
}

#[test]
fn a2_craft_rig_weld_resets_explicit_false_at_completion() {
    let mut rig = unit("rig", "constructionrig");
    rig.movement = mobile();
    rig.build.list = vec![DefRef::new("product")];
    let mut product = unit("product", "craft");
    product.movement = mobile();
    let mut sim = sim(Profile::Fo4, vec![rig, product]);
    let rig = spawn(&mut sim, "rig", p(0, 0));
    let product_key = key(&sim, "product");
    let out = sim.step(&[order(
        1,
        Order::Build {
            builder: rig,
            what: product_key,
            at: Some(p(10, 0)),
        },
    )]);
    accepted(&out.events);
    let placeholder = EntityId(rig.0 + 1);
    assert_eq!(sim.unit(placeholder).unwrap().progress, Some(Fixed::ZERO));
    assert_eq!(sim.force_onto_map(placeholder), Some(true));
    accepted(&sim.step(&[set(1, placeholder, false)]).events);
    assert_eq!(sim.force_onto_map(placeholder), Some(false));
    let completed = wait_built(&mut sim, 6);
    assert_eq!(completed, placeholder);
    assert_eq!(sim.unit(completed).unwrap().progress, None);
    assert_eq!(sim.force_onto_map(completed), Some(true));
    assert_eq!(sim.unit(completed).unwrap().owner, PlayerId(0));
}
