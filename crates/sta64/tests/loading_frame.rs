//! UI-08: the loading screen in real frames.
//!
//! `STA64_UI_LOADING=1` draws the screen over the bench scene `bench/scenes/hud.ini`
//! (the title and sky of the fake `two_bases()` map) with a scripted load,
//! `STA64_UI_LOADING_AT=<stage>:<done>/<total>`. The run prints a `loading layout:`
//! line and a `mark` line per progress mark; the checks read the bar's rectangle
//! from the line and look for the theme's colours in it.
//!
//! The sky is checked with a data directory of six synthetic faces
//! (`textures/fx_sky0.png` .. `5.png`, `STA64_UI_LOADING_SKY`), so no install is
//! needed.
//!
//! Needs a real GPU, so it is opt-in like the other frame tests: it prints
//! `SKIP` and passes unless `STA64_GPU_TESTS=1`; `sta64` exits 77 without a
//! hardware adapter and the test skips.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-loading-frame-{name}-{}", std::process::id()));
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

/// Six faces of one deep blue, brighter toward the top, as PNGs under `textures/`.
fn sky_dir() -> PathBuf {
    let dir = tmp("faces");
    std::fs::create_dir_all(dir.join("textures")).unwrap();
    for i in 0..6 {
        let mut data = Vec::new();
        for y in 0..64u32 {
            for x in 0..64u32 {
                let k = 1.0 - y as f32 / 64.0 * 0.5;
                data.extend_from_slice(&[(40.0 * k) as u8, (90.0 * k) as u8, (200.0 * k) as u8 + (x % 3) as u8]);
            }
        }
        let file = std::fs::File::create(dir.join(format!("textures/fx_sky{i}.png"))).unwrap();
        let mut enc = png::Encoder::new(file, 64, 64);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&data).unwrap();
    }
    dir
}

/// `None` when the test should skip.
fn shoot(name: &str, at: &str, sky: Option<&Path>) -> Option<Shot> {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return None;
    }
    let dir = tmp(name);
    let mut c = Command::new(env!("CARGO_BIN_EXE_sta64"));
    c.args(["--headless", "--clock", "bench", "--frames", "0", "--frame-size", &format!("{W}x{H}"), "--frame-dir"])
        .arg(&dir)
        .arg("--scene")
        .arg(root().join("bench/scenes/hud.ini"))
        .env("STA64_UI_LOADING", "1")
        .env("STA64_UI_LOADING_AT", at)
        .env("STA64_BENCH_OUT", dir.join("out"));
    for k in ["STA64_UI_HUD", "STA64_UI_RADAR", "STA64_UI_LOADING_SKY"] {
        c.env_remove(k);
    }
    if let Some(sky) = sky {
        c.env("STA64_UI_LOADING_SKY", sky);
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
            if (0..3).all(|i| (i32::from(p[i]) - want[i]).abs() <= tol) {
                n += 1;
            }
        }
    }
    n
}

/// Mean of each colour channel in a rectangle.
fn mean(s: &Shot, a: Area) -> [f32; 3] {
    let (mut sum, mut n) = ([0.0f32; 3], 0.0);
    for y in a.1..a.3.min(H) {
        for x in a.0..a.2.min(W) {
            for (i, v) in sum.iter_mut().enumerate() {
                *v += f32::from(s.px[y * W + x][i]);
            }
            n += 1.0;
        }
    }
    sum.map(|v| v / n)
}

fn errors(out: &Output) -> Vec<String> {
    let all = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    all.lines().filter(|l| l.contains("ERROR") || l.contains("panicked")).map(str::to_owned).collect()
}

/// The last `loading layout:` line, as words: `key=value`.
struct Layout(BTreeMap<String, String>);

impl Layout {
    fn of(out: &Output) -> Layout {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().rfind(|l| l.starts_with("loading layout:")).expect("a `loading layout:` line");
        Layout(line.trim_start_matches("loading layout:").split_whitespace().map(|w| w.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())).unwrap()).collect())
    }

    fn get(&self, k: &str) -> &str {
        &self.0[k]
    }

    /// A rectangle in pixels of the frame (units are of a 1080-line screen).
    fn area(&self, k: &str) -> Area {
        let v: Vec<f32> = self.get(k).split(',').map(|n| n.parse().unwrap()).collect();
        let f = H as f32 / 1080.0;
        ((v[0] * f).round() as usize, (v[1] * f).round() as usize, ((v[0] + v[2]) * f).round() as usize, ((v[1] + v[3]) * f).round() as usize)
    }
}

fn marks(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.strip_prefix("mark ")).map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect()
}

// Theme colours (`ui/theme/tokens.rs`).
const GOLD: u32 = 0xf4b84c; // fill, title, accent
const TRACK: u32 = 0x20203a; // the empty part of the bar
const BLUE_SKY: u32 = 0x285ac8;

