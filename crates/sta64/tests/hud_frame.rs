//! UI-04: the HUD in real frames, the `ui-hud-layout` bench (the scenario in
//! `bench/scenarios/ui-hud-layout.md` runs the same scene through `sta-bench`;
//! this checks what is in the pixels).
//!
//! `STA64_UI_HUD=1` draws the HUD over the bench scene `bench/scenes/hud.ini`
//! (a yard, a flagship and two escorts of player 1, a raider and a rival base),
//! on `fake::db()` content. The run prints its layout as a `hud layout:` line;
//! the checks read the rectangles from it and look for the theme's colours in
//! them.
//!
//! Needs a real GPU, so it is opt-in like the other frame tests: it prints
//! `SKIP` and passes unless `STA64_GPU_TESTS=1`; `sta64` exits 77 without a
//! hardware adapter and the test skips.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn sta64() -> &'static str {
    env!("CARGO_BIN_EXE_sta64")
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-hud-frame-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[derive(Clone, Copy)]
struct Size {
    w: usize,
    h: usize,
}

const WIDE: Size = Size { w: 1280, h: 720 };
const FOUR_THREE: Size = Size { w: 1024, h: 768 };

/// What to run: the env switches.
#[derive(Default, Clone)]
struct Run {
    hud: bool,
    select: &'static str,
    page: &'static str,
    cfg: Option<PathBuf>,
}

struct Shot {
    px: Vec<[u8; 4]>,
    size: Size,
    png: Vec<u8>,
    out: Output,
}

/// `None` when the test should skip.
fn shoot(name: &str, size: Size, run: &Run) -> Option<Shot> {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return None;
    }
    let dir = tmp(name);
    let mut c = Command::new(sta64());
    c.args(["--headless", "--clock", "bench", "--frames", "0", "--frame-size", &format!("{}x{}", size.w, size.h), "--frame-dir"])
        .arg(&dir)
        .arg("--scene")
        .arg(root().join("bench/scenes/hud.ini"));
    for k in ["STA64_UI_HUD", "STA64_UI_RADAR", "STA64_UI_HUD_SELECT", "STA64_UI_HUD_PAGE", "STA64_UI_HUD_CFG"] {
        c.env_remove(k);
    }
    if run.hud {
        c.env("STA64_UI_HUD", "1").env("STA64_UI_HUD_SELECT", run.select).env("STA64_UI_HUD_PAGE", run.page);
        if let Some(cfg) = &run.cfg {
            c.env("STA64_UI_HUD_CFG", cfg);
        }
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
    assert_eq!((info.width as usize, info.height as usize), (size.w, size.h));
    let n = info.color_type.samples();
    let px = buf.chunks(n).map(|p| [p[0], p[1], p[2], if n == 4 { p[3] } else { 255 }]).collect();
    Some(Shot { px, size, png, out })
}

type Area = (usize, usize, usize, usize);

/// Pixels within `tol` of an `0xRRGGBB` colour in a rectangle (x0, y0, x1, y1).
fn near(s: &Shot, rgb: u32, tol: i32, a: Area) -> usize {
    let want = [(rgb >> 16) as i32, (rgb >> 8 & 255) as i32, (rgb & 255) as i32];
    let mut n = 0;
    for y in a.1..a.3.min(s.size.h) {
        for x in a.0..a.2.min(s.size.w) {
            let p = s.px[y * s.size.w + x];
            if (0..3).all(|i| (i32::from(p[i]) - want[i]).abs() <= tol) {
                n += 1;
            }
        }
    }
    n
}

/// Pixels that differ between two shots in a rectangle.
fn changed(a: &Shot, b: &Shot, area: Area) -> usize {
    let mut n = 0;
    for y in area.1..area.3.min(a.size.h) {
        for x in area.0..area.2.min(a.size.w) {
            n += usize::from(a.px[y * a.size.w + x] != b.px[y * b.size.w + x]);
        }
    }
    n
}

fn errors(out: &Output) -> Vec<String> {
    let all = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    all.lines().filter(|l| l.contains("ERROR") || l.contains("panicked")).map(str::to_owned).collect()
}

/// The `hud layout:` line, as words: `key=value`.
struct Layout(BTreeMap<String, String>);

impl Layout {
    fn of(out: &Output) -> Layout {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().rfind(|l| l.starts_with("hud layout:")).expect("a `hud layout:` line").to_owned();
        let mut m = BTreeMap::new();
        // `cells=[0:A 1:B]` and `strip=[...]` hold spaces: take them whole.
        let mut rest = line.trim_start_matches("hud layout:").trim().to_owned();
        for key in ["cells", "strip"] {
            if let Some(i) = rest.find(&format!("{key}=[")) {
                let end = rest[i..].find(']').unwrap() + i;
                m.insert(key.to_owned(), rest[i + key.len() + 2..end].to_owned());
                rest.replace_range(i..=end, "");
            }
        }
        for w in rest.split_whitespace() {
            let (k, v) = w.split_once('=').unwrap();
            m.insert(k.to_owned(), v.to_owned());
        }
        Layout(m)
    }

