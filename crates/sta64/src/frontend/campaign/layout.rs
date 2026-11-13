//! Where the campaign screens keep their parts, as regions of the 800x600 design
//! screen (the box and the fit are [`stock::layout`](super::super::screens::stock::layout)'s).
//!
//! **Evidence.** The campaign *select* is Single Player's four cards; their regions are
//! `stock::layout::Slot::Card` (art rectangles measured on Armada II by Refit's bench,
//! [`Grade::Measured`]) and are reused here for the card controls. Everything below is
//! [`Grade::Arranged`], by evidence:
//!
//! - **Mission list.** The stock `do_missionSelect` is a dialog of one row per mission
//!   with O.K. and Cancel at its foot (Refit `ui/MENUS.md`); it names no coordinates and
//!   the install carries none, so the list's window is ours: a rows column on the left,
//!   what the picked mission is on the right, the foot row at the stock bottom-row
//!   height (y 560, the measured bound Game Setup uses).
//! - **Briefing and debrief.** Armada II has none: a won mission shows no screen of its
//!   own (`do_success_mission` only advances the counter; the Admiral's Log comes up after
//!   a win and its Done returns to Single Player), measured by Refit's bench. Both screens
//!   follow the plan's flow (UI-25), not a stock screen. Armada I and Fleet Operations
//!   were **not measured**; the same regions are used for every game.

use crate::frontend::screens::stock::layout::{Grade, Region};

/// Each control of the campaign screens, by what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CSlot {
    // Mission list.
    ListTitle,
    ListRows,
    ListDetails,
    ListBack,
    ListPlay,
    // Briefing.
    BriefTitle,
    BriefText,
    BriefGoals,
    BriefBack,
    BriefStart,
    // Debrief.
    DebTitle,
    DebResult,
    DebText,
    DebList,
    DebRetry,
    DebNext,
}

const fn a(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region { x, y, w, h, grade: Grade::Arranged }
}

/// The region of `slot` in design pixels.
pub fn region(slot: CSlot) -> Region {
    match slot {
        CSlot::ListTitle | CSlot::BriefTitle | CSlot::DebTitle => a(0.0, 0.0, 800.0, 30.0),
        CSlot::ListRows => a(40.0, 50.0, 360.0, 480.0),
        CSlot::ListDetails => a(430.0, 50.0, 330.0, 480.0),
        CSlot::ListBack | CSlot::BriefBack | CSlot::DebList => a(24.0, 560.0, 200.0, 34.0),
        CSlot::ListPlay | CSlot::BriefStart | CSlot::DebNext => a(576.0, 560.0, 200.0, 34.0),
        CSlot::BriefText => a(40.0, 50.0, 720.0, 310.0),
        CSlot::BriefGoals => a(40.0, 370.0, 720.0, 170.0),
        CSlot::DebResult => a(40.0, 50.0, 720.0, 60.0),
        CSlot::DebText => a(40.0, 120.0, 720.0, 410.0),
        CSlot::DebRetry => a(300.0, 560.0, 200.0, 34.0),
    }
}

/// The card regions of Single Player for `n` campaigns: Armada II's four are the
/// measured/arranged `Card` regions; a shell with more (Armada I's five) stacks
/// them in rows, arranged.
pub fn card_region(i: usize, n: usize) -> Region {
    if n <= 4 {
        crate::frontend::screens::stock::layout::region(crate::frontend::screens::stock::layout::Slot::Card(i))
    } else {
        let pitch = 520.0 / n as f32;
        a(150.0, 20.0 + pitch * i as f32, 500.0, pitch - 10.0)
    }
}

/// Which slots a screen has, for the layout tests.
#[cfg(test)]
pub fn slots_of(screen: crate::frontend::screens::Screen) -> &'static [CSlot] {
    use crate::frontend::screens::Screen;
    match screen {
        Screen::Missions => &[CSlot::ListTitle, CSlot::ListRows, CSlot::ListDetails, CSlot::ListBack, CSlot::ListPlay],
        Screen::Briefing => &[CSlot::BriefTitle, CSlot::BriefText, CSlot::BriefGoals, CSlot::BriefBack, CSlot::BriefStart],
        Screen::Debrief => &[CSlot::DebTitle, CSlot::DebResult, CSlot::DebText, CSlot::DebList, CSlot::DebRetry, CSlot::DebNext],
        _ => &[],
    }
}
