//! Opt-in, read-only dispatch check for the installed Legacy corpus.

use super::{chunks, load};
use crate::model::Model;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Counts {
    scenes: usize,
    libraries: usize,
    geometry: usize,
    historical_refused: usize,
    warning_files: usize,
    geometry_warning_files: usize,
}

fn walk(root: &Path, files: &mut Vec<PathBuf>) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| panic!("read_dir entry {}: {e}", dir.display()));
            let path = entry.path();
            let kind = entry
                .file_type()
                .unwrap_or_else(|e| panic!("file_type {}: {e}", path.display()));
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("m3d"))
            {
                files.push(path);
            }
        }
    }
}

fn is_chunk(path: &Path) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("chunks"))
}

fn historical_version(bytes: &[u8]) -> bool {
    bytes.starts_with(b"StarTrekDB")
        && bytes
            .get(10..14)
            .is_some_and(|raw| raw == 1.0_f32.to_le_bytes() || raw == 1.2_f32.to_le_bytes())
}

fn scene(bytes: &[u8]) -> Result<Model, String> {
    if bytes.starts_with(b"Storm3D_SW") || bytes.starts_with(b"StarTrekDB") {
        a2fmt::sod::Sod::parse(bytes)
            .map(|sod| Model::from_sod(&sod))
            .map_err(|e| format!("{e:?}"))
    } else {
        load(bytes)
    }
}

#[test]
fn installed_legacy_corpus_by_explicit_role() {
    let Some(raw_roots) = std::env::var_os("STA64_TEST_M3D_ROOTS") else {
        eprintln!("SKIP installed_legacy_corpus_by_explicit_role: set STA64_TEST_M3D_ROOTS");
        return;
    };
    let roots: Vec<_> = std::env::split_paths(&raw_roots).collect();
    assert!(!roots.is_empty(), "STA64_TEST_M3D_ROOTS contains no paths");
    let mut files = Vec::new();
    for root in &roots {
        assert!(root.is_dir(), "M3D corpus root is not a directory: {}", root.display());
        walk(root, &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "no .m3d files found under STA64_TEST_M3D_ROOTS");

    let mut count = Counts::default();
    for path in &files {
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        if is_chunk(path) {
            let material = chunks::load_materials(&bytes);
            let geometry = chunks::load_geometry(&bytes);
            match (material, geometry) {
                (Ok(_), Err(_)) => count.libraries += 1,
                (Err(_), Ok(geometry)) => {
                    count.geometry += 1;
                    count.geometry_warning_files += usize::from(!geometry.attribute_warnings().is_empty());
                }
                (Ok(_), Ok(_)) => panic!("ambiguous chunk role: {}", path.display()),
                (Err(material_error), Err(geometry_error)) => panic!(
                    "unreadable chunk {}: material: {material_error}; geometry: {geometry_error}",
                    path.display()
                ),
            }
        } else {
            let parsed = scene(&bytes);
            if historical_version(&bytes) {
                assert!(
                    parsed
                        .as_ref()
                        .is_err_and(|error| error.starts_with("UnsupportedVersion(")),
                    "historical stream must retain its explicit native version refusal: {}",
                    path.display()
                );
                count.historical_refused += 1;
            } else {
                let model = parsed.unwrap_or_else(|e| panic!("scene {}: {e}", path.display()));
                count.scenes += 1;
                count.warning_files += usize::from(!model.warnings.is_empty());
            }
        }
        let after = fs::read(path).unwrap_or_else(|e| panic!("reread {}: {e}", path.display()));
        assert!(bytes == after, "input changed while decoding {}", path.display());
    }
    eprintln!(
        "M3D installed corpus: {} files; scenes {}; material libraries {}; geometry variants {}; historical native refusals {}; scene warning files {}; geometry attribute warning files {}",
        files.len(),
        count.scenes,
        count.libraries,
        count.geometry,
        count.historical_refused,
        count.warning_files,
        count.geometry_warning_files
    );
}