/// How full the bar looks: gold over gold and track, within the bar's rectangle.
fn filled(s: &Shot, l: &Layout) -> f32 {
    let bar = l.area("bar");
    let (g, t) = (near(s, GOLD, 12, bar) as f32, near(s, TRACK, 8, bar) as f32);
    assert!(g + t > 300.0, "the bar is not where the layout says: gold {g}, track {t}");
    g / (g + t)
}

#[test]
fn the_bar_and_the_words_show_the_load() {
    let Some(a) = shoot("a", "assets:3/6", None) else { return };
    assert!(errors(&a.out).is_empty(), "shader or pipeline errors: {:?}", errors(&a.out));
    let Some(a2) = shoot("a2", "assets:3/6", None) else { return };
    assert_eq!(a.png, a2.png, "the loading frame must repeat byte for byte");

    let l = Layout::of(&a.out);
    assert_eq!((l.get("screen"), l.get("pct"), l.get("stage"), l.get("sky")), ("1920x1080", "60", "assets", "stars"));
    // mount + content + map are 35%, half of the assets stage's 50% is 25%.
    let f = filled(&a, &l);
    eprintln!("bar filled {f:.3} (layout says {}%)", l.get("pct"));
    assert!((f - 0.60).abs() < 0.03, "{f}");
    // The percentage in title colour at the right, over the bar; the kicker and the rule at the left.
    let foot = (W * 3 / 4, H * 82 / 100, W - 40, H * 93 / 100);
    assert!(near(&a, GOLD, 12, foot) > 80, "percentage text");
    assert!(near(&a, GOLD, 12, (60, 90, 400, 180)) > 300, "kicker and rule");
    // The ring's dots: the lit head is gold, the rest of the ring track-coloured.
    let ring = l.area("ring");
    assert!(near(&a, GOLD, 40, ring) > 5, "ring head");
    // A load further on fills more of the bar.
    let Some(b) = shoot("b", "sim:1/2", None) else { return };
    let (lb, fb) = (Layout::of(&b.out), filled(&b, &Layout::of(&b.out)));
    assert_eq!((lb.get("pct"), lb.get("stage")), ("93", "sim"));
    assert!(fb > f + 0.2 && (fb - 0.93).abs() < 0.03, "{fb}");
    assert_ne!(a.png, b.png);

    // The marks of the scripted load, in order, as the bench's marks file writes them.
    let m = marks(&a.out);
    eprintln!("{}", m.join("\n"));
    let kinds: Vec<&str> = m.iter().map(|l| l.split(' ').nth(1).unwrap()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "load.stage").count(), 4, "{m:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "load.stage.end").count(), 3, "{m:?}");
    assert!(m[0].ends_with("load.begin") && m.last().unwrap().ends_with("load.progress pct=60"), "{m:?}");
    assert!(!m.iter().any(|l| l.contains("load.done")), "the load is not over");
    let file = std::fs::read_to_string(std::env::temp_dir().join(format!("sta64-loading-frame-a-{}", std::process::id())).join("out/load.marks")).expect("the marks file");
    assert_eq!(file.lines().count(), m.len());
    assert!(file.contains(" load.stage stage=assets pct=35"), "{file}");
}

#[test]
fn the_map_sky_shows_through_when_the_install_has_it() {
    let Some(stars) = shoot("stars", "assets:3/6", None) else { return };
    let sky = sky_dir();
    let Some(a) = shoot("sky", "assets:3/6", Some(&sky)) else { return };
    assert!(errors(&a.out).is_empty(), "{:?}", errors(&a.out));
    let l = Layout::of(&a.out);
    assert_eq!(l.get("sky"), "faces:6");
    assert_eq!(Layout::of(&stars.out).get("sky"), "stars");
    // The right of the screen is clear of the text's shade: the sky's blue there, black without.
    let right = (W * 62 / 100, H * 15 / 100, W * 95 / 100, H * 60 / 100);
    let (with, without) = (mean(&a, right), mean(&stars, right));
    eprintln!("right of the screen: sky {with:?}, stars {without:?}");
    assert!(with[2] > 60.0 && with[2] > with[0] * 2.0, "the sky's blue: {with:?}");
    assert!(without[2] < 30.0, "no sky without the faces: {without:?}");
    // The same sky colour the faces were made of.
    assert!(near(&a, BLUE_SKY, 70, right) > 1000);
    // The text and the bar are over it just the same.
    assert!((filled(&a, &l) - 0.60).abs() < 0.03);
    assert!(near(&a, GOLD, 12, (60, 90, 400, 180)) > 300);
    // It turns: the shot at another moment of the same load would differ, but one shot repeats.
    let Some(a2) = shoot("sky2", "assets:3/6", Some(&sky)) else { return };
    assert_eq!(a.png, a2.png, "the sky frame must repeat byte for byte");
}
