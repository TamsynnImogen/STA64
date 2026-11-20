//! The Armada I control panel: twelve fixed slots, their commands placed by `game_cfg.h`.
//!
//! Slot rectangles are the cfg's `controlButton1..12` (panel-relative, read from data); which
//! command sits in which slot is `BUTTON_LOCATION_*` of `game_cfg.h` ([`ButtonSlots`], D9/D11).
//! A press goes through the shared [`Act`] path, exactly as an Armada II palette cell does
//! (`hud::press`), so an order here is the order there.
//!
//! Pages: the top page, the Orders page behind the Orders button (alerts, patrol, scout, search
//! and destroy, guard, decommission, self-destruct, back), and the build page (a builder's list
//! in the free slots, with cancel and back where the header puts them).
//!
//! The commands the sim cannot take are drawn locked (disabled) on mobile craft, so the grid keeps
//! the stock slot positions: Repair, Stand Down, the alert states, Patrol, Guard, Decommission and
//! Self-Destruct (their sim orders or the HUD's target pick are later work; TODO lines in the
//! PR). A station shows none of the Orders-page commands, locked or not: which of them the stock
//! HUD shows for a station is not measured (TODO line). Not drawn: Transport Special, Upgrade and
//! Research Upgrade (later). Transport has no control-grid slot where the install's command data gives it none; it
//! sits on the speed rail ([`super::speed`]), whose buttons the same grid carries (cells from
//! `speed::RAIL_BASE`).

use bevy::prelude::*;
use sta_content::{ButtonSlots, SlotCommand, button_slots::SLOTS};

use super::super::{StockState, layout::StockLayout};
use super::{costs, speed};
use crate::ui::{
    i18n::{Strings, tr},
    hud::{
        HudState, commands,
        layout::R,
        model::{self, Act, Button, HudFeed, Look, Page, TargetMode, UnitOrder},
    },
    gui_sprites::GuiImage,
    theme::Theme,
};

/// The slot rectangles in screen units, index 0 for slot 1; `None` where the sheet has no key
/// (or a zero rectangle).
pub(in crate::ui::hud) fn cells(layout: &StockLayout) -> [Option<R>; SLOTS as usize] {
    let origin = layout.panel("control").map_or(Vec2::ZERO, |r| Vec2::new(r.x, r.y));
    std::array::from_fn(|i| layout.at(origin, &format!("controlButton{}", i + 1)))
}

fn button(slot: u8, label: &str, act: Act, look: Look, icons: &[&str]) -> Button {
    Button {
        cell: usize::from(slot - 1),
        label: label.to_uppercase(),
        act,
        look,
        tip: label.to_owned(),
        verbose: None,
        sprite_names: icons.iter().map(|s| (*s).to_owned()).collect(),
        keep_label: false,
    }
}

