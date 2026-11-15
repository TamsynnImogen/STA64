//! Read-only MMM path queries against authored map geometry (ADR 0004 D13).

use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;

fn geometry() -> Vec<(String, Vec<[f32; 3]>)> {
    vec![
        ("FlyBy".into(), vec![[12.5, -2.0, 30.0], [40.0, 5.0, -8.0]]),
        ("EmptyRoute".into(), vec![]),
    ]
}

fn runtime(source: &str) -> MissionRuntime {
    let rt = MissionRuntime::from_source_with(
        "authored-path-queries.lua",
        source,
        17,
        MissionConfig { paths: geometry(), ..MissionConfig::default() },
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn clean(rt: &mut MissionRuntime) {
    assert!(rt.take_commands().is_empty(), "path queries queued commands");
    assert!(rt.unsupported().is_empty(), "path queries registered stubs: {:?}", rt.unsupported());
}

#[test]
fn existence_count_and_points_follow_authored_geometry() {
    let mut rt = runtime("");
    let found: (bool, bool, bool, bool) = rt
        .eval("return Path.exists('FlyBy'), Path.exists('flyby'), Path.exists('EMPTYROUTE'), Path.exists('missing')")
        .unwrap();
    assert_eq!(found, (true, true, true, false));
    let counts: (usize, usize, usize) = rt
        .eval("return Path('FLYBY').count, Path('emptyroute').count, Path('missing').count")
        .unwrap();
    assert_eq!(counts, (2, 0, 0));
    let points: (usize, f64, f64, f64, f64, f64, f64, f64) = rt
        .eval(
            r#"
            local p = Path('fLyBy').points
            local doubled = p[1] + p[1]
            assert(p[1]:distance(Vector(12.5, -2, 30)) == 0)
            assert(p[0] == nil and p[3] == nil)
            return #p, p[1].x, p[1].y, p[1].z,
                   p[2].x, p[2].y, p[2].z, doubled.x
            "#,
        )
        .unwrap();
    assert_eq!(points, (2, 12.5, -2.0, 30.0, 40.0, 5.0, -8.0, 25.0));
    assert_eq!(rt.eval::<String>("return Path('fLyBy').name").unwrap(), "FlyBy");
    let empty: (usize, usize) = rt.eval("return #Path('EmptyRoute').points, #Path('missing').points").unwrap();
    assert_eq!(empty, (0, 0));
    clean(&mut rt);
}

#[test]
fn point_tables_are_copies_and_instances_are_isolated() {
    let mut first = runtime("");
    first
        .eval::<()>(
            r#"
            local points = Path('FlyBy').points
            points[1].x = 900
            points[2] = Vector(0, 0, 0)
            points[3] = Vector(1, 1, 1)
            "#,
        )
        .unwrap();
    let fresh: (usize, f64, f64, f64) = first
        .eval("local p = Path('flyby').points; return #p, p[1].x, p[2].x, p[2].z")
        .unwrap();
    assert_eq!(fresh, (2, 12.5, 40.0, -8.0));
    clean(&mut first);

    let mut second = runtime("");
    let independent: (usize, f64) = second.eval("local p = Path('FLYBY').points; return #p, p[1].x").unwrap();
    assert_eq!(independent, (2, 12.5));
    clean(&mut second);
}

#[test]
fn optional_path_pattern_does_not_create_an_absent_path() {
    let mut rt = runtime(
        r#"
        local optionalPath = 'optional_cineractive'
        if Path.exists(optionalPath) then
            Chosen = Path(optionalPath).points[1]
        else
            Chosen = Path('FlyBy').points[1]
        end
        Absent = Path('optional_cineractive')
        "#,
    );
    let result: (bool, usize, f64, bool) = rt
        .eval("return Path.exists('optional_cineractive'), Absent.count, Chosen.x, Path.exists('FlyBy')")
        .unwrap();
    assert_eq!(result, (false, 0, 12.5, true));
    clean(&mut rt);
}

#[test]
fn exists_requires_exactly_one_nonempty_utf8_string() {
    let mut rt = runtime("");
    for expression in [
        "Path.exists()",
        "Path.exists('FlyBy', 'extra')",
        "Path.exists('')",
        "Path.exists(nil)",
        "Path.exists(42)",
        "Path.exists(true)",
        "Path.exists({})",
        "Path.exists(string.char(255))",
    ] {
        assert!(rt.eval::<bool>(&format!("return {expression}")).is_err(), "accepted {expression}");
    }
    assert!(rt.eval::<bool>("return Path.exists('FlyBy')").unwrap());
    clean(&mut rt);
}

#[test]
fn mutation_methods_remain_unsupported() {
    let mut rt = runtime("");
    rt.eval::<()>("Path('FlyBy'):remove(1)").unwrap();
    assert_eq!(rt.unsupported(), [("Path.remove".into(), 1)]);
    assert!(rt.take_commands().is_empty());
    assert!(rt.eval::<bool>("return Path.exists('FlyBy')").unwrap());
}

#[test]
fn duplicate_names_use_the_first_authored_path_and_empty_config_is_absent() {
    let mut duplicate = MissionRuntime::from_source_with(
        "duplicate-paths.lua",
        "",
        17,
        MissionConfig {
            paths: vec![("FIRST".into(), vec![[1.0, 2.0, 3.0]]), ("first".into(), vec![[9.0, 8.0, 7.0]])],
            ..MissionConfig::default()
        },
    );
    let first: (bool, String, usize, f64) = duplicate
        .eval("local p=Path('first'); return Path.exists('FiRsT'), p.name, p.count, p.points[1].x")
        .unwrap();
    assert_eq!(first, (true, "FIRST".into(), 1, 1.0));
    clean(&mut duplicate);

    let mut empty = MissionRuntime::from_source_with("no-paths.lua", "", 17, MissionConfig::default());
    let absent: (bool, usize, usize) = empty
        .eval("return Path.exists('FIRST'), Path('FIRST').count, #Path('FIRST').points")
        .unwrap();
    assert_eq!(absent, (false, 0, 0));
    clean(&mut empty);
}

#[test]
fn mixed_case_handle_reaches_the_camera_with_its_authored_label() {
    use sta_mission::cineractive::{Cineractive, Track};
    use sta_sim::command::{CommandKind, Edit};
    use sta_sim::present::Presentation;

    let mut rt = runtime("Cineractive.active=true; Cineractive:lookFrom(Path('fLyBy'), 1)");
    let mut camera = Cineractive::default();
    for command in rt.take_commands() {
        let CommandKind::Edit(Edit::Present(presentation)) = command.kind else {
            panic!("camera queued a non-presentation command");
        };
        camera.observe(&presentation, 0, &geometry());
        if let Presentation::Call { name, args } = &presentation {
            assert_eq!(name, "Cineractive.lookFrom");
            assert!(matches!(&args[0], sta_sim::present::Arg::Text(name) if name == "FlyBy"));
        }
    }
    let shot = camera.shot.expect("authored path resolved by the camera");
    let Track::Path(track) = shot.eye.what else {
        panic!("camera did not retain the path track");
    };
    assert_eq!(track.points, [[12.5, -2.0, 30.0], [40.0, 5.0, -8.0]]);
    assert!(rt.unsupported().is_empty());
}

#[test]
#[ignore = "requires STA64_A1_PREMONITIONS pointing to an external, unshipped mission script"]
fn installed_premonitions_optional_path_helper_uses_authored_geometry() {
    let source_path = std::env::var("STA64_A1_PREMONITIONS").expect("set STA64_A1_PREMONITIONS");
    let source = std::fs::read_to_string(source_path).expect("read external mission script");
    let start = source.find("local function optionalPath(").expect("optionalPath helper");
    let after = &source[start..];
    let end = after.find("\nlocal function ").expect("next local function after optionalPath");
    let helper = &after[..end];
    let script = format!("{helper}\nOptionalPathProbe = optionalPath\n");
    let mut rt = MissionRuntime::from_source_with(
        "external-optional-path-helper.lua",
        &script,
        17,
        MissionConfig {
            paths: vec![("AuthoredRoute".into(), vec![[4.0, 5.0, 6.0]]), ("EMPTY".into(), vec![])],
            ..MissionConfig::default()
        },
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let selected: (bool, usize, f64, f64, f64, bool, bool) = rt
        .eval(
            r#"
            local p = OptionalPathProbe('authoredroute')
            return p ~= nil, p.count, p.points[1].x, p.points[1].y, p.points[1].z,
                   OptionalPathProbe('EMPTY') == nil, OptionalPathProbe('absent') == nil
            "#,
        )
        .unwrap();
    assert_eq!(selected, (true, 1, 4.0, 5.0, 6.0, true, true));
    clean(&mut rt);
}
