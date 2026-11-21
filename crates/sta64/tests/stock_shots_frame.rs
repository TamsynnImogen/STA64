//! Real stock shots in a headless frame (#1041): the `firing` bench scene (a ship firing at a cube,
//! side on) on the mounted install's own content, under the Stock render profile, as frames at a few
//! ticks under the bench clock.
//!
//! What a frame can show that the unit tests cannot: the texture rectangle, the axes and the frame
//! order of a real shot sprite. The cheap checks, none of which needs stock numbers in the repository:
//! - the shots add light: some frame has pixels brighter than the scene without them (the per-pixel
//!   minimum over all frames, the scene being still);
//! - the beam is a band along the line of fire, about as wide as the weapon's `Radius` says, lit all
//!   along it (neither half dark) and across it (symmetric about its axis);
//! - a sprite shot's lit area is compact, about as big as `2 x Radius`, and near the line of fire;
//! - frames repeat run to run (the bench clock).
//!
//! Needs a real GPU and an install, so it is opt-in: it prints `SKIP` and passes unless
//! `STA64_GPU_TESTS=1` and `STA64_TEST_INSTALL` (and optionally `STA64_TEST_MOD`) are set; `sta64`
//! exits 77 without a hardware adapter and the test skips. Frames stay in a temporary directory
//! (kept with `STA64_KEEP_FRAMES=1`, printed), never in the repository.

use std::path::{Path, PathBuf};
use std::process::Command;

const TICKS: [u32; 16] = [2, 8, 14, 20, 26, 32, 38, 44, 50, 56, 62, 68, 74, 80, 90, 100];
const W: usize = 640;
const H: usize = 360;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-stock-shots-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Grey-scale-free RGB frames at [`TICKS`], or `None` to skip.
fn frames(name: &str, install: &str, active_mod: Option<&str>, profile: &str) -> Option<(PathBuf, Vec<Vec<[u8; 3]>>)> {
    let out = tmp(name);
    let ticks = TICKS.map(|t| t.to_string()).join(",");
    let mut c = Command::new(env!("CARGO_BIN_EXE_sta64"));
    c.args(["--headless", "--clock", "bench", "--install-content", "--frames", &ticks, "--frame-size", &format!("{W}x{H}"), "--frame-dir"])
        .arg(&out)
        .arg("--scene")
        .arg(root().join("bench/scenes/firing.ini"))
        .arg("--install-root")
        .arg(install)
        .env("STA64_RENDER_PROFILE", profile);
    if let Some(m) = active_mod {
        c.args(["--mod", m]);
    }
    let run = c.output().unwrap();
    match run.status.code() {
        Some(0) => {}
        Some(77) => {
            eprintln!("SKIP: no GPU adapter for headless frames");
            return None;
        }
        c => panic!("headless frame run failed ({c:?}): {}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr)),
    }
    let log = format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    assert!(!log.contains("ERROR"), "shader or pipeline error:\n{log}");
    let frames = TICKS
        .iter()
        .map(|t| {
            let png = std::fs::read(out.join(format!("tick-{t:06}.png"))).expect("a frame");
            rgb(&png)
        })
        .collect();
    Some((out, frames))
}

fn rgb(png: &[u8]) -> Vec<[u8; 3]> {
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.width as usize, info.height as usize), (W, H));
    let n = info.color_type.samples();
    buf.chunks_exact(n).map(|p| [p[0], p[1], p[2]]).collect()
}

/// What a frame shows beyond the still scene: per pixel, its sum of channels minus the lowest the pixel
/// has over all frames (shots come and go, the scene stands).
fn excess(frames: &[Vec<[u8; 3]>]) -> Vec<Vec<i32>> {
    let sum = |p: &[u8; 3]| i32::from(p[0]) + i32::from(p[1]) + i32::from(p[2]);
    let base: Vec<i32> = (0..W * H).map(|i| frames.iter().map(|f| sum(&f[i])).min().unwrap()).collect();
    frames.iter().map(|f| (0..W * H).map(|i| sum(&f[i]) - base[i]).collect()).collect()
}

/// The weapons of the scene's shooter and their `Radius`: `(class, radius)` of every hardpoint's shot
/// that has a stock drawing.
fn shooter_shots(install: &str, active_mod: Option<&str>) -> Vec<(String, f32)> {
    let vfs = sta_vfs::Vfs::mount_install(Path::new(install), active_mod).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
    let unit = db.units().find(|u| u.odf.eq_ignore_ascii_case("fgalaxy")).expect("the scene's shooter is in the install");
    unit.hardpoints
        .iter()
        .filter_map(|h| db.weapon(h.weapon.key))
        .filter(|w| matches!(w.class.0.as_str(), "phaser" | "photontorpedo"))
        .filter_map(|w| Some((w.class.0.clone(), w.look.radius?)))
        .collect()
}

