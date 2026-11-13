use super::*;

fn paths() -> Vec<(String, Vec<[f32; 3]>)> {
    vec![
        // Five points, 100 units apart on x: four segments.
        ("RUN".into(), (0..5).map(|i| [i as f32 * 100.0, 0.0, 0.0]).collect()),
        // A first segment of 100, then one of 300: constant speed makes the second last 3x.
        ("UNEVEN".into(), vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [400.0, 0.0, 0.0]]),
        ("ONE".into(), vec![[5.0, 6.0, 7.0]]),
        ("Camera_1".into(), vec![[10.0, 0.0, 20.0]]),
    ]
}

fn call(name: &str, args: Vec<Arg>) -> Presentation {
    Presentation::Call { name: name.into(), args }
}

fn num(x: f64) -> Arg {
    Arg::Num(Fixed::from_f64(x))
}

fn on() -> Presentation {
    Presentation::Cineractive { on: true, skippable: true }
}

const HZ: u64 = TICK_HZ as u64;

#[test]
fn a_path_shot_lasts_segments_over_speed_and_done_follows_it() {
    let mut c = Cineractive::default();
    c.observe(&on(), 10, &paths());
    // `CineractiveShot('RUN', 0.5, 'Camera_1', 0)`: 4 segments at 0.5/s = 8 s.
    c.observe(&call("A1.CineractiveShot", vec![Arg::Text("RUN".into()), num(0.5), Arg::Text("Camera_1".into()), Arg::Int(0)]), 100, &paths());
    assert_eq!(c.engine_values(100), Some((0, 0)), "the shot's own tick reads 0");
    assert!(!c.done_at(100 + 8 * HZ - 1));
    assert!(c.done_at(100 + 8 * HZ));
    // The camera ends at the last point, looking at Camera_1's point.
    let (eye, look) = c.pose((100 + 8 * HZ) as f64, &|_| None).unwrap();
    assert!(len(sub(eye, [400.0, 0.0, 0.0])) < 1e-6, "{eye:?}");
    assert_eq!(look, [10.0, 0.0, 20.0]);
    // Halfway it is at the middle point.
    let (mid, _) = c.pose((100 + 4 * HZ) as f64, &|_| None).unwrap();
    assert!(len(sub(mid, [200.0, 0.0, 0.0])) < 1e-6, "{mid:?}");
}

#[test]
fn object_and_point_shots_are_done_on_the_next_update() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    c.observe(&call("A1.CineractiveShot", vec![Arg::Text("ONE".into()), Arg::Int(0), Arg::Entity(EntityId(7))]), 5, &paths());
    assert!(!c.done_at(5));
    assert!(c.done_at(6));
    let (eye, look) = c.pose(6.0, &|id| (id == EntityId(7)).then_some(([1.0, 2.0, 3.0], [0.0, 0.0, 1.0]))).unwrap();
    assert_eq!((eye, look), ([5.0, 6.0, 7.0], [1.0, 2.0, 3.0]));
    // A followed object that is gone gives no pose (the client holds the last one).
    assert!(c.pose(6.0, &|_| None).is_none());
}

#[test]
fn a_speed_zero_path_is_never_done_and_a_one_point_path_at_once() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    c.observe(&call("A2.CineractiveShot", vec![Arg::Text("RUN".into()), num(0.0), Arg::Entity(EntityId(1))]), 1, &paths());
    assert!(!c.done_at(1_000_000));
    c.observe(&call("A2.CineractiveShot", vec![Arg::Text("ONE".into()), num(0.3), Arg::Entity(EntityId(1))]), 2, &paths());
    assert!(c.done_at(3));
}

#[test]
fn unknown_paths_and_points_leave_the_shot_but_still_read_zero_on_their_tick() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    c.observe(&call("A1.CineractiveShot", vec![Arg::Entity(EntityId(1)), Arg::Entity(EntityId(2))]), 1, &paths());
    let before = c.shot.clone();
    c.observe(&call("A1.CineractiveShot", vec![Arg::Text("NOPE".into()), num(1.0), Arg::Entity(EntityId(2))]), 4, &paths());
    c.observe(&call("A1.CineractiveShot", vec![Arg::Text("ONE".into()), Arg::Int(3), Arg::Entity(EntityId(2))]), 5, &paths());
    assert_eq!(c.shot, before);
    assert!(!c.done_at(5));
    assert!(c.done_at(6));
}

