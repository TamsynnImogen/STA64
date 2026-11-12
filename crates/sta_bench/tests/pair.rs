//! Paired scenes (TOOL-15) without any install: a synthetic original-game run
//! (refit.json, `Scene.marks` with `t=`, a tiny frame) against STA64 on the
//! bench's fake content, through the library and through `sta-bench pair`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_bench::pair::{self, Image, PairDesc};
use sta_bench::run::RunConfig;

fn pairs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/pairs")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-pair-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// A made-up original run of `move.ini`: the ship starts 0.4 s after its
/// order and reaches 1200 units ahead; the turner never moves.
fn synthetic_refit(dir: &Path) {
    fs::create_dir_all(dir.join("frames")).unwrap();
    let mut marks = String::from("epoch 0x1 0x2 1000\n1000 ready t=0.800 objects=2\n1200 cmd t=2.000 text=goto ship 2000 0 3200\n1300 cmd t=2.100 text=goto turner 3200 0 2000\n");
    for i in 0..=100 {
        let t = 0.8 + f64::from(i) * 0.25;
        let rel = t - 0.8;
        let z = if rel < 1.6 { 2000.0 } else { (2000.0 + (rel - 1.6) * 60.0).min(3200.0) };
        marks += &format!("{} state t={t:.3} obj=ship pos=2000.0,0.0,{z:.1} hull=2000.0 sh=1500.0 sp=100.0 q=0\n", 1000 + i * 250);
        marks += &format!("{} state t={t:.3} obj=turner pos=2400.0,0.0,2000.0 hull=500.0 sh=400.0 sp=0.0 q=0\n", 1000 + i * 250);
    }
    fs::write(dir.join("Scene.marks"), marks).unwrap();
    fs::write(dir.join("Scene.objects.jsonl"), "{\"header\":true,\"count\":0,\"tick\":0,\"time\":25.0}\n").unwrap();
    let img = Image { w: 8, h: 6, rgb: (0..8 * 6 * 3).map(|i| (i * 5 % 256) as u8).collect() };
    pair::write_png(&dir.join("frames/f0.png"), &img).unwrap();
    fs::write(
        dir.join("refit.json"),
        r#"{"schema":1,"game":"a2","session":"synthetic","launch":["-nointro","a2_borg01"],"refit_root":"-","refit_commit":"0000000000","size":[8,6],
 "ready_t":0.8,"duration":25.0,"end_t":24.9,
 "commands":[{"at":1.0,"text":"goto ship 2000 0 3200","landed_t":1.2,"answer":"ok"},{"at":1.0,"text":"goto turner 3200 0 2000","landed_t":1.3,"answer":"ok"}],
 "frames":[{"at":12.0,"t":11.9,"tick":700,"file":"frames/f0.png","objects":null}],
 "notes":["synthetic"]}"#,
    )
    .unwrap();
}

fn sta64_side(desc: &PairDesc, refit: &Path, sta: &Path) {
    let meta = pair::read_meta(refit, "refit.json").unwrap();
    let sched = pair::schedule(desc, Some(&meta));
    assert_eq!(sched.iter().map(|c| c.t).collect::<Vec<_>>(), [1.2, 1.3], "commands on the original's landing times");
    let run = pair::run_sta64(&desc.scene_text(&["fake"]), Arc::new(RunConfig::fake()), &sched, 24.9, sta).unwrap();
    assert_eq!(run.ticks, pair::tick_of(24.9));
    assert!(run.answers.iter().all(|a| !a.2.starts_with('!')), "{:?}", run.answers);
    // A stand-in frame, as `sta64 --headless` would leave it (no GPU in CI).
    fs::create_dir_all(sta.join("frames")).unwrap();
    let times = pair::frame_times(desc, Some(&meta));
    assert_eq!(times, [11.9]);
    let tick = pair::tick_of(times[0]);
    let rel = format!("frames/tick-{tick:06}.png");
    pair::write_png(&sta.join(&rel), &Image { w: 4, h: 3, rgb: vec![40; 4 * 3 * 3] }).unwrap();
    fs::write(sta.join("run.json"), pair::sta64_run_json("fake", &run, &[(times[0], tick, Some(rel))], "synthetic frame")).unwrap();
}

