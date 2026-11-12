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

/// CFG canvas coordinates mapped independently to the active screen axes.
#[derive(Clone, Debug)]
pub struct StockLayout {
    pub cfg: GuiLayout,
    pub screen: Vec2,
    pub canvas: Vec2,
    pub scale: Vec2,
}

impl StockLayout {
    pub fn new(cfg: &GuiLayout, screen: Vec2) -> Self {
        let canvas = if cfg.canvas.0 > 0 && cfg.canvas.1 > 0 {
            Vec2::new(cfg.canvas.0 as f32, cfg.canvas.1 as f32)
        } else {
            Vec2::new(1600.0, 1200.0)
        };
        Self {
            cfg: cfg.clone(),
            screen,
            canvas,
            scale: screen / canvas,
        }
    }

    fn scaled(&self, r: Rect) -> R {
        R::new(
            (r.x as f32 * self.scale.x + 0.5).floor(),
            (r.y as f32 * self.scale.y + 0.5).floor(),
            (r.w as f32 * self.scale.x + 0.5).floor(),
            (r.h as f32 * self.scale.y + 0.5).floor(),
        )
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

    /// A visible rectangle; authored zero or negative dimensions disable it.
    pub fn rect(&self, key: &str) -> Option<R> {
        self.raw_rect(key)
            .filter(|r| r.w > 0 && r.h > 0)
            .map(|r| self.scaled(r))
    }

    pub fn point(&self, key: &str) -> Option<Vec2> {
        self.raw_rect(key).map(|r| {
            Vec2::new(
                (r.x as f32 * self.scale.x + 0.5).floor(),
                (r.y as f32 * self.scale.y + 0.5).floor(),
            )
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

    pub fn panel(&self, name: &str) -> Option<R> {
        self.rect(&format!("{name}PanelArea"))
    }

    pub fn info_area(&self, variant: usize) -> Option<R> {
        let panel = self.cfg.panels.get("info")?;
        let raw = panel
            .variants
            .get(&u32::try_from(variant).ok()?)
            .copied()
            .or(panel.area)?;
        (raw.w > 0 && raw.h > 0).then(|| self.scaled(raw))
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
        let mut x = (self.number("popupPaletteXA").unwrap_or(30.0) as f32 * self.scale.x).round();
        // A window wider than the sheet's canvas (21:9 on the 16:9 variant) stretches the
        // left panels with the x scale; the palette starts no further left than the
        // minimap panel's right edge (the 16:9 sheet's 264 sits inside its 351 wide minimap).
        if self.screen.x * self.canvas.y > self.screen.y * self.canvas.x * 1.05
            && let Some(map) = self.panel("minimap")
        {
            x = x.max(map.x + map.w);
        }
        Some(PaletteLayout {
            origin: Vec2::new(
                x,
                (self.number("popupPaletteYA").unwrap_or(30.0) as f32 * self.scale.y).round(),
            ),
            cell: Vec2::new(cell.w, cell.h),
            gap: Vec2::new((gap * self.scale.x).round(), (gap * self.scale.y).round()),
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
        let l = StockLayout::new(&cfg, Vec2::new(800.0, 900.0));
        assert_eq!(l.scale, Vec2::new(0.5, 0.75));
        assert_eq!(l.panel("INFO"), Some(R::new(5.0, 15.0, 200.0, 150.0)));
        assert_eq!(l.info_area(1), Some(R::new(15.0, 30.0, 50.0, 60.0)));
        assert_eq!(l.info_area(2), None);
        assert_eq!(l.raw_rect("INFOPANELAREA_2"), Some(rect(30, 40, 0, 80)));
        assert_eq!(l.point("infobaroffset_0"), Some(Vec2::new(9.0, -3.0)));
        assert_eq!(l.rect("InfoBarOffset_0"), None);
        assert_eq!(
            l.at(Vec2::new(100.0, 200.0), "InfoPanelArea_1"),
            Some(R::new(115.0, 230.0, 50.0, 60.0))
        );
        assert_eq!(l.name("INFORMATIONFONT"), Some("font"));
        assert_eq!(l.name("INFOBACKGROUNDPANEL"), Some("info_sprite"));
        assert_eq!(l.number("INFOBACKGROUNDPANELSIZE"), Some(2.0));
        assert_eq!(l.colour("INFOCOLOR"), Some(Color::srgba(0.2, 0.3, 0.4, 0.5)));
        assert_eq!(
            l.pieces("infoBackgroundPanel"),
            vec![Piece {
                rect: R::new(0.0, 0.0, 20.0, 15.0),
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

    /// Fleet Ops' 16:9 sheet (canvas 2134x1200, minimap panel 351 wide, palette at 264) on
    /// ultrawide windows: the command cells start right of the minimap panel. Windows the
    /// sheet fits (4:3, 16:10, 16:9) and Classic's 4:3 sheet place it as before.
    #[test]
    fn ultrawide_palette_clears_the_minimap_panel() {
        let mut cfg = GuiLayout { canvas: (2134, 1200), ..Default::default() };
        cfg.rects.insert("paletteSingleButtonArea".into(), rect(0, 0, 88, 88));
        cfg.rects.insert("minimapPanelArea".into(), rect(-1, 818, 351, 383));
        cfg.numbers.insert("popupPaletteXA".into(), 264.0);
        cfg.numbers.insert("popupPaletteYA".into(), 940.0);
        cfg.numbers.insert("staticPaletteWidthA".into(), 14.0);
        cfg.numbers.insert("staticPaletteHeightA".into(), 20.0);
        for screen in [Vec2::new(3440.0, 1440.0), Vec2::new(2560.0, 1080.0), Vec2::new(5120.0, 1440.0)] {
            let l = StockLayout::new(&cfg, screen);
            let map = l.panel("minimap").unwrap();
            let p = l.palette().unwrap();
            for row in 0..p.rows {
                for col in 0..p.columns {
                    let c = p.cell_rect(col, row);
                    let inside = c.x < map.x + map.w && c.x + c.w > map.x && c.y < map.y + map.h && c.y + c.h > map.y;
                    assert!(!inside, "{screen:?}: cell {col},{row} {c:?} overlaps the minimap {map:?}");
                }
            }
        }
        // Unchanged where the sheet fits: the authored x, scaled.
        for screen in [Vec2::new(1920.0, 1080.0), Vec2::new(2134.0, 1200.0), Vec2::new(1600.0, 1200.0), Vec2::new(1920.0, 1200.0)] {
            let x = StockLayout::new(&cfg, screen).palette().unwrap().origin.x;
            assert_eq!(x, (264.0 * screen.x / 2134.0).round(), "{screen:?}");
        }
        // Classic's sheet (4:3 canvas, palette at 355 right of its 350 wide minimap) is not moved.
        let mut classic = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        classic.rects.insert("paletteSingleButtonArea".into(), rect(0, 0, 80, 80));
        classic.rects.insert("minimapPanelArea".into(), rect(0, 816, 350, 384));
        classic.numbers.insert("popupPaletteXA".into(), 355.0);
        classic.numbers.insert("popupPaletteYA".into(), 825.0);
        for screen in [Vec2::new(3440.0, 1440.0), Vec2::new(1920.0, 1080.0), Vec2::new(1600.0, 1200.0)] {
            let x = StockLayout::new(&classic, screen).palette().unwrap().origin.x;
            assert_eq!(x, (355.0 * screen.x / 1600.0).round(), "{screen:?}");
        }
    }

    /// Install-gated: Fleet Ops 4.0's own federation 16x9 sheet at 21:9.
    #[test]
    fn installed_fleet_ops_palette_clears_the_minimap_at_21_9() {
        let roots = std::path::Path::new("/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots");
        let Ok(vfs) = sta_vfs::Vfs::mount_install(roots, Some("Fleet Ops 4.0")) else {
            eprintln!("FO4 not installed: skipped");
            return;
        };
        let Some(cfg) = crate::game::gui_layout(&vfs, "gui_federation16x9.cfg") else { return };
        for screen in [Vec2::new(3440.0, 1440.0), Vec2::new(2560.0, 1080.0)] {
            let l = StockLayout::new(&cfg, screen);
            let (Some(map), Some(p)) = (l.panel("minimap"), l.palette()) else { continue };
            for row in 0..p.rows {
                for col in 0..p.columns {
                    let c = p.cell_rect(col, row);
                    assert!(c.x >= map.x + map.w, "{screen:?}: {c:?} vs {map:?}");
                }
            }
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
        assert_eq!(p.origin, Vec2::new(15.0, 23.0));
        assert_eq!(p.cell_rect(2, 1), R::new(95.0, -7.0, 40.0, 30.0));
        cfg.numbers.insert("paletteSingleButtonGap".into(), 2.0);
        cfg.numbers.insert("staticPaletteWidthA".into(), 10.0);
        cfg.numbers.insert("staticPaletteHeightA".into(), 3.0);
        cfg.numbers.insert("popupPaletteXA".into(), 0.0);
        let p = StockLayout::new(&cfg, l.screen).palette().unwrap();
        assert_eq!((p.columns, p.rows), (10, 3));
        assert_eq!(p.origin.x, 0.0);
        assert_eq!(p.gap, Vec2::new(1.0, 2.0));
        cfg.numbers.insert("staticPaletteWidthA".into(), 0.0);
        assert!(StockLayout::new(&cfg, l.screen).palette().is_none());
    }

    #[test]
    fn stock_info_variant_differs_from_lcars_height() {
        assert_eq!(info_variant(&Info::None), 0);
        let single = Single {
            id: EntityId(1),
            name: String::new(),
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
