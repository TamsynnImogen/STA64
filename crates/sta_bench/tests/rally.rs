//! Real fake-content bench acceptance of point, entity, and clear rally commands.

use std::sync::Arc;

use sta_bench::parse::{SceneFile, parse_command};
use sta_bench::run::{self, Bench, RunConfig};

#[test]
fn point_and_clear_scenario_passes() {
    let cfg = Arc::new(RunConfig::fake());
    let path = cfg.scenario_dir.join("rally.md");
    let result = run::run_scenario_file(&cfg, &path).expect("rally scenario parses");
    assert!(result.passed, "{}", result.summary());
}

#[test]
fn entity_rally_resolves_position_when_ship_launches() {
    let cfg = Arc::new(RunConfig::fake());
    let scene = SceneFile::parse(
        "[Scene]\nMapObjects=strip\n[Object.yard]\nOdf=fx_yard\nTeam=1\nX=0\nY=0\nZ=0\n[Object.marker]\nOdf=fx_frigate\nTeam=1\nX=1000\nY=0\nZ=0\n",
    ).expect("scene parses");
    let mut bench = Bench::build(cfg, &scene).expect("scene builds");
    let cargo = parse_command("cargo yard dilithium 1000").unwrap().unwrap();
    assert_eq!(bench.exec(&cargo.command), "ok");
    assert!(bench.advance()); // Bench edits land after player orders in a tick.
    for line in [
        "rally yard marker",
        "goto marker 2000 0 0",
        "build yard fx_frigate 0 0 0",
    ] {
        let cmd = parse_command(line).unwrap().unwrap();
        assert_eq!(bench.exec(&cmd.command), "ok", "{line}");
    }
    while bench.tick() < 450 {
        assert!(bench.advance());
    }
    let marker = bench.resolve("marker").unwrap();
    let marker_pos = bench.view().unit(marker).unwrap().pos.x.to_f32();
    assert!(
        marker_pos > 1700.0,
        "marker did not move before launch: {marker_pos}"
    );
    while bench.tick() < 900 {
        assert!(bench.advance());
    }
    let launched = bench.resolve("#3").unwrap();
    let ship = bench.view().unit(launched).expect("launched frigate");
    assert!(
        ship.pos.x.to_f32() > 1700.0,
        "ship followed stale target position: {:?}",
        ship.pos
    );
}