#[test]
fn a_pair_compares_and_repeats() {
    let desc = PairDesc::load(&pairs().join("move.ini"), "a2").unwrap();
    let root = tmp("lib");
    let refit = root.join("refit");
    synthetic_refit(&refit);
    let sta = root.join("sta64");
    sta64_side(&desc, &refit, &sta);

    let marks = fs::read_to_string(sta.join("Scene.marks")).unwrap();
    assert!(marks.lines().next().unwrap().contains("ready t=0.000"));
    assert!(marks.contains("state t=") && marks.contains("obj=ship") && marks.contains("cmd t=1.200 text=goto ship"));

    let a = pair::compare(&desc, &refit, &sta, &root.join("a")).unwrap();
    let b = pair::compare(&desc, &refit, &sta, &root.join("b")).unwrap();
    assert_eq!(a.json, b.json, "a report is reproducible from the two directories");
    let j = pair::parse_json(&a.json).expect("report.json is JSON");
    assert_eq!(j.get("pair").str(), Some("move"));
    let states = j.get("states").arr();
    let ship = states.iter().find(|s| s.get("obj").str() == Some("ship")).expect("ship series");
    assert!(ship.get("samples").num().unwrap() > 90.0);
    let m = ship.get("motion");
    assert!((m.get("refit").get("start").num().unwrap() - 1.75).abs() < 0.01, "{}", a.json);
    assert!(m.get("sta64").get("start").num().is_some(), "STA64's ship moves too");
    assert!(m.get("sta64").get("top_speed").num().unwrap() > 0.0);
    let f = &j.get("frames").arr()[0];
    assert_eq!(f.get("size").arr().len(), 2);
    assert!(f.get("mean").num().is_some() && f.get("within").eq(&pair::Jv::Null));
    assert!(root.join("a/frames/f0.png").is_file());
    let side = pair::read_png(&root.join("a/frames/f0.png")).unwrap();
    assert_eq!((side.w, side.h), (24, 6));
    assert!(a.md.contains("## State series") && a.md.contains("synthetic"));
    assert_eq!(j.get("out_of_tolerance").num(), Some(a.out_of_tolerance as f64));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_cli_runs_a_pair_on_fake_content() {
    let root = tmp("cli");
    let refit = root.join("refit");
    synthetic_refit(&refit);
    let out = root.join("out");
    let args = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = ["pair", "move", "--game", "a2", "--fake", "--no-frames", "--refit-out"].map(String::from).into();
        v.push(refit.display().to_string());
        v.extend(["--out".into(), out.display().to_string()]);
        v.extend(extra.iter().map(|s| (*s).to_owned()));
        v
    };
    let code = sta_bench::cli::main(args(&[]));
    assert!(code == 0 || code == 1, "exit {code}");
    let first = fs::read_to_string(out.join("report.json")).unwrap();
    assert!(first.contains("skipped (--no-frames)"));
    assert_eq!(sta_bench::cli::main(args(&[])), code);
    assert_eq!(fs::read_to_string(out.join("report.json")).unwrap(), first, "same inputs, same report");
    // Usage errors.
    assert_eq!(sta_bench::cli::main(vec!["pair".into(), "move".into(), "--game".into(), "a1".into()]), 2);
    assert_eq!(sta_bench::cli::main(vec!["pair".into(), "move".into()]), 2);
    let inside = pairs().join("../../target/pair-out");
    let mut a = args(&[]);
    let i = a.iter().position(|x| x == "--out").unwrap();
    a[i + 1] = inside.display().to_string();
    assert_eq!(sta_bench::cli::main(a), 2, "refuses a report inside the checkout");
    let _ = fs::remove_dir_all(&root);
}
