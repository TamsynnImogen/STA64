//! SIM-68: the shield-percent calls on the Armada I pool (the hull). Authored numbers.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

fn db(profile: Profile) -> Arc<ContentDb> {
    let mut b = ContentDb::builder().profile(profile).features(profile.features());
    let mut ship = UnitDef::new("ship", "craft");
    ship.hull = Fixed::from_int(800);
    ship.race = "fx".into();
    b = b.unit(ship);
    Arc::new(b.build().unwrap())
}

/// A world with one craft; its hull is lowered to 200 of 800 before the view is taken.
fn scene(profile: Profile) -> (WorldView, Arc<ContentDb>, EntityId) {
    let d = db(profile);
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 3, map: Arc::new(MapDef::empty()), content: Arc::clone(&d), players: vec![seat(0), seat(1)], options: GameOptions::default() });
    let id = sim.spawn_def(d.key("ship").unwrap(), PlayerId(1), Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO }, (Fixed::ZERO, Fixed::ONE)).unwrap();
    let cmd = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::HullValue(id, Fixed::from_int(200))) };
    let out = sim.step(&[cmd]);
    (out.view.as_ref().clone(), d, id)
}

fn run(profile: Profile, script: &str) -> (Vec<Command>, EntityId) {
    let (view, db, id) = scene(profile);
    let cfg = MissionConfig { content: Some(db), view: Some(view), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("a1.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!("u = {}; {script}", id.0)).unwrap_or_else(|e| panic!("{script}: {e}"));
    (rt.take_commands(), id)
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
fn set_shield_percent_sets_the_hull_to_that_fraction_of_its_maximum_without_a_clamp() {
    let (commands, id) = run(Profile::Sta1Classic, "A1.SetShieldPercent(u, 0.25); A1.SetShieldPercent(u, 1.5); A1.SetShieldPercent(u, 0)");
    assert_eq!(
        edits(&commands),
        vec![
            Edit::HullValue(id, Fixed::from_int(200)),
            Edit::HullValue(id, Fixed::from_int(1200)),
            Edit::HullValue(id, Fixed::ZERO),
        ]
    );
}

#[test]
fn set_max_shields_sets_the_hull_maximum_then_the_hull() {
    let (commands, id) = run(Profile::Sta1Classic, "A1.SetMaxShields(u, 950)");
    assert_eq!(edits(&commands), vec![Edit::MaxHull(id, Fixed::from_int(950)), Edit::HullValue(id, Fixed::from_int(950))]);
}

#[test]
fn set_max_shields_of_zero_or_less_leaves_the_hull_alone() {
    let (commands, _) = run(Profile::Sta1Classic, "A1.SetMaxShields(u, 0); A1.SetMaxShields(u, -40)");
    assert!(commands.is_empty(), "{commands:?}");
}

#[test]
fn the_getters_answer_the_hull_over_its_maximum() {
    let (commands, _) = run(
        Profile::Sta1Classic,
        "assert(A1.GetShieldPercent(u) == 0.25); assert(A1.GetRealShieldPercent(u) == 0.25)",
    );
    assert!(commands.is_empty());
}

#[test]
fn a_lua_snippet_sets_the_pool_through_the_call() {
    let (commands, id) = run(Profile::Sta1Classic, "local f = A1.GetShieldPercent(u) * 2; A1.SetShieldPercent(u, f)");
    assert_eq!(edits(&commands), vec![Edit::HullValue(id, Fixed::from_int(400))]);
}

#[test]
fn a_unit_without_the_feature_keeps_the_shield_path() {
    let (commands, id) = run(Profile::Native, "A1.SetShieldPercent(u, 0.5); A1.SetMaxShields(u, 900); assert(A1.GetShieldPercent(u) == 0)");
    assert_eq!(edits(&commands), vec![Edit::ShieldPercent(id, Fixed::from_f64(0.5)), Edit::MaxShields(id, Fixed::from_int(900))]);
}

#[test]
fn bad_arguments_and_dead_handles_behave_as_before() {
    let (view, db, id) = scene(Profile::Sta1Classic);
    let cfg = MissionConfig { content: Some(db), view: Some(view), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("a1.lua", "", 3, cfg);
    rt.eval::<()>(&format!("u = {}", id.0)).unwrap();
    assert!(rt.eval::<()>("A1.SetShieldPercent(u)").is_err());
    assert!(rt.eval::<()>("A1.SetShieldPercent(u, 0/0)").is_err());
    assert!(rt.eval::<()>("A1.SetShieldPercent(999999, 0.5)").is_err());
    rt.eval::<()>("assert(A1.GetRealShieldPercent(999999) == -33)").unwrap();
    assert!(rt.take_commands().is_empty());
}
