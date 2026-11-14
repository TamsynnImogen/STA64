//! Authored VFS fixtures for legacy texture-folder lookup. No game art is used.

use sta_assets::{TextureFormat, decode_rgba, find_texture};
use sta_vfs::Vfs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let n = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("sta_assets_legacy_{}_{}", std::process::id(), n));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn mount(&self, active_mod: Option<&str>) -> Vfs {
        Vfs::mount_install(Path::new(&self.0), active_mod).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn found(vfs: &Vfs, name: &str, path: &str, format: TextureFormat) {
    assert_eq!(find_texture(vfs, name), Some((path.into(), format)));
}

#[test]
fn finds_legacy_folders_by_basename_and_preserves_dotted_names() {
    let f = Fixture::new();
    f.write("Data/Textures/Index8/MbgDom1.3.TGA", b"unused");
    f.write("Data/Textures/Compressed/Other.TGA", b"unused");
    let vfs = f.mount(None);
    found(
        &vfs,
        "mbgdom1.3",
        "textures/index8/mbgdom1.3.tga",
        TextureFormat::Tga,
    );
    found(
        &vfs,
        "Textures\\Index8\\MBGDOM1.3.tGa",
        "textures/index8/mbgdom1.3.tga",
        TextureFormat::Tga,
    );
    found(
        &vfs,
        "OTHER",
        "textures/compressed/other.tga",
        TextureFormat::Tga,
    );
    assert_eq!(find_texture(&vfs, "missing"), None);
}

#[test]
fn same_layer_uses_format_then_folder_order() {
    let f = Fixture::new();
    for dir in [
        "Textures",
        "Textures/RGB",
        "Textures/Index8",
        "Textures/Compressed",
    ] {
        f.write(&format!("Data/{dir}/tie.tga"), b"unused");
    }
    f.write("Data/Textures/RGB/rgb_first.tga", b"unused");
    f.write("Data/Textures/Index8/rgb_first.tga", b"unused");
    f.write("Data/Textures/Index8/index_first.tga", b"unused");
    f.write("Data/Textures/Compressed/index_first.tga", b"unused");
    f.write("Data/Textures/Index8/format.tga", b"unused");
    f.write("Data/Textures/Compressed/format.dds", b"unused");
    f.write("Data/Textures/RGB/format.png", b"unused");
    let vfs = f.mount(None);
    found(&vfs, "tie", "textures/tie.tga", TextureFormat::Tga);
    found(
        &vfs,
        "rgb_first",
        "textures/rgb/rgb_first.tga",
        TextureFormat::Tga,
    );
    found(
        &vfs,
        "index_first",
        "textures/index8/index_first.tga",
        TextureFormat::Tga,
    );
    found(
        &vfs,
        "format",
        "textures/compressed/format.dds",
        TextureFormat::Dds,
    );
}

#[test]
fn child_legacy_folders_override_parent_root_and_rgb() {
    let f = Fixture::new();
    f.write("Data/Mods/Parent/info.ini", b"[Mod]\n");
    f.write("Data/Mods/Child/info.ini", b"[Mod]\nParentMod=Parent\n");
    f.write("Data/Mods/Parent/Textures/RGB/indexed.dds", b"unused");
    f.write("Data/Textures/indexed.ktx2", b"unused");
    f.write("Data/Mods/Child/Textures/Index8/indexed.tga", b"unused");
    f.write("Data/Mods/Parent/Textures/compressed.dds", b"unused");
    f.write(
        "Data/Mods/Child/Textures/Compressed/compressed.tga",
        b"unused",
    );
    let vfs = f.mount(Some("Child"));
    found(
        &vfs,
        "indexed",
        "textures/index8/indexed.tga",
        TextureFormat::Tga,
    );
    found(
        &vfs,
        "compressed",
        "textures/compressed/compressed.tga",
        TextureFormat::Tga,
    );
}

fn tga_header(image_type: u8, pixels: u8, palette_len: u16, palette_bits: u8) -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[1] = u8::from(palette_len != 0);
    bytes[2] = image_type;
    bytes[5..7].copy_from_slice(&palette_len.to_le_bytes());
    bytes[7] = palette_bits;
    bytes[12..14].copy_from_slice(&(pixels as u16).to_le_bytes());
    bytes[14..16].copy_from_slice(&1u16.to_le_bytes());
    bytes[16] = if palette_len != 0 || image_type == 11 {
        8
    } else {
        24
    };
    bytes[17] = 0x20;
    bytes
}

#[test]
fn discovered_indexed_and_rle_textures_decode_to_rgba() {
    let f = Fixture::new();
    let mut indexed = tga_header(1, 2, 2, 24);
    indexed.extend_from_slice(&[0, 0, 255, 0, 255, 0]); // BGR red, green palette
    indexed.extend_from_slice(&[0, 1]);
    f.write("Data/Textures/Index8/Palette.TGA", &indexed);

    let mut rle = tga_header(10, 2, 0, 0);
    rle.extend_from_slice(&[0x81, 255, 0, 0]); // two blue BGR pixels
    f.write("Data/Textures/Compressed/Run.TGA", &rle);

    let mut gray = tga_header(11, 2, 0, 0);
    gray.extend_from_slice(&[0x81, 42]);
    f.write("Data/Textures/Compressed/Gray.TGA", &gray);

    let vfs = f.mount(None);
    for (name, expected) in [
        ("palette", vec![255, 0, 0, 255, 0, 255, 0, 255]),
        ("run", vec![0, 0, 255, 255, 0, 0, 255, 255]),
        ("gray", vec![42, 42, 42, 255, 42, 42, 42, 255]),
    ] {
        let (path, format) = find_texture(&vfs, name).unwrap();
        assert_eq!(format, TextureFormat::Tga);
        let image = decode_rgba(&vfs, &path, format).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, expected);
    }
}
