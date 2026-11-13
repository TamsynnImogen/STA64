//! Where the multiplayer screens keep their parts: regions of the 800x600 design screen,
//! fitted to a window by [`stock::layout::fit_box`](crate::frontend::screens::stock::layout::fit_box).
//!
//! **Evidence.** The stock games draw these screens from the executable; no cfg or sprite
//! file carries their positions. Marked [`Grade::Measured`]: what Refit's bench measured on
//! Armada II's multiplayer **Game Setup** (the same dialog Instant Action uses, with the
//! extra columns of a networked game; `ui/MENUS.md`, `menus/screens/game_setup.rs`): the
//! ready light's column (x 14, a 10x10 picture), the player, side and team columns at
//! x 41, 203 and 375, the log and chat line under the table (left of x 450, the chat line's
//! row at y 533), the map's labels, preview and settings (right of x 480; preview at
//! 627,129, 155x155), the bottom row (below y 560). Everything else is **Arranged**, ours:
//!
//! - the **Multiplayer Connection** screen (stock: the player's name and one button per
//!   connection type; Refit arranges it, nothing measured), the **host** and **join** forms
//!   (stock: a Manual IP dialog, a game name, a password form; none measured; the original
//!   Internet lobby with its list of games has no counterpart because the relay lists no
//!   games), and the room code, the open/closed toggle and the status lines of the lobby;
//! - the stock "Armada" (diplomacy) column and the colour palette have no counterpart in a
//!   `GameSetup` and are not drawn; the chat target toggle (All, x 315 y 533) is not drawn
//!   (chat goes to everyone).
//!
//! Armada I and Fleet Operations: **not measured**. The same regions are used for every game.

use bevy::math::Vec2;

use crate::frontend::screens::stock::layout::{Grade, Region, row_pitch};

/// Each part of the multiplayer screens, by what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MSlot {
    Title,
    // Multiplayer Connection.
    NameLabel,
    NameField,
    RelayLabel,
    RelayField,
    ConnNotice,
    HostBtn,
    JoinBtn,
    Back,
    // Host form.
    HostPassLabel,
    HostPassField,
    CreateBtn,
    HostNotice,
    // Join form.
    CodeLabel,
    CodeField,
    JoinPassLabel,
    JoinPassField,
    JoinNowBtn,
    RejoinBtn,
    RejoinNote,
    JoinNotice,
    // Lobby: the table.
    HeadPlayer,
    HeadSide,
    HeadTeam,
    Light(usize),
    Player(usize),
    Kick(usize),
    Side(usize),
    Team(usize),
    // Lobby: under the table.
    Log,
    ChatLine,
    // Lobby: the map window and the room.
    MapInfo,
    Preview,
    SelectMap,
    Resources,
    Officers,
    RoomInfo,
    OpenToggle,
    Status,
    Leave,
    Launch,
}

/// Seats the table has room for.
pub const ROWS: usize = crate::frontend::screens::stock::layout::TABLE_ROWS;

const fn m(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region { x, y, w, h, grade: Grade::Measured }
}
const fn a(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region { x, y, w, h, grade: Grade::Arranged }
}

/// The multiplayer table's columns (measured on a networked Game Setup): the light, the
/// player, the side and the team, and the table's right edge.
pub const LIGHT_X: f32 = 14.0;
pub const COL_X: [f32; 3] = [41.0, 203.0, 375.0];
pub const COL_RIGHT: f32 = 450.0;
pub const TABLE_TOP: f32 = 80.0;
/// Width of the Kick button at the right end of the player column (arranged).
const KICK_W: f32 = 52.0;

