//! The Armada I speed rail's TRANSPORT button (UI-40 follow-up). STA64's rule: no control-grid slot
//! where the install's command data gives the command none; on the Armada I HUD the transport command
//! sits on the speed rail above the ship display.
//!
//! Rules and their grade (read from the executable, static, and from the install's cfg and manual):
//! - The rail has seven positions, left to right, in the sheet's `speedSingleButtonArea` rectangle
//!   moved right by the earlier positions: a button position advances by its width plus
//!   `speedSingleButtonGap`, the separator position by the separator's width
//!   (`speedSeparatorArea`) plus the gap. Positions 0 to 4 are production jobs or special weapons,
//!   position 5 is the separator (`speedSeparator` sprite), position 6 is TRANSPORT.
//! - The separator and the transport slot belong to every selected craft; a selection of several
//!   units shows one transport button (the shared capability, as for every other command).
//! - The button's art is the install's `b_transport` and its tooltip keys are
//!   `GUI_CP_TRANS_TOOLTIP` and `GUI_CP_TRANS_VTOOLTIP`; the press arms the same target pick as any
//!   other TRANSPORT press (`Act::Target(TargetMode::Transport)`), and the click issues
//!   `Order::Transport` for the ready carriers through the normal command queue.
//! - Interim (marked, TODO line): the stock button is enabled whenever the carrier has crew; here it
//!   is shown disabled when the sim would refuse every selected carrier (UI-40's rule), so the
//!   press never ends in a silent refusal.
//! - Not built: the T hotkey (the client's own `select_type` default is T), the special weapon
//!   slots, the pressed look while a transport runs.

use bevy::prelude::*;

use super::{super::layout::StockLayout, panel_origin};
use crate::ui::{
    hud::{
        commands,
        layout::R,
        model::{Act, Button, HudFeed, Look, TargetMode},
    },
    i18n::{Strings, tr},
};

/// Positions on the rail, left to right (7 buttons, 0 to 6).
const POSITIONS: usize = 7;
/// The separator's position (the slot just before transport).
const SEPARATOR: usize = 5;
/// The transport button's position.
const TRANSPORT: usize = 6;

/// `Button.cell` values at or above this are rail positions (`cell - RAIL_BASE`), not control slots.
pub(in crate::ui::hud) const RAIL_BASE: usize = 100;

/// The rectangle of rail position `at` in screen units, from the sheet's keys; `None` without the
/// rail's keys or past the last position.
pub(in crate::ui::hud) fn rect(layout: &StockLayout, at: usize) -> Option<R> {
    if at >= POSITIONS {
        return None;
    }
    let (_, origin) = panel_origin(layout, "speed")?;
    let button = layout.raw_rect("speedSingleButtonArea").filter(|r| r.w > 0 && r.h > 0)?;
    let separator = layout.raw_rect("speedSeparatorArea").filter(|r| r.w > 0 && r.h > 0)?;
    let gap = layout.number("speedSingleButtonGap").filter(|g| g.is_finite() && *g >= 0.0).map_or(0, |g| g as i32);
    let before: i32 = (0..at).map(|k| if k == SEPARATOR { separator.w } else { button.w } + gap).sum();
    layout.at_shifted(origin, if at == SEPARATOR { "speedSeparatorArea" } else { "speedSingleButtonArea" }, before)
}

/// The first five rail positions are the producer's queue in Armada I and A1Compat.
pub(in crate::ui::hud) fn queue_slots(layout: &StockLayout) -> Vec<(usize, R)> {
    (0..SEPARATOR).filter_map(|index| rect(layout, index).map(|r| (index, r))).collect()
}

/// The rail's buttons for the selection: the transport button when the selection has a transporter
/// (the separator before it is drawn with the button, by `control::spawn_grid`).
pub(in crate::ui::hud) fn buttons(feed: &HudFeed, strings: Option<&Strings>) -> Vec<Button> {
    let caps = commands::shared(feed);
    if !caps.transport {
        return Vec::new();
    }
    let word = |key: &str, fallback: &str| {
        strings.map(|s| s.tooltip_text(key)).filter(|t| *t != key).map_or_else(|| fallback.to_owned(), |t| t.replace('^', ""))
    };
    let label = tr("Transport");
    vec![
        Button {
            cell: RAIL_BASE + TRANSPORT,
            label: label.to_uppercase(),
            act: Act::Target(TargetMode::Transport),
            look: if caps.transport_ready { Look::Ready } else { Look::Locked },
            tip: word("GUI_CP_TRANS_TOOLTIP", &label),
            verbose: strings.map(|s| s.tooltip_text("GUI_CP_TRANS_VTOOLTIP")).filter(|t| *t != "GUI_CP_TRANS_VTOOLTIP").map(|t| t.replace('^', "")),
            sprite_names: vec!["b_transport".into()],
            keep_label: false,
        },
    ]
}

/// The separator's rectangle and sprite (`speedSeparatorArea`, `speedSeparator`).
pub(in crate::ui::hud) fn separator(layout: &StockLayout) -> Option<(R, String)> {
    Some((rect(layout, SEPARATOR)?, layout.name("speedSeparator")?.to_owned()))
}
