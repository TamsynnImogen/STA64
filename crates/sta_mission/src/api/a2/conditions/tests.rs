//! Rule-condition queries (LUA-10) against fake content, a real sim and the
//! runtime's event path. Asset-free; not stock-mission tests.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

struct Scene {
    sim: Sim,
    view: WorldView,
    db: Arc<ContentDb>,
    /// Cruiser of player 0, cruiser of player 1, player-0 station, far player-1 frigate.
    ids: [EntityId; 4],
}

fn scene() -> Scene {
    // The fake station carries the native `is_starbase` flag.
    let fake_db = fake::db();
    let mut builder = ContentDb::builder().profile(fake_db.profile()).features(fake_db.features().clone());
    for unit in fake_db.units() {
        let mut unit = unit.clone();
        unit.is_starbase |= unit.odf == "fx_station";
        builder = builder.unit(unit);
    }
    for weapon in fake_db.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    let db = Arc::new(builder.build().unwrap());
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let spawn = |seq, odf: &str, player, x: f32| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key(odf).expect("fake odf"),
            player: PlayerId(player),
            pos: Vec3::from_f32([x, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let out = sim.step(&[
        spawn(1, "fx_cruiser", 0, 0.0),
        spawn(2, "fx_cruiser", 1, 100.0),
        spawn(3, "fx_station", 0, 300.0),
        spawn(4, "fx_frigate", 1, 5000.0),
    ]);
    let at = |x: f32| out.view.units.iter().find(|u| u.pos == Vec3::from_f32([x, 0.0, 0.0])).unwrap().id;
    Scene { ids: [at(0.0), at(100.0), at(300.0), at(5000.0)], view: (*out.view).clone(), db, sim }
}

fn runtime(s: &Scene) -> MissionRuntime {
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some(s.view.clone()), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("cond.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!("a, b, s, far = {}, {}, {}, {}", s.ids[0].0, s.ids[1].0, s.ids[2].0, s.ids[3].0)).unwrap();
    rt
}

fn eval<T: crate::runtime::EvalResult>(rt: &MissionRuntime, chunk: &str) -> T {
    rt.eval(chunk).unwrap_or_else(|e| panic!("{chunk}: {e}"))
}

#[test]
fn counts_follow_team_odf_radius_and_visibility() {
    let s = scene();
    let rt = runtime(&s);
    assert_eq!(eval::<i64>(&rt, "return A2.CountUnits(-1, nil)"), 4);
    assert_eq!(eval::<i64>(&rt, "return A2.CountUnits(0, 0)"), 2, "0 is the null pointer");
    assert_eq!(eval::<i64>(&rt, "return A2.CountUnits(1, 'fx_cruiser')"), 1);
    assert_eq!(eval::<i64>(&rt, "return A2.CountUnits(1, 'FX_CRUISER.ODF')"), 1);
    assert_eq!(eval::<i64>(&rt, "return A2.CountUnitsNearObject(a, 150, -1, nil)"), 1);
    assert_eq!(eval::<i64>(&rt, "return A2.CountUnitsNearObject(a, 400, 0, nil)"), 1, "the centre is not counted");
    assert_eq!(eval::<i64>(&rt, "return A2.CountUnitsNearObject(a, 99999, 1, 0)"), 2);
    assert!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)") <= 2);
    assert!(eval::<i64>(&rt, "return A2.CountVisibleUnitsNearObject(a, 99999, -1, true, nil)") <= 3);
    assert_eq!(eval::<i64>(&rt, "return A2.CountStations(0)"), 1);
    assert_eq!(eval::<i64>(&rt, "return A2.CountStations(1)"), 0);
    assert!(rt.unsupported().is_empty());
    // Strict arguments.
    for bad in [
        "A2.CountUnits(0)",
        "A2.CountUnits(0.5, nil)",
        "A2.CountUnits(300, nil)",
        "A2.CountUnits(0, true)",
        "A2.CountUnitsNearObject(999999, 10, 0, nil)",
        "A2.CountVisibleUnits(0, 1, 1, nil)",
    ] {
        assert!(rt.eval::<i64>(&format!("return {bad}")).is_err(), "{bad}");
    }
}

#[test]
fn resources_time_crew_and_exploding_read_the_view() {
    let mut s = scene();
    let out = s.sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 5,
        kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(321))),
    }]);
    s.view = (*out.view).clone();
    let rt = runtime(&s);
    assert_eq!(eval::<i64>(&rt, "return A2.GetDilithium(0)"), 321);
    assert_eq!(eval::<i64>(&rt, "return A2.GetLatinum(1)"), 0);
    assert_eq!(eval::<i64>(&rt, "return A2.GetMetal(1)"), 0);
    assert!(rt.eval::<i64>("return A2.GetMetal(-1)").is_err());
    let now: f64 = eval(&rt, "return A2.GetTime()");
    assert!(now > 0.0);
    assert!(eval::<bool>(&rt, &format!("return A2.HasTimeElapsed({})", now - 0.01)));
    assert!(!eval::<bool>(&rt, &format!("return A2.HasTimeElapsed({})", now + 100.0)));
    assert!(!eval::<bool>(&rt, "return A2.IsExploding(a)"));
    assert!(!eval::<bool>(&rt, "return A2.IsExploding(123456)"), "a missing object is not exploding");
}

