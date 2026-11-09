//! Where the HUD's panels go (HUD.md, "The bottom row", "The info panel").
//!
//! Everything is in theme units: a screen [`SCREEN_H`] units tall and
//! `SCREEN_H * aspect` wide, which the plugin maps onto the window with
//! `UiScale`. The rectangles of the race's GUI cfg ([`GuiLayout`], CON-12)
//! are in the cfg's canvas; they size the panels, scaled by one factor
//! (`SCREEN_H / canvas height`, so a wide screen is not stretched), and where
//! a cfg has none the defaults are stock's (a 1600 x 1200 canvas).
//!
//! - **Resources strip**: `resourcePanelArea`'s size, centred at the top.
//! - **Info panel**: `infoPanelArea` and its `_n` variants are the three
//!   heights; sorted by height they are low (one object), middle (a builder)
//!   and high (a group).
//! - **Minimap**: the radar's frame, bottom left ([`RADAR_PANEL`]).
//! - **Grid** (the 5 x 3 command bar): [`GRID_COLS`] x [`GRID_ROWS`] cells,
//!   placed against the minimap and the info panel by [`HudLayout::bottom_row`]:
//!   beside the minimap when the row has room (16:9 and wider), else above it
//!   (4:3).

use bevy::math::Vec2;
use sta_content::gui_cfg::{GuiLayout, Panel, Rect};

/// Screen height in theme units, the theme's 1080-line screen.
pub const SCREEN_H: f32 = 1080.0;
/// Margin between a panel and the screen edge.
pub const MARGIN: f32 = 10.0;
/// Gap between panels in the bottom row.
pub const GAP: f32 = 12.0;
/// The command grid's columns and rows (Refit's `grid`).
pub const GRID_COLS: usize = 5;
pub const GRID_ROWS: usize = 3;
/// A grid cell's side and the gap between cells.
pub const CELL: f32 = 56.0;
pub const CELL_GAP: f32 = 4.0;
/// Padding inside the grid's frame.
pub const GRID_PAD: f32 = 8.0;
/// The radar's panel (UI-05) as the kit lays it out; measured on a frame run
/// (`tests/hud_frame.rs` checks the radar fills it).
pub const RADAR_PANEL: Vec2 = Vec2::new(347.0, 458.0);

/// A rectangle in theme units, top left origin.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
}

/// The three heights of the info panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InfoHeight {
    /// One object.
    Low,
    /// A builder (its queue shows).
    Middle,
    /// A group.
    High,
}

impl InfoHeight {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Stock's info panel on its 1600 x 1200 canvas: the three heights.
const STOCK_INFO: [(i32, i32); 3] = [(880, 85), (880, 193), (880, 310)];
/// The smallest each height may be, in units: what the panel's rows need.
const INFO_MIN: [Vec2; 3] = [Vec2::new(420.0, 76.0), Vec2::new(420.0, 160.0), Vec2::new(420.0, 240.0)];
const STOCK_CANVAS: (i32, i32) = (1600, 1200);

/// The panels' places for one screen.
#[derive(Debug, Clone, PartialEq)]
pub struct HudLayout {
    /// Screen size in theme units.
    pub screen: Vec2,
    pub resources: R,
    pub minimap: R,
    /// Info panel sizes, low / middle / high.
    pub info: [Vec2; 3],
    pub grid_size: Vec2,
    /// Resources the race has no use for (`noDilithium`, `noLatinum`,
    /// `noMetal`, `noBiomatter`).
    pub no_resource: [bool; 4],
}

/// Where the grid and the info panel are for one selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BottomRow {
    pub grid: R,
    pub info: R,
    /// The grid sits beside the minimap rather than above it.
    pub grid_beside: bool,
}

