//! RND-01: nothing under `render/` reads Bevy's `Time`; animation time is
//! `AnimClock` (docs/TESTBENCH.md, "Determinism and the animation clock").

#[test]
fn no_bevy_time_in_render() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render");
    let needles = ["Res<Time", "Time<Real", "Time<Virtual", "Time<Fixed"];
    let mut bad = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("clock_guard.rs") {
            let src = std::fs::read_to_string(&path).unwrap();
            for (i, line) in src.lines().enumerate() {
                if needles.iter().any(|n| line.contains(n)) {
                    bad.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "use AnimClock instead of Time in render/: {bad:?}");
}