#[test]
fn real_stock_shots_are_lit_bands_and_sprites_of_the_data_size() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return;
    }
    let Ok(install) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let Some((dir, stock)) = frames("stock", &install, active_mod.as_deref(), "stock") else { return };
    let again = frames("again", &install, active_mod.as_deref(), "stock").unwrap().1;
    assert!(stock == again, "frames differ between runs under the bench clock");

    let ex = excess(&stock);
    let shots = shooter_shots(&install, active_mod.as_deref());
    let radius = |class: &str| shots.iter().find(|s| s.0 == class).map(|s| s.1);
    // Pixels a world unit covers at the scene camera's distance (the bench camera's default 45 degree view, the
    // orbit's distance 700 from the middle of the line of fire).
    let scale = H as f32 / (2.0 * 700.0 * (22.5f32).to_radians().tan());
    let thr = 40;

    // The line of fire: the one row the beam lies along, from the median over the columns between the ship and
    // the target's shield (a shot sprite crossing a few of them does not move a median).
    let cols = 215..350;
    let row_profile = |e: &[i32]| -> Vec<i32> {
        (0..H)
            .map(|y| {
                let mut v: Vec<i32> = cols.clone().map(|x| e[y * W + x]).collect();
                v.sort_unstable();
                v[v.len() / 2]
            })
            .collect()
    };
    let beam_frame = ex
        .iter()
        .enumerate()
        .map(|(f, e)| (f, row_profile(e)))
        .max_by_key(|(_, p)| p.iter().copied().max().unwrap())
        .unwrap();
    let (f, profile) = beam_frame;
    let peak = *profile.iter().max().unwrap();
    let y0 = profile.iter().position(|v| *v == peak).unwrap();
    eprintln!("beam frame: tick {}, line of fire at row {y0}, peak excess {peak}", TICKS[f]);
    assert!(peak > 150, "no beam is drawn along the line of fire (peak excess {peak})");

    // 1. Its width about the weapon's `Radius`: the rows around the line lit above a fifth of the peak.
    if let Some(r) = radius("phaser") {
        // The core is a pixel or two; the faint edges of the texture count at a twentieth of it.
        let lit = |y: usize| profile[y] * 20 >= peak;
        let (mut up, mut down) = (0, 0);
        while lit(y0 - up - 1) {
            up += 1;
        }
        while lit(y0 + down + 1) {
            down += 1;
        }
        let width = (up + down + 1) as f32;
        let expect = r * scale;
        eprintln!("beam width {width} px ({up} above, {down} below), Radius {r} x {scale:.2} px/unit = {expect:.1} px");
        assert!(width >= (0.4 * expect).max(1.0) && width <= 3.0 * expect + 1.0, "the ribbon is {width} px wide, the data's Radius says about {expect:.1}");
        // Both sides of the line are lit, not one half of the texture.
        let side = |range: std::ops::Range<usize>| range.map(|y| profile[y]).sum::<i32>();
        let (above, below) = (side(y0 - 3..y0), side(y0 + 1..y0 + 4));
        eprintln!("light above the line {above}, below {below}");
        assert!(above * 6 >= below && below * 6 >= above, "the ribbon is dark on one side ({above} against {below})");
    }

    // 2. Lit all along: between its first and last lit column no gap (neither half dark).
    let e = &ex[f];
    let lit_cols: Vec<usize> = (200..W)
        .filter(|x| (y0 - 1..=y0 + 1).map(|y| e[y * W + x]).sum::<i32>() > 3 * thr)
        .collect();
    let (first, last) = (*lit_cols.first().expect("lit columns"), *lit_cols.last().unwrap());
    let span = last - first + 1;
    eprintln!("lit columns {first}..={last}: {} of {span}", lit_cols.len());
    assert!(span > 100, "the beam is only {span} px long");
    assert!(lit_cols.len() * 4 >= span * 3, "the ribbon has dark stretches: {} of {span} columns lit", lit_cols.len());

    // 3. A sprite shot (the torpedo): a compact lit blob near the line, about 2 x Radius across.
    if let Some(r) = radius("photontorpedo") {
        let expect = 2.0 * r * scale;
        let mut found = Vec::new();
        for (g, e) in ex.iter().enumerate() {
            // The columns between the ship and the shield; the rows clear of the line, above and below.
            let off = |x: usize, y: usize| y.abs_diff(y0) >= 2 && e[y * W + x] > thr;
            let cx = (260..355).max_by_key(|x| (y0 - 7..y0 - 2).chain(y0 + 3..y0 + 8).map(|y| e[y * W + x].max(0)).sum::<i32>()).unwrap();
            let pts: Vec<(usize, usize)> = (cx - 14..=cx + 14).flat_map(|x| (y0 - 14..=y0 + 14).map(move |y| (x, y))).filter(|(x, y)| off(*x, *y)).collect();
            if pts.len() < 8 {
                continue;
            }
            let ext = |f: fn(&(usize, usize)) -> usize| (pts.iter().map(f).min().unwrap(), pts.iter().map(f).max().unwrap());
            let ((x0, x1), (ya, yb)) = (ext(|p| p.0), ext(|p| p.1));
            let (up, down) = (y0 - ya, yb - y0);
            found.push((g, pts.len(), x1 - x0 + 1, up, down));
        }
        eprintln!("sprite blobs (tick, lit px, width, rows above the line, below): {found:?}; 2 x Radius = {expect:.1} px");
        assert!(!found.is_empty(), "no shot sprite is lit beside the line of fire");
        for (g, _n, w, up, down) in &found {
            let h = (up + down) as f32;
            assert!(*w as f32 >= 0.25 * expect && *w as f32 <= 1.8 * expect, "tick {}: the sprite is {w} px wide, 2 x Radius is {expect:.1}", TICKS[*g]);
            assert!(h >= 0.25 * expect && h <= 1.8 * expect, "tick {}: the sprite is {h} px high, 2 x Radius is {expect:.1}", TICKS[*g]);
            // On the line of fire, to within a few pixels (the torpedo leaves a hardpoint of its own, so not exactly on it).
            assert!(up.abs_diff(*down) <= 8, "tick {}: the sprite is off the line ({up} above, {down} below)", TICKS[*g]);
        }
    }
    if std::env::var("STA64_KEEP_FRAMES").as_deref() != Ok("1") {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
