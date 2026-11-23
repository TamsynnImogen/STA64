//! Pure, authored stock HUD geometry from a resolved race GUI CFG.

use bevy::prelude::{Color, Vec2};
use sta_content::gui_cfg::{GuiLayout, Rect};

use super::super::layout::R;
use super::super::model::Info;
use crate::ui::profile::UiProfile;

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

/// The canvas the palette origin keys (`popupPaletteXA`, `popupPaletteYA`) are authored on.
const PALETTE_REFERENCE: Vec2 = Vec2::new(1600.0, 1200.0);

/// How the stock HUD sits on a window whose aspect is not the sheet's, wider or narrower (ADR 0008 H7; the
/// player's `hud_layout` setting). Governs the stock-look HUD only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudFit {
    /// H2: one factor for both axes, centred; the sides belong to the world.
    #[default]
    Centred,
    /// The sheet's canvas scaled to the whole window per axis, as the original engines do
    /// (this changes the HUD on any window whose aspect is not the sheet's, wider or narrower).
    Stretched,
}

impl HudFit {
    pub const ALL: [Self; 2] = [Self::Centred, Self::Stretched];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Centred => "Centred",
            Self::Stretched => "Stretched",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Centred => "centred",
            Self::Stretched => "stretched",
        }
    }

    /// Case-insensitive; `centered` is accepted as `centred`.
    pub fn from_key(key: &str) -> Option<Self> {
        let key = key.trim().to_ascii_lowercase();
        let key = if key == "centered" { "centred" } else { key.as_str() };
        Self::ALL.into_iter().find(|fit| fit.key() == key)
    }
}