/// The region of `slot`.
pub fn region(slot: MSlot) -> Region {
    let col = |c: usize, i: usize| {
        let next = COL_X.get(c + 1).copied().unwrap_or(COL_RIGHT);
        (COL_X[c], TABLE_TOP + row_pitch() * i as f32, next - COL_X[c] - 10.0, row_pitch())
    };
    match slot {
        MSlot::Title => m(0.0, 0.0, 800.0, 30.0),
        // Multiplayer Connection: arranged.
        MSlot::NameLabel => a(200.0, 100.0, 400.0, 26.0),
        MSlot::NameField => a(200.0, 128.0, 400.0, 40.0),
        MSlot::RelayLabel => a(200.0, 184.0, 400.0, 26.0),
        MSlot::RelayField => a(200.0, 212.0, 400.0, 40.0),
        MSlot::ConnNotice => a(150.0, 272.0, 500.0, 70.0),
        MSlot::HostBtn => a(250.0, 360.0, 300.0, 44.0),
        MSlot::JoinBtn => a(250.0, 412.0, 300.0, 44.0),
        MSlot::Back => a(24.0, 560.0, 200.0, 34.0),
        // Host form.
        MSlot::HostPassLabel => a(200.0, 120.0, 400.0, 26.0),
        MSlot::HostPassField => a(200.0, 148.0, 400.0, 40.0),
        MSlot::CreateBtn => a(250.0, 220.0, 300.0, 44.0),
        MSlot::HostNotice => a(150.0, 290.0, 500.0, 130.0),
        // Join form.
        MSlot::CodeLabel => a(200.0, 90.0, 400.0, 26.0),
        MSlot::CodeField => a(200.0, 118.0, 400.0, 40.0),
        MSlot::JoinPassLabel => a(200.0, 170.0, 400.0, 26.0),
        MSlot::JoinPassField => a(200.0, 198.0, 400.0, 40.0),
        MSlot::JoinNowBtn => a(250.0, 262.0, 300.0, 44.0),
        MSlot::RejoinBtn => a(250.0, 314.0, 300.0, 44.0),
        MSlot::RejoinNote => a(150.0, 362.0, 500.0, 50.0),
        MSlot::JoinNotice => a(150.0, 420.0, 500.0, 110.0),
        // The table: columns measured, row pitch as Instant Action's.
        MSlot::HeadPlayer => m(COL_X[0], 40.0, col(0, 0).2, 40.0),
        MSlot::HeadSide => m(COL_X[1], 40.0, col(1, 0).2, 40.0),
        MSlot::HeadTeam => m(COL_X[2], 40.0, col(2, 0).2, 40.0),
        MSlot::Light(i) => {
            let (_, y, _, h) = col(0, i);
            m(LIGHT_X, y + (h - 10.0) / 2.0, 10.0, 10.0)
        }
        MSlot::Player(i) => {
            let (x, y, w, h) = col(0, i);
            a(x, y, w - KICK_W - 4.0, h).arranged()
        }
        MSlot::Kick(i) => {
            let (x, y, w, h) = col(0, i);
            a(x + w - KICK_W, y, KICK_W, h)
        }
        MSlot::Side(i) => {
            let (x, y, w, h) = col(1, i);
            a(x, y, w, h)
        }
        MSlot::Team(i) => {
            let (x, y, w, h) = col(2, i);
            a(x, y, w, h)
        }
        // Under the table, left of x 450; the chat line's row is at y 533.
        MSlot::Log => a(24.0, 326.0, 426.0, 192.0),
        MSlot::ChatLine => a(24.0, 524.0, 426.0, 30.0),
        // The map window, as Instant Action's.
        MSlot::MapInfo => m(480.0, 40.0, 300.0, 85.0),
        MSlot::Preview => m(627.0, 129.0, 155.0, 155.0),
        MSlot::SelectMap => a(480.0, 129.0, 140.0, 40.0),
        MSlot::Resources => a(480.0, 177.0, 140.0, 40.0),
        MSlot::Officers => a(480.0, 225.0, 140.0, 40.0),
        MSlot::RoomInfo => a(480.0, 292.0, 300.0, 30.0),
        MSlot::OpenToggle => a(480.0, 326.0, 300.0, 34.0),
        MSlot::Status => a(480.0, 368.0, 300.0, 150.0),
        MSlot::Leave => a(24.0, 560.0, 200.0, 34.0),
        MSlot::Launch => a(576.0, 560.0, 200.0, 34.0),
    }
}

/// Where the map preview puts a start position (shared with Instant Action's).
pub use crate::frontend::screens::stock::layout::preview_point;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_keeps_the_measured_columns() {
        let r = |s| region(s).rect();
        assert_eq!(r(MSlot::Light(0)).min.x, 14.0);
        assert_eq!((r(MSlot::Light(0)).width(), r(MSlot::Light(0)).height()), (10.0, 10.0));
        assert_eq!([r(MSlot::Player(0)).min.x, r(MSlot::Side(0)).min.x, r(MSlot::Team(0)).min.x], COL_X);
        // The table stays left of the map column; the log and the chat line under it, left of x 450.
        assert!(r(MSlot::Team(7)).max.x <= 450.0 && r(MSlot::Team(7)).max.x <= r(MSlot::MapInfo).min.x);
        assert!(r(MSlot::Log).max.x <= 450.0 && r(MSlot::ChatLine).max.x <= 450.0);
        assert!(r(MSlot::Side(7)).max.y <= r(MSlot::Log).min.y);
        assert!(r(MSlot::ChatLine).max.y <= r(MSlot::Launch).min.y);
        assert!(r(MSlot::Launch).min.y >= 560.0 && r(MSlot::Leave).min.y >= 560.0);
        assert_eq!(region(MSlot::Preview).rect(), bevy::math::Rect::new(627.0, 129.0, 782.0, 284.0));
        assert!(r(MSlot::Resources).max.x <= r(MSlot::Preview).min.x);
        // The light is the only measured part of a row that stays a picture.
        assert_eq!(region(MSlot::Light(3)).grade, Grade::Measured);
        assert_eq!(region(MSlot::Kick(3)).grade, Grade::Arranged);
        assert_eq!(region(MSlot::NameField).grade, Grade::Arranged);
    }
}
