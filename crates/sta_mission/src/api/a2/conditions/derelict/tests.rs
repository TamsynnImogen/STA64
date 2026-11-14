//! LUA-26: `IsDerelict` in both bindings and its use by `IsCompromised`, against a real sim
//! whose content runs the casualty model (so the crew edits go through the engine's latch).

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

struct Scene {
    sim: Sim,
    db: Arc<ContentDb>,
    ids: [EntityId; 2],
    seq: u32,
    view: WorldView,
}

fn scene() -> Scene {
    let mut features = Profile::Sta2Classic.features();
    features.crew_loss_on_hit = true;
    let mut unit = UnitDef::new("ship", "craft");
    unit.hull = Fixed::from_int(1000);
    unit.crew = Fixed::from_int(50);
    unit.maximum_crew = Some(Fixed::from_int(50));
    let db = Arc::new(ContentDb::builder().profile(Profile::Sta2Classic).features(features).unit(unit).build().unwrap());
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 5,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let at = |x: f32| Vec3::from_f32([x, 0.0, 0.0]);
    let a = sim.spawn_def(db.key("ship").unwrap(), PlayerId(0), at(0.0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let b = sim.spawn_def(db.key("ship").unwrap(), PlayerId(0), at(100.0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let view = (*sim.step(&[]).view).clone();
    Scene { sim, db, ids: [a, b], seq: 0, view }
}

impl Scene {
    fn edit(&mut self, edits: &[Edit]) {
        let cmds: Vec<Command> = edits
            .iter()
            .map(|e| {
                self.seq += 1;
                Command { player: PlayerId::SCRIPT, seq: self.seq, kind: CommandKind::Edit(e.clone()) }
            })
            .collect();
        self.view = (*self.sim.step(&cmds).view).clone();
    }

    fn latched(&self, i: usize) -> bool {
        self.sim.is_derelict(self.ids[i]).expect("live unit")
    }

    fn runtime(&self) -> MissionRuntime {
        let cfg = MissionConfig { content: Some(self.db.clone()), view: Some(self.view.clone()), ..MissionConfig::default() };
        let rt = MissionRuntime::from_source_with("derelict.lua", "", 3, cfg);
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        rt.eval::<()>(&format!("a, b = {}, {}", self.ids[0].0, self.ids[1].0)).unwrap();
        rt
    }
}

fn truth(rt: &MissionRuntime, chunk: &str) -> bool {
    rt.eval::<bool>(&format!("return {chunk}")).unwrap_or_else(|e| panic!("{chunk}: {e}"))
}

#[test]
fn a_dead_missing_or_never_existing_handle_is_derelict() {
    let s = scene();
    let rt = s.runtime();
    assert!(truth(&rt, "A2.IsDerelict(123456)"));
    assert!(truth(&rt, "A2.IsDerelict(0)"));
    assert!(truth(&rt, "A1.IsDerelict(123456)"));
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn a_crewed_craft_is_not_derelict_and_the_engines_latch_makes_it_so() {
    let mut s = scene();
    let rt = s.runtime();
    assert!(!truth(&rt, "A2.IsDerelict(a)"));
    assert!(!truth(&rt, "A1.IsDerelict(a)"));
    // Crew to zero through the sim's own check: the latch is set (SubsystemRow).
    s.edit(&[Edit::UnitCrew(s.ids[0], Fixed::ZERO)]);
    assert!(s.latched(0), "the sim latched the unit");
    let rt = s.runtime();
    assert!(truth(&rt, "A2.IsDerelict(a)"));
    assert!(truth(&rt, "A1.IsDerelict(a)"));
    assert!(!truth(&rt, "A2.IsDerelict(b)"), "the other craft keeps its crew");
    // Crew back to one or more: the latch clears and so does the answer.
    s.edit(&[Edit::UnitCrew(s.ids[0], Fixed::from_int(5))]);
    s.edit(&[]);
    assert!(!s.latched(0));
    assert!(!truth(&s.runtime(), "A2.IsDerelict(a)"));
}

#[test]
fn craft_cannot_die_tops_the_crew_up_instead_of_latching() {
    let mut s = scene();
    s.edit(&[Edit::Immortal(s.ids[0], true), Edit::UnitCrew(s.ids[0], Fixed::ZERO)]);
    assert!(!s.latched(0));
    let rt = s.runtime();
    assert!(!truth(&rt, "A2.IsDerelict(a)"), "topped up, not derelict");
    assert!(!truth(&rt, "A1.IsDerelict(a)"));
    // With ButItCanBeDerelicted the same crew loss latches.
    s.edit(&[Edit::DerelictAllowed(s.ids[1], true), Edit::Immortal(s.ids[1], true), Edit::UnitCrew(s.ids[1], Fixed::ZERO)]);
    assert!(s.latched(1));
    assert!(truth(&s.runtime(), "A2.IsDerelict(b)"));
}

#[test]
fn is_derelict_checks_its_arguments_like_the_other_conditions() {
    let s = scene();
    let rt = s.runtime();
    assert!(rt.eval::<bool>("return A2.IsDerelict()").is_err());
    assert!(rt.eval::<bool>("return A2.IsDerelict(a, b)").is_err());
    assert!(rt.eval::<bool>("return A2.IsDerelict(1.5)").is_err());
}

#[test]
fn is_compromised_reads_the_same_latch() {
    let mut s = scene();
    assert!(!truth(&s.runtime(), "A2.IsCompromised(a, 0)"));
    s.edit(&[Edit::UnitCrew(s.ids[0], Fixed::ZERO)]);
    let rt = s.runtime();
    assert!(truth(&rt, "A2.IsCompromised(a, 0)"), "a derelict is compromised for its owner");
    assert!(!truth(&rt, "A2.IsCompromised(b, 0)"));
    assert!(truth(&rt, "A2.IsCompromised(123456, 0)"), "a dead handle stays compromised");
    s.edit(&[Edit::Immortal(s.ids[1], true), Edit::UnitCrew(s.ids[1], Fixed::ZERO)]);
    assert!(!truth(&s.runtime(), "A2.IsCompromised(b, 0)"), "the top-up case is not derelict");
}