/// The command a slot key stands for on the top and Orders pages: label, icon names, and the
/// act, `None` when the selection cannot have it. Icons are `gui_global.spr` names.
fn command(c: SlotCommand, feed: &HudFeed) -> Option<(&'static str, &'static [&'static str], Act, Look)> {
    let caps = commands::shared(feed);
    let builder = caps.build.then(|| model::builder(feed)).flatten();
    let mobile = caps.move_order;
    let ready = Look::Ready;
    Some(match c {
        SlotCommand::Move if mobile => ("Move", &["b_move"], Act::Target(TargetMode::Move), ready),
        SlotCommand::Attack if caps.attack => ("Attack", &["b_attack"], Act::Target(TargetMode::Attack), ready),
        SlotCommand::Stop if caps.stop => ("Stop", &["b_stop"], Act::Stop, ready),
        SlotCommand::Orders if mobile => ("Orders", &["b_order"], Act::OpenOrders, ready),
        SlotCommand::Harvest if caps.mine => ("Harvest", &["b_harvest"], Act::Target(TargetMode::Mine), ready),
        SlotCommand::Recrew if caps.recrew => {
            ("Recrew", &["b_recrew"], Act::Recrew, if caps.recrew_ready { ready } else { Look::Locked })
        }
        SlotCommand::Build if builder.is_some() => ("Build", &["b_build"], Act::OpenBuild, ready),
        SlotCommand::RallyPoint if caps.rally => ("Rally point", &["b_rally"], Act::Target(TargetMode::Rally), ready),
        // The sim refuses or has no player order for these yet: shown, disabled.
        SlotCommand::Repair if mobile => ("Repair", &["b_repair"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::StandDown if mobile => ("Stand down", &["b_standdown"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::GreenAlert if mobile => ("Green alert", &["b_galert"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::YellowAlert if mobile => ("Yellow alert", &["b_yalert"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::RedAlert if mobile => ("Red alert", &["b_ralert"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::Patrol if mobile => ("Patrol", &["b_patrol"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::Guard if mobile => ("Guard", &["b_guard"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::Decommission if mobile => ("Decommission", &["b_decom"], Act::Target(TargetMode::Decommission), Look::Ready),
        SlotCommand::SelfDestruct if mobile => ("Self destruct", &["b_gselfdes"], Act::Order(UnitOrder::Unavailable), Look::Locked),
        SlotCommand::Scout if mobile => ("Scout", &["b_scout"], Act::Order(UnitOrder::Scout), ready),
        SlotCommand::SearchAndDestroy if caps.attack && mobile => {
            ("Search and destroy", &["b_sandd"], Act::Order(UnitOrder::SearchAndDestroy), ready)
        }
        _ => return None,
    })
}

/// The buttons of a page, one per slot (cell = slot - 1). A locked button carries
/// `UnitOrder::Unavailable`, which `hud::command` never turns into an order.
pub(in crate::ui::hud) fn buttons(feed: &HudFeed, page: Page, slots: &ButtonSlots, strings: Option<&Strings>) -> Vec<Button> {
    if feed.own_selection().is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Button> = Vec::new();
    let mut put = |b: Button| {
        if !out.iter().any(|o| o.cell == b.cell) {
            out.push(b);
        }
    };
    match page {
        Page::Build => {
            let cancel = slots.slot(SlotCommand::Cancel);
            let back = slots.slot(SlotCommand::Back);
            let free = (1..=SLOTS).filter(|s| Some(*s) != cancel && Some(*s) != back);
            // A build item sits in the free slot its build cell counts (`BuildItem::slot`): an item a
            // text tree leaves out leaves its slot empty and the others keep theirs.
            let items = model::build_buttons_by_slot(feed);
            for (cell, mut item) in items {
                let Some(slot) = free.clone().nth(cell) else { continue };
                item.cell = usize::from(slot - 1);
                annotate(&mut item, feed, strings);
                put(item);
            }
            // TODO(UI-19): a build list longer than the free slots needs paging; the original's
            // rule is not measured.
            if let (Some(slot), Some(builder)) = (cancel, model::builder(feed)) {
                put(button(slot, &tr("Cancel"), Act::Cancel(builder.id), Look::Ready, &["b_cancel"]));
            }
            if let Some(slot) = back {
                put(button(slot, &tr("Back"), Act::Back, Look::Ready, &["b_back"]));
            }
        }
        Page::Orders => {
            for (slot, c) in slots.page(true) {
                if let Some((label, icons, act, look)) = command(c, feed) {
                    put(button(slot, &tr(label), act, look, icons));
                }
            }
            if let Some(slot) = slots.slot(SlotCommand::Back) {
                put(button(slot, &tr("Back"), Act::Back, Look::Ready, &["b_back"]));
            }
        }
        _ => {
            for (slot, c) in slots.page(false) {
                if let Some((label, icons, act, look)) = command(c, feed) {
                    put(button(slot, &tr(label), act, look, icons));
                }
            }
        }
    }
    out.sort_by_key(|b| b.cell);
    out
}

/// A build or research button's tooltips with its `needs:` line (UI-42): the short one reads
/// `Name (needs: ...)` (the game's own bracket words), the verbose one gets the line on its own
/// row. Content only; the tooltip widget and its placement are unchanged.
fn annotate(button: &mut Button, feed: &HudFeed, strings: Option<&Strings>) {
    let Act::Build { what, .. } = &button.act else { return };
    let Some(db) = feed.content.as_deref() else { return };
    let Some(needs) = costs::needs(db, *what) else { return };
    let line = costs::line(strings, &model::resource_model(feed), &needs, costs::shortfall(feed, *what));
    if line.is_empty() {
        return;
    }
    // The game's hotkey marker (`^Build`) is not shown in this composed text.
    let resolve = |t: &str| strings.map_or(t, |s| s.tooltip_text(t)).replace('^', "");
    let (open, close) = match strings {
        Some(s) => (s.tooltip_text("GUI_CP_START_EXTRA").to_owned(), s.tooltip_text("GUI_CP_END_EXTRA").to_owned()),
        None => ("(".into(), ")".into()),
    };
    let (open, close) = if open == "GUI_CP_START_EXTRA" || close == "GUI_CP_END_EXTRA" { ("(".into(), ")".into()) } else { (open, close) };
    button.tip = format!("{} {open}{line}{close}", resolve(&button.tip));
    button.verbose = Some(match button.verbose.as_deref() {
        Some(v) => format!("{}\n{line}", resolve(v)),
        None => line,
    });
}

/// Spawn the grid's buttons into `parent`, one per slot rectangle the sheet has.
pub(in crate::ui::hud) fn spawn_grid(
    commands: &mut Commands,
    parent: Entity,
    theme: &Theme,
    layout: &StockLayout,
    buttons: &[Button],
    state: &StockState,
    hud: &HudState,
) -> Vec<R> {
    let rects = cells(layout);
    let mut solid = Vec::new();
    for b in buttons {
        let rect = if b.cell >= speed::RAIL_BASE {
            // A speed rail position: the stock sheet places it, and its separator goes with it.
            let Some(rect) = speed::rect(layout, b.cell - speed::RAIL_BASE) else { continue };
            if let Some((r, name)) = speed::separator(layout) {
                commands.spawn((super::node(r), bevy::picking::Pickable::IGNORE, GuiImage::new(name), ChildOf(parent)));
            }
            Some(rect)
        } else {
            rects.get(b.cell).copied().flatten()
        };
        let Some(rect) = rect else { continue };
        solid.push(rect);
        super::super::spawn_button(commands, parent, theme, rect, b, &state.native, hud);
    }
    solid
}

/// Orders the page back to the top when its selection is gone or cannot take orders.
pub(in crate::ui::hud) fn settle_page(feed: &HudFeed, page: Page) -> Page {
    match page {
        Page::Orders if !commands::shared(feed).move_order => Page::Top,
        Page::Build if !commands::shared(feed).build || model::builder(feed).is_none() => Page::Top,
        Page::Formations | Page::Native(_) => Page::Top,
        other => other,
    }
}