#[test]
fn under_attack_reads_the_sims_last_enemy_hit_strictly_and_is_false_for_a_dead_handle() {
    let s = scene();
    let mut rt = runtime(&s);
    assert!(!eval::<bool>(&rt, "return A2.UnderAttack(a, 1.0)"));
    let hz = sta_core::TICK_HZ as u64;
    let mut view = s.view.clone();
    let mut out = Vec::new();
    view.attrs.enemy_hit = vec![(s.ids[0], 100)];
    view.tick = 100 + hz - 1;
    rt.think(&view, &[], &mut out);
    assert!(eval::<bool>(&rt, "return A2.UnderAttack(a, 1.0)"));
    assert!(!eval::<bool>(&rt, "return A2.UnderAttack(b, 9.0)"), "another object was not hit");
    view.tick = 100 + hz;
    rt.think(&view, &[], &mut out);
    assert!(!eval::<bool>(&rt, "return A2.UnderAttack(a, 1.0)"), "exactly the window is not inside it (strict)");
    assert!(eval::<bool>(&rt, "return A2.UnderAttack(a, 9.0)"));
    // Hit events alone no longer count: the sim's stamp (enemy hits only) is the source.
    view.attrs.enemy_hit.clear();
    let hit = SimEvent::Hit { target: s.ids[0], by: s.ids[1], weapon: sta_content::ContentKey::NONE, facing: 0, shield: Fixed::ZERO, hull: Fixed::from_int(5), at: Vec3::ZERO };
    rt.think(&view, &[hit], &mut out);
    assert!(!eval::<bool>(&rt, "return A2.UnderAttack(a, 9.0)"));
    assert!(!eval::<bool>(&rt, "return A2.UnderAttack(123456, 1.0)"), "a handle that is gone is false");
}

#[test]
fn playback_state_follows_media_calls_and_client_reports() {
    let s = scene();
    let rt = runtime(&s);
    assert!(!eval::<bool>(&rt, "return A2.IsMoviePlaying()"));
    assert!(eval::<bool>(&rt, "return A2.IsAudioMessageDone(77)"), "an id never started is done");
    eval::<()>(&rt, "id = Media:playSound('x.ogg'); Media:playMovie('m.bik')");
    assert!(eval::<bool>(&rt, "return A2.IsMoviePlaying()"));
    assert!(!eval::<bool>(&rt, "return A2.IsAudioMessageDone(id)"));
    eval::<()>(&rt, "A2.AudioMessageDone(id); A2.MovieDone()");
    assert!(eval::<bool>(&rt, "return A2.IsAudioMessageDone(id)"));
    assert!(!eval::<bool>(&rt, "return A2.IsMoviePlaying()"));
    eval::<()>(&rt, "id = Media:playSound('y.ogg'); Media:playMovie('m.bik')");
    eval::<()>(&rt, "Media:stopSounds(); Media:stopMovie()");
    assert!(eval::<bool>(&rt, "return A2.IsAudioMessageDone(id)") && !eval::<bool>(&rt, "return A2.IsMoviePlaying()"));
}

#[test]
fn path_distance_reads_host_paths_and_reports_a_missing_one() {
    let s = scene();
    let rt = runtime(&s);
    // The map lacks the path: the engine's lookup is null and the answer is the far constant (LUA-36).
    assert_eq!(eval::<f64>(&rt, "return A2.GetDistance(a, 'P', 0)"), crate::api::ABSENT_DISTANCE);
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    eval::<()>(&rt, "A2.paths.P = { {x=30,y=0,z=0}, {x=0,y=40,z=0} }");
    assert_eq!(eval::<f64>(&rt, "return A2.GetDistance(a, 'P', 0)"), 30.0);
    assert_eq!(eval::<f64>(&rt, "return A2.GetDistance(a, 'P', 1)"), 40.0);
    assert_eq!(eval::<f64>(&rt, "return A2.GetDistance(a, 'P', 2)"), 40.0, "the index is clamped (LUA-36)");
    assert_eq!(eval::<f64>(&rt, "return A2.GetDistance(a, b)"), 100.0, "the two-object overload is unchanged");
}

