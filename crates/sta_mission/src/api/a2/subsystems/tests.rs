use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;
use sta_core::{EntityId, Fixed, PlayerId, Vec3};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::view::UnitFlags;
use sta_sim::{MoveStats, Sim, WorldSource};

fn scene() -> (Sim, EntityId) {
    let mut sim = Sim::new(17);
    let id = sim.spawn(
        0,
        Vec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 0.2),
    );
    (sim, id)
}

fn runtime(sim: &Sim) -> MissionRuntime {
    let cfg = MissionConfig {
        view: Some((*sim.view()).clone()),
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("switches.lua", "", 17, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

#[test]
fn disable_and_reenable_queue_script_edits_then_apply_and_replay() {
    let (mut sim, id) = scene();
    let mut replay = sim.clone();
    let mut rt = runtime(&sim);

    rt.eval::<()>(&format!(
        "A2.DisableEngines({}, true); A2.DisableWeapons({}, true)",
        id.0, id.0
    ))
    .unwrap();
    // Lua has queued edits; neither view nor sim is mutated until the tick.
    assert!(
        !sim.view()
            .unit(id)
            .unwrap()
            .flags
            .contains(UnitFlags::ENGINES_OFF | UnitFlags::WEAPONS_OFF)
    );
    let disable = rt.take_commands();
    assert_eq!(disable.len(), 2);
    assert_eq!(
        (disable[0].player, &disable[0].kind),
        (
            PlayerId::SCRIPT,
            &CommandKind::Edit(Edit::Engines(id, false))
        )
    );
    assert_eq!(
        (disable[1].player, &disable[1].kind),
        (
            PlayerId::SCRIPT,
            &CommandKind::Edit(Edit::Weapons(id, false))
        )
    );
    let output = sim.step(&disable);
    assert_eq!(replay.step(&disable).events, output.events);
    assert!(
        !output
            .events
            .iter()
            .any(|e| matches!(e, sta_sim::SimEvent::Rejected { .. }))
    );
    let flags = output.view.unit(id).unwrap().flags;
    assert!(flags.contains(UnitFlags::ENGINES_OFF) && flags.contains(UnitFlags::WEAPONS_OFF));

    rt.eval::<()>(&format!(
        "A2.DisableEngines({}, false); A2.DisableWeapons({}, false)",
        id.0, id.0
    ))
    .unwrap();
    let enable = rt.take_commands();
    assert_eq!(enable.len(), 2);
    assert_eq!(
        (enable[0].player, &enable[0].kind),
        (
            PlayerId::SCRIPT,
            &CommandKind::Edit(Edit::Engines(id, true))
        )
    );
    assert_eq!(
        (enable[1].player, &enable[1].kind),
        (
            PlayerId::SCRIPT,
            &CommandKind::Edit(Edit::Weapons(id, true))
        )
    );
    let output = sim.step(&enable);
    assert_eq!(replay.step(&enable).events, output.events);
    assert!(
        !output
            .events
            .iter()
            .any(|e| matches!(e, sta_sim::SimEvent::Rejected { .. }))
    );
    let flags = output.view.unit(id).unwrap().flags;
    assert!(!flags.contains(UnitFlags::ENGINES_OFF) && !flags.contains(UnitFlags::WEAPONS_OFF));
    assert_eq!(sim.digest(), replay.digest());
}

#[test]
fn invalid_arguments_and_dead_handles_do_not_queue_partly() {
    let (sim, id) = scene();
    let mut rt = runtime(&sim);
    for script in [
        "A2.DisableEngines(1)",
        "A2.DisableWeapons(1, true, false)",
        "A2.DisableEngines(-1, true)",
        "A2.DisableWeapons(1.5, true)",
        "A2.DisableEngines(4294967296, true)",
        "A2.DisableWeapons('1', true)",
        "A2.DisableEngines(1, 1)",
        "A2.DisableWeapons(1, nil)",
        "A2.DisableEngines(99999, true)",
        "A2.DisableWeapons(99999, true)",
    ] {
        assert!(rt.eval::<()>(script).is_err(), "{script}");
        assert!(
            rt.take_commands().is_empty(),
            "{script} partly queued an edit"
        );
    }
    let mut dying = (*sim.view()).clone();
    dying.units.iter_mut().find(|u| u.id == id).unwrap().flags = UnitFlags::DYING;
    let cfg = MissionConfig {
        view: Some(dying),
        ..MissionConfig::default()
    };
    let mut rt = MissionRuntime::from_source_with("dying-switch.lua", "", 17, cfg);
    assert!(
        rt.eval::<()>(&format!("A2.DisableEngines({}, true)", id.0))
            .is_err()
    );
    assert!(
        rt.eval::<()>(&format!("A2.DisableWeapons({}, true)", id.0))
            .is_err()
    );
    assert!(rt.take_commands().is_empty());
}
