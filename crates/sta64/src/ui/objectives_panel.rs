//! The stock look of the briefing and objectives panel (the mission's objectives window and message box).
//!
//! **Rule.** In a stock presentation profile the panel is the game's own picture: the pieces of the gui
//! cfg's `objectivesBackgroundPanel` (`objectivesBackgroundPanelSize`, `objectivesBackgroundPanel_<n>`),
//! each cropped from the sprite `<name>.<n>` of `gui_global.spr`, drawn in the panel area
//! (`objectivesPanelArea`) of the cfg's canvas, with the text on the cfg's text rectangles and the OK button
//! on `objectivesOkayButtonArea`. The mission's text is STA64's own (objectives state from the mission
//! runtime). Evidence: the keys and sprite names are read from data (Armada I, Armada II, Fleet Operations);
//! the appearance of the Armada II window is seen in one frame of the original.
//!
//! **Group.** The panel is one art group: on a window whose aspect is not the cfg canvas's it keeps its
//! aspect and moves as a whole, centred with the HUD's sheet ([`HudFit::Centred`], ADR 0008 H1/H2), or fills
//! the window with the sheet when the player chose [`HudFit::Stretched`].
//!
//! **Black area.** A cfg that carries `objectivesBlackArea` has a black rectangle under the pieces
//! (Armada I's frame leaves its window to it); its opacity is **not measured** (drawn opaque, interim).
//!
//! **Fallback.** Without the cfg's pieces, a sprite, a texture or a consistent tiling (the pieces must lie
//! inside the panel area: a cfg whose pieces tile more than its panel area is not read, as its placement is
//! **not measured**), the panel keeps STA64's own look and the reason is logged once.
//!
//! **Not measured here (interim, TODO.md).** The fonts and sizes of the stock texts; where Armada I's single
//! text area starts (its text is set inside the black area); the done mark of an objective line; that the
//! stock window pauses the game and owns the mouse while it shows (Armada II: seen on the bench).

use std::sync::Arc;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;

use super::gui_sprites::{self, GuiImageSampling, GuiSprites};
use super::hud::layout::R;
use super::hud::stock::layout::{HudFit, StockLayout, fit_canvas};

/// One piece of the panel's picture.
#[derive(Clone, Debug)]
pub(crate) struct ArtPiece {
    #[allow(dead_code)] // read by the tests
    pub frame: usize,
    /// Panel-relative, in canvas units.
    pub rect: [f32; 4],
    pub image: Handle<Image>,
    pub crop: Rect,
}

/// The loaded picture of the panel.
#[derive(Clone, Debug)]
pub(crate) struct Art {
    pub pieces: Vec<ArtPiece>,
    /// `objectivesBlackArea`, panel-relative, in canvas units.
    pub black: Option<[f32; 4]>,
}

/// Why the picture could not be had.
#[derive(Debug)]
pub(crate) struct ArtError {
    /// No cfg or no install yet: tried again at the next call, not remembered.
    pub transient: bool,
    pub why: String,
}

impl ArtError {
    fn data(why: impl Into<String>) -> Self {
        Self { transient: false, why: why.into() }
    }
}

/// The picture, loaded once per mount and cfg; a failure from the data is kept and logged once.
#[derive(Resource, Default)]
pub struct ObjectivesArt {
    cache: GuiSprites,
    loaded: Option<Result<Arc<Art>, String>>,
    /// What the cfg said when `loaded` was made: a different cfg is read again.
    sig: Option<String>,
    /// Bumped on every load: a change of the picture redraws the panel.
    pub(crate) generation: u32,
}

impl ObjectivesArt {
    /// Whether a picture is held.
    #[cfg(test)]
    pub(crate) fn is_loaded(&self) -> bool {
        matches!(self.loaded, Some(Ok(_)))
    }

    /// Let go of a loaded picture and its sprite pages (the panel closed); a failure from the data stays.
    pub(crate) fn release(&mut self) {
        if matches!(self.loaded, Some(Ok(_))) {
            self.loaded = None;
            self.cache = GuiSprites::default();
        }
    }

