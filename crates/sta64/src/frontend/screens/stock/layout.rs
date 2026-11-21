//! Where the stock front end keeps its parts, as regions of the 800x600 design
//! screen, and how they land in a window.
//!
//! **Evidence.** The stock games draw their menus from the executable (fixed-pixel
//! dialogs over 800x600 bitmaps); no cfg or sprite file of the install carries their
//! positions, so nothing here can be read at run time. The numbers marked
//! [`Grade::Measured`] are the design coordinates Refit (the user's own first-party
//! mod, `ui/MENUS.md` and its `menus/screens/`) measured on Armada II's running
//! screens: Game Setup's table, map window and bottom row, and the single player
//! cards. The main menu is an authored arrangement. Everything marked [`Grade::Arranged`] is a
//! choice of ours made to fit the controls this front end has (our own look, our own
//! widgets); it is not measured. Fleet Operations' and Armada I's screens are **not
//! measured**: the same regions are used for every game until they are.
//!
//! **Fit.** The screen is a box of the design's shape, as large as the window allows,
//! with its aspect clamped to 4:3 .. 16:9 and centred, so a 21:9 window gets a 16:9
//! box with the scene showing at the sides instead of stretched tables. Regions are
//! fractions of the box. The main menu's typography scales with the box height;
//! the other screens keep the theme's own logical text and control sizes.

use bevy::math::{Rect, Vec2};

/// The design screen the stock menus are laid out on.
pub const DESIGN: Vec2 = Vec2::new(800.0, 600.0);
pub const MIN_ASPECT: f32 = 4.0 / 3.0;
pub const MAX_ASPECT: f32 = 16.0 / 9.0;

/// How a region's numbers are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    /// Measured on the stock Armada II screen (Refit's bench), design pixels.
    Measured,
    /// Our arrangement; the stock screen's own value is not measured.
    Arranged,
}

/// A rectangle in design pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub grade: Grade,
}

const fn m(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region { x, y, w, h, grade: Grade::Measured }
}
const fn a(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region { x, y, w, h, grade: Grade::Arranged }
}

impl Region {
    /// The region in a box of `b` logical pixels.
    pub fn px(self, b: Vec2) -> Rect {
        let k = b / DESIGN;
        Rect::new(self.x * k.x, self.y * k.y, (self.x + self.w) * k.x, (self.y + self.h) * k.y)
    }

    /// The region as a design-space rectangle.
    pub fn rect(self) -> Rect {
        Rect::new(self.x, self.y, self.x + self.w, self.y + self.h)
    }

    /// The same region with another grade of evidence (a part of a measured one).
    pub const fn arranged(self) -> Region {
        Region { grade: Grade::Arranged, ..self }
    }
}

/// The box a screen is laid out in, for a window `avail` logical pixels large (the
/// window's size divided by the UI scale).
pub fn fit_box(avail: Vec2) -> Vec2 {
    if avail.x <= 0.0 || avail.y <= 0.0 {
        return Vec2::ZERO;
    }
    let aspect = (avail.x / avail.y).clamp(MIN_ASPECT, MAX_ASPECT);
    let h = avail.y.min(avail.x / aspect);
    Vec2::new((h * aspect).floor(), h.floor())
}

/// Each control of the stock screens, by what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    // Main menu.
    MainSinglePlayer,
    MainKicker,
    MainTitle,
    MainDivider,
    MainInstantAction,
    MainMultiplayer,
    MainMapEditor,
    MainOptions,
    MainReplayIntro,
    MainCredits,
    MainExit,
    MainMods,
    // Single player.
    Card(usize),
    SpLoad,
    SpBack,
    // Game setup (Instant Action).
    SetupTitle,
    HeadControl,
    HeadSide,
    HeadTeam,
    SeatColour(usize),
    SeatControl(usize),
    SeatSide(usize),
    SeatTeam(usize),
    ModeLabel,
    ModePlay,
    ModeObserve,
    RandomColour,
    Palette(usize),
    MapInfo,
    Preview,
    SelectMap,
    Resources,
    Officers,
    Advanced,
    SetupBack,
    Launch,
    AdvancedTitle,
    AdvancedResources,
    AdvancedOfficers,
    AdvancedSpeed,
    AdvancedDefault,
    AdvancedCancel,
    AdvancedClose,
    // Options.
    OptionsTitle,
    OptionsBody,
    OptionsBack,
}

/// Seats the Game Setup table has room for.
pub const TABLE_ROWS: usize = 8;

