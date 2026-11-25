//! The Armada I speed rail: authored special slots 1..5, then the separator and TRANSPORT.
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
//! - The T hotkey remains the client's own `select_type` default.

use bevy::prelude::*;
use sta_content::{A2Effect, A2Target};
use sta_sim::specials::SpecialKind;

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

/// The rail's authored special positions and transport button.
pub(in crate::ui::hud) fn buttons(feed: &HudFeed, strings: Option<&Strings>) -> Vec<Button> {
    let caps = commands::shared(feed);
    let word = |key: &str, fallback: &str| {
        strings.map(|s| s.tooltip_text(key)).filter(|t| *t != key).map_or_else(|| fallback.replace('^', ""), |t| t.replace('^', ""))
    };
    let mut rail: [Option<(bool, i64, Button)>; SEPARATOR] = Default::default();
    if let Some(db) = feed.content.as_deref() {
        for (slot, kind) in caps.specials {
            let mut units = feed.selected_units().into_iter().filter(|u| u.owner == feed.player);
            let Some(first) = units.next() else { continue };
            let Some((_, _, weapon)) = db.unit(first.def)
                .and_then(|def| sta_sim::specials::specials_indexed(db, def).nth(slot as usize)) else { continue };
            if db.tech_tree().is_some_and(|t| t.entry(&weapon.odf).map_or(!weapon.campaign_transwarp(), |e| e.withheld)) {
                continue;
            }
            let Some(position) = weapon.raw.i64("buttonSlot").filter(|n| (1..=5).contains(n)) else { continue };
            if !units.all(|u| db.unit(u.def)
                .and_then(|def| sta_sim::specials::specials_indexed(db, def).nth(slot as usize))
                .is_some_and(|(_, other_kind, other)| other_kind == kind && other.raw.i64("buttonSlot") == Some(position))) {
                continue;
            }
            let channel_on = feed.own_selection().into_iter().any(|id|
                sta_sim::specials::a1_special_active(&feed.view, id, slot));
            let targeted = match kind {
                SpecialKind::A1(sta_content::A1SpecialKind::Gemini | sta_content::A1SpecialKind::EngineeringTeam | sta_content::A1SpecialKind::Stasis) => true,
                SpecialKind::PointDefence if weapon.a1_special.is_some() => false,
                SpecialKind::A1(_) => false,
                SpecialKind::A2(target) if weapon.a2_special.is_some_and(|rule|
                    matches!(rule.effect, A2Effect::TranswarpGate | A2Effect::TractorBeam(_))) => {
                    !channel_on && matches!(target, A2Target::Entity | A2Target::Point)
                }
                _ => continue,
            };
            let ready = feed.own_selection().into_iter().any(|id| match kind {
                SpecialKind::A2(_) => sta_sim::specials::a2_special_ready(&feed.view, db, id, slot),
                _ => sta_sim::specials::a1_special_ready(&feed.view, db, id, slot),
            });
            let researched = feed.own_selection().into_iter().all(|id| match kind {
                SpecialKind::A2(_) => sta_sim::specials::a2_special_researched(&feed.view, db, id, slot),
                _ => sta_sim::specials::a1_special_researched(&feed.view, db, id, slot),
            });
            let stem = weapon.odf.rsplit(['/', '\\']).next().unwrap_or(&weapon.odf).trim_end_matches(".odf");
            let label = weapon.raw.str("wpnName").unwrap_or(stem);
            let index = position as usize - 1;
            let priority = weapon.raw.i64("buttonPriority").unwrap_or(0);
            let rail_slot = &mut rail[index];
            if rail_slot.as_ref().is_some_and(|(r, p, _)| (*r, *p) >= (researched, priority)) {
                continue;
            }
            *rail_slot = Some((researched, priority, Button {
                cell: RAIL_BASE + index,
                label: tr(label).to_uppercase(),
                act: if targeted { Act::Target(TargetMode::Special(slot)) } else { Act::Special(slot) },
                look: if ready { Look::Ready } else { Look::Locked },
                tip: weapon.raw.str("tooltip").map_or_else(|| tr(label), |tip| word(tip, tip)),
                verbose: weapon.raw.str("verboseTooltip").map(|tip| word(tip, tip)),
                sprite_names: vec![format!("b_{stem}")],
                keep_label: false,
            }));
        }
    }
    let mut out: Vec<_> = rail.into_iter().flatten().map(|(_, _, b)| b).collect();
    if !caps.transport { return out; }
    let label = tr("Transport");
    out.push(
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
    );
    out
}

/// The separator's rectangle and sprite (`speedSeparatorArea`, `speedSeparator`).
pub(in crate::ui::hud) fn separator(layout: &StockLayout) -> Option<(R, String)> {
    Some((rect(layout, SEPARATOR)?, layout.name("speedSeparator")?.to_owned()))
}