    /// The picture, or `None` (and the reason logged once) when the data does not give one.
    pub(crate) fn get(
        &mut self,
        gui: Option<&sta_content::GuiLayout>,
        vfs: Option<&sta_vfs::Vfs>,
        images: &mut Assets<Image>,
    ) -> Option<Arc<Art>> {
        if let Some(g) = gui {
            let sig = signature(g);
            if self.sig.as_deref() != Some(&sig) {
                self.sig = Some(sig);
                self.loaded = None;
            }
        }
        if self.loaded.is_none() {
            let result = match (gui, vfs) {
                (Some(g), Some(v)) => load_art(g, v, images, &mut self.cache),
                _ => Err(ArtError { transient: true, why: "no install or gui cfg yet".to_owned() }),
            };
            match result {
                Ok(art) => {
                    self.generation += 1;
                    self.loaded = Some(Ok(Arc::new(art)));
                }
                Err(e) if e.transient => return None,
                Err(e) => {
                    warn!("objectives panel: {}; the panel keeps its own look", e.why);
                    self.generation += 1;
                    self.loaded = Some(Err(e.why));
                }
            }
        }
        self.loaded.as_ref().and_then(|r| r.as_ref().ok()).cloned()
    }
}

/// The cfg's panel keys, as one string to compare.
fn signature(gui: &sta_content::GuiLayout) -> String {
    let canvas = canvas_of(Some(gui));
    let stock = StockLayout::new(gui, Vec2::new(canvas.0, canvas.1));
    let pieces: Vec<_> = stock.pieces("objectivesBackgroundPanel").into_iter().map(|p| (p.frame, p.name, p.rect.x as i32, p.rect.y as i32, p.rect.w as i32, p.rect.h as i32)).collect();
    format!("{canvas:?} {:?} {:?} {pieces:?}", panel_area(Some(gui)), stock.rect("objectivesBlackArea").map(|r| (r.x, r.y, r.w, r.h)))
}

/// Every `objectivesBackgroundPanel_<n>` piece of the cfg cropped from its sprite `objectives_panel.<n>`
/// (the name the cfg's `objectivesBackgroundPanel` gives) in `gui_global.spr`.
pub(crate) fn load_art(gui: &sta_content::GuiLayout, vfs: &sta_vfs::Vfs, images: &mut Assets<Image>, cache: &mut GuiSprites) -> Result<Art, ArtError> {
    let canvas = canvas_of(Some(gui));
    let stock = StockLayout::new(gui, Vec2::new(canvas.0, canvas.1));
    let pieces = stock.pieces("objectivesBackgroundPanel");
    if pieces.is_empty() {
        return Err(ArtError::data("the gui cfg has no `objectivesBackgroundPanel` pieces"));
    }
    let area = panel_area(Some(gui));
    let (mut right, mut bottom) = (0.0f32, 0.0f32);
    for p in &pieces {
        right = right.max(p.rect.right());
        bottom = bottom.max(p.rect.bottom());
    }
    if right > area[2] + 1.0 || bottom > area[3] + 1.0 {
        return Err(ArtError::data("the cfg's pieces tile more than its `objectivesPanelArea`, which is not read"));
    }
    let table = gui_sprites::read_table(vfs, "sprites/gui_global.spr");
    let mut out = Vec::new();
    for piece in pieces {
        let name = format!("{}.{}", piece.name, piece.frame);
        let sprite = table.sprite(&name).ok_or_else(|| ArtError::data(format!("sprite `{name}` is not in gui_global.spr")))?;
        let (handle, size) = gui_sprites::page(sprite, GuiImageSampling::Nearest, vfs, cache, images, CompressedImageFormats::empty())
            .ok_or_else(|| ArtError::data(format!("texture of `{name}` is missing")))?;
        let crop = gui_sprites::crop(sprite, size).ok_or_else(|| ArtError::data(format!("sprite `{name}` has no usable rectangle")))?;
        out.push(ArtPiece { frame: piece.frame, rect: [piece.rect.x, piece.rect.y, piece.rect.w, piece.rect.h], image: handle, crop });
    }
    let black = stock.rect("objectivesBlackArea").map(|r| [r.x, r.y, r.w, r.h]);
    Ok(Art { pieces: out, black })
}