/// Game Setup's player panel stays left of x 454. These are arranged regions
/// within it; the palette and match-mode controls occupy the space below rows.
pub const TABLE_TOP: f32 = 119.0;
pub const TABLE_BOTTOM: f32 = 383.0;
pub const COL_X: [f32; 3] = [67.0, 217.0, 365.0];
pub const COL_RIGHT: f32 = 449.0;

/// Design height of one table row.
pub fn row_pitch() -> f32 {
    (TABLE_BOTTOM - TABLE_TOP) / TABLE_ROWS as f32
}

/// The map image uses the design height as its single scale, so a wide box
/// gives it side margins instead of stretching the authored square texture.
pub fn placed_rect(slot: Slot, b: Vec2) -> Rect {
    if slot == Slot::Preview {
        let size = region(slot).h * b.y / DESIGN.y;
        let centre_x = 621.0 * b.x / DESIGN.x;
        let top = region(slot).y * b.y / DESIGN.y;
        Rect::new(centre_x - size / 2.0, top, centre_x + size / 2.0, top + size)
    } else {
        region(slot).px(b)
    }
}

/// The region of `slot`.
pub fn region(slot: Slot) -> Region {
    let col_w = |i: usize| {
        let next = COL_X.get(i + 1).copied().unwrap_or(COL_RIGHT);
        next - COL_X[i] - 10.0
    };
    match slot {
        // Main menu: our own left-aligned list. The 1920x1080 reference informs
        // the arrangement, but these design-space regions are not stock measurements.
        Slot::MainKicker => a(64.0, 128.0, 250.0, 23.0),
        Slot::MainTitle => a(63.0, 85.0, 680.0, 105.0),
        Slot::MainDivider => a(64.0, 190.5, 122.0, 1.0),
        Slot::MainSinglePlayer => main_row(0),
        Slot::MainInstantAction => main_row(1),
        Slot::MainMultiplayer => main_row(2),
        Slot::MainMapEditor => main_row(3),
        Slot::MainMods => main_row(4),
        Slot::MainOptions => main_row(5),
        Slot::MainReplayIntro => main_row(6),
        Slot::MainCredits => main_row(7),
        Slot::MainExit => main_row(8),
        // Single player: four cards in the quadrants the stock panels use (art rectangles
        // measured; clipped above the footer here), the footer below.
        Slot::Card(0) => m(24.0, 16.0, 330.0, 205.0),
        Slot::Card(1) => m(436.0, 22.0, 310.0, 216.0),
        Slot::Card(2) => a(24.0, 338.0, 330.0, 200.0),
        Slot::Card(_) => a(431.0, 340.0, 322.0, 198.0),
        Slot::SpBack => a(24.0, 560.0, 200.0, 34.0),
        Slot::SpLoad => a(576.0, 560.0, 200.0, 34.0),
        // Game setup.
        Slot::SetupTitle => a(26.0, 8.0, 750.0, 40.0),
        Slot::HeadControl => a(COL_X[0], 76.0, col_w(0), 34.0),
        Slot::HeadSide => a(COL_X[1], 76.0, col_w(1), 34.0),
        Slot::HeadTeam => a(COL_X[2], 76.0, col_w(2), 34.0),
        Slot::SeatColour(i) => a(42.0, TABLE_TOP + row_pitch() * i as f32 + 8.0, 17.0, 17.0),
        Slot::SeatControl(i) => row(0, i),
        Slot::SeatSide(i) => row(1, i),
        Slot::SeatTeam(i) => row(2, i),
        // Match mode is outside the map's seat rows, in the free area below the table.
        Slot::ModeLabel => a(42.0, 472.0, 52.0, 34.0),
        Slot::ModePlay => a(101.0, 472.0, 148.0, 34.0),
        Slot::ModeObserve => a(257.0, 472.0, 166.0, 34.0),
        Slot::RandomColour => a(42.0, 409.0, 140.0, 34.0),
        Slot::Palette(i) => a(192.0 + (i % 8) as f32 * 27.0, 412.0 + (i / 8) as f32 * 25.0, 17.0, 17.0),
        // Map title and preview in the right-hand panel (arranged).
        Slot::MapInfo => a(483.0, 76.0, 280.0, 62.0),
        Slot::Preview => a(518.0, 146.0, 206.0, 206.0),
        // Available map and option controls sit below the preview.
        Slot::SelectMap => a(483.0, 366.0, 280.0, 34.0),
        Slot::Resources => a(483.0, 407.0, 280.0, 34.0),
        Slot::Officers => a(483.0, 448.0, 280.0, 34.0),
        Slot::Advanced => a(483.0, 489.0, 280.0, 34.0),
        // The footer is anchored below both panels.
        Slot::SetupBack => a(24.0, 554.0, 200.0, 34.0),
        Slot::Launch => a(576.0, 554.0, 200.0, 34.0),
        Slot::AdvancedTitle => a(173.0, 117.0, 454.0, 40.0),
        Slot::AdvancedResources => a(173.0, 196.0, 215.0, 40.0),
        Slot::AdvancedOfficers => a(173.0, 250.0, 215.0, 40.0),
        Slot::AdvancedSpeed => a(412.0, 196.0, 215.0, 40.0),
        Slot::AdvancedClose => a(173.0, 439.0, 130.0, 36.0),
        Slot::AdvancedCancel => a(335.0, 439.0, 130.0, 36.0),
        Slot::AdvancedDefault => a(497.0, 439.0, 130.0, 36.0),
        // Options: not measured.
        Slot::OptionsTitle => a(100.0, 16.0, 600.0, 30.0),
        Slot::OptionsBody => a(100.0, 60.0, 600.0, 470.0),
        Slot::OptionsBack => a(24.0, 560.0, 200.0, 34.0),
    }
}

