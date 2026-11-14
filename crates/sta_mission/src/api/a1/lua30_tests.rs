//! LUA-30 tests: the `Destroy<System>` calls, `Attack(unit, Vector)`,
//! `ReplaceObject` and `GetAttackLocation` of the `A1` table.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order, Subsystem};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::world::NEUTRAL;
use sta_sim::{Sim, SimEvent, WorldSource};

use super::TOKEN_BASE;
use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

const HP: i32 = 10;

fn db() -> Arc<ContentDb> {
    let ship = |odf: &str, sensors: i32| {
        let mut u = UnitDef::new(odf, "craft");
        u.race = "fx".into();
        u.model = odf.into();
        u.hull = Fixed::from_int(100);
        u.radius = Fixed::from_int(10);
        u.system_hit_points = [Fixed::from_int(HP), Fixed::from_int(HP), Fixed::from_int(HP), Fixed::from_int(HP), Fixed::from_int(sensors)];
        u
    };
    let mut station = UnitDef::new("t_station", "starbase");
    station.race = "fx".into();
    station.model = "t_station".into();
    station.hull = Fixed::from_int(100);
    Arc::new(ContentDb::builder().unit(ship("t_ship", HP)).unit(ship("t_other", 0)).unit(station).build().expect("content"))
}

struct Scene {
    sim: Sim,
    rt: MissionRuntime,
    /// Ship of player 0 at the origin, ship of player 1 at z 100, a station of player 0, a neutral ship.
    ids: [EntityId; 4],
    db: Arc<ContentDb>,
}