#[test]
fn constant_speed_spreads_the_eye_path_by_length() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    c.observe(&call("A2.CineractiveShot", vec![Arg::Text("UNEVEN".into()), num(1.0), Arg::Bool(true)]), 0, &paths());
    // Without: 2 segments at 1/s = 2 s.
    assert!(c.done_at(2 * HZ));
    c.observe(&call("A2.CineractiveConstantSpeed", vec![Arg::Bool(true)]), 0, &paths());
    // With: 100 u/s over 400 units = 4 s.
    assert!(!c.done_at(4 * HZ - 1));
    assert!(c.done_at(4 * HZ));
    // A new shot turns it off again.
    c.observe(&call("A2.CineractiveShot", vec![Arg::Text("UNEVEN".into()), num(1.0), Arg::Bool(true)]), 0, &paths());
    assert!(!c.shot.as_ref().unwrap().constant_speed);
}

#[test]
fn an_offset_path_counts_towards_done_and_a_new_shot_resets_the_offset() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    c.observe(&call("A1.CineractiveShot", vec![Arg::Entity(EntityId(1)), Arg::Entity(EntityId(1))]), 0, &paths());
    // `CineractiveCameraOffset('RUN', 1.0, true)`: points 1..4 relative to point 0, 3 segments = 3 s.
    c.observe(&call("A1.CineractiveCameraOffset", vec![Arg::Text("RUN".into()), num(1.0), Arg::Bool(true)]), 0, &paths());
    assert!(!c.done_at(3 * HZ - 1));
    assert!(c.done_at(3 * HZ));
    // Relative to an object facing +x: the offset's z (forward) is world +x.
    let at = |_| Some(([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
    c.observe(&call("A1.CineractiveCameraOffset", vec![num(0.0), num(10.0), num(-50.0), Arg::Bool(true)]), 0, &paths());
    let (eye, _) = c.pose(1.0, &at).unwrap();
    assert!(len(sub(eye, [-50.0, 10.0, 0.0])) < 1e-6, "{eye:?}");
    c.observe(&call("A1.CineractiveShot", vec![Arg::Entity(EntityId(1)), Arg::Entity(EntityId(1))]), 0, &paths());
    assert_eq!(c.shot.as_ref().unwrap().offset.what, Offset::Fixed([0.0; 3]));
}

#[test]
fn skip_sets_cancel_until_the_next_start_and_off_stops_the_variables() {
    let mut c = Cineractive::default();
    c.skip();
    assert!(!c.cancel, "no cineractive, nothing to skip");
    c.observe(&on(), 0, &paths());
    c.skip();
    assert_eq!(c.engine_values(1), Some((1, 1)));
    c.observe(&Presentation::Cineractive { on: false, skippable: false }, 2, &paths());
    assert_eq!(c.engine_values(3), None);
    c.observe(&on(), 4, &paths());
    assert!(!c.cancel);
    // A second start while one runs changes nothing.
    c.skip();
    c.observe(&on(), 5, &paths());
    assert!(c.cancel);
}

#[test]
fn mmm_look_from_a_path_looks_along_it_and_look_at_sets_a_target() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    assert!(c.pose(0.0, &|_| None).is_none(), "the camera holds until a shot is placed");
    c.observe(&call("Cineractive.lookFrom", vec![Arg::Text("RUN".into()), num(1.0)]), 0, &paths());
    let (eye, look) = c.pose(HZ as f64, &|_| None).unwrap();
    assert!(len(sub(eye, [100.0, 0.0, 0.0])) < 1e-6);
    // `CineractiveShot(name, speed, true)`: no target, looking along the (fixed, zero) offset
    // track: world +z.
    assert!(len(sub(look, add(eye, [0.0, 0.0, 1.0]))) < 1e-9, "{look:?}");
    // An integer speed is the same float.
    let mut d = Cineractive::default();
    d.observe(&on(), 0, &paths());
    d.observe(&call("Cineractive.lookFrom", vec![Arg::Text("RUN".into()), Arg::Int(1)]), 0, &paths());
    assert_eq!(d.shot, c.shot);
    c.observe(&Presentation::CameraOffset { offset: sta_core::Vec3 { x: Fixed::ZERO, y: Fixed::from_int(100), z: Fixed::ZERO }, relative: true }, 0, &paths());
    c.observe(&call("Cineractive.lookAt", vec![Arg::Entity(EntityId(3))]), HZ, &paths());
    let (eye, look) = c.pose(HZ as f64, &|_| Some(([0.0, 0.0, 900.0], [0.0, 0.0, 1.0]))).unwrap();
    assert!(len(sub(eye, [100.0, 100.0, 0.0])) < 1e-6, "{eye:?}");
    assert_eq!(look, [0.0, 0.0, 900.0]);
    // Members do not restart the path: 4 s in total.
    assert!(c.done_at(4 * HZ));
}

#[test]
fn the_hermite_curve_passes_its_points_and_is_smooth_there() {
    let p = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 0.0, 100.0], [0.0, 50.0, 100.0]];
    for (i, q) in p.iter().enumerate() {
        assert!(len(sub(hermite(&p, i as f64).0, *q)) < 1e-9);
    }
    let (a, b) = (hermite(&p, 1.0 - 1e-6).1, hermite(&p, 1.0 + 1e-6).1);
    assert!(len(sub(norm(a), norm(b))) < 1e-3, "{a:?} {b:?}");
}

