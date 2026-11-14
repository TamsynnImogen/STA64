//! Pure, authored stock HUD geometry from a resolved race GUI CFG.

use bevy::prelude::{Color, Vec2};
use sta_content::gui_cfg::{GuiLayout, Rect};

use super::super::layout::R;
use super::super::model::Info;

/// A configured background piece, with its native SPR frame index.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub rect: R,
    pub name: String,
    pub frame: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaletteLayout {
    pub origin: Vec2,
    pub cell: Vec2,
    pub gap: Vec2,
    pub columns: usize,
    pub rows: usize,
}

impl PaletteLayout {
    pub fn cell_rect(&self, col: usize, row: usize) -> R {
        R::new(
            self.origin.x + (self.cell.x + self.gap.x) * col as f32,
            self.origin.y - (self.cell.y + self.gap.y) * row as f32,
            self.cell.x,
            self.cell.y,
        )
    }
}

/// CFG canvas coordinates mapped to the screen by one factor for both axes. The sheet's
/// canvas, scaled and centred, is [`StockLayout::hud`]: the one rectangle every consumer
/// (panels, pointer, minimap, notices) shares; the screen outside it belongs to the world.
#[derive(Clone, Debug)]
pub struct StockLayout {
    pub cfg: GuiLayout,
    pub screen: Vec2,
    pub canvas: Vec2,
    /// Screen units per canvas unit, the same on both axes.
    pub scale: f32,
    /// The scaled canvas, centred in the screen (whole units).
    pub hud: R,
}

/// The sheet canvas scaled by one factor (window height over sheet height, or window width
/// over sheet width when the window is narrower than the sheet) and centred in `screen`.
/// The origin snaps to whole units, so a window of the sheet's own aspect gets zero margins.
pub fn fit_canvas(canvas: Vec2, screen: Vec2) -> (f32, R) {
    let scale = (screen.y / canvas.y).min(screen.x / canvas.x);
    let size = canvas * scale;
    let origin = ((screen - size) / 2.0).round().max(Vec2::ZERO);
    (scale, R::new(origin.x, origin.y, size.x, size.y))
}

impl StockLayout {
    pub fn new(cfg: &GuiLayout, screen: Vec2) -> Self {
        let canvas = if cfg.canvas.0 > 0 && cfg.canvas.1 > 0 {
            Vec2::new(cfg.canvas.0 as f32, cfg.canvas.1 as f32)
        } else {
            Vec2::new(1600.0, 1200.0)
        };
        let (scale, hud) = fit_canvas(canvas, screen);
        Self { cfg: cfg.clone(), screen, canvas, scale, hud }
    }

