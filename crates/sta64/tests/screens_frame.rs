//! UI-07: the front-end screens in real frames.
//!
//! `STA64_UI_SCREEN=<name>` draws a screen over the bench scene
//! `bench/scenes/hud.ini` on the fake content (`frontend/screens/frame.rs`);
//! this looks for the LCARS theme's colours where the screen stands, and for
//! the bare scene everywhere else.
//!
//! Needs a real GPU, so it is opt-in like the other frame tests: it prints
//! `SKIP` and passes unless `STA64_GPU_TESTS=1`; `sta64` exits 77 without a
//! hardware adapter and the test skips.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn sta64() -> &'static str {
    env!("CARGO_BIN_EXE_sta64")
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-screens-frame-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const W: usize = 1280;
const H: usize = 720;

struct Shot {
    px: Vec<[u8; 4]>,
    png: Vec<u8>,
    out: Output,
}

/// `None` when the test should skip.
fn shoot(name: &str, screen: Option<&str>) -> Option<Shot> {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return None;
    }
    let dir = tmp(name);
    let mut c = Command::new(sta64());
    c.args(["--headless", "--clock", "bench", "--frames", "0", "--frame-size", &format!("{W}x{H}"), "--frame-dir"])
        .arg(&dir)
        .arg("--scene")
        .arg(root().join("bench/scenes/hud.ini"));
    for k in ["STA64_UI_HUD", "STA64_UI_RADAR", "STA64_UI_SCREEN"] {
        c.env_remove(k);
    }
    if let Some(s) = screen {
        c.env("STA64_UI_SCREEN", s);
    }
    let out = c.output().unwrap();
    match out.status.code() {
        Some(0) => {}
        Some(77) => {
            eprintln!("SKIP: no GPU adapter for headless frames: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
            return None;
        }
        c => panic!("headless frame run failed ({c:?}): {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    }
    let png = std::fs::read(dir.join("tick-000000.png")).expect("the frame");
    let mut r = png::Decoder::new(std::io::Cursor::new(&png[..])).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.width as usize, info.height as usize), (W, H));
    let n = info.color_type.samples();
    let px = buf.chunks(n).map(|p| [p[0], p[1], p[2], if n == 4 { p[3] } else { 255 }]).collect();
    Some(Shot { px, png, out })
}

type Area = (usize, usize, usize, usize);

/// Pixels within `tol` of an `0xRRGGBB` colour in a rectangle (x0, y0, x1, y1).
fn near(s: &Shot, rgb: u32, tol: i32, a: Area) -> usize {
    let want = [(rgb >> 16) as i32, (rgb >> 8 & 255) as i32, (rgb & 255) as i32];
    let mut n = 0;
    for y in a.1..a.3.min(H) {
        for x in a.0..a.2.min(W) {
            let p = s.px[y * W + x];
            n += usize::from((0..3).all(|i| (i32::from(p[i]) - want[i]).abs() <= tol));
        }
    }
    n
}

/// Pixels that differ between two shots in a rectangle.
fn changed(a: &Shot, b: &Shot, area: Area) -> usize {
    let mut n = 0;
    for y in area.1..area.3.min(H) {
        for x in area.0..area.2.min(W) {
            n += usize::from(a.px[y * W + x] != b.px[y * W + x]);
        }
    }
    n
}

fn errors(out: &Output) -> Vec<String> {
    let all = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    all.lines().filter(|l| l.contains("ERROR") || l.contains("panicked")).map(str::to_owned).collect()
}

fn screen_line(out: &Output) -> Option<String> {
    String::from_utf8_lossy(&out.stdout).lines().find(|l| l.starts_with("screen:")).map(str::to_owned)
}

// Theme colours (`ui/theme/tokens.rs`).
const GOLD: u32 = 0xf4b84c; // title, selection, the bottom bars
const LAVENDER: u32 = 0x9c9cde; // buttons, the frame's elbows

/// The Instant Action panel is 960 units wide of a 1920-unit screen: the
/// middle half of the frame, whatever the scene is behind it.
const PANEL: Area = (W / 4 - 4, 0, W * 3 / 4 + 4, H);

#[test]
fn instant_action_is_drawn_in_the_middle_of_the_frame() {
    let Some(bare) = shoot("bare", None) else { return };
    let Some(a) = shoot("setup", Some("setup")) else { return };
    assert!(errors(&a.out).is_empty(), "shader or pipeline errors: {:?}", errors(&a.out));
    assert_eq!(screen_line(&a.out).as_deref(), Some("screen: Setup"));
    let Some(a2) = shoot("setup2", Some("setup")) else { return };
    assert_eq!(a.png, a2.png, "the screen's frame must repeat byte for byte");
    assert_ne!(a.png, bare.png, "the screen must change the frame");

    // Buttons and elbows in lavender, the title and the bars in gold; neither colour is in the bare scene.
    let (lav, gold) = (near(&a, LAVENDER, 10, PANEL), near(&a, GOLD, 10, PANEL));
    eprintln!("panel: lavender {lav} px, gold {gold} px");
    assert!(lav > 8000, "lavender {lav}");
    assert!(gold > 300, "gold {gold}");
    assert_eq!(near(&bare, LAVENDER, 10, PANEL), 0);
    assert_eq!(near(&bare, GOLD, 10, PANEL), 0);
    // The screen clears nothing of its own: the scene shows either side of the panel.
    assert_eq!(changed(&a, &bare, (0, 0, W / 4 - 8, H)), 0);
    assert_eq!(changed(&a, &bare, (W * 3 / 4 + 8, 0, W, H)), 0);
}

#[test]
fn each_screen_has_its_own_frame() {
    let Some(setup) = shoot("each-setup", Some("setup")) else { return };
    let mut pngs = vec![setup.png.clone()];
    for (name, expect) in [("main", "Main"), ("single", "SinglePlayer"), ("map", "MapSelect"), ("settings", "Settings"), ("mods", "ModSelect")] {
        let Some(s) = shoot(&format!("each-{name}"), Some(name)) else { return };
        assert!(errors(&s.out).is_empty(), "{name}: {:?}", errors(&s.out));
        assert_eq!(screen_line(&s.out), Some(format!("screen: {expect}")), "{name}");
        assert!(near(&s, LAVENDER, 10, (0, 0, W, H)) > 2000, "{name}: no lavender");
        assert!(near(&s, GOLD, 10, (0, 0, W, H)) > 400, "{name}: no gold");
        assert!(!pngs.contains(&s.png), "{name} looks like another screen");
        pngs.push(s.png);
    }
}
