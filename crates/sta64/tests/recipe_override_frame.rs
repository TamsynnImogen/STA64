//! RND-28 (MODDING_DECISIONS D12): a recipe ini in a mod folder changes a headless
//! frame, and a shadowing `.wesl` replaces the embedded shader in the bench.
//!
//! Both run the `firing` scene (a beam weapon, `fx_beam` on the bench's fake
//! content) as frames at a few ticks, with no files and then with files in a mod's
//! folder under a temporary install, and compare the frames. Frames repeat under
//! `--clock bench`, so no difference means the files did nothing.
//!
//! Needs a real GPU, so it is opt-in like the other frame tests: it prints `SKIP`
//! and passes unless `STA64_GPU_TESTS=1`; `sta64` exits 77 without a hardware adapter
//! and the test skips.

use std::path::{Path, PathBuf};
use std::process::Command;

const TICKS: &str = "4,8,12,16";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-recipe-frame-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An install holding one mod, `Test`, with `files` in its folder.
fn install(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = tmp(name);
    std::fs::create_dir_all(dir.join("Data/Mods/Test")).unwrap();
    std::fs::write(dir.join("Data/Mods/Test/info.ini"), "[Mod]\nName=Test\n").unwrap();
    for (path, text) in files {
        let p = dir.join("Data/Mods/Test").join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

/// The frames' bytes, or `None` to skip.
fn frames(name: &str, install: Option<&Path>, shadow: bool) -> Option<Vec<Vec<u8>>> {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return None;
    }
    let out = tmp(name);
    let mut c = Command::new(env!("CARGO_BIN_EXE_sta64"));
    c.args(["--headless", "--clock", "bench", "--frames", TICKS, "--frame-size", "640x360", "--frame-dir"])
        .arg(&out)
        .arg("--scene")
        .arg(root().join("bench/scenes/firing.ini"))
        .env_remove("STA64_DEV_SHADER_SHADOW");
    if let Some(i) = install {
        c.arg("--install-root").arg(i).args(["--mod", "Test"]);
    }
    if shadow {
        c.env("STA64_DEV_SHADER_SHADOW", "1");
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
    Some(TICKS.split(',').map(|t| std::fs::read(out.join(format!("tick-{:06}.png", t.parse::<u32>().unwrap()))).expect("a frame")).collect())
}

fn pixels(png: &[u8]) -> Vec<u8> {
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    r.next_frame(&mut buf).unwrap();
    buf
}

/// Differing bytes over all frames (the frames of a loaded machine can differ by a
/// few pixels run to run, so callers compare against that floor).
fn differ(a: &[Vec<u8>], b: &[Vec<u8>]) -> usize {
    a.iter().zip(b).map(|(x, y)| pixels(x).iter().zip(pixels(y)).filter(|(p, q)| **p != *q).count()).sum()
}

#[test]
fn a_recipe_in_a_mod_folder_changes_the_frame() {
    let Some(plain) = frames("plain", None, false) else { return };
    // The same run twice: the noise floor a difference below must clear.
    let again = frames("again", None, false).unwrap();
    let floor = differ(&plain, &again);
    let beam = "[Ordnance.fx_beam]\nKind=beam\nColour=0,255,0\nCoreColour=200,255,200\nWidth=40\nCoreWidth=20\nBrightness=3\n";
    let modded = install("recipe", &[("ordnance/ordnance.ini", beam)]);
    let with = frames("with", Some(&modded), false).unwrap();
    let changed = differ(&plain, &with);
    assert!(changed > floor * 4 + 100, "the mod's recipe left the frames as they were ({changed} bytes, noise {floor})");
    // A mod with no recipe leaves the frames alone (stock output unchanged).
    let empty = install("empty", &[]);
    let none = frames("none", Some(&empty), false).unwrap();
    assert!(differ(&plain, &none) <= floor * 2 + 20, "a mod without files changed the frame");
}

#[test]
fn a_shadowing_wesl_replaces_the_embedded_shader() {
    let Some(plain) = frames("plain2", None, false) else { return };
    // The beam's own shader with its colour sent through green: the same file, one line changed.
    let quad = std::fs::read_to_string(root().join("crates/sta64/src/render/fx/quad.wesl")).unwrap();
    let marked = "return vec4<f32>(c * k.a.y, 0.0);";
    assert!(quad.contains(marked), "the test's edit point moved");
    let shadow = quad.replace(marked, "return vec4<f32>(vec3<f32>(0.0, 1.0, 0.0) * length(c) * k.a.y, 0.0);");
    let modded = install("shadow", &[("shaders/render/fx/quad.wesl", &shadow)]);
    // Off: the files are not read, whatever the mod holds.
    let off = frames("shadow-off", Some(&modded), false).unwrap();
    let again = frames("again2", None, false).unwrap();
    let floor = differ(&plain, &again);
    assert!(differ(&plain, &off) <= floor * 2 + 20, "a shadow applied without the developer switch");
    let on = frames("shadow-on", Some(&modded), true).unwrap();
    assert!(differ(&plain, &on) > floor * 4 + 100, "the shadowing shader did not replace the embedded one");
}