fn main_row(index: usize) -> Region {
    a(72.0, 199.0 + index as f32 * (110.0 / 3.0), 285.0, 32.0)
}

fn row(col: usize, i: usize) -> Region {
    let next = COL_X.get(col + 1).copied().unwrap_or(COL_RIGHT);
    Region { x: COL_X[col], y: TABLE_TOP + row_pitch() * i as f32, w: next - COL_X[col] - 10.0, h: row_pitch(), grade: Grade::Arranged }
}

/// Where the map preview puts a start position: `pos` (x, z) inside the map's
/// bounds `min`..`max` (x, z), as a fraction of the frame, z downwards.
pub fn preview_point(pos: (f32, f32), min: (f32, f32), max: (f32, f32)) -> (f32, f32) {
    let span = |v: f32, lo: f32, hi: f32| if hi > lo { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.5 };
    (span(pos.0, min.0, max.0), span(pos.1, min.1, max.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_keeps_the_design_shape_between_4_3_and_16_9() {
        // (window, expected aspect)
        for (avail, want) in [
            (Vec2::new(3440.0, 1440.0), 16.0 / 9.0),
            (Vec2::new(1920.0, 1080.0), 16.0 / 9.0),
            (Vec2::new(1920.0, 1200.0), 1.6),
            (Vec2::new(1600.0, 1200.0), 4.0 / 3.0),
            (Vec2::new(1000.0, 1000.0), 4.0 / 3.0),
        ] {
            let b = fit_box(avail);
            assert!(b.x <= avail.x && b.y <= avail.y, "{avail}: {b}");
            assert!((b.x / b.y - want).abs() < 0.01, "{avail}: {b}");
        }
        assert_eq!(fit_box(Vec2::ZERO), Vec2::ZERO);
    }

    #[test]
    fn measured_bounds_hold_between_regions() {
        // Player rows stay left of the map panel; map info, preview, options
        // and footer follow from top to bottom.
        let r = |s| region(s).rect();
        assert!(r(Slot::SeatTeam(7)).max.x <= 450.0 && r(Slot::SeatTeam(7)).max.x <= r(Slot::MapInfo).min.x);
        assert!(r(Slot::MapInfo).max.y <= r(Slot::Preview).min.y);
        assert!(r(Slot::Preview).max.y <= r(Slot::Resources).min.y);
        assert!(r(Slot::SeatControl(7)).max.y <= r(Slot::Launch).min.y);
        assert!(r(Slot::Launch).min.y >= 550.0);
        // The three table columns retain their arranged panel positions.
        assert_eq!([r(Slot::SeatControl(0)).min.x, r(Slot::SeatSide(0)).min.x, r(Slot::SeatTeam(0)).min.x], COL_X);
        assert_eq!(region(Slot::Preview).rect(), Rect::new(518.0, 146.0, 724.0, 352.0));
    }

    #[test]
    fn preview_points_stay_in_the_frame() {
        assert_eq!(preview_point((0.0, 0.0), (-10.0, -10.0), (10.0, 10.0)), (0.5, 0.5));
        assert_eq!(preview_point((99.0, -99.0), (-10.0, -10.0), (10.0, 10.0)), (1.0, 0.0));
        assert_eq!(preview_point((3.0, 3.0), (1.0, 1.0), (1.0, 1.0)), (0.5, 0.5));
    }
}