    /// The layout for a window of this aspect, in the HUD's 1080-line theme units.
    pub fn for_aspect(cfg: &GuiLayout, aspect: f32) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
        Self::new(cfg, Vec2::new(super::super::layout::SCREEN_H * aspect, super::super::layout::SCREEN_H))
    }

    /// Whether a screen point (theme units) is inside the HUD sheet's rectangle.
    pub fn in_hud(&self, p: Vec2) -> bool {
        self.hud.contains(p)
    }

    /// A canvas rectangle, panel-relative: scaled, not moved by the centring offset.
    fn scaled(&self, r: Rect) -> R {
        R::new(
            (r.x as f32 * self.scale + 0.5).floor(),
            (r.y as f32 * self.scale + 0.5).floor(),
            (r.w as f32 * self.scale + 0.5).floor(),
            (r.h as f32 * self.scale + 0.5).floor(),
        )
    }

    /// A canvas rectangle in screen units: scaled and moved by the centring offset.
    fn placed(&self, r: Rect) -> R {
        let mut out = self.scaled(r);
        out.x += self.hud.x;
        out.y += self.hud.y;
        out
    }

    /// Includes zero sized rectangles, which can still carry an offset.
    pub fn raw_rect(&self, key: &str) -> Option<Rect> {
        if let Some((_, r)) = self.cfg.rects.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            return Some(*r);
        }
        for (prefix, panel) in &self.cfg.panels {
            if key.eq_ignore_ascii_case(&format!("{prefix}PanelArea")) {
                return panel.area;
            }
            if key.eq_ignore_ascii_case(&format!("{prefix}BackgroundPanelArea")) {
                return panel.background_area;
            }
            if let Some(suffix) = key.to_ascii_lowercase().strip_prefix(&format!("{prefix}panelarea_"))
                && let Ok(n) = suffix.parse::<u32>()
            {
                return panel.variants.get(&n).copied();
            }
            if let Some(suffix) = key
                .to_ascii_lowercase()
                .strip_prefix(&format!("{prefix}backgroundpanel_"))
                && let Ok(n) = suffix.parse::<u32>()
            {
                return panel.pieces.get(&n).copied();
            }
        }
        None
    }

    /// A visible rectangle, panel-relative (not moved by the centring offset; use `panel`,
    /// `info_area` or `at` for screen positions); authored zero or negative sizes disable it.
    pub fn rect(&self, key: &str) -> Option<R> {
        self.raw_rect(key)
            .filter(|r| r.w > 0 && r.h > 0)
            .map(|r| self.scaled(r))
    }

    pub fn point(&self, key: &str) -> Option<Vec2> {
        self.raw_rect(key).map(|r| {
            Vec2::new((r.x as f32 * self.scale + 0.5).floor(), (r.y as f32 * self.scale + 0.5).floor())
        })
    }

    pub fn name(&self, key: &str) -> Option<&str> {
        if let Some((_, value)) = self.cfg.names.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            return Some(value);
        }
        self.cfg.panels.iter().find_map(|(prefix, panel)| {
            if key.eq_ignore_ascii_case(&format!("{prefix}BackgroundPanel")) {
                panel.background.as_deref()
            } else if key.eq_ignore_ascii_case(&format!("{prefix}DefaultCursor")) {
                panel.cursor.as_deref()
            } else {
                None
            }
        })
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        if let Some((_, value)) = self.cfg.numbers.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            return Some(*value);
        }
        self.cfg.panels.iter().find_map(|(prefix, panel)| {
            key.eq_ignore_ascii_case(&format!("{prefix}BackgroundPanelSize"))
                .then_some(panel.background_size)
                .flatten()
                .map(|n| n as f64)
        })
    }

    pub fn colour(&self, key: &str) -> Option<Color> {
        self.cfg
            .colours
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, c)| Color::srgba(c.r, c.g, c.b, c.a))
    }

    /// A panel's rectangle in screen units (the only absolute keys are the panel areas).
    pub fn panel(&self, name: &str) -> Option<R> {
        self.raw_rect(&format!("{name}PanelArea"))
            .filter(|r| r.w > 0 && r.h > 0)
            .map(|r| self.placed(r))
    }

    pub fn info_area(&self, variant: usize) -> Option<R> {
        let panel = self.cfg.panels.get("info")?;
        let raw = panel
            .variants
            .get(&u32::try_from(variant).ok()?)
            .copied()
            .or(panel.area)?;
        (raw.w > 0 && raw.h > 0).then(|| self.placed(raw))
    }

    pub fn info_origin(&self, variant: usize) -> Vec2 {
        self.info_area(variant).map_or(Vec2::ZERO, |r| Vec2::new(r.x, r.y))
    }

    pub fn at(&self, origin: Vec2, key: &str) -> Option<R> {
        self.rect(key).map(|mut r| {
            r.x += origin.x;
            r.y += origin.y;
            r
        })
    }

    /// Count is authoritative. Only existing numbered CFG rectangles below it draw.
    pub fn pieces(&self, prefix: &str) -> Vec<Piece> {
        let count = self.number(&format!("{prefix}Size")).unwrap_or(0.0);
        if !count.is_finite() || count <= 0.0 {
            return Vec::new();
        }
        let name = self.name(prefix).unwrap_or("default_sprite");
        let mut out = Vec::new();
        // Iterate the bounded CFG maps, never a potentially untrusted Size range.
        let add = |frame: usize, rect: Rect, out: &mut Vec<Piece>| {
            if (frame as f64) < count && rect.w > 0 && rect.h > 0 {
                out.push(Piece {
                    rect: self.scaled(rect),
                    name: name.to_owned(),
                    frame,
                });
            }
        };
        let key_prefix = format!("{prefix}_");
        for (key, rect) in &self.cfg.rects {
            if let Some(suffix) = key.to_ascii_lowercase().strip_prefix(&key_prefix.to_ascii_lowercase())
                && let Ok(frame) = suffix.parse::<usize>()
            {
                add(frame, *rect, &mut out);
            }
        }
        if let Some(panel_prefix) = prefix.to_ascii_lowercase().strip_suffix("backgroundpanel")
            && let Some(panel) = self.cfg.panels.get(panel_prefix)
        {
            for (&frame, &rect) in &panel.pieces {
                add(frame as usize, rect, &mut out);
            }
        }
        out.sort_by_key(|piece| piece.frame);
        out
    }

    /// The stock A palette uses the configured cell, grid bounds and popup origin.
    pub fn palette(&self) -> Option<PaletteLayout> {
        let cell = self.rect("paletteSingleButtonArea")?;
        let gap = self.number("paletteSingleButtonGap").unwrap_or(0.0) as f32;
        let bounds = |key: &str, default: usize| -> Option<usize> {
            match self.number(key) {
                None => Some(default),
                Some(n) if n.is_finite() && n > 0.0 && n.fract() == 0.0 && n <= usize::MAX as f64 => Some(n as usize),
                _ => None,
            }
        };
        Some(PaletteLayout {
            origin: Vec2::new(
                (self.number("popupPaletteXA").unwrap_or(30.0) as f32 * self.scale).round() + self.hud.x,
                (self.number("popupPaletteYA").unwrap_or(30.0) as f32 * self.scale).round() + self.hud.y,
            ),
            cell: Vec2::new(cell.w, cell.h),
            gap: Vec2::splat((gap * self.scale).round()),
            columns: bounds("staticPaletteWidthA", 3)?,
            rows: bounds("staticPaletteHeightA", 7)?,
        })
    }
}