#[test]
fn calls_without_data_raise_and_are_counted_never_guessed() {
    let s = scene();
    let rt = runtime(&s);
    for (call, name) in [
        ("A2.IsShipAliveAndOffMap(a)", "IsShipAliveAndOffMap"),
        ("A2.IsCargoShipReadyToLoad(a)", "IsCargoShipReadyToLoad"),
        ("A2.IsCargoShipFullAndReadyToSend(a)", "IsCargoShipFullAndReadyToSend"),
        ("A2.GetLatinumTraded(0, 1)", "GetLatinumTraded"),
    ] {
        let e = rt.eval::<bool>(&format!("return {call}")).expect_err(call).to_string();
        assert!(e.contains(&format!("A2.{name}")), "{e}");
    }
    let counted = rt.unsupported();
    assert_eq!(counted.len(), 4, "{counted:?}");
    assert!(counted.iter().all(|(_, n)| *n == 1));
}

#[test]
fn is_compromised_is_lost_to_the_expected_team() {
    let mut s = scene();
    let rt = runtime(&s);
    // The object belongs to the team named: not compromised. Any other
    // team, or an object that is gone: compromised (LUA-12).
    assert!(!eval::<bool>(&rt, "return A2.IsCompromised(a, 0)"));
    assert!(eval::<bool>(&rt, "return A2.IsCompromised(a, 1)"));
    assert!(!eval::<bool>(&rt, "return A2.IsCompromised(b, 1)"));
    assert!(!eval::<bool>(&rt, "return A2.IsCompromised(s, 0)"), "a station counts as a craft");
    assert!(eval::<bool>(&rt, "return A2.IsCompromised(123456, 0)"));
    assert!(rt.eval::<bool>("return A2.IsCompromised(a)").is_err());
    assert!(rt.unsupported().is_empty());
    // Crew under one with a positive maximum is the derelict state.
    let id = s.ids[0];
    s.view.units.iter_mut().find(|u| u.id == id).unwrap().crew = Some([Fixed::ZERO, Fixed::from_int(10)]);
    let rt = runtime(&s);
    assert!(eval::<bool>(&rt, "return A2.IsCompromised(a, 0)"));
    s.view.units.iter_mut().find(|u| u.id == id).unwrap().crew = Some([Fixed::from_int(3), Fixed::from_int(10)]);
    assert!(!eval::<bool>(&runtime(&s), "return A2.IsCompromised(a, 0)"));
}

#[test]
fn is_compromised_under_armada_teams_dying_crew_none_and_fixtures() {
    let mut s = scene();
    let mk = |s: &Scene| {
        let cfg = MissionConfig {
            content: Some(s.db.clone()),
            view: Some(s.view.clone()),
            player_numbering: crate::api::PlayerNumbering::ArmadaTeams,
            ..MissionConfig::default()
        };
        let rt = MissionRuntime::from_source_with("cond.lua", "", 3, cfg);
        rt.eval::<()>(&format!("a, b = {}, {}", s.ids[0].0, s.ids[1].0)).unwrap();
        rt
    };
    // Slot 0 is team 1 for converted missions.
    let rt = mk(&s);
    assert!(!eval::<bool>(&rt, "return A2.IsCompromised(a, 1)"));
    assert!(eval::<bool>(&rt, "return A2.IsCompromised(a, 0)"));
    assert!(!eval::<bool>(&rt, "return A2.IsCompromised(b, 2)"));
    // Crew unknown, or a zero maximum, is not a derelict; a dying unit is lost.
    let id = s.ids[0];
    s.view.units.iter_mut().find(|u| u.id == id).unwrap().crew = None;
    assert!(!eval::<bool>(&mk(&s), "return A2.IsCompromised(a, 1)"));
    s.view.units.iter_mut().find(|u| u.id == id).unwrap().crew = Some([Fixed::ZERO, Fixed::ZERO]);
    assert!(!eval::<bool>(&mk(&s), "return A2.IsCompromised(a, 1)"));
    let u = s.view.units.iter_mut().find(|u| u.id == id).unwrap();
    u.flags = u.flags.with(sta_sim::view::UnitFlags::DYING, true);
    assert!(eval::<bool>(&mk(&s), "return A2.IsCompromised(a, 1)"));
}