/// CFG canvas coordinates mapped to the screen: by one factor for both axes and centred
/// ([`HudFit::Centred`]) or per axis over the whole window ([`HudFit::Stretched`]). The
/// sheet's canvas so placed is [`StockLayout::hud`]: the one rectangle every consumer
/// (panels, pointer, minimap, notices) shares; the screen outside it belongs to the world.
#[derive(Clone, Debug)]
pub struct StockLayout {
    pub cfg: GuiLayout,
    pub screen: Vec2,
    pub canvas: Vec2,
    pub fit: HudFit,
    /// Screen units per canvas unit vertically; also horizontally when centred.
    pub scale: f32,
    /// Screen units per canvas unit horizontally (`scale` unless stretched).
    pub scale_x: f32,
    /// The scaled canvas (whole units): centred in the screen, or the whole screen.
    pub hud: R,
    a2_info: bool,
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
        Self::with_fit(cfg, screen, HudFit::Centred)
    }

    pub fn with_fit(cfg: &GuiLayout, screen: Vec2, fit: HudFit) -> Self {
        let canvas = if cfg.canvas.0 > 0 && cfg.canvas.1 > 0 {
            Vec2::new(cfg.canvas.0 as f32, cfg.canvas.1 as f32)
        } else {
            Vec2::new(1600.0, 1200.0)
        };
        let (scale, scale_x, hud) = match fit {
            HudFit::Centred => {
                let (scale, hud) = fit_canvas(canvas, screen);
                (scale, scale, hud)
            }
            HudFit::Stretched => (screen.y / canvas.y, screen.x / canvas.x, R::new(0.0, 0.0, screen.x, screen.y)),
        };
        Self { cfg: cfg.clone(), screen, canvas, fit, scale, scale_x, hud, a2_info: false }
    }

    /// The layout for a window of this aspect, in the HUD's 1080-line theme units.
    pub fn for_aspect(cfg: &GuiLayout, aspect: f32) -> Self {
        Self::for_aspect_fit(cfg, aspect, HudFit::Centred)
    }

    pub fn for_aspect_fit(cfg: &GuiLayout, aspect: f32, fit: HudFit) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
        Self::with_fit(cfg, Vec2::new(super::super::layout::SCREEN_H * aspect, super::super::layout::SCREEN_H), fit)
    }

    pub(in crate::ui::hud) fn with_info_profile(mut self, profile: UiProfile) -> Self {
        self.a2_info = profile == UiProfile::Armada2;
        self
    }

    /// Stock ShipDisplay selects low when empty. A single selection uses the
    /// tall card when it has a build queue; group thresholds depend on the profile.
    pub(in crate::ui::hud) fn info_variant(&self, info: &Info) -> usize {
        match info {
            Info::None => 0,
            Info::Single(single) => 1 + usize::from(single.has_build_queue),
            Info::Group(group) => 1 + usize::from(group.len() > if self.a2_info { 8 } else { 16 }),
        }
    }

    /// The two Armada II group heights share the same sixteen authored slots.
    pub(super) fn info_group_slots(&self, variant: usize) -> (bool, usize) {
        if self.a2_info { (false, 16) } else { (variant == 2, usize::MAX) }
    }

    /// Whether a screen point (theme units) is inside the HUD sheet's rectangle.
    pub fn in_hud(&self, p: Vec2) -> bool {
        self.hud.contains(p)
    }

    /// A canvas rectangle, panel-relative: scaled, not moved by the centring offset.
    fn scaled(&self, r: Rect) -> R {
        R::new(
            (r.x as f32 * self.scale_x + 0.5).floor(),
            (r.y as f32 * self.scale + 0.5).floor(),
            (r.w as f32 * self.scale_x + 0.5).floor(),
            (r.h as f32 * self.scale + 0.5).floor(),
        )
    }

    /// One edge of the sheet, in whole screen units, from its absolute canvas coordinate `u` on the
    /// horizontal axis (`x`) or the vertical one. Every piece edge goes through here and is rounded
    /// once, so two edges at the same canvas coordinate are the same screen coordinate.
    ///
    /// Deliberate rule of STA64: pieces that abut in the sheet abut on screen, with no gap and no
    /// overlap, in both fits and at any window size and scale factor. At sizes where the sheet's units do
    /// not map to whole pixels the original leaves one-pixel gaps between abutting tiles (grade: measured on
    /// one native frame of the original at a fractional scale, and read from the executable, static), while
    /// its art is seamless at scale 1 (measured on a native frame); at scale 1 this is identical to the
    /// original.
    fn edge(&self, u: f32, horizontal: bool) -> f32 {
        if horizontal {
            self.hud.x + (u * self.scale_x + 0.5).floor()
        } else {
            self.hud.y + (u * self.scale + 0.5).floor()
        }
    }

    /// A canvas rectangle whose parent starts at canvas point `origin` (the panel's own origin, which
    /// [`Self::placed`] puts on screen with the same edge function), relative to that parent's screen
    /// origin. Adding the placed origin gives `edge(origin + x)`, the absolute edge, rounded once.
    fn scaled_in(&self, origin: (i32, i32), r: Rect) -> R {
        let (ox, oy) = (origin.0 as f32, origin.1 as f32);
        let (px, py) = (self.edge(ox, true), self.edge(oy, false));
        let l = self.edge(ox + r.x as f32, true);
        let t = self.edge(oy + r.y as f32, false);
        let right = self.edge(ox + r.x as f32 + r.w as f32, true);
        let bottom = self.edge(oy + r.y as f32 + r.h as f32, false);
        R::new(l - px, t - py, right - l, bottom - t)
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
            Vec2::new((r.x as f32 * self.scale_x + 0.5).floor(), (r.y as f32 * self.scale + 0.5).floor())
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

    pub(in crate::ui::hud) fn has_a2_info(&self) -> bool {
        self.a2_info
    }

    pub fn info_area(&self, variant: usize) -> Option<R> {
        self.info_raw(variant).map(|raw| self.placed(raw))
    }

    pub fn info_origin(&self, variant: usize) -> Vec2 {
        self.info_area(variant).map_or(Vec2::ZERO, |r| Vec2::new(r.x, r.y))
    }

    /// Place a panel-relative ShipDisplay rectangle from the active canvas origin.
    pub(in crate::ui::hud) fn info_at(&self, variant: usize, key: &str) -> Option<R> {
        let area = self.info_raw(variant)?;
        let rect = self.raw_rect(key)?;
        self.at_canvas(area, rect)
    }

    /// The mouseover strip follows the active height's configured XY offset.
    pub(super) fn info_bar_at(&self, variant: usize, key: &str) -> Option<R> {
        let mut area = self.info_raw(variant)?;
        if let Some(offset) = self.raw_rect(&format!("infoBarOffset_{variant}")) {
            area.x = area.x.saturating_add(offset.x);
            area.y = area.y.saturating_add(offset.y);
        }
        self.at_canvas(area, self.raw_rect(key)?)
    }

    /// The frame has its own panel-relative origin; its SPR pieces are local to it.
    pub(super) fn info_frame(&self, variant: usize, prefix: &str) -> Vec<(usize, String, R)> {
        let Some(mut area) = self.info_raw(variant) else { return Vec::new() };
        if let Some(background) = self.raw_rect(&format!("{prefix}Area")) {
            if background.w <= 0 || background.h <= 0 {
                return Vec::new();
            }
            area.x = area.x.saturating_add(background.x);
            area.y = area.y.saturating_add(background.y);
        }
        self.canvas_pieces(prefix).into_iter().filter_map(|(frame, name, rect)| {
            self.at_canvas(area, rect).map(|rect| (frame, name, rect))
        }).collect()
    }

    fn at_canvas(&self, area: Rect, rect: Rect) -> Option<R> {
        let mut placed = self.scaled_in((area.x, area.y), rect);
        placed.x += self.edge(area.x as f32, true);
        placed.y += self.edge(area.y as f32, false);
        (placed.w > 0.0 && placed.h > 0.0).then_some(placed)
    }

    pub fn at(&self, origin: Vec2, key: &str) -> Option<R> {
        self.rect(key).map(|mut r| {
            r.x += origin.x;
            r.y += origin.y;
            r
        })
    }

    /// A panel-relative rectangle moved right by `dx` canvas units before scaling (a position on a
    /// row of equal slots, such as the Armada I speed rail), then by `origin`.
    pub fn at_shifted(&self, origin: Vec2, key: &str, dx: i32) -> Option<R> {
        let mut raw = self.raw_rect(key).filter(|r| r.w > 0 && r.h > 0)?;
        raw.x = raw.x.saturating_add(dx);
        let mut r = self.scaled(raw);
        r.x += origin.x;
        r.y += origin.y;
        Some(r)
    }

    /// Count is authoritative. Only existing numbered CFG rectangles below it draw.
    /// Rectangles are relative to the canvas origin; see [`Self::pieces_from`].
    pub fn pieces(&self, prefix: &str) -> Vec<Piece> {
        self.pieces_from(prefix, (0, 0))
    }

    /// The pieces of [`Self::pieces`] for a panel whose area starts at canvas point `origin`, relative
    /// to that panel's placed origin (`panel`, `info_area`). Every edge is the sheet's absolute edge
    /// rounded once, so neighbouring pieces share it exactly (see `edge`).
    pub fn pieces_from(&self, prefix: &str, origin: (i32, i32)) -> Vec<Piece> {
        self.canvas_pieces(prefix)
            .into_iter()
            .map(|(frame, name, rect)| Piece { rect: self.scaled_in(origin, rect), name, frame })
            .collect()
    }

    /// The pieces of the panel `panel` (`<panel>PanelArea`): [`Self::pieces_from`] at its canvas origin.
    pub fn pieces_in(&self, panel: &str, prefix: &str) -> Vec<Piece> {
        let origin = self.raw_rect(&format!("{panel}PanelArea")).map_or((0, 0), |r| (r.x, r.y));
        self.pieces_from(prefix, origin)
    }

    /// The canvas origin of the info panel's variant (the area [`Self::info_area`] places).
    pub fn info_canvas_origin(&self, variant: usize) -> (i32, i32) {
        self.info_raw(variant).map_or((0, 0), |r| (r.x, r.y))
    }

    fn info_raw(&self, variant: usize) -> Option<Rect> {
        let panel = self.cfg.panels.get("info")?;
        panel.variants.get(&u32::try_from(variant).ok()?).copied().or(panel.area).filter(|r| r.w > 0 && r.h > 0)
    }

    /// The pieces of [`Self::pieces`] in canvas units, unplaced (for a caller that places them itself).
    pub fn canvas_pieces(&self, prefix: &str) -> Vec<(usize, String, Rect)> {
        let count = self.number(&format!("{prefix}Size")).unwrap_or(0.0);
        if !count.is_finite() || count <= 0.0 {
            return Vec::new();
        }
        let name = self.name(prefix).unwrap_or("default_sprite");
        let mut out = Vec::new();
        // Iterate the bounded CFG maps, never a potentially untrusted Size range.
        let add = |frame: usize, rect: Rect, out: &mut Vec<(usize, String, Rect)>| {
            if (frame as f64) < count && rect.w > 0 && rect.h > 0 {
                out.push((frame, name.to_owned(), rect));
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
        out.sort_by_key(|piece| piece.0);
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
        // The origin keys are read against a fixed 1600x1200 reference canvas, then scaled
        // with the sheet's own canvas like every other key. Evidence per axis:
        // - x: the Armada II engine reads `popupPaletteXA/XB` against a hard-coded 1600
        //   (measured by Refit on a widened canvas; second-hand here, not measured by us).
        //   For Roots the evidence is the sheets' own values (read from data): its 16:9 and
        //   16:10 sheets (canvas 2134 and 1920 wide) author 264 and 293, which are 352 on the
        //   4:3 sheet's 1600, right of the 351 wide minimap panel. Roots positions the palette
        //   with its own add-on, so the engine's rule need not be Roots' rule.
        // - y: read from data, the 5:4 sheet only (1280 high, `popupPaletteYA` 957 = 1020 =
        //   940 + 80 in canvas units); Refit's measurement had a 1200 high canvas and says
        //   nothing about y on another height.
        // Alternative reading: "x derived from the minimap panel's right edge" (350). It gives
        // the same x on the 16:9 and 16:10 sheets and predicts y 957 on the 5:4 sheet.
        // TODO(UI-33): the 5:4 sheet's y is unmeasured: this rule says 1020.8, the alternative
        // 957; a bench frame of the original Roots HUD at 1280x1024 settles it.
        // Sheets whose canvas is 1600x1200 (Armada II, Classic, Roots 4:3) are the identity.
        let origin = Vec2::new(
            self.number("popupPaletteXA").unwrap_or(30.0) as f32 * (self.canvas.x / PALETTE_REFERENCE.x),
            self.number("popupPaletteYA").unwrap_or(30.0) as f32 * (self.canvas.y / PALETTE_REFERENCE.y),
        );
        Some(PaletteLayout {
            origin: Vec2::new(
                (origin.x * self.scale_x).round() + self.hud.x,
                (origin.y * self.scale).round() + self.hud.y,
            ),
            cell: Vec2::new(cell.w, cell.h),
            gap: Vec2::new((gap * self.scale_x).round(), (gap * self.scale).round()),
            columns: bounds("staticPaletteWidthA", 3)?,
            rows: bounds("staticPaletteHeightA", 7)?,
        })
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

    #[test]
    fn adjacent_pieces_share_rounded_horizontal_and_vertical_edges() {
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.numbers.insert("frameSize".into(), 3.0);
        cfg.names.insert("frame".into(), "frame_sprite".into());
        cfg.rects.insert("frame_0".into(), rect(1, 1, 1, 1));
        cfg.rects.insert("frame_1".into(), rect(2, 1, 3, 1));
        cfg.rects.insert("frame_2".into(), rect(1, 2, 1, 3));
        cfg.rects.insert("singleControl".into(), rect(1, 1, 1, 1));
        cfg.rects.insert("singlePanelArea".into(), rect(1, 1, 1, 1));

        for (sx, sy, fit) in [
            (0.75, 0.75, HudFit::Centred),
            (1.0, 1.0, HudFit::Centred),
            (1.25, 1.25, HudFit::Centred),
            (1.5, 1.5, HudFit::Centred),
            (1.25, 0.75, HudFit::Stretched),
            (0.75, 1.25, HudFit::Stretched),
        ] {
            let layout = StockLayout::with_fit(&cfg, Vec2::new(1600.0 * sx, 1200.0 * sy), fit);
            let pieces = layout.pieces("frame");
            assert_eq!(pieces.iter().map(|p| (p.frame, p.name.as_str())).collect::<Vec<_>>(),
                [(0, "frame_sprite"), (1, "frame_sprite"), (2, "frame_sprite")]);
            assert_eq!(pieces[0].rect.right(), pieces[1].rect.x, "horizontal seam at {sx} x {sy}");
            assert_eq!(pieces[0].rect.bottom(), pieces[2].rect.y, "vertical seam at {sx} x {sy}");
        }

        // The other rectangle consumers retain their independent size rounding.
        let layout = StockLayout::new(&cfg, Vec2::new(2000.0, 1500.0));
        assert_eq!(layout.rect("singleControl"), Some(R::new(1.0, 1.0, 1.0, 1.0)));
        assert_eq!(layout.panel("single"), Some(R::new(1.0, 1.0, 1.0, 1.0)));
        assert_eq!(layout.pieces("frame")[0].rect, R::new(1.0, 1.0, 2.0, 2.0));
    }

    /// The palette origin is read against a 1600x1200 reference (see `palette`): Fleet Ops'
    /// 16:9 sheet (canvas 2134x1200, minimap panel 351 wide, popupPaletteXA 264) and 16:10
    /// sheet (1920x1200, 293) start the command cells at 352 canvas units, right of the
    /// minimap panel, on every window; its 5:4 sheet (1600x1280, YA 957) at 1020. Classic's
    /// and the 4:3 sheets (canvas 1600x1200) are placed as authored.
    #[test]
    fn palette_origin_is_read_against_the_1600x1200_reference() {
        // (canvas, popupPaletteXA, YA, expected canvas origin)
        for (canvas, xa, ya, want) in [
            ((2134, 1200), 264.0, 940.0, (264.0 * 2134.0 / 1600.0, 940.0)),
            ((1920, 1200), 293.0, 940.0, (293.0 * 1920.0 / 1600.0, 940.0)),
            ((1600, 1280), 352.0, 957.0, (352.0, 957.0 * 1280.0 / 1200.0)),
            ((1600, 1200), 352.0, 940.0, (352.0, 940.0)),
            ((1600, 1200), 355.0, 825.0, (355.0, 825.0)),
        ] {
            let mut cfg = GuiLayout { canvas, ..Default::default() };
            cfg.rects.insert("paletteSingleButtonArea".into(), rect(0, 0, 88, 88));
            cfg.rects.insert("minimapPanelArea".into(), rect(-1, 818 + canvas.1 - 1200, 351, 383));
            cfg.numbers.insert("popupPaletteXA".into(), xa);
            cfg.numbers.insert("popupPaletteYA".into(), ya);
            for fit in HudFit::ALL {
            for screen in [Vec2::new(1920.0, 1080.0), Vec2::new(2752.0, 1152.0), Vec2::new(3440.0, 1440.0), Vec2::new(1024.0, 768.0), Vec2::new(canvas.0 as f32, canvas.1 as f32)] {
                let l = StockLayout::with_fit(&cfg, screen, fit);
                let p = l.palette().unwrap();
                let why = format!("{canvas:?} {xa},{ya} {fit:?} on {screen:?}");
                let got = (p.origin - Vec2::new(l.hud.x, l.hud.y)) / Vec2::new(l.scale_x, l.scale);
                assert!((got.x - want.0 as f32).abs() <= 0.5 / l.scale_x + 1e-3 && (got.y - want.1 as f32).abs() <= 0.5 / l.scale + 1e-3, "{why}: {got:?} vs {want:?}");
                // No command cell lies over the minimap panel.
                let map = l.panel("minimap").unwrap();
                for row in 0..3 {
                    for col in 0..3 {
                        let c = p.cell_rect(col, row);
                        let inside = c.x < map.right() && c.right() > map.x && c.y < map.bottom() && c.bottom() > map.y;
                        assert!(!inside, "{why}: cell {col},{row} {c:?} overlaps the minimap {map:?}");
                    }
                }
            }
            }
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
    /// no margins, the same rounded rectangles for every rect key. The palette origin follows
    /// the 1600x1200 reference rule (see `palette`); the 640x480 case is synthetic (Armada I's
    /// sheets have no palette keys, so that path is not reached in the game).
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
            // Palette origin keys are read against 1600x1200, then scaled like every other key.
            let (px, py) = (400.0 * canvas.0 as f32 / 1600.0, 900.0 * canvas.1 as f32 / 1200.0);
            assert_eq!(l.palette().unwrap().origin, Vec2::new((px * sx).round(), (py * sy).round()), "{canvas:?}");
        }
    }

    /// A 2134x1200 sheet on a 1920x1080 window differs from 16:9 by 0.03 percent: every key
    /// lands within one unit of the old per-axis mapping and the HUD keeps zero vertical margin.
    #[test]
    fn a_near_same_aspect_window_stays_within_one_unit_of_the_per_axis_layout() {
        let canvas = (2134, 1200);
        let screen = Vec2::new(1920.0, 1080.0);
        let mut cfg = sheet(canvas);
        cfg.rects.insert("hull".into(), rect(13, 77, 123, 31));
        let l = StockLayout::new(&cfg, screen);
        assert_eq!(l.hud.y, 0.0);
        let (sx, sy) = (screen.x / canvas.0 as f32, screen.y / canvas.1 as f32);
        let old = |r: Rect| R::new(r.x as f32 * sx, r.y as f32 * sy, r.w as f32 * sx, r.h as f32 * sy);
        let close = |a: R, b: R| (a.x - b.x).abs() <= 1.0 && (a.y - b.y).abs() <= 1.0 && (a.w - b.w).abs() <= 1.0 && (a.h - b.h).abs() <= 1.0;
        for key in ["hull", "minimapDisplayArea", "paletteSingleButtonArea"] {
            let (got, want) = (l.rect(key).unwrap(), old(cfg.rects[key]));
            assert!(close(got, want), "{key}: {got:?} vs {want:?}");
        }
        let (got, want) = (l.panel("minimap").unwrap(), old(cfg.rects["minimapPanelArea"]));
        assert!(close(got, want), "minimap: {got:?} vs {want:?}");
        let o = l.palette().unwrap().origin;
        assert!((o.x - 400.0 * canvas.0 as f32 / 1600.0 * sx).abs() <= 1.0 && (o.y - 900.0 * sy).abs() <= 1.0, "{o:?}");
    }

    /// The player's HUD layout choice on a 2752x1152 window (a 2.39:1 playtest window)
    /// against a 16:9 sheet: centred is the one-factor rectangle, stretched the whole window.
    #[test]
    fn hud_fit_centred_and_stretched_on_a_wide_window() {
        let cfg = sheet((2134, 1200));
        let screen = Vec2::new(2752.0, 1152.0);
        let c = StockLayout::with_fit(&cfg, screen, HudFit::Centred);
        assert_eq!(c.fit, HudFit::Centred);
        assert_eq!(c.scale, 1152.0 / 1200.0);
        assert_eq!(c.scale_x, c.scale);
        let size = Vec2::new(2134.0, 1200.0) * c.scale;
        assert_eq!(c.hud, R::new(((screen.x - size.x) / 2.0).round(), 0.0, size.x, size.y));
        assert!(c.hud.x > 300.0 && c.hud.right() < screen.x - 300.0);
        // Same as the plain constructor: centred is the default.
        assert_eq!(StockLayout::new(&cfg, screen).hud, c.hud);
        // Panel groups keep their internal proportions: sizes and offsets between panels of
        // the sheet follow the one factor (within a rounded unit), so the square minimap
        // stays square and the display keeps its place inside the panel.
        let (map, display) = (c.panel("minimap").unwrap(), c.rect("minimapDisplayArea").unwrap());
        assert!((map.w - map.h).abs() <= 1.0 && (display.w - display.h).abs() <= 1.0);
        assert!((display.w / map.w - 200.0 / 400.0).abs() < 0.01, "{display:?} in {map:?}");
        assert!((display.x - 20.0 * c.scale).abs() <= 1.0 && (display.y - 20.0 * c.scale).abs() <= 1.0);

        let s = StockLayout::with_fit(&cfg, screen, HudFit::Stretched);
        assert_eq!(s.fit, HudFit::Stretched);
        assert_eq!(s.hud, R::new(0.0, 0.0, 2752.0, 1152.0));
        assert_eq!(s.scale_x, 2752.0 / 2134.0);
        assert_eq!(s.scale, 1152.0 / 1200.0);
        // Per axis, as the original engines draw it: the square minimap is wider than tall,
        // panels start at the window's left edge and the palette origin follows the x scale.
        let map = s.panel("minimap").unwrap();
        assert_eq!(map.x, 0.0);
        assert!(map.w > map.h * 1.2, "{map:?}");
        // Read against the 1600 reference (2134 wide canvas), then scaled per axis.
        assert_eq!(s.palette().unwrap().origin.x, (400.0 * 2134.0 / 1600.0 * s.scale_x).round());
        let by_aspect = StockLayout::for_aspect_fit(&cfg, 2752.0 / 1152.0, HudFit::Stretched);
        assert_eq!(by_aspect.hud, R::new(0.0, 0.0, 1080.0 * 2752.0 / 1152.0, 1080.0));
    }

    #[test]
    fn hud_fit_keys_round_trip_and_unknown_is_none() {
        for fit in HudFit::ALL {
            assert_eq!(HudFit::from_key(fit.key()), Some(fit));
        }
        assert_eq!(HudFit::from_key("anchored"), None);
        assert_eq!(HudFit::from_key("Centered"), Some(HudFit::Centred));
        assert_eq!(HudFit::default(), HudFit::Centred);
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
        let layout = StockLayout::new(&GuiLayout::default(), Vec2::new(1600.0, 1200.0));
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
            has_build_queue: false,
        };
        assert_eq!(layout.info_variant(&Info::Single(single.clone())), 1);
        assert_eq!(
            layout.info_variant(&Info::Single(Single {
                builder: true,
                ..single.clone()
            })),
            1
        );
        assert_eq!(
            layout.info_variant(&Info::Single(Single {
                has_build_queue: true,
                ..single
            })),
            2
        );
        let tile = Tile {
            id: EntityId(1),
            name: String::new(),
            hull: 1.0,
            shield: None,
        };
        for count in [2, 16] {
            assert_eq!(layout.info_variant(&Info::Group(vec![tile.clone(); count])), 1);
        }
        for count in [17, 30] {
            assert_eq!(layout.info_variant(&Info::Group(vec![tile.clone(); count])), 2);
        }
    }

    const WINDOWS: [(f32, f32); 7] =
        [(1280.0, 720.0), (1366.0, 768.0), (1440.0, 1080.0), (1920.0, 1080.0), (2560.0, 1080.0), (2752.0, 1152.0), (3440.0, 1440.0)];
    const FACTORS: [f32; 3] = [1.0, 1.25, 1.5];

    /// The edge a UI layout puts on the physical grid for a logical coordinate: rounded absolute.
    fn physical(v: f32, factor: f32) -> f32 {
        (v * factor).round()
    }

    /// Every pair of the prefix's pieces that abut in the sheet (or are disjoint) is placed so that it
    /// abuts (or stays disjoint) on screen, in logical units and after physical-pixel rounding at
    /// `factor`, and each placed piece's edges stay within one unit of the separately rounded ones.
    fn assert_joined(l: &StockLayout, prefix: &str, origin: (i32, i32), factor: f32, label: &str) {
        let raw: Vec<Rect> = l.canvas_pieces(prefix).into_iter().map(|p| p.2).collect();
        let placed = l.pieces_from(prefix, origin);
        assert_eq!(raw.len(), placed.len());
        let base = l.placed(rect(origin.0, origin.1, 0, 0));
        let abs: Vec<R> = placed.iter().map(|p| R::new(base.x + p.rect.x, base.y + p.rect.y, p.rect.w, p.rect.h)).collect();
        for (r, a) in raw.iter().zip(&abs) {
            assert!(a.w >= 0.0 && a.h >= 0.0, "{label} {prefix}: negative size");
            // Once-rounded edges keep every piece at least `floor(size)` units wide; only a sliver thinner than
            // one screen unit can round to nothing.
            assert!(a.w > 0.0 || (r.w as f32 * l.scale_x) < 1.0, "{label} {prefix}: a piece vanished");
            assert!(a.h > 0.0 || (r.h as f32 * l.scale) < 1.0, "{label} {prefix}: a piece vanished");
            let old = l.scaled(*r);
            assert!((a.x - (base.x + old.x)).abs() <= 1.0 && (a.y - (base.y + old.y)).abs() <= 1.0, "{label} {prefix}: moved by more than the rounding");
            assert!((a.right() - (base.x + old.x + old.w)).abs() <= 1.0, "{label} {prefix}: right edge moved by more than the rounding");
            assert!((a.bottom() - (base.y + old.y + old.h)).abs() <= 1.0, "{label} {prefix}: bottom edge moved by more than the rounding");
        }
        for (i, (ra, a)) in raw.iter().zip(&abs).enumerate() {
            for (rb, b) in raw.iter().zip(&abs).skip(i + 1) {
                let rows = ra.y < rb.y + rb.h && rb.y < ra.y + ra.h;
                let cols = ra.x < rb.x + rb.w && rb.x < ra.x + ra.w;
                for (lo_r, lo, hi_r, hi, shared, what) in [
                    (ra, a, rb, b, rows, "x"),
                    (rb, b, ra, a, rows, "x"),
                ] {
                    if shared && lo_r.x + lo_r.w == hi_r.x {
                        assert_eq!(lo.right(), hi.x, "{label} {prefix}: {what} join opened");
                        assert_eq!(physical(lo.right(), factor), physical(hi.x, factor), "{label} {prefix}: physical {what} join opened");
                    }
                }
                for (lo_r, lo, hi_r, hi) in [(ra, a, rb, b), (rb, b, ra, a)] {
                    if cols && lo_r.y + lo_r.h == hi_r.y {
                        assert_eq!(lo.bottom(), hi.y, "{label} {prefix}: y join opened");
                        assert_eq!(physical(lo.bottom(), factor), physical(hi.y, factor), "{label} {prefix}: physical y join opened");
                    }
                }
                if !rows || !cols {
                    continue;
                }
            }
        }
        // Disjoint in the sheet stays disjoint on screen (no overlap).
        for (i, (ra, a)) in raw.iter().zip(&abs).enumerate() {
            for (rb, b) in raw.iter().zip(&abs).skip(i + 1) {
                let sheet_overlap = ra.x < rb.x + rb.w && rb.x < ra.x + ra.w && ra.y < rb.y + rb.h && rb.y < ra.y + ra.h;
                let screen_overlap = a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
                assert!(sheet_overlap || !screen_overlap, "{label} {prefix}: pieces overlap on screen");
            }
        }
    }

    /// A row of three tiles and a column under them, abutting in the sheet's units, inside two panels.
    fn abutting_sheet() -> GuiLayout {
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        let mut info = Panel { name: "Info".into(), area: Some(rect(333, 777, 600, 300)), ..Default::default() };
        info.background = Some("tile".into());
        info.background_size = Some(5);
        for (i, r) in [rect(0, 0, 16, 41), rect(16, 0, 107, 41), rect(123, 0, 150, 41), rect(0, 41, 273, 63), rect(0, 104, 273, 7)].into_iter().enumerate() {
            info.pieces.insert(i as u32, r);
        }
        cfg.panels.insert("info".into(), info);
        cfg
    }

    /// Regression, authored numbers: at the factor 0.9 (1440x1080 on a 1600x1200 sheet) the pieces 16|107|150
    /// wide were rounded one by one and left a one unit gap before the third piece.
    #[test]
    fn neighbouring_pieces_abut_where_separate_rounding_opened_a_gap() {
        let cfg = abutting_sheet();
        let l = StockLayout::new(&cfg, Vec2::new(1440.0, 1080.0));
        assert_eq!(l.scale, 0.9);
        // The separately rounded rectangles (the old placement) do not join: 14+96 = 110, then 111.
        let old: Vec<R> = l.canvas_pieces("infoBackgroundPanel").iter().map(|p| l.scaled(p.2)).collect();
        assert_eq!(old[1].right() + 1.0, old[2].x);
        let p = l.pieces_from("infoBackgroundPanel", (333, 777));
        assert_eq!(p[0].rect.right(), p[1].rect.x);
        assert_eq!(p[1].rect.right(), p[2].rect.x);
        assert_eq!(p[0].rect.bottom(), p[3].rect.y);
        assert_eq!(p[3].rect.bottom(), p[4].rect.y);
    }

    /// Both fits, the listed window sizes, scale factors 1, 1.25 and 1.5 (logical window = physical / factor).
    #[test]
    fn pieces_abut_at_every_window_size_fit_and_scale_factor() {
        let cfg = abutting_sheet();
        for fit in [HudFit::Centred, HudFit::Stretched] {
            for (w, h) in WINDOWS {
                for factor in FACTORS {
                    let l = StockLayout::with_fit(&cfg, Vec2::new(w / factor, h / factor), fit);
                    assert_joined(&l, "infoBackgroundPanel", (333, 777), factor, &format!("{fit:?} {w}x{h} @{factor}"));
                    assert_joined(&l, "infoBackgroundPanel", (0, 0), factor, &format!("{fit:?} {w}x{h} @{factor} from origin"));
                }
            }
        }
    }

    /// With `STA64_TEST_INSTALL` and/or `STA64_TEST_A1_INSTALL` (install roots, read at run time): the
    /// adjacency rule over every installed `gui*.cfg` and every piece set it declares.
    #[test]
    fn installed_gui_cfgs_have_no_gap_or_overlap_between_adjacent_pieces() {
        let roots: Vec<_> = ["STA64_TEST_INSTALL", "STA64_TEST_A1_INSTALL"].iter().filter_map(std::env::var_os).collect();
        if roots.is_empty() {
            eprintln!("SKIP: set STA64_TEST_INSTALL and/or STA64_TEST_A1_INSTALL to an install root");
            return;
        }
        let mut checked = 0;
        for root in roots {
            let Ok(vfs) = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None) else { continue };
            for path in vfs.list("misc") {
                let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                let lower = name.to_ascii_lowercase();
                if !lower.starts_with("gui") || !lower.ends_with(".cfg") {
                    continue;
                }
                let Some(cfg) = crate::game::gui_layout(&vfs, &name) else { continue };
                let mut prefixes: Vec<(String, (i32, i32))> = Vec::new();
                for (key, panel) in &cfg.panels {
                    let origin = panel.area.map_or((0, 0), |r| (r.x, r.y));
                    prefixes.push((format!("{key}BackgroundPanel"), origin));
                }
                for (key, _) in &cfg.numbers {
                    if let Some(prefix) = key.strip_suffix("Size").filter(|p| !p.is_empty()) {
                        let panel = prefix.strip_suffix("Panel").unwrap_or(prefix);
                        let origin = cfg.rects.iter().find(|(k, _)| k.eq_ignore_ascii_case(&format!("{panel}PanelArea"))).map_or((0, 0), |(_, r)| (r.x, r.y));
                        prefixes.push((prefix.to_owned(), origin));
                    }
                }
                for fit in [HudFit::Centred, HudFit::Stretched] {
                    for (w, h) in WINDOWS {
                        for factor in FACTORS {
                            let l = StockLayout::with_fit(&cfg, Vec2::new(w / factor, h / factor), fit);
                            for (prefix, origin) in &prefixes {
                                if l.canvas_pieces(prefix).len() > 1 {
                                    assert_joined(&l, prefix, *origin, factor, &format!("{name} {fit:?} {w}x{h} @{factor}"));
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(checked > 0, "an install root is set but no gui cfg piece set was checked: wrong path or no gui*.cfg");
    }
}