/// Native stock panel variants, independent of LCARS' height categories.
pub fn info_variant(info: &Info) -> usize {
    match info {
        Info::None => 0,
        Info::Single(s) => {
            if s.builder {
                2
            } else {
                1
            }
        }
        Info::Group(group) => {
            if group.len() <= 8 {
                1
            } else {
                2
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::model::{Single, Tile};
    use super::*;
    use sta_content::gui_cfg::{Colour, Panel};
    use sta_sim::EntityId;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn typed_panels_and_generic_keys_scale_separately() {
        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        let mut info = Panel {
            name: "Info".into(),
            area: Some(rect(10, 20, 400, 200)),
            ..Default::default()
        };
        info.variants.insert(1, rect(30, 40, 100, 80));
        info.variants.insert(2, rect(30, 40, 0, 80));
        info.background = Some("info_sprite".into());
        info.background_size = Some(2);
        info.pieces.insert(0, rect(0, 0, 40, 20));
        info.pieces.insert(1, rect(40, 0, 0, 20));
        cfg.panels.insert("info".into(), info);
        cfg.rects.insert("InfoBarOffset_0".into(), rect(17, -4, 0, 0));
        cfg.names.insert("InformationFont".into(), "font".into());
        cfg.colours.insert(
            "InfoColor".into(),
            Colour {
                r: 0.2,
                g: 0.3,
                b: 0.4,
                a: 0.5,
            },
        );
        // 800x900 against a 1600x1200 sheet: one factor 0.5 (the width), centred vertically.
        let l = StockLayout::new(&cfg, Vec2::new(800.0, 900.0));
        assert_eq!(l.scale, 0.5);
        assert_eq!(l.hud, R::new(0.0, 150.0, 800.0, 600.0));
        assert_eq!(l.panel("INFO"), Some(R::new(5.0, 160.0, 200.0, 100.0)));
        assert_eq!(l.info_area(1), Some(R::new(15.0, 170.0, 50.0, 40.0)));
        assert_eq!(l.info_area(2), None);
        assert_eq!(l.raw_rect("INFOPANELAREA_2"), Some(rect(30, 40, 0, 80)));
        assert_eq!(l.point("infobaroffset_0"), Some(Vec2::new(9.0, -2.0)));
        assert_eq!(l.rect("InfoBarOffset_0"), None);
        assert_eq!(
            l.at(Vec2::new(100.0, 200.0), "InfoPanelArea_1"),
            Some(R::new(115.0, 220.0, 50.0, 40.0))
        );
        assert_eq!(l.name("INFORMATIONFONT"), Some("font"));
        assert_eq!(l.name("INFOBACKGROUNDPANEL"), Some("info_sprite"));
        assert_eq!(l.number("INFOBACKGROUNDPANELSIZE"), Some(2.0));
        assert_eq!(l.colour("INFOCOLOR"), Some(Color::srgba(0.2, 0.3, 0.4, 0.5)));
        assert_eq!(
            l.pieces("infoBackgroundPanel"),
            vec![Piece {
                rect: R::new(0.0, 0.0, 20.0, 10.0),
                name: "info_sprite".into(),
                frame: 0
            }]
        );
    }

    #[test]
    fn pieces_use_count_and_generic_resource_panel() {
        let mut cfg = GuiLayout {
            canvas: (0, 1200),
            ..Default::default()
        };
        cfg.names.insert("ResourcePanel".into(), "resource".into());
        cfg.numbers.insert("resourcePanelSize".into(), 2.0);
        cfg.rects.insert("resourcePanel_0".into(), rect(0, 0, 40, 20));
        cfg.rects.insert("resourcePanel_1".into(), rect(40, 0, 20, 20));
        cfg.rects.insert("resourcePanel_99".into(), rect(0, 0, 20, 20));
        let l = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert_eq!(l.canvas, Vec2::new(1600.0, 1200.0));
        assert_eq!(
            l.pieces("RESOURCEPANEL").iter().map(|p| p.frame).collect::<Vec<_>>(),
            [0, 1]
        );
        cfg.numbers.insert("resourcePanelSize".into(), 0.0);
        assert!(StockLayout::new(&cfg, l.screen).pieces("resourcePanel").is_empty());
        cfg.numbers.remove("resourcePanelSize");
        assert!(StockLayout::new(&cfg, l.screen).pieces("resourcePanel").is_empty());
        cfg.numbers.insert("resourcePanelSize".into(), 1e18);
        assert_eq!(StockLayout::new(&cfg, l.screen).pieces("resourcePanel").len(), 3);
        cfg.names.remove("ResourcePanel");
        let unnamed = StockLayout::new(&cfg, l.screen).pieces("resourcePanel");
        assert_eq!(unnamed[0].name, "default_sprite");
    }

    /// Fleet Ops' 16:9 sheet (canvas 2134x1200, minimap panel 351 wide, palette at 264)
    /// on a 21:9 window: the HUD is the 16:9 sheet scaled by one factor and centred, so the
    /// palette sits where it does at 16:9 (inside the authored minimap panel, as stock has
    /// it) plus the centring offset; nothing is moved relative to the minimap.
    #[test]
    fn palette_moves_with_the_centred_sheet() {
        let mut cfg = GuiLayout { canvas: (2134, 1200), ..Default::default() };
        cfg.rects.insert("paletteSingleButtonArea".into(), rect(0, 0, 88, 88));
        cfg.rects.insert("minimapPanelArea".into(), rect(-1, 818, 351, 383));
        cfg.numbers.insert("popupPaletteXA".into(), 264.0);
        cfg.numbers.insert("popupPaletteYA".into(), 940.0);
        let base = StockLayout::new(&cfg, Vec2::new(2134.0, 1200.0));
        assert_eq!(base.hud, R::new(0.0, 0.0, 2134.0, 1200.0));
        for screen in [Vec2::new(3440.0, 1440.0), Vec2::new(2560.0, 1080.0), Vec2::new(5120.0, 1440.0)] {
            let l = StockLayout::new(&cfg, screen);
            let (map, p) = (l.panel("minimap").unwrap(), l.palette().unwrap());
            let (m0, p0) = (base.panel("minimap").unwrap(), base.palette().unwrap());
            let k = screen.y / 1200.0;
            assert_eq!(p.origin.x - map.x, (264.0 * k).round() - (-k + 0.5).floor(), "{screen:?}");
            assert_eq!(p.origin.y - map.y, (940.0 * k).round() - (818.0 * k + 0.5).floor(), "{screen:?}");
            assert!(p.origin.x >= l.hud.x && p0.origin.x >= m0.x);
        }
    }

    /// A sheet of the given canvas with a left panel, a minimap panel and a palette.
    fn sheet(canvas: (i32, i32)) -> GuiLayout {
        let mut cfg = GuiLayout { canvas, ..Default::default() };
        cfg.rects.insert("minimapPanelArea".into(), rect(0, canvas.1 * 2 / 3, canvas.1 / 3, canvas.1 / 3));
        cfg.rects.insert("minimapDisplayArea".into(), rect(20, 20, 200, 200));
        cfg.rects.insert("paletteSingleButtonArea".into(), rect(0, 0, 80, 80));
        cfg.numbers.insert("popupPaletteXA".into(), 400.0);
        cfg.numbers.insert("popupPaletteYA".into(), 900.0);
        cfg
    }

    /// UI-37 (ADR 0008 H1 to H3): windows of 4:3, 16:9, 21:9 and 32:9 against a 4:3 and a
    /// 16:9 sheet: one factor, the sheet's own aspect, centred, never larger than the window.
    #[test]
    fn one_factor_clamped_to_the_sheet_and_centred() {
        for canvas in [(1600, 1200), (2134, 1200)] {
            let cfg = sheet(canvas);
            let sheet_aspect = canvas.0 as f32 / canvas.1 as f32;
            for aspect in [4.0 / 3.0, 16.0 / 9.0, 21.0 / 9.0, 32.0 / 9.0] {
                let l = StockLayout::for_aspect(&cfg, aspect);
                let h = l.hud;
                let why = format!("{canvas:?} at {aspect}");
                assert_eq!(l.screen.y, 1080.0, "{why}");
                // Uniform: the HUD keeps the sheet's aspect, whatever the window.
                assert!((h.w / h.h - sheet_aspect).abs() < 2e-3, "{why}: {h:?}");
                assert!(h.w <= l.screen.x + 0.5 && h.h <= l.screen.y + 0.5, "{why}: {h:?}");
                // Height-limited unless the window is narrower than the sheet.
                let want = (1080.0 / canvas.1 as f32).min(l.screen.x / canvas.0 as f32);
                assert_eq!(l.scale, want, "{why}");
                // Centred, to whole units, margins equal within rounding.
                assert!((h.x - (l.screen.x - h.w - h.x)).abs() <= 1.0, "{why}: {h:?}");
                assert!((h.y - (l.screen.y - h.h - h.y)).abs() <= 1.0, "{why}: {h:?}");
                // The minimap panel is the canvas rectangle scaled by that one factor,
                // moved by the centring offset: square stays square.
                let map = l.panel("minimap").unwrap();
                assert!((map.w - map.h).abs() <= 1.0, "{why}: {map:?}");
                assert_eq!(map.x, h.x + (0.0 * l.scale + 0.5).floor(), "{why}");
                assert_eq!(map.y, h.y + ((canvas.1 * 2 / 3) as f32 * l.scale + 0.5).floor(), "{why}");
                let display = l.rect("minimapDisplayArea").unwrap();
                assert!((display.w - display.h).abs() <= 1.0 && display.w > 0.0, "{why}: {display:?}");
                // Everything lies inside the HUD rectangle.
                assert!(map.x >= h.x && map.right() <= h.right() + 1.0 && map.bottom() <= h.bottom() + 1.0, "{why}");
                let cell = l.palette().unwrap().cell_rect(0, 0);
                assert!(cell.x >= h.x && cell.y >= h.y, "{why}: {cell:?}");
                // A window wider than the sheet leaves margins left and right only.
                if aspect > sheet_aspect * 1.01 {
                    assert!(h.x > 0.0 && h.y == 0.0 && h.h == 1080.0, "{why}: {h:?}");
                    assert!(!l.in_hud(Vec2::new(h.x - 1.0, 500.0)) && !l.in_hud(Vec2::new(h.right() + 1.0, 500.0)), "{why}");
                    assert!(l.in_hud(Vec2::new(h.x + 1.0, 500.0)), "{why}");
                }
            }
        }
        // Narrower than the sheet (4:3 window, 16:9 sheet): the width limits, centred vertically.
        let l = StockLayout::for_aspect(&sheet((2134, 1200)), 4.0 / 3.0);
        assert_eq!(l.scale, 1440.0 / 2134.0);
        assert_eq!(l.hud.x, 0.0);
        assert!(l.hud.y > 0.0 && (l.hud.y - (1080.0 - l.hud.h - l.hud.y)).abs() <= 1.0);
    }

    /// A window of the sheet's own aspect lays out exactly as the per-axis mapping did:
    /// no margins, the same rounded rectangles for every key kind.
    #[test]
    fn same_aspect_window_is_todays_layout() {
        for (canvas, screen) in [((1600, 1200), Vec2::new(1440.0, 1080.0)), ((2134, 1200), Vec2::new(2134.0, 1200.0)), ((640, 480), Vec2::new(1440.0, 1080.0))] {
            let mut cfg = sheet(canvas);
            cfg.rects.insert("hull".into(), rect(13, 77, 123, 31));
            let l = StockLayout::new(&cfg, screen);
            assert_eq!(l.hud, R::new(0.0, 0.0, screen.x, screen.y), "{canvas:?}");
            let (sx, sy) = (screen.x / canvas.0 as f32, screen.y / canvas.1 as f32);
            assert_eq!((l.scale - sx).abs() + (l.scale - sy).abs(), 0.0, "{canvas:?}");
            let old = |r: Rect| R::new(
                (r.x as f32 * sx + 0.5).floor(), (r.y as f32 * sy + 0.5).floor(),
                (r.w as f32 * sx + 0.5).floor(), (r.h as f32 * sy + 0.5).floor());
            for key in ["hull", "minimapDisplayArea", "paletteSingleButtonArea"] {
                assert_eq!(l.rect(key), Some(old(cfg.rects[key])), "{canvas:?} {key}");
            }
            assert_eq!(l.panel("minimap"), Some(old(cfg.rects["minimapPanelArea"])), "{canvas:?}");
            assert_eq!(l.palette().unwrap().origin, Vec2::new((400.0 * sx).round(), (900.0 * sy).round()), "{canvas:?}");
        }
    }

    #[test]
    fn a_degenerate_screen_or_aspect_cannot_produce_nan() {
        let cfg = sheet((1600, 1200));
        for aspect in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let l = StockLayout::for_aspect(&cfg, aspect);
            assert!(l.hud.x.is_finite() && l.hud.w.is_finite() && l.scale > 0.0, "{aspect}");
        }
    }

    #[test]
    fn palette_uses_authored_dimensions_and_native_defaults() {
        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        cfg.rects.insert("paletteSingleButtonArea".into(), rect(0, 0, 80, 40));
        let l = StockLayout::new(&cfg, Vec2::new(800.0, 900.0));
        let p = l.palette().unwrap();
        assert_eq!((p.columns, p.rows), (3, 7));
        // Width-limited (800 of 1600): factor 0.5, HUD 800x600 centred at y 150.
        assert_eq!(p.origin, Vec2::new(15.0, 165.0));
        assert_eq!(p.cell_rect(2, 1), R::new(95.0, 145.0, 40.0, 20.0));
        cfg.numbers.insert("paletteSingleButtonGap".into(), 2.0);
        cfg.numbers.insert("staticPaletteWidthA".into(), 10.0);
        cfg.numbers.insert("staticPaletteHeightA".into(), 3.0);
        cfg.numbers.insert("popupPaletteXA".into(), 0.0);
        let p = StockLayout::new(&cfg, l.screen).palette().unwrap();
        assert_eq!((p.columns, p.rows), (10, 3));
        assert_eq!(p.origin.x, 0.0);
        assert_eq!(p.gap, Vec2::new(1.0, 1.0));
        cfg.numbers.insert("staticPaletteWidthA".into(), 0.0);
        assert!(StockLayout::new(&cfg, l.screen).palette().is_none());
    }

    #[test]
    fn stock_info_variant_differs_from_lcars_height() {
        assert_eq!(info_variant(&Info::None), 0);
        let single = Single {
            id: EntityId(1),
            name: String::new(),
            captain: None,
            registry: None,
            class: String::new(),
            own: true,
            hull: 1.0,
            hull_now: 1,
            hull_max: 1,
            shield: None,
            progress: None,
            builder: false,
        };
        assert_eq!(info_variant(&Info::Single(single.clone())), 1);
        assert_eq!(
            info_variant(&Info::Single(Single {
                builder: true,
                ..single
            })),
            2
        );
        let tile = Tile {
            id: EntityId(1),
            name: String::new(),
            hull: 1.0,
        };
        assert_eq!(info_variant(&Info::Group(vec![tile.clone(); 8])), 1);
        assert_eq!(info_variant(&Info::Group(vec![tile; 9])), 2);
    }
}