#[test]
fn the_same_requests_give_the_same_state() {
    let run = || {
        let mut c = Cineractive::default();
        c.observe(&on(), 3, &paths());
        c.observe(&call("A1.CineractiveShot", vec![Arg::Text("RUN".into()), num(0.8), Arg::Text("Camera_1".into()), Arg::Int(0)]), 9, &paths());
        (c.clone(), c.pose(57.5, &|_| None))
    };
    assert_eq!(run(), run());
}

mod runtime {
    use crate::api::MissionConfig;
    use crate::runtime::MissionRuntime;
    use sta_sim::setup::Controller;
    use sta_sim::view::WorldView;

    /// A converted mission's shape (the prelude of `tools/dsl_to_lua.py`): engine variables
    /// written through `Mission.setEngineValue`, a step held by `CineractiveStart(script, false)`.
    const MISSION: &str = r#"
        local Mission = { engineVars = {
            isCineractiveDone = { value = -1 }, isCineractiveCancel = { value = -1 } } }
        function Mission.setEngineValue(var, v) var.value = v end
        local S = {}
        -- Globals the script never set read as permissive stubs: keep results in a table.
        R = { tick = -1 }
        function Mission:update(t)
            R.tick = R.tick + 1
            local done, cancel = self.engineVars.isCineractiveDone.value, self.engineVars.isCineractiveCancel.value
            if R.tick == 1 then
                A2.CineractiveStart(S, false)
                A2.CineractiveShot('RUN', 1.0, 'Camera_1', 0)
            elseif R.tick > 1 then
                if done == 1 and not R.done then R.done = R.tick end
                if cancel == 1 and not R.cancel then R.cancel = R.tick end
                if A2.ScriptWaits(S) == 0 and not R.released then R.released = R.tick end
                if (R.done or R.cancel) and Cineractive.active then A2.CineractiveEnd() end
            end
        end
        MMM.register(Mission)
    "#;

    fn view(tick: u64) -> WorldView {
        WorldView { tick, ..WorldView::default() }
    }

    fn mission() -> MissionRuntime {
        let cfg = MissionConfig { paths: super::paths(), ..MissionConfig::default() };
        let rt = MissionRuntime::from_source_with("missions/T.lua", MISSION, 1, cfg);
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        rt
    }

    fn get(rt: &MissionRuntime, name: &str) -> Option<i64> {
        rt.eval::<Option<i64>>(&format!("return R.{name}")).unwrap()
    }

    #[test]
    fn the_script_sees_done_when_the_path_ends_and_its_held_step_is_released() {
        let mut rt = mission();
        let mut out = Vec::new();
        for t in 0..200 {
            rt.think(&view(t), &[], &mut out);
        }
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        // Shot on tick 1, RUN has 4 segments at 1/s: done 4 s (120 ticks) later.
        assert_eq!(get(&rt, "done"), Some(121));
        assert_eq!(get(&rt, "released"), Some(121));
        assert_eq!(get(&rt, "cancel"), None);
        assert!(!rt.cineractive().active, "the script ended it");
    }

    #[test]
    fn a_skip_reads_as_cancel_on_the_next_tick() {
        let mut rt = mission();
        let mut out = Vec::new();
        for t in 0..10 {
            rt.think(&view(t), &[], &mut out);
        }
        rt.skip_cineractive();
        for t in 10..20 {
            rt.think(&view(t), &[], &mut out);
        }
        assert_eq!(get(&rt, "cancel"), Some(10));
        assert_eq!(get(&rt, "released"), Some(10));
        assert_eq!(get(&rt, "done"), None);
    }

    #[test]
    fn a_release_writes_the_cancel_and_lets_the_held_step_go() {
        // The same mission with a shot that never ends (speed 0).
        let src = MISSION.replace("A2.CineractiveShot('RUN', 1.0, 'Camera_1', 0)", "A2.CineractiveShot('RUN', 0.0, 'Camera_1', 0)");
        let cfg = MissionConfig { paths: super::paths(), ..MissionConfig::default() };
        let mut rt = MissionRuntime::from_source_with("missions/T.lua", &src, 1, cfg);
        let mut out = Vec::new();
        for t in 0..50 {
            rt.think(&view(t), &[], &mut out);
        }
        assert_eq!(get(&rt, "released"), None, "a speed-0 path never finishes");
        rt.release_cineractive();
        assert!(!rt.cineractive().active);
        for t in 50..55 {
            rt.think(&view(t), &[], &mut out);
        }
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert_eq!(get(&rt, "cancel"), Some(50));
        assert_eq!(get(&rt, "released"), Some(50));
    }