impl HudLayout {
    /// The layout of a screen `aspect` wide (width over height), from the
    /// race's cfg when there is one.
    pub fn new(cfg: Option<&GuiLayout>, aspect: f32) -> HudLayout {
        let screen = Vec2::new(SCREEN_H * aspect, SCREEN_H);
        let canvas = cfg.map(|l| l.canvas).filter(|c| c.0 > 0 && c.1 > 0).unwrap_or(STOCK_CANVAS);
        let s = SCREEN_H / canvas.1 as f32;
        let panel = |name: &str| cfg.and_then(|l| l.panels.get(name));

        let res = panel("resource").and_then(|p| p.area).filter(|r| r.w > 0 && r.h > 0);
        let (rw, rh) = res.map_or((560.0, 40.0), |r| ((r.w as f32 * s).max(280.0), (r.h as f32 * s).max(36.0)));
        let resources = R::new(((screen.x - rw) / 2.0).round(), 8.0, rw, rh);

        let info = info_sizes(panel("info"), s);

        let no = |name: &str| {
            let key = format!("no{name}");
            cfg.is_some_and(|l| l.numbers.iter().any(|(k, v)| k.eq_ignore_ascii_case(&key) && *v != 0.0))
        };
        let minimap = R::new(MARGIN, screen.y - MARGIN - RADAR_PANEL.y, RADAR_PANEL.x, RADAR_PANEL.y);
        let grid_size = Vec2::new(
            GRID_COLS as f32 * CELL + (GRID_COLS - 1) as f32 * CELL_GAP + 2.0 * GRID_PAD,
            GRID_ROWS as f32 * CELL + (GRID_ROWS - 1) as f32 * CELL_GAP + 2.0 * GRID_PAD,
        );
        HudLayout {
            screen,
            resources,
            minimap,
            info,
            grid_size,
            no_resource: [no("Dilithium"), no("Latinum"), no("Metal"), no("Biomatter")],
        }
    }

    /// The bottom row for an info panel of `height` (`None`: nothing selected,
    /// no panel), as HUD.md lays it out:
    ///
    /// 1. the info panel centred, if the grid fits between the minimap and it;
    /// 2. else moved right just far enough for the grid to sit beside the
    ///    minimap, if the row holds both;
    /// 3. else centred in the row clear of the minimap, the grid above the
    ///    minimap.
    pub fn bottom_row(&self, height: Option<InfoHeight>) -> BottomRow {
        let g = self.grid_size;
        let size = height.map_or(Vec2::ZERO, |h| self.info[h.index()]);
        let floor = self.screen.y - MARGIN;
        let row_left = self.minimap.right() + GAP;
        let row_right = self.screen.x - MARGIN;
        let beside = R::new(row_left, floor - g.y, g.x, g.y);
        let above = R::new(self.minimap.x, self.minimap.y - GAP - g.y, g.x, g.y);
        let info_at = |x: f32| R::new(x, floor - size.y, size.x, size.y);
        let centred = (self.screen.x - size.x) / 2.0;
        if height.is_none() || centred >= beside.right() + GAP {
            return BottomRow { grid: beside, info: info_at(centred), grid_beside: true };
        }
        let moved = beside.right() + GAP;
        if moved + size.x <= row_right {
            return BottomRow { grid: beside, info: info_at(moved), grid_beside: true };
        }
        let mid = row_left + ((row_right - row_left) - size.x).max(0.0) / 2.0;
        BottomRow { grid: above, info: info_at(mid), grid_beside: false }
    }

    /// Top of the notices column: under the resources strip.
    pub fn notices_top(&self) -> f32 {
        self.resources.bottom() + GAP
    }

    /// Every rectangle the pointer must not pass through to the map, for a
    /// selection with an info panel of `height`.
    pub fn solid(&self, height: Option<InfoHeight>) -> Vec<R> {
        let row = self.bottom_row(height);
        let mut v = vec![self.resources, self.minimap, row.grid];
        if height.is_some() {
            v.push(row.info);
        }
        v
    }
}