    fn get(&self, k: &str) -> &str {
        &self.0[k]
    }

    /// A rectangle in theme units.
    fn rect(&self, k: &str) -> [f32; 4] {
        let v: Vec<f32> = self.get(k).split(',').map(|n| n.parse().unwrap()).collect();
        [v[0], v[1], v[2], v[3]]
    }

    /// A rectangle in pixels of a frame `h` lines tall (units are of a 1080-line screen).
    fn area(&self, k: &str, h: usize) -> Area {
        let r = self.rect(k);
        let f = h as f32 / 1080.0;
        ((r[0] * f).round() as usize, (r[1] * f).round() as usize, ((r[0] + r[2]) * f).round() as usize, ((r[1] + r[3]) * f).round() as usize)
    }
}

// Theme colours (`ui/theme/tokens.rs`).
const GOLD: u32 = 0xf4b84c; // accent, title, fills
const LAVENDER: u32 = 0x9c9cde; // accent2, buttons, frames
const SKY: u32 = 0x7ea8d4; // accent3: the dilithium mark
const WARN: u32 = 0xf2c23b; // the latinum mark
const GOOD: u32 = 0x5fd36a; // the biomatter mark, own blips
const BAD: u32 = 0xe5483b; // the raider
const OFF: u32 = 0x2e2e44; // a disabled button

fn overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn yard() -> Run {
    Run { hud: true, select: "yard", ..Run::default() }
}

#[test]
fn hud_layout_draws_every_panel_where_the_layout_says() {
    let Some(off) = shoot("off", WIDE, &Run::default()) else { return };
    let Some(a) = shoot("a", WIDE, &yard()) else { return };
    assert!(errors(&a.out).is_empty(), "shader or pipeline errors: {:?}", errors(&a.out));
    let Some(a2) = shoot("a2", WIDE, &yard()) else { return };
    assert_eq!(a.png, a2.png, "the HUD frame must repeat byte for byte");
    assert_ne!(a.png, off.png, "the HUD must change the frame");

    let l = Layout::of(&a.out);
    assert_eq!((l.get("screen"), l.get("info"), l.get("page"), l.get("beside")), ("1920x1080", "middle", "Top", "true"), "a yard is a builder: the middle panel");
    assert_eq!(l.get("cells"), "0:BUILD 6:STOP 7:LINE 8:COLUMN 9:WEDGE");
    assert!(l.get("strip").starts_with("dilithium="), "{}", l.get("strip"));
    // No panel on another, the grid beside the minimap, everything on the screen.
    let (res, mini, grid, info) = (l.rect("resources"), l.rect("minimap"), l.rect("grid"), l.rect("info_rect"));
    for (x, y) in [(res, mini), (res, grid), (res, info), (mini, grid), (mini, info), (grid, info)] {
        assert!(!overlap(x, y), "{x:?} over {y:?}");
    }
    assert!(info[0] + info[2] <= 1920.0 && info[1] + info[3] <= 1080.0);

    let h = WIDE.h;
    let (r_a, m_a, g_a, i_a) = (l.area("resources", h), l.area("minimap", h), l.area("grid", h), l.area("info_rect", h));
    // The strip: a mark in each resource's colour (none of them in the bare frame).
    for (rgb, what) in [(SKY, "dilithium"), (LAVENDER, "metal"), (WARN, "latinum"), (GOOD, "biomatter")] {
        let n = near(&a, rgb, 10, r_a);
        eprintln!("strip {what}: {n} px");
        assert!(n >= 15, "{what} mark ({rgb:06x}): {n} px");
        assert_eq!(near(&off, rgb, 10, r_a), 0, "{what} colour is already in the bare frame");
    }
    // The grid: five buttons (BUILD STOP LINE COLUMN WEDGE) in lavender, the
    // empty cells dim; the bare frame has neither.
    let (lav, dim) = (near(&a, LAVENDER, 10, g_a), near(&a, OFF, 24, g_a));
    eprintln!("grid: lavender {lav}, dim {dim}");
    assert!(lav > 4000, "{lav}");
    assert!(dim > 1500, "{dim}");
    assert_eq!(near(&off, LAVENDER, 10, g_a), 0);
    // The info panel: the side bar in lavender, the gold of the title, the bars and the foot.
    let (lav_i, gold_i) = (near(&a, LAVENDER, 10, i_a), near(&a, GOLD, 10, i_a));
    eprintln!("info: lavender {lav_i}, gold {gold_i}");
    assert!(lav_i > 600, "{lav_i}");
    assert!(gold_i > 1500, "{gold_i}");
    // The radar is the HUD's: the elbow, the filters and the blips in its place.
    let (gold_m, lav_m) = (near(&a, GOLD, 10, m_a), near(&a, LAVENDER, 10, m_a));
    let (own, foe) = (near(&a, GOOD, 14, m_a), near(&a, BAD, 14, m_a));
    eprintln!("minimap: gold {gold_m}, lavender {lav_m}, own {own}, foe {foe}");
    assert!(gold_m > 300 && lav_m > 1500, "{gold_m} {lav_m}");
    assert!(own >= 15 && foe >= 6, "blips {own} {foe}");
    // The radar fills the room the layout gives it: its elbow is at the rectangle's corner.
    assert!(near(&a, LAVENDER, 10, (m_a.0, m_a.1, m_a.0 + 40, m_a.1 + 40)) > 150, "the radar's top left corner is not at the minimap rectangle");
    assert!(near(&a, GOLD, 10, (m_a.0, m_a.3 - 12, m_a.2, m_a.3 + 1)) > 50, "the radar's foot is not at the rectangle's bottom");
    // Nothing else of the frame is touched: the top right is the bare scene.
    assert_eq!(changed(&a, &off, (840, 0, WIDE.w, 400)), 0);
}