    #[test]
    fn an_mmm_skip_without_hooks_still_ends_a_skippable_cineractive() {
        let src = "MMM.register(function(t) if not rawget(_G, 'STARTED') then rawset(_G, 'STARTED', true) Cineractive.active = true end end)";
        let mut rt = MissionRuntime::from_source("missions/T.lua", src, 1);
        let mut out = Vec::new();
        rt.think(&view(0), &[], &mut out);
        assert!(rt.cineractive().active);
        rt.skip_cineractive();
        rt.think(&view(1), &[], &mut out);
        assert!(!rt.cineractive().active);
    }

    #[test]
    fn a_skip_does_not_end_a_converted_missions_cineractive() {
        let mut rt = mission();
        let mut out = Vec::new();
        for t in 0..5 {
            rt.think(&view(t), &[], &mut out);
        }
        // The script below ends it itself once it reads the cancel; before its update it is on.
        rt.skip_cineractive();
        assert!(rt.cineractive().active);
        rt.think(&view(5), &[], &mut out);
        assert_eq!(get(&rt, "cancel"), Some(5));
    }

    #[test]
    fn an_mmm_skip_ends_a_skippable_cineractive_with_finish_true() {
        let src = r#"
            local M = {}
            R = {}
            function M:setup() Cineractive.active = true; Cineractive:lookFrom(Path('RUN'), 0.1) end
            function M:update(t) end
            function M:cineractiveFinish(forced) R.forced = forced end
            MMM.register(M)
        "#;
        let cfg = MissionConfig { paths: super::paths(), ..MissionConfig::default() };
        let mut rt = MissionRuntime::from_source_with("missions/T.lua", src, 1, cfg);
        let mut out = Vec::new();
        rt.think(&view(0), &[], &mut out);
        assert!(rt.cineractive().active);
        rt.skip_cineractive();
        rt.think(&view(1), &[], &mut out);
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert_eq!(rt.eval::<Option<bool>>("return R.forced").unwrap(), Some(true));
        assert!(!rt.cineractive().active);
    }
}

#[test]
fn the_no_target_bool_picks_the_eye_or_the_offset_direction() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    // `(path, float, false)`: look along the eye path (+x).
    c.observe(&call("A2.CineractiveShot", vec![Arg::Text("RUN".into()), num(1.0), Arg::Bool(false)]), 0, &paths());
    let (eye, look) = c.pose(15.0, &|_| None).unwrap();
    assert!(look[0] > eye[0] && (look[2] - eye[2]).abs() < 1e-9, "{eye:?} {look:?}");
    // `true`: along the offset track; a path offset looks along its own direction.
    c.observe(&call("A2.CineractiveShot", vec![Arg::Text("RUN".into()), num(1.0), Arg::Bool(true)]), 0, &paths());
    c.observe(&call("A2.CineractiveCameraOffset", vec![Arg::Text("UNEVEN".into()), num(1.0), Arg::Bool(false)]), 0, &paths());
    let (eye, look) = c.pose(15.0, &|_| None).unwrap();
    assert!(look[0] > eye[0], "{eye:?} {look:?}");
}

#[test]
fn an_offset_path_follows_the_whole_curve_from_its_second_point() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    c.observe(&call("A1.CineractiveShot", vec![Arg::Text("ONE".into()), Arg::Int(0), Arg::Entity(EntityId(1))]), 0, &paths());
    c.observe(&call("A1.CineractiveCameraOffset", vec![Arg::Text("RUN".into()), num(1.0), Arg::Bool(false)]), 0, &paths());
    // At the start the offset is point 1 minus point 0; at the end, point 4 minus point 0.
    let at = |_| Some(([0.0; 3], [0.0, 0.0, 1.0]));
    assert!(len(sub(c.pose(0.0, &at).unwrap().0, [105.0, 6.0, 7.0])) < 1e-6);
    assert!(len(sub(c.pose((3 * HZ) as f64, &at).unwrap().0, [405.0, 6.0, 7.0])) < 1e-6);
}

#[test]
fn a_release_in_the_same_breath_as_the_end_still_writes_the_cancel_once() {
    let mut c = Cineractive::default();
    c.observe(&on(), 0, &paths());
    c.observe(&call("A2.CineractiveShot", vec![Arg::Text("RUN".into()), num(0.0), Arg::Bool(false)]), 1, &paths());
    c.skip();
    c.observe(&Presentation::Cineractive { on: false, skippable: true }, 5, &paths());
    assert_eq!(c.take_engine_values(6), Some((0, 1)));
    assert_eq!(c.take_engine_values(7), None);
}
