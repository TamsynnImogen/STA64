//! UI-06: the front end's menu scene, run on its own (`STA64_MENU_SCENE=1 sta64 --headless`).
//!
//! - The headless run needs no GPU and no install: the scene is built on the fake content and
//!   `two_bases()`, steps its sim for a number of ticks and writes its marks to
//!   `$STA64_BENCH_OUT/marks`, in the bench's file format, as `render.menu.*` kinds.
//! - The frame tests need a real GPU, so they are opt-in like the other frame tests: they
//!   print `SKIP` and pass unless `STA64_GPU_TESTS=1`, and `sta64` exits 77 without a
//!   hardware adapter. With `STA64_TEST_INSTALL` (and optionally `STA64_TEST_MOD`) set, one
//!   more frame test draws the scene over the install: its content, models and sky.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn sta64() -> &'static str {
    env!("CARGO_BIN_EXE_sta64")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-menu-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn base() -> Command {
    let mut c = Command::new(sta64());
    for k in ["STA64_MENU_VIEW", "STA64_MENU_VIEWS", "STA64_MENU_DEPART", "STA64_BENCH_OUT", "STA64_UI_HUD", "STA64_UI_RADAR"] {
        c.env_remove(k);
    }
    c.env("STA64_MENU_SCENE", "1").args(["--headless", "--clock", "bench"]);
    c
}

fn marks(out: &Path) -> Vec<(u64, String)> {
    std::fs::read_to_string(out.join("marks"))
        .expect("the marks file")
        .lines()
        .map(|l| {
            let (tick, rest) = l.trim_start().split_once(' ').unwrap();
            (tick.parse().unwrap(), rest.to_owned())
        })
        .collect()
}

#[test]
fn the_scene_loads_headless_and_marks_view_switches_decloaks_and_the_departure() {
    let out = tmp("marks");
    let run = |dir: &Path| {
        base()
            .args(["--ticks", "600"])
            .env("STA64_BENCH_OUT", dir)
            .env("STA64_MENU_VIEWS", "klingon@2,borg@10")
            .env("STA64_MENU_DEPART", "14")
            .output()
            .unwrap()
    };
    let o = run(&out);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let m = marks(&out);
    let text: Vec<&str> = m.iter().map(|(_, l)| l.as_str()).collect();
    let views: Vec<&&str> = text.iter().filter(|l| l.starts_with("render.menu.view")).collect();
    assert_eq!(views, [&"render.menu.view view=main from=-", &"render.menu.view view=klingon from=main", &"render.menu.view view=borg from=klingon"], "{text:?}");
    // The switch to klingon was asked for at 2 s: tick 60.
    let at = m.iter().find(|(_, l)| l.contains("view=klingon")).unwrap().0;
    assert!((60..=62).contains(&at), "{at}");
    // Five Klingon ships decloak, one after the other, once the camera has arrived.
    let starts: Vec<&(u64, String)> = m.iter().filter(|(_, l)| l.starts_with("render.menu.decloak") && l.contains("state=start")).collect();
    assert_eq!(starts.len(), 5, "{text:?}");
    assert!(starts[0].0 > 60 && starts.windows(2).all(|w| w[0].0 < w[1].0), "{starts:?}");
    assert!(text.contains(&"render.menu.warp state=start view=borg"), "{text:?}");
    assert!(text.contains(&"render.menu.warp state=tunnel"));
    // The same run again writes the same file.
    let again = tmp("marks-again");
    assert!(run(&again).status.success());
    assert_eq!(std::fs::read_to_string(out.join("marks")).unwrap(), std::fs::read_to_string(again.join("marks")).unwrap());
    let _ = std::fs::remove_dir_all(out);
    let _ = std::fs::remove_dir_all(again);
}

struct Shot {
    px: Vec<[u8; 4]>,
    hash: u64,
}

/// The PNG's pixels and an FNV hash of them.
fn read(path: &Path, size: (usize, usize)) -> Shot {
    let png = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut r = png::Decoder::new(std::io::Cursor::new(&png[..])).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.width as usize, info.height as usize), size);
    let n = info.color_type.samples();
    let px: Vec<[u8; 4]> = buf.chunks(n).map(|p| [p[0], p[1], p[2], if n == 4 { p[3] } else { 255 }]).collect();
    let hash = buf.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    Shot { px, hash }
}