#[test]
fn the_build_page_lists_what_the_yard_builds() {
    let Some(a) = shoot("page-top", WIDE, &yard()) else { return };
    let Some(b) = shoot("page-build", WIDE, &Run { page: "build", ..yard() }) else { return };
    assert!(errors(&b.out).is_empty(), "{:?}", errors(&b.out));
    let l = Layout::of(&b.out);
    assert_eq!(l.get("page"), "Build");
    // The yard's list from the content (cruiser, frigate, dread), cancel on T and back on G.
    assert_eq!(l.get("cells"), "0:FX_CRUISER 1:FX_FRIGATE 2:FX_DREAD 4:CANCEL 9:BACK");
    let g = l.area("grid", WIDE.h);
    assert!(changed(&a, &b, g) > 800, "the build page must look different from the top page");
    // Cancel and back are lavender, so are the first two items; the dread is not
    // available (the fake tech tree: it needs a station) or is: either way four live buttons.
    assert!(near(&b, LAVENDER, 10, g) > 2500);
}

#[test]
fn a_group_takes_the_high_panel_and_a_cfg_sizes_it() {
    let group = Run { hud: true, select: "flagship,escort,escort2", ..Run::default() };
    let Some(single) = shoot("single", WIDE, &Run { select: "flagship", ..group.clone() }) else { return };
    let Some(g) = shoot("group", WIDE, &group) else { return };
    assert!(errors(&g.out).is_empty(), "{:?}", errors(&g.out));
    let (ls, lg) = (Layout::of(&single.out), Layout::of(&g.out));
    assert_eq!((ls.get("info"), lg.get("info")), ("low", "high"));
    let (rs, rg) = (ls.rect("info_rect"), lg.rect("info_rect"));
    assert!(rg[3] > rs[3] * 2.0, "the group's panel is the tall one: {rg:?} against {rs:?}");
    assert_eq!(rs[1] + rs[3], rg[1] + rg[3], "both stand on the same floor");
    // The grid is where it was, and no longer offers Build (no builder).
    assert_eq!(lg.rect("grid"), ls.rect("grid"));
    assert_eq!(lg.get("cells"), "6:STOP 7:LINE 8:COLUMN 9:WEDGE");
    // Three tiles, each with its hull strip.
    let i = lg.area("info_rect", WIDE.h);
    assert!(near(&g, GOOD, 10, i) > 150, "hull strips");

    // A race's GUI cfg (CON-12): Fleet Operations' federation file, three heights
    // of 90 / 180 / 270 units on its 1200-line canvas.
    let cfg = Run { cfg: Some(root().join("fixtures/gui/fo/gui_federation16x9.cfg")), ..group };
    let Some(c) = shoot("cfg", WIDE, &cfg) else { return };
    let lc = Layout::of(&c.out);
    let r = lc.rect("info_rect");
    assert_eq!((r[2], r[3]), (540.0, 270.0), "the cfg's tallest height");
    assert_ne!(c.png, g.png);
    let Some(c1) = shoot("cfg-one", WIDE, &Run { select: "flagship", ..cfg }) else { return };
    assert_eq!(Layout::of(&c1.out).rect("info_rect")[3], 90.0, "the cfg's lowest height");
}

#[test]
fn a_narrow_screen_puts_the_grid_above_the_radar() {
    let Some(a) = shoot("four-three", FOUR_THREE, &yard()) else { return };
    assert!(errors(&a.out).is_empty(), "{:?}", errors(&a.out));
    let l = Layout::of(&a.out);
    assert_eq!((l.get("screen"), l.get("beside")), ("1440x1080", "false"));
    let (mini, grid, info) = (l.rect("minimap"), l.rect("grid"), l.rect("info_rect"));
    assert!(grid[1] + grid[3] <= mini[1], "the grid is above the minimap");
    assert!(!overlap(grid, info) && !overlap(mini, info));
    let h = FOUR_THREE.h;
    let (g, m, i) = (l.area("grid", h), l.area("minimap", h), l.area("info_rect", h));
    assert!(near(&a, LAVENDER, 10, g) > 3000, "the grid is drawn where the layout says");
    assert!(near(&a, GOLD, 10, i) > 1500, "so is the info panel");
    assert!(near(&a, GOLD, 10, m) > 300, "so is the radar");
}