/// The three info heights from the cfg's `infoPanelArea` and variants (sorted
/// by height), stock's when it has fewer than three; never below [`INFO_MIN`].
fn info_sizes(p: Option<&Panel>, s: f32) -> [Vec2; 3] {
    let mut rects: Vec<Rect> = p
        .into_iter()
        .flat_map(|p| p.area.into_iter().chain(p.variants.values().copied()))
        .filter(|r| r.w > 0 && r.h > 0)
        .collect();
    rects.sort_by_key(|r| r.h);
    rects.dedup_by_key(|r| r.h);
    let pick: Vec<(i32, i32)> = if rects.len() >= 3 {
        let n = rects.len();
        [rects[0], rects[n / 2], rects[n - 1]].iter().map(|r| (r.w, r.h)).collect()
    } else {
        STOCK_INFO.to_vec()
    };
    let mut out = [Vec2::ZERO; 3];
    for i in 0..3 {
        out[i] = Vec2::new(pick[i].0 as f32 * s, pick[i].1 as f32 * s).max(INFO_MIN[i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::gui_cfg::GuiCfg;

    const FO: &str = "screenWidth = 2134\nscreenHeight = 1200\nresourcePanelArea = 0 8 935 40\nnoLatinum = 1\n\
        infoPanelArea = 0 900 600 300\ninfoPanelArea_1 = 0 1000 600 200\ninfoPanelArea_2 = 0 1100 600 100\n";

    fn fo() -> HudLayout {
        let (cfg, w) = GuiCfg::parse("gui_fo.cfg", FO);
        assert!(w.is_empty(), "{w:?}");
        HudLayout::new(Some(&cfg.layout((640, 480))), 16.0 / 9.0)
    }

    #[test]
    fn the_cfg_sizes_the_panels_and_sorts_the_heights() {
        let l = fo();
        // One factor, 1080 / 1200: no stretch on a wide screen.
        let w = 935.0f32 * (1080.0 / 1200.0);
        assert_eq!(l.resources.w, w);
        assert_eq!(l.resources.x, ((1920.0 - w) / 2.0).round());
        // The variants are the three heights, shortest first, never below the minimum.
        assert_eq!(l.info[0], Vec2::new(540.0, 90.0));
        assert_eq!(l.info[1], Vec2::new(540.0, 180.0));
        assert_eq!(l.info[2], Vec2::new(540.0, 270.0));
        assert_eq!(l.no_resource, [false, true, false, false]);
    }

    #[test]
    fn stock_sizes_without_a_cfg() {
        let l = HudLayout::new(None, 16.0 / 9.0);
        assert_eq!(l.screen, Vec2::new(1920.0, 1080.0));
        assert!((l.info[1].y - 173.7).abs() < 0.01 && l.info[1].x == 792.0);
        assert_eq!(l.no_resource, [false; 4]);
        // One area alone is not three heights: stock's three.
        let (cfg, _) = GuiCfg::parse("a.cfg", "infoPanelArea = 146 364 247 116\n");
        assert_eq!(HudLayout::new(Some(&cfg.layout((640, 480))), 4.0 / 3.0).info[0].x, 880.0 * 1080.0 / 480.0);
    }

    #[test]
    fn a_wide_screen_puts_the_grid_beside_the_minimap() {
        let l = HudLayout::new(None, 16.0 / 9.0);
        let r = l.bottom_row(Some(InfoHeight::Low));
        assert!(r.grid_beside);
        assert_eq!(r.grid.x, l.minimap.right() + GAP);
        assert_eq!(r.grid.bottom(), 1070.0);
        // The panel moved right just far enough, and nothing overlaps.
        assert_eq!(r.info.x, r.grid.right() + GAP);
        assert!(r.info.right() <= l.screen.x - MARGIN);
        // Nothing selected: no panel, the grid keeps its place.
        assert_eq!(l.bottom_row(None).grid, r.grid);
    }

    #[test]
    fn a_narrow_screen_puts_the_grid_above_the_minimap() {
        let l = HudLayout::new(None, 4.0 / 3.0);
        let r = l.bottom_row(Some(InfoHeight::Low));
        assert!(!r.grid_beside);
        assert_eq!(r.grid.x, l.minimap.x);
        assert!(r.grid.bottom() <= l.minimap.y);
        // The panel is clear of the minimap.
        assert!(r.info.x >= l.minimap.right());
    }

    #[test]
    fn a_very_wide_screen_centres_the_panel() {
        let l = HudLayout::new(None, 32.0 / 9.0);
        let r = l.bottom_row(Some(InfoHeight::Middle));
        assert!(r.grid_beside);
        assert_eq!(r.info.x, (l.screen.x - r.info.w) / 2.0);
        assert!(r.info.x >= r.grid.right() + GAP);
    }

    #[test]
    fn heights_grow_the_panel_upward() {
        let l = HudLayout::new(None, 16.0 / 9.0);
        let lo = l.bottom_row(Some(InfoHeight::Low)).info;
        let hi = l.bottom_row(Some(InfoHeight::High)).info;
        assert_eq!(lo.bottom(), hi.bottom());
        assert!(hi.h > lo.h);
        assert!(l.solid(Some(InfoHeight::High)).len() == 4 && l.solid(None).len() == 3);
    }
}