/// The canvas of the cfg the panel is authored on; 1600 x 1200 without one.
pub(crate) fn canvas_of(gui: Option<&sta_content::GuiLayout>) -> (f32, f32) {
    gui.map(|g| g.canvas).filter(|c| c.0 > 0 && c.1 > 0).map_or((1600.0, 1200.0), |c| (c.0 as f32, c.1 as f32))
}

/// `objectivesPanelArea` (a panel key the cfg keeps with its panel), in canvas units.
pub(crate) fn panel_area(gui: Option<&sta_content::GuiLayout>) -> [f32; 4] {
    gui.and_then(|c| c.panels.iter().find(|(k, _)| k.eq_ignore_ascii_case("objectives")).and_then(|(_, p)| p.area))
        .filter(|r| r.w > 0 && r.h > 0)
        .map_or([192.0, 80.0, 1280.0, 1024.0], |r| [r.x as f32, r.y as f32, r.w as f32, r.h as f32])
}

/// The panel on a window of `aspect`, as fractions of the window (`x y w h`): the cfg canvas keeps its aspect
/// and is centred (the group, [`HudFit::Centred`]) or fills the window ([`HudFit::Stretched`]), and the panel
/// area sits in it as the cfg places it.
pub(crate) fn panel_fractions(canvas: (f32, f32), area: [f32; 4], aspect: f32, fit: HudFit) -> [f32; 4] {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 4.0 / 3.0 };
    let screen = Vec2::new(aspect * 1000.0, 1000.0);
    let group = match fit {
        HudFit::Centred => fit_canvas(Vec2::new(canvas.0, canvas.1), screen).1,
        HudFit::Stretched => R::new(0.0, 0.0, screen.x, screen.y),
    };
    let (sx, sy) = (group.w / canvas.0, group.h / canvas.1);
    [(group.x + area[0] * sx) / screen.x, (group.y + area[1] * sy) / screen.y, area[2] * sx / screen.x, area[3] * sy / screen.y]
}

/// A piece's edges rounded to whole units of the panel so neighbours abut without seams: the left and top
/// edges are the rounded origins, the size the difference of the rounded far edges.
pub(crate) fn snapped(rect: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (rect[0].round(), rect[1].round());
    let (x1, y1) = ((rect[0] + rect[2]).round(), (rect[1] + rect[3]).round());
    [x0, y0, x1 - x0, y1 - y0]
}

/// The objective line's mark in the stock panel: open `[  ]`, complete `[X]` (the open mark seen in one frame of
/// the Armada II window; the done form is interim).
pub(crate) fn mark(done: bool) -> &'static str {
    if done { "[X]" } else { "[  ]" }
}

