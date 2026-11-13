//! The stock-layout front end (UI-22): main, single player, Instant Action (game
//! setup) and options, arranged as the stock games arrange them and drawn with the
//! widget kit (UI-21) in the active profile's look, never the stock art. Replaces
//! UI-07's layouts as the default ([`ScreenLayout::Stock`]); map select and mod
//! select keep UI-07's list screens, and [`ScreenLayout::Plain`] keeps the old
//! layouts of every screen.
//!
//! - [`layout`]: the regions (design 800x600) and how they fit a window; each is
//!   graded *measured* (Refit's bench on Armada II) or *arranged* (ours, not measured).
//! - Game setup: a dropdown per seat for who plays, the side (every playable faction
//!   of the install, in its skirmish order) and the team; the map's labels and a
//!   preview of its start positions; resources and officer limit beside it;
//!   Launch writes [`StartGame`](super::StartGame) with the draft's `GameSetup`.
//! - Texts: the mounted `label.map` key where there is one, else translated English.

use bevy::math::Vec2;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub mod layout;
mod screens;
#[cfg(test)]
mod tests;

pub use layout::{Grade, Region, Slot};
pub use screens::{Placed, SeatDrop, StockBoxNode, cap, preview_dots, race_ids, spawn};

use super::{Col, Screen, ScreenData, SeatControl, SetupDraft};
use crate::ui::theme::DropdownChanged;

/// Which layouts the front end shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenLayout {
    /// The stock games' arrangement (UI-22).
    #[default]
    Stock,
    /// UI-07's LCARS-panel layouts.
    Plain,
}

/// Whether the stock layout has this screen (the others keep UI-07's).
pub fn covers(screen: Screen) -> bool {
    matches!(screen, Screen::Main | Screen::SinglePlayer | Screen::Setup | Screen::Settings)
}

/// The size of the box the stock screens are laid out in, logical px: the window
/// divided by the UI scale, fitted ([`layout::fit_box`]).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct StockBox(pub Vec2);

impl Default for StockBox {
    fn default() -> Self {
        StockBox(Vec2::new(1440.0, 1080.0))
    }
}

pub(super) fn track_box(window: Query<&Window, With<PrimaryWindow>>, scale: Option<Res<UiScale>>, mut out: ResMut<StockBox>) {
    let Ok(w) = window.single() else { return };
    let s = scale.map_or(1.0, |s| s.0).max(0.01);
    let want = layout::fit_box(Vec2::new(w.width() / s, w.height() / s));
    if want != Vec2::ZERO && out.0 != want {
        out.0 = want;
    }
}

/// A seat's dropdown changed: the draft follows.
pub(super) fn on_dropdowns(mut changes: MessageReader<DropdownChanged>, seats: Query<&SeatDrop>, data: Res<ScreenData>, draft: Option<ResMut<SetupDraft>>) {
    let Some(mut draft) = draft else { return };
    for c in changes.read() {
        let Ok(&SeatDrop { row, col }) = seats.get(c.entity) else { continue };
        match col {
            Col::Control => set_control(&mut draft, row, [SeatControl::Human, SeatControl::Computer, SeatControl::Idle][c.index.min(2)]),
            Col::Race => {
                if let (Some(id), Some(seat)) = (data.races().get(c.index), draft.seats.get_mut(row)) {
                    seat.race.clone_from(id);
                }
            }
            Col::Team => {
                let last = draft.seats.len().saturating_sub(1);
                if let Some(seat) = draft.seats.get_mut(row) {
                    seat.team = c.index.min(last) as u8;
                }
            }
        }
    }
}

/// Give seat `row` the control `to`, keeping one player: taking the player's seat
/// hands the old one to a computer, and a player stepping off passes the role to the
/// next seat (a lone seat stays the player).
pub fn set_control(draft: &mut SetupDraft, row: usize, to: SeatControl) {
    let Some(old) = draft.seats.get(row).map(|s| s.control) else { return };
    if old == to {
        return;
    }
    if old == SeatControl::Human {
        let other = (row + 1) % draft.seats.len();
        if other == row {
            return;
        }
        draft.seats[other].control = SeatControl::Human;
    } else if to == SeatControl::Human {
        for s in &mut draft.seats {
            if s.control == SeatControl::Human {
                s.control = SeatControl::Computer;
            }
        }
    }
    draft.seats[row].control = to;
}