fn scene() -> Scene {
    let db = db();
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let spawn = |seq, odf: &str, player, x: f32, z: f32| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key(odf).expect("odf"),
            player,
            pos: Vec3::from_f32([x, 0.0, z]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let out = sim.step(&[
        spawn(1, "t_ship", PlayerId(0), 0.0, 0.0),
        spawn(2, "t_ship", PlayerId(1), 0.0, 100.0),
        spawn(3, "t_station", PlayerId(0), -100.0, 0.0),
        spawn(4, "t_ship", NEUTRAL, 300.0, 0.0),
    ]);
    let at = |x: f32, z: f32| out.view.units.iter().find(|u| u.pos == Vec3::from_f32([x, 0.0, z])).expect("unit").id;
    let ids = [at(0.0, 0.0), at(0.0, 100.0), at(-100.0, 0.0), at(300.0, 0.0)];
    let cfg = MissionConfig { content: Some(db.clone()), view: Some((*out.view).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("a1.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!("a, b, s, n = {}, {}, {}, {}", ids[0].0, ids[1].0, ids[2].0, ids[3].0)).unwrap();
    rt.take_commands();
    Scene { sim, rt, ids, db }
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
fn destroy_system_switches_the_system_off_in_the_sim() {
    let hit = Fixed::from_f64(100_000.0);
    let cases = [
        ("DestroyEngines", Subsystem::Engines),
        ("DestroyLifeSupport", Subsystem::LifeSupport),
        ("DestroyWeapons", Subsystem::Weapons),
        ("DestroyShieldGenerator", Subsystem::Shields),
        ("DestroySensors", Subsystem::Sensors),
    ];
    for (name, sub) in cases {
        let mut s = scene();
        let id = s.ids[1];
        s.rt.eval::<()>(&format!("A1.{name}(b)")).unwrap();
        let commands = s.rt.take_commands();
        assert_eq!(edits(&commands), vec![Edit::SubsystemDamage(id, sub, hit), Edit::Subsystem(id, sub, false)], "{name}");
        let before = s.sim.subsystem_state(id, sub).unwrap();
        assert!(before.0, "{name}: on before");
        s.sim.step(&commands);
        assert_eq!(s.sim.subsystem_state(id, sub), Some((false, Fixed::from_int(HP))), "{name}");
        // The other systems stay as they were.
        for other in [Subsystem::Engines, Subsystem::LifeSupport, Subsystem::Weapons, Subsystem::Shields, Subsystem::Sensors] {
            if other != sub {
                assert!(s.sim.subsystem_state(id, other).unwrap().0, "{name} switched {other:?} off");
            }
        }
    }
}

#[test]
fn destroy_critical_kills_a_craft_in_the_sim() {
    let mut s = scene();
    s.rt.eval::<()>("A1.DestroyCritical(b)").unwrap();
    let commands = s.rt.take_commands();
    assert_eq!(edits(&commands), vec![Edit::Damage(s.ids[1], Fixed::from_f64(100_000.0))]);
    let out = s.sim.step(&commands);
    let gone = out.view.units.iter().find(|u| u.id == s.ids[1]).is_none_or(|u| u.flags.contains(UnitFlags::DYING));
    assert!(gone, "{:?}", out.events);
}

#[test]
fn repeated_critical_hits_in_one_tick_queue_one_edit() {
    let mut s = scene();
    let hit = Fixed::from_f64(100_000.0);
    s.rt.eval::<()>("for i = 1, 50 do A1.DestroyCritical(b) end; A1.DestroyCritical(a)").unwrap();
    let got = edits(&s.rt.take_commands());
    assert_eq!(got, vec![Edit::Damage(s.ids[1], hit), Edit::Damage(s.ids[0], hit)]);
    // A new tick (the commands were never stepped, so b is still alive in the view): the set is reset.
    let out = s.sim.step(&[]);
    let mut later = Vec::new();
    s.rt.think(&out.view, &out.events, &mut later);
    s.rt.eval::<()>("A1.DestroyCritical(b); A1.DestroyCritical(b)").unwrap();
    assert_eq!(edits(&s.rt.take_commands()), vec![Edit::Damage(s.ids[1], hit)]);
}

#[test]
fn a_system_without_hit_points_is_left_alone() {
    let mut s = scene();
    // `t_other` has no sensor hit points: nothing is queued, no switch-off either.
    let spawn = Command {
        player: PlayerId::SCRIPT,
        seq: 9,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: s.db.key("t_other").unwrap(),
            player: PlayerId(1),
            pos: Vec3::from_f32([500.0, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let out = s.sim.step(&[spawn]);
    let blind = out.view.units.iter().find(|u| u.pos == Vec3::from_f32([500.0, 0.0, 0.0])).expect("unit").id;
    let mut later = Vec::new();
    s.rt.think(&out.view, &out.events, &mut later);
    s.rt.eval::<()>(&format!("A1.DestroySensors({})", blind.0)).unwrap();
    assert!(s.rt.take_commands().is_empty());
    s.rt.eval::<()>(&format!("A1.DestroyEngines({})", blind.0)).unwrap();
    assert_eq!(s.rt.take_commands().len(), 2, "its engines have hit points");
}

#[test]
fn destroy_calls_only_touch_live_craft() {
    for name in ["DestroyCritical", "DestroyEngines", "DestroyLifeSupport", "DestroyWeapons", "DestroyShieldGenerator", "DestroySensors"] {
        let mut s = scene();
        // Nothing, nothing live, and a station (not a Craft): no edit, no error.
        s.rt.eval::<()>(&format!("A1.{name}(0); A1.{name}(9999); A1.{name}(s)")).unwrap();
        assert!(s.rt.take_commands().is_empty(), "{name}");
        assert!(s.rt.eval::<()>(&format!("A1.{name}()")).is_err());
        assert!(s.rt.eval::<()>(&format!("A1.{name}(a, 1)")).is_err());
        assert!(s.rt.eval::<()>(&format!("A1.{name}('x')")).is_err());
    }
}

#[test]
fn attack_at_a_point_is_an_attack_point_order_from_the_owner() {
    let mut s = scene();
    s.rt.eval::<()>("A1.Attack(a, Vector(1, 2, 3))").unwrap();
    let commands = s.rt.take_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].player, PlayerId(0));
    let CommandKind::Order(Order::AttackPoint { units, at }) = &commands[0].kind else { panic!("{:?}", commands[0]) };
    assert_eq!(units, &vec![s.ids[0]]);
    assert_eq!(*at, Vec3::from_f32([1.0, 2.0, 3.0]));
}

#[test]
fn replace_object_removes_and_builds_at_the_same_place() {
    let mut s = scene();
    let token: i64 = s.rt.eval("return A1.ReplaceObject(b, 't_other')").unwrap();
    assert!(token >= i64::from(TOKEN_BASE), "a pending handle, got {token}");
    let commands = s.rt.take_commands();
    let got = edits(&commands);
    assert_eq!(got.len(), 2, "{got:?}");
    let Edit::SpawnRequest { odf, player, pos, .. } = &got[0] else { panic!("{:?}", got[0]) };
    assert_eq!(*odf, s.db.key("t_other").unwrap());
    assert_eq!(*player, PlayerId(1));
    assert_eq!(*pos, Vec3::from_f32([0.0, 0.0, 100.0]));
    assert_eq!(got[1], Edit::Kill(s.ids[1]));
    // A pending token as the second object of GetAttackLocation raises, it is not silently "absent".
    let err = s.rt.eval::<()>(&format!("A1.GetAttackLocation(a, {token})")).unwrap_err().to_string();
    assert!(err.contains("still being built"), "{err}");
    // Deliver the receipt: the handle names the new object, at the old place and owner.
    let out = s.sim.step(&commands);
    let new = out
        .events
        .iter()
        .find_map(|e| match e {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .expect("receipt");
    let mut later = Vec::new();
    s.rt.think(&out.view, &out.events, &mut later);
    assert!(s.rt.eval::<bool>(&format!("return A1.IsAlive({token})")).unwrap());
    assert_eq!(s.rt.eval::<f64>(&format!("return A1.GetLocation({token}).z")).unwrap(), 100.0);
    let unit = out.view.units.iter().find(|u| u.id == new).expect("new unit");
    assert_eq!(unit.owner, PlayerId(1));
    assert_eq!(unit.def, s.db.key("t_other").unwrap());
    // GetAttackLocation resolves the token in its second argument too: toward the new unit.
    let v: (f64, f64, f64) = s.rt.eval(&format!("local v = A1.GetAttackLocation(a, {token}); return v.x, v.y, v.z")).unwrap();
    assert_eq!(v, (0.0, 0.0, 10.0));
}

#[test]
fn replace_object_of_nothing_returns_zero() {
    let mut s = scene();
    let zero: i64 = s.rt.eval("return A1.ReplaceObject(0, 't_other') + A1.ReplaceObject(9999, 't_other')").unwrap();
    assert_eq!(zero, 0);
    assert!(s.rt.take_commands().is_empty());
}

#[test]
fn replace_object_that_cannot_build_keeps_the_old_object() {
    let mut s = scene();
    // Unknown ODF; and a neutral craft, which the spawn rules refuse to rebuild.
    assert!(s.rt.eval::<()>("A1.ReplaceObject(a, 'no_such_odf')").is_err());
    let err = s.rt.eval::<()>("A1.ReplaceObject(n, 't_other')").unwrap_err().to_string();
    assert!(err.contains("neutral"), "{err}");
    assert!(s.rt.take_commands().is_empty(), "no kill, no spawn");
}

#[test]
fn attack_location_is_the_armada_ii_geometry() {
    let s = scene();
    let toward: (f64, f64, f64) = s.rt.eval("local v = A1.GetAttackLocation(a, b); return v.x, v.y, v.z").unwrap();
    assert_eq!(toward, (0.0, 0.0, 10.0));
    let alone: (f64, f64, f64) = s.rt.eval("local v = A1.GetAttackLocation(a, 0); return v.x, v.y, v.z").unwrap();
    assert_eq!(alone, (10.0, 0.0, 0.0));
    let a2: (f64, f64, f64) = s.rt.eval("local v = A2.GetAttackLocation(a, b); return v.x, v.y, v.z").unwrap();
    assert_eq!(a2, toward);
}
