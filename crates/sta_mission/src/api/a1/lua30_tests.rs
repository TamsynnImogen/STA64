//! LUA-30 tests: the `Destroy<System>` calls, `Attack(unit, Vector)`,
//! `ReplaceObject` and `GetAttackLocation` of the `A1` table.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order, Subsystem};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

use super::TOKEN_BASE;
use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn runtime() -> (MissionRuntime, [EntityId; 2], Arc<ContentDb>) {
    let db = Arc::new(fake::db());
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let spawn = |seq, player, x: f32| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key("fx_cruiser").expect("fake odf"),
            player: PlayerId(player),
            pos: Vec3::from_f32([x, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let out = sim.step(&[spawn(1, 0, 0.0), spawn(2, 1, 100.0)]);
    let at = |x: f32| out.view.units.iter().find(|u| u.pos == Vec3::from_f32([x, 0.0, 0.0])).unwrap().id;
    let ids = [at(0.0), at(100.0)];
    let cfg = MissionConfig { content: Some(db.clone()), view: Some((*out.view).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("a1.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!("a, b = {}, {}", ids[0].0, ids[1].0)).unwrap();
    rt.take_commands();
    (rt, ids, db)
}

fn edits(commands: &[Command]) -> Vec<Edit> {
    commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Edit(e) => {
                assert_eq!(c.player, PlayerId::SCRIPT);
                e.clone()
            }
            other => panic!("not an edit: {other:?}"),
        })
        .collect()
}

#[test]
fn destroy_system_queues_one_lethal_typed_hit() {
    let hit = Fixed::from_f64(100_000.0);
    let cases = [
        ("DestroyEngines", Subsystem::Engines),
        ("DestroyLifeSupport", Subsystem::LifeSupport),
        ("DestroyWeapons", Subsystem::Weapons),
        ("DestroyShieldGenerator", Subsystem::Shields),
        ("DestroySensors", Subsystem::Sensors),
    ];
    for (name, sub) in cases {
        let (mut rt, ids, _) = runtime();
        rt.eval::<()>(&format!("A1.{name}(b)")).unwrap();
        assert_eq!(edits(&rt.take_commands()), vec![Edit::SubsystemDamage(ids[1], sub, hit)], "{name}");
    }
}

#[test]
fn destroy_critical_is_a_lethal_hull_hit() {
    let (mut rt, ids, _) = runtime();
    rt.eval::<()>("A1.DestroyCritical(a)").unwrap();
    assert_eq!(edits(&rt.take_commands()), vec![Edit::Damage(ids[0], Fixed::from_f64(100_000.0))]);
}

#[test]
fn destroy_calls_on_nothing_do_nothing() {
    for name in ["DestroyCritical", "DestroyEngines", "DestroyLifeSupport", "DestroyWeapons", "DestroyShieldGenerator", "DestroySensors"] {
        let (mut rt, _, _) = runtime();
        rt.eval::<()>(&format!("A1.{name}(0); A1.{name}(9999)")).unwrap();
        assert!(rt.take_commands().is_empty(), "{name}");
    }
}

#[test]
fn destroy_calls_still_validate_their_arguments() {
    let (mut rt, _, _) = runtime();
    assert!(rt.eval::<()>("A1.DestroyCritical()").is_err());
    assert!(rt.eval::<()>("A1.DestroyEngines(a, 1)").is_err());
    assert!(rt.eval::<()>("A1.DestroySensors('x')").is_err());
}

#[test]
fn attack_at_a_point_is_an_attack_point_order_from_the_owner() {
    let (mut rt, ids, _) = runtime();
    rt.eval::<()>("A1.Attack(a, Vector(1, 2, 3))").unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].player, PlayerId(0));
    let CommandKind::Order(Order::AttackPoint { units, at }) = &commands[0].kind else { panic!("{:?}", commands[0]) };
    assert_eq!(units, &vec![ids[0]]);
    assert_eq!(*at, Vec3::from_f32([1.0, 2.0, 3.0]));
}

#[test]
fn replace_object_removes_and_builds_at_the_same_place() {
    let (mut rt, ids, db) = runtime();
    let token: i64 = rt.eval("return A1.ReplaceObject(b, 'fx_frigate')").unwrap();
    assert!(token >= i64::from(TOKEN_BASE), "a pending handle, got {token}");
    let edits = edits(&rt.take_commands());
    assert_eq!(edits.len(), 2, "{edits:?}");
    assert_eq!(edits[0], Edit::Kill(ids[1]));
    let Edit::SpawnRequest { odf, player, pos, .. } = &edits[1] else { panic!("{:?}", edits[1]) };
    assert_eq!(*odf, db.key("fx_frigate").unwrap());
    assert_eq!(*player, PlayerId(1));
    assert_eq!(*pos, Vec3::from_f32([100.0, 0.0, 0.0]));
}

#[test]
fn replace_object_of_nothing_returns_zero_and_unknown_odf_queues_nothing() {
    let (mut rt, _, _) = runtime();
    let zero: i64 = rt.eval("return A1.ReplaceObject(0, 'fx_frigate') + A1.ReplaceObject(9999, 'fx_frigate')").unwrap();
    assert_eq!(zero, 0);
    assert!(rt.take_commands().is_empty());
    assert!(rt.eval::<()>("A1.ReplaceObject(a, 'no_such_odf')").is_err());
    assert!(rt.take_commands().is_empty(), "the old object stays when the replacement cannot be built");
}

#[test]
fn attack_location_matches_the_armada_ii_geometry() {
    let (mut rt, _, _) = runtime();
    let same: bool = rt
        .eval(
            "local ok1, v1 = pcall(A1.GetAttackLocation, a, b)
             local ok2, v2 = pcall(A2.GetAttackLocation, a, b)
             if ok1 ~= ok2 then return false end
             if not ok1 then return true end
             return v1.x == v2.x and v1.y == v2.y and v1.z == v2.z",
        )
        .unwrap();
    assert!(same);
}