/// Authored data for the tests: a cfg, a sprite table and a texture that match no stock sheet.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::fs;

    /// A 4:3 canvas of 800 x 600 with a 400 x 300 panel of two pieces, a black area, and its text rectangles.
    pub const CFG: &str = "\
screenWidth = 800
screenHeight = 600
objectivesPanelArea = 100 50 400 300
objectivesBackgroundPanel = \"objectives_panel\"
objectivesBackgroundPanelSize = 2
objectivesBackgroundPanel_0 = 0 0 200 300
objectivesBackgroundPanel_1 = 200 0 200 300
objectivesBlackArea = 20 60 360 180
objectivesTitleInformationArea = 20 10 360 40
objectivesSummaryInformationArea = 20 60 360 30
objectivesTextInformationArea = 20 90 360 100
objectivesHeaderInformationArea = 20 200 360 30
objectivesOkayButtonArea = 250 260 120 30
";

    /// The same panel with the single text area of a cfg that has no title rectangle.
    pub const CFG_SINGLE: &str = "\
screenWidth = 800
screenHeight = 600
objectivesPanelArea = 100 50 400 300
objectivesBackgroundPanel = \"objectives_panel\"
objectivesBackgroundPanelSize = 2
objectivesBackgroundPanel_0 = 0 0 200 300
objectivesBackgroundPanel_1 = 200 0 200 300
objectivesBlackArea = 20 60 360 180
objectivesTextInformationArea = 20 4 350 224
objectivesTextColor = 1.0 0.5 0.0
objectivesOkayButtonArea = 250 260 120 30
";

    /// A cfg whose pieces tile more than its panel area.
    pub const CFG_OVERSIZE: &str = "\
screenWidth = 800
screenHeight = 600
objectivesPanelArea = 100 50 400 300
objectivesBackgroundPanel = \"objectives_panel\"
objectivesBackgroundPanelSize = 2
objectivesBackgroundPanel_0 = 0 0 400 300
objectivesBackgroundPanel_1 = 400 0 400 300
";

    pub const SPR: &[u8] = b"objectives_panel.0 tile 0 0 1 1\nobjectives_panel.1 tile 1 0 1 1\n";

    pub fn gui(text: &str) -> sta_content::GuiLayout {
        let (cfg, warnings) = sta_content::GuiCfg::parse("synthetic.cfg", text);
        assert!(warnings.is_empty(), "{warnings:?}");
        cfg.layout((800, 600))
    }

    /// A 2x1 true-colour TGA with alpha: one translucent pixel and one opaque.
    pub fn tga() -> Vec<u8> {
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12] = 2;
        bytes[14] = 1;
        bytes[16] = 32;
        bytes[17] = 0x28;
        bytes.extend_from_slice(&[40, 50, 60, 128, 90, 90, 90, 255]);
        bytes
    }

    /// A temporary install root, removed when dropped.
    pub struct Tmp(std::path::PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub fn mount(files: &[(&str, Vec<u8>)]) -> (sta_vfs::Vfs, Tmp) {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("sta64-objectives-panel-{}-{nonce}", std::process::id()));
        let data = root.join("Data");
        for (name, bytes) in files {
            let path = data.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fs::create_dir_all(&data).unwrap();
        (sta_vfs::Vfs::mount_install(&root, None).unwrap(), Tmp(root))
    }

    /// An install with the panel's sprites and texture.
    pub fn art_install() -> (sta_vfs::Vfs, Tmp) {
        mount(&[("sprites/gui_global.spr", SPR.to_vec()), ("textures/rgb/tile.tga", tga())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_keeps_its_aspect_and_is_centred_as_a_whole_on_a_wide_window() {
        // A 4:3 canvas on a 4:3 window: the area is where the cfg puts it, as fractions of the window.
        let area = [192.0, 80.0, 1280.0, 1024.0];
        let f = panel_fractions((1600.0, 1200.0), area, 4.0 / 3.0, HudFit::Centred);
        assert!((f[0] - 0.12).abs() < 1e-4 && (f[1] - 80.0 / 1200.0).abs() < 1e-4 && (f[2] - 0.8).abs() < 1e-4, "{f:?}");
        // 16:9: the group is 4:3 centred, so the panel is as high as before and 3/4 as wide in window terms.
        let w = panel_fractions((1600.0, 1200.0), area, 16.0 / 9.0, HudFit::Centred);
        assert!((w[3] - f[3]).abs() < 1e-4, "same height fraction: {w:?}");
        let real_aspect = (w[2] * 16.0) / (w[3] * 9.0);
        assert!((real_aspect - 1280.0 / 1024.0).abs() < 1e-3, "the panel's own aspect is kept: {real_aspect}");
        let group_left = (1.0 - 0.75) / 2.0;
        assert!((w[0] - (group_left + 0.12 * 0.75)).abs() < 1e-3, "{w:?}");
        // Stretched fills the window per axis.
        let s = panel_fractions((1600.0, 1200.0), area, 16.0 / 9.0, HudFit::Stretched);
        assert!((s[0] - 0.12).abs() < 1e-4 && (s[2] - 0.8).abs() < 1e-4, "{s:?}");
        // A canvas of another size (a 16:9 sheet) uses its own aspect.
        let sheet = panel_fractions((2134.0, 1200.0), [0.0, 0.0, 2134.0, 1200.0], 2134.0 / 1200.0, HudFit::Centred);
        assert!(sheet[0].abs() < 1e-3 && (sheet[2] - 1.0).abs() < 1e-3, "{sheet:?}");
    }

    #[test]
    fn pieces_snap_so_neighbours_abut() {
        let a = snapped([0.4, 0.0, 85.4, 10.0]);
        let b = snapped([85.8, 0.0, 85.4, 10.0]);
        assert_eq!(a[0] + a[2], b[0], "{a:?} {b:?}");
        assert_eq!(snapped([0.0, 0.0, 256.0, 256.0]), [0.0, 0.0, 256.0, 256.0]);
    }

    #[test]
    fn the_marks_of_an_objective_line() {
        assert_eq!(mark(false), "[  ]");
        assert_eq!(mark(true), "[X]");
    }

    use super::fixtures::*;

    #[test]
    fn the_art_is_the_cfg_pieces_cropped_from_the_table_and_keeps_the_texture_alpha() {
        let (vfs, _t) = art_install();
        let mut images = Assets::<Image>::default();
        let mut cache = GuiSprites::default();
        let art = load_art(&gui(CFG), &vfs, &mut images, &mut cache).expect("art");
        assert_eq!(art.pieces.iter().map(|p| p.frame).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(art.pieces[1].rect, [200.0, 0.0, 200.0, 300.0]);
        assert_eq!(art.black, Some([20.0, 60.0, 360.0, 180.0]));
        let data = images.get(&art.pieces[0].image).unwrap().data.as_ref().unwrap().clone();
        assert_eq!((data[3], data[7]), (128, 255), "no colour key: the texture's own alpha");
        // A cfg without the black area has none.
        let plain = gui(&CFG.replace("objectivesBlackArea = 20 60 360 180\n", ""));
        assert_eq!(load_art(&plain, &vfs, &mut images, &mut cache).unwrap().black, None);
    }

    #[test]
    fn missing_keys_sprites_textures_or_a_tiling_larger_than_the_panel_give_no_art_and_say_why() {
        let mut images = Assets::<Image>::default();
        let mut cache = GuiSprites::default();
        let (no_table, _t1) = mount(&[]);
        let why = load_art(&gui(CFG), &no_table, &mut images, &mut cache).unwrap_err().why;
        assert!(why.contains("objectives_panel.0"), "{why}");
        let (no_texture, _t2) = mount(&[("sprites/gui_global.spr", SPR.to_vec())]);
        assert!(load_art(&gui(CFG), &no_texture, &mut images, &mut cache).unwrap_err().why.contains("texture"));
        let (vfs, _t3) = art_install();
        let why = load_art(&gui("screenWidth = 800\nscreenHeight = 600\n"), &vfs, &mut images, &mut cache).unwrap_err().why;
        assert!(why.contains("no `objectivesBackgroundPanel` pieces"), "{why}");
        let why = load_art(&gui(CFG_OVERSIZE), &vfs, &mut images, &mut cache).unwrap_err().why;
        assert!(why.contains("tile more than"), "{why}");
    }

    #[test]
    fn a_transient_failure_is_retried_a_data_failure_is_kept_and_a_new_cfg_is_read_again() {
        let (vfs, _t) = art_install();
        let mut images = Assets::<Image>::default();
        let mut art = ObjectivesArt::default();
        assert!(art.get(None, Some(&vfs), &mut images).is_none() && art.generation == 0, "no cfg yet: nothing remembered");
        assert!(art.get(Some(&gui(CFG)), None, &mut images).is_none() && art.generation == 0);
        let first = art.get(Some(&gui(CFG)), Some(&vfs), &mut images).expect("loads");
        assert_eq!(art.generation, 1);
        assert!(Arc::ptr_eq(&first, &art.get(Some(&gui(CFG)), Some(&vfs), &mut images).unwrap()), "the same cfg is not read again");
        // A cfg the data cannot satisfy is kept as a failure (one log line), and a good one replaces it.
        assert!(art.get(Some(&gui(CFG_OVERSIZE)), Some(&vfs), &mut images).is_none());
        let after_failure = art.generation;
        assert!(art.get(Some(&gui(CFG_OVERSIZE)), Some(&vfs), &mut images).is_none() && art.generation == after_failure);
        assert!(art.get(Some(&gui(CFG)), Some(&vfs), &mut images).is_some());
    }
}