fn lit(s: &Shot) -> usize {
    s.px.iter().filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60).count()
}

fn differing(a: &Shot, b: &Shot) -> usize {
    a.px.iter().zip(&b.px).filter(|(p, q)| (0..3).any(|i| (p[i] as i32 - q[i] as i32).abs() > 24)).count()
}

/// `None` when the test should skip.
fn shoot(name: &str, install: Option<&str>, size: (usize, usize)) -> Option<(PathBuf, Output)> {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return None;
    }
    let dir = tmp(name);
    let mut c = base();
    c.args(["--frames", "150,600", "--frame-size", &format!("{}x{}", size.0, size.1), "--frame-dir"])
        .arg(&dir)
        .env("STA64_BENCH_OUT", &dir)
        // At 6 s the camera goes to the Klingons; by 20 s the last ship has decloaked.
        .env("STA64_MENU_VIEWS", "klingon@6");
    if let Some(root) = install {
        c.arg("--install-root").arg(root);
        if let Ok(m) = std::env::var("STA64_TEST_MOD")
            && !m.is_empty()
        {
            c.arg("--mod").arg(m);
        }
    }
    let out = c.output().unwrap();
    match out.status.code() {
        Some(0) => Some((dir, out)),
        Some(77) => {
            eprintln!("SKIP: no GPU adapter for headless frames: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
            None
        }
        c => panic!("headless frame run failed ({c:?}): {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    }
}

const SIZE: (usize, usize) = (960, 540);

fn check_frames(dir: &Path) -> (Shot, Shot) {
    let main = read(&dir.join("tick-000150.png"), SIZE);
    let klingon = read(&dir.join("tick-000600.png"), SIZE);
    let total = SIZE.0 * SIZE.1;
    // The main view: a lit planet and stars, not a black frame.
    assert!(lit(&main) > total / 20, "the main view is mostly black: {} of {total} lit", lit(&main));
    assert!(lit(&klingon) > total / 100, "the Klingon view is black");
    // The camera has moved to another place on the map.
    assert!(differing(&main, &klingon) > total / 10, "the two views are the same picture ({} pixels differ)", differing(&main, &klingon));
    let m = marks(dir);
    assert!(m.iter().any(|(t, l)| (180..=182).contains(t) && l == "render.menu.view view=klingon from=main"), "{m:?}");
    (main, klingon)
}

#[test]
fn frames_of_the_scene_show_the_main_view_and_the_klingon_view() {
    let Some((dir, out)) = shoot("fake", None, SIZE) else { return };
    let (main, klingon) = check_frames(&dir);
    println!("{}", String::from_utf8_lossy(&out.stdout));
    println!("main view: {} of {} pixels lit, klingon view: {} lit, {} differ", lit(&main), SIZE.0 * SIZE.1, lit(&klingon), differing(&main, &klingon));
    // The same run draws the same pictures.
    let Some((again, _)) = shoot("fake-again", None, SIZE) else { return };
    let (m2, k2) = check_frames(&again);
    assert_eq!((main.hash, klingon.hash), (m2.hash, k2.hash), "the frames are not repeatable");
    if let Ok(keep) = std::env::var("STA64_GPU_OUT") {
        let _ = std::fs::create_dir_all(&keep);
        for f in ["tick-000150.png", "tick-000600.png"] {
            let _ = std::fs::copy(dir.join(f), Path::new(&keep).join(format!("menu-fake-{f}")));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(again);
}

/// The scene over an install: its models, its sky and its planet. Needs `STA64_TEST_INSTALL`.
#[test]
fn frames_of_the_scene_over_the_install() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let Some((dir, _)) = shoot("install", Some(&root), SIZE) else { return };
    let (main, klingon) = check_frames(&dir);
    println!("install: main view {} lit, klingon view {} lit", lit(&main), lit(&klingon));
    if let Ok(keep) = std::env::var("STA64_GPU_OUT") {
        let _ = std::fs::create_dir_all(&keep);
        for f in ["tick-000150.png", "tick-000600.png"] {
            let _ = std::fs::copy(dir.join(f), Path::new(&keep).join(format!("menu-install-{f}")));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
