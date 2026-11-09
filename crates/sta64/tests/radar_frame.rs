//! UI-05: the radar in a real frame (1280x720, drawn over the bench scene's
//! stand-ins by `STA64_UI_RADAR=1`).
//!
//! Needs a real GPU, so it is opt-in like the other frame tests: it prints
//! `SKIP` and passes unless `STA64_GPU_TESTS=1`; `sta64` exits 77 without a
//! hardware adapter (software ones count as none) and the test skips.
//!
//! Checks: the run logs no ERROR (a shader or pipeline error only logs), the
//! same frame twice is the same bytes, the radar changes the frame, and the
//! bottom-left corner holds the LCARS panel (gold and lavender of the theme)
//! with blips in the relation colours (own green, enemy red, the moon dim).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const W: usize = 1280;
const H: usize = 720;

fn sta64() -> &'static str {
    env!("CARGO_BIN_EXE_sta64")
}

fn scene() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/radar.ini")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-radar-frame-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &Path, radar: bool) -> Output {
    let mut c = Command::new(sta64());
    c.args(["--headless", "--clock", "bench", "--frames", "0", "--frame-size", &format!("{W}x{H}"), "--frame-dir"])
        .arg(dir)
        .arg("--scene")
        .arg(scene());
    if radar {
        c.env("STA64_UI_RADAR", "1");
    } else {
        c.env_remove("STA64_UI_RADAR");
    }
    c.output().unwrap()
}

/// `None` when the test should skip.
fn frame(name: &str, radar: bool) -> Option<(Vec<u8>, Output)> {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return None;
    }
    let dir = tmp(name);
    let out = run(&dir, radar);
    match out.status.code() {
        Some(0) => {}
        Some(77) => {
            eprintln!("SKIP: no GPU adapter for headless frames: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
            return None;
        }
        c => panic!("headless frame run failed ({c:?}): {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    }
    let png = std::fs::read(dir.join("tick-000000.png")).expect("the frame");
    Some((png, out))
}

/// RGBA pixels of a PNG frame.
fn decode(png: &[u8]) -> Vec<[u8; 4]> {
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.width as usize, info.height as usize), (W, H));
    let n = info.color_type.samples();
    buf.chunks(n).map(|p| [p[0], p[1], p[2], if n == 4 { p[3] } else { 255 }]).collect()
}

/// Pixels within `tol` of an `0xRRGGBB` colour in a rectangle (x0, y0, x1, y1).
fn near(px: &[[u8; 4]], rgb: u32, tol: i32, area: (usize, usize, usize, usize)) -> usize {
    let want = [(rgb >> 16) as i32, (rgb >> 8 & 255) as i32, (rgb & 255) as i32];
    let mut n = 0;
    for y in area.1..area.3 {
        for x in area.0..area.2 {
            let p = px[y * W + x];
            if (0..3).all(|i| (i32::from(p[i]) - want[i]).abs() <= tol) {
                n += 1;
            }
        }
    }
    n
}

fn errors(out: &Output) -> Vec<String> {
    let all = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    all.lines().filter(|l| l.contains("ERROR") || l.contains("panicked")).map(str::to_owned).collect()
}

#[test]
fn radar_frame_has_the_panel_and_blips() {
    let Some((a, out)) = frame("a", true) else { return };
    assert!(errors(&out).is_empty(), "shader or pipeline errors: {:?}", errors(&out));
    let Some((b, out_b)) = frame("b", true) else { return };
    assert!(errors(&out_b).is_empty());
    assert_eq!(a, b, "the radar frame must repeat byte for byte");
    let Some((off, _)) = frame("off", false) else { return };
    assert_ne!(a, off, "the radar must change the frame");

    let px = decode(&a);
    let base = decode(&off);
    // Bottom left, where the panel is (Refit: the minimap at the bottom left).
    let corner = (0, H / 2, 480, H);
    let gold = near(&px, 0xf4b84c, 10, corner); // title, end bar (accent)
    let lavender = near(&px, 0x9c9cde, 10, corner); // elbow, side bar, frame (accent2)
    let green = near(&px, 0x5fd36a, 14, corner); // own blips (good)
    let red = near(&px, 0xe5483b, 14, corner); // the raider (bad)
    let dim = near(&px, 0x55556e, 10, corner); // the moon (text_off)
    eprintln!("panel pixels: gold {gold} lavender {lavender} own {green} enemy {red} moon {dim}");
    assert!(gold > 300, "gold {gold}");
    assert!(lavender > 1500, "lavender {lavender}");
    // The radar's own: not in the frame without it.
    for (rgb, got, least) in [(0x5fd36a, green, 20), (0xe5483b, red, 8), (0x55556e, dim, 20)] {
        assert!(got >= least, "colour {rgb:06x}: {got} pixels in the panel, wanted {least}");
        assert_eq!(near(&base, rgb, 14, corner), 0, "colour {rgb:06x} is already in the frame without the radar");
    }
    // The top right is untouched by the UI.
    let far = (W / 2, 0, W, H / 2);
    assert_eq!(
        (0..H / 2).flat_map(|y| (W / 2..W).map(move |x| (x, y))).filter(|&(x, y)| px[y * W + x] != base[y * W + x]).count(),
        0,
        "pixels changed outside the corner: {far:?}"
    );
}