/// Script reads of sight leave another team's unit out in the tick after it first appears
/// (the sensors have not scanned it); the viewer's own seat and its mutual confirmed allies
/// count at once, and a unit that leaves sight and returns is counted at once.
#[test]
fn visible_counts_skip_the_unit_that_just_appeared() {
    let mut s = scene();
    let mut rt = runtime(&s);
    let mut out = Vec::new();
    // The world the mission starts in is seen as it stands (the cruiser at 100 is in range).
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)"), 1);
    rt.think(&s.view, &[], &mut out);
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)"), 1, "already there");
    let spawn = |seq, player, x: f32| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: s.db.key("fx_cruiser").expect("fake odf"),
            player: PlayerId(player),
            pos: Vec3::from_f32([x, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let step = s.sim.step(&[spawn(10, 1, 150.0), spawn(11, 0, 60.0)]);
    rt.think(&step.view, &step.events, &mut out);
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)"), 1, "new in range, not scanned yet");
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 0, false, nil)"), 3, "own units count at once");
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnitsNearObject(a, 99999, 1, true, nil)"), 1);
    // The other sight reads agree in the same tick.
    let fresh_enemy = step.view.units.iter().find(|u| u.owner == PlayerId(1) && u.pos == Vec3::from_f32([150.0, 0.0, 0.0])).unwrap().id;
    assert!(!eval::<bool>(&rt, &format!("return A2.IsVisibleToTeam({}, 0)", fresh_enemy.0)));
    assert!(eval::<bool>(&rt, &format!("return A2.IsVisibleToTeam({}, 1)", fresh_enemy.0)), "its own seat");
    let step = s.sim.step(&[]);
    rt.think(&step.view, &step.events, &mut out);
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)"), 2, "one tick later");
    assert!(eval::<bool>(&rt, &format!("return A2.IsVisibleToTeam({}, 0)", fresh_enemy.0)));

    // A unit that left sight and comes back is counted at once: it is no longer new.
    let mut hidden = (*step.view).clone();
    for (p, seen) in &mut hidden.visibility {
        if *p == PlayerId(0) {
            seen.retain(|id| *id != fresh_enemy);
        }
    }
    rt.think(&hidden, &[], &mut out);
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)"), 1, "out of sight");
    rt.think(&step.view, &[], &mut out);
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)"), 2, "back in sight at once");
}

/// A fresh unit of a mutual confirmed ally shares the viewer's sight at once.
#[test]
fn a_fresh_allied_unit_counts_at_once() {
    let mut s = scene();
    let mut rt = runtime(&s);
    let mut out = Vec::new();
    rt.think(&s.view, &[], &mut out);
    rt.eval::<()>("A2.SetRelation(0, 1, 2); A2.SetRelation(1, 0, 2)").unwrap();
    let spawn = Command {
        player: PlayerId::SCRIPT,
        seq: 20,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: s.db.key("fx_cruiser").expect("fake odf"),
            player: PlayerId(1),
            pos: Vec3::from_f32([200.0, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let step = s.sim.step(&[spawn]);
    rt.think(&step.view, &step.events, &mut out);
    assert_eq!(eval::<i64>(&rt, "return A2.CountVisibleUnits(0, 1, false, nil)"), 2, "ally's new unit, same tick");
}

#[test]
fn is_ship_alive_and_off_map_reads_the_map_bounds_the_host_passes() {
    let s = scene();
    let b = |v: f32| Vec3::from_f32([v, -v, v]);
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some(s.view.clone()), map_bounds: Some((b(-1000.0), b(1000.0))), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("cond.lua", "", 3, cfg);
    rt.eval::<()>(&format!("a, s, far = {}, {}, {}", s.ids[0].0, s.ids[2].0, s.ids[3].0)).unwrap();
    assert!(!eval::<bool>(&rt, "return A2.IsShipAliveAndOffMap(a)"), "a craft inside the map");
    assert!(eval::<bool>(&rt, "return A2.IsShipAliveAndOffMap(far)"), "a craft at x 5000, outside");
    assert!(!eval::<bool>(&rt, "return A2.IsShipAliveAndOffMap(999999)"), "no such object");
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn count_capital_ships_counts_ship_types_a_to_d_and_f() {
    let s = scene();
    let rt = runtime(&s);
    // The fake content has no `shipType`: no capital ships.
    assert_eq!(eval::<i64>(&rt, "return A2.CountCapitalShips(0)"), 0);
    let fake_db = fake::db();
    for (ty, want) in [("A", 1), ("c", 1), ("F", 1), ("E", 0), ("X", 0)] {
        let mut b = ContentDb::builder().profile(fake_db.profile()).features(fake_db.features().clone());
        for u in s.db.units() {
            let mut u = u.clone();
            if u.odf == "fx_cruiser" || u.odf == "fx_station" {
                u.stats.ship_type = Some(ty.to_ascii_lowercase());
            }
            b = b.unit(u);
        }
        for w in s.db.weapons() {
            b = b.weapon(w.clone());
        }
        let cfg = MissionConfig { content: Some(Arc::new(b.build().unwrap())), view: Some(s.view.clone()), ..MissionConfig::default() };
        let rt = MissionRuntime::from_source_with("cond.lua", "", 3, cfg);
        // Player 0 has one cruiser and one station: only the craft counts.
        assert_eq!(eval::<i64>(&rt, "return A2.CountCapitalShips(0)"), want, "shipType {ty}");
    }
}
