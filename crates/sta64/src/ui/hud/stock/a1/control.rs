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
//! the stock slot positions: Repair, Stand Down, the alert states, Patrol and Guard
//! (their sim orders or the HUD's target pick are later work; TODO lines in the
//! PR). A station shows none of the Orders-page commands, locked or not: which of them the stock
//! HUD shows for a station is not measured (TODO line). Self-Destruct uses a shared A2 immediate
//! special when the selected craft carry one; otherwise its old locked control remains.
//! Not drawn: Transport Special, Upgrade and
//! Research Upgrade (later). Transport has no control-grid slot where the install's command data gives it none; it
//! sits on the speed rail ([`super::speed`]), whose buttons the same grid carries (cells from
//! `speed::RAIL_BASE`).

use bevy::prelude::*;
use sta_content::{A2Effect, A2Target, ButtonSlots, SlotCommand, button_slots::SLOTS};
use sta_sim::ContentKey;
use sta_sim::specials::{SpecialKind, specials_indexed};

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

fn put(out: &mut Vec<Button>, button: Button) {
    if !out.iter().any(|existing| existing.cell == button.cell) { out.push(button); }
}

/// The common immediate self-destruct weapon, if every selected craft has it
/// at the same sim slot. Other immediate A2 weapons cannot own this control.
fn self_destruct_slot(feed: &HudFeed) -> Option<u8> {
    let db = feed.content.as_deref()?;
    commands::shared(feed).specials.into_iter().find_map(|(slot, kind)| {
        if kind != SpecialKind::A2(A2Target::Immediate) { return None; }
        feed.selected_units().into_iter().filter(|u| u.owner == feed.player).all(|u| {
            db.unit(u.def)
                .and_then(|def| specials_indexed(db, def).nth(slot as usize))
                .is_some_and(|(_, _, weapon)| weapon.a2_special.is_some_and(|rule| matches!(rule.effect, A2Effect::SelfDestruct(_))))
        }).then_some(slot)
    })
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
        SlotCommand::SelfDestruct if mobile => {
            let (act, look) = if let (Some(slot), Some(db)) = (self_destruct_slot(feed), feed.content.as_deref()) {
                let ready = feed.own_selection().into_iter().any(|id|
                    sta_sim::specials::a2_special_ready(&feed.view, db, id, slot));
                (Act::Special(slot), if ready { Look::Ready } else { Look::Locked })
            } else {
                (Act::Order(UnitOrder::Unavailable), Look::Locked)
            };
            ("Self destruct", &["b_gselfdes"], act, look)
        }
        SlotCommand::Scout if mobile => ("Scout", &["b_scout"], Act::Order(UnitOrder::Scout), ready),
        SlotCommand::SearchAndDestroy if caps.attack && mobile => {
            ("Search and destroy", &["b_sandd"], Act::Order(UnitOrder::SearchAndDestroy), ready)
        }
        _ => return None,
    })
}

/// The selected A1 starbase's officer quarter, when its own queue view can admit another.
/// The ordinary build-list membership remains the command path's authority.
fn officer_upgrade_key(feed: &HudFeed) -> Option<ContentKey> {
    let db = feed.content.as_deref()?;
    if !db.a1_officers() { return None; }
    let base = model::builder(feed)?;
    let policy = db.unit(base.def)?.a1_base_officers.as_ref()?;
    let key = policy.upgrade?;
    if db.unit(key)?.a1_officer_upgrade_gain.is_none()
        || !db.unit(base.def)?.build.list.iter().any(|item| item.key == key)
    {
        return None;
    }
    Some(key)
}

fn officer_upgrade(feed: &HudFeed) -> Option<(ContentKey, u32)> {
    let db = feed.content.as_deref()?;
    let base = model::builder(feed)?;
    let key = officer_upgrade_key(feed)?;
    let policy = db.unit(base.def)?.a1_base_officers.as_ref()?;
    let state = feed.production.iter().find(|row| row.builder == base.id)?.officer_upgrades.as_ref()?;
    (state.completed < policy.maximum_upgrades).then_some((key, state.remaining))
}

/// The buttons of a page, one per slot (cell = slot - 1). A locked button carries
/// `UnitOrder::Unavailable`, which `hud::command` never turns into an order.
pub(in crate::ui::hud) fn buttons(feed: &HudFeed, page: Page, slots: &ButtonSlots, strings: Option<&Strings>) -> Vec<Button> {
    if feed.own_selection().is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Button> = Vec::new();
    match page {
        Page::Build => {
            let cancel = slots.slot(SlotCommand::Cancel);
            let back = slots.slot(SlotCommand::Back);
            let free = (1..=SLOTS).filter(|s| Some(*s) != cancel && Some(*s) != back);
            // A build item sits in the free slot its build cell counts (`BuildItem::slot`): an item a
            // text tree leaves out leaves its slot empty and the others keep theirs.
            let upgrade = officer_upgrade_key(feed);
            let items = model::build_buttons_by_slot(feed);
            let skipped = items.iter().find(|(_, item)| matches!(item.act, Act::Build { what, .. } if Some(what) == upgrade)).map(|(slot, _)| *slot);
            for (cell, mut item) in items {
                if matches!(item.act, Act::Build { what, .. } if Some(what) == upgrade) { continue; }
                let cell = cell - usize::from(skipped.is_some_and(|slot| slot < cell));
                let Some(slot) = free.clone().nth(cell) else { continue };
                item.cell = usize::from(slot - 1);
                annotate(&mut item, feed, strings);
                put(&mut out, item);
            }
            // TODO(UI-19): a build list longer than the free slots needs paging; the original's
            // rule is not measured.
            if let (Some(slot), Some(builder)) = (cancel, model::builder(feed)) {
                put(&mut out, button(slot, &tr("Cancel"), Act::Cancel(builder.id), Look::Ready, &["b_cancel"]));
            }
            if let Some(slot) = back {
                put(&mut out, button(slot, &tr("Back"), Act::Back, Look::Ready, &["b_back"]));
            }
        }
        Page::Orders => {
            for (slot, c) in slots.page(true) {
                if let Some((label, icons, act, look)) = command(c, feed) {
                    put(&mut out, button(slot, &tr(label), act, look, icons));
                }
            }
            if let Some(slot) = slots.slot(SlotCommand::Back) {
                put(&mut out, button(slot, &tr("Back"), Act::Back, Look::Ready, &["b_back"]));
            }
        }
        _ => {
            for (slot, c) in slots.page(false) {
                if let Some((label, icons, act, look)) = command(c, feed) {
                    put(&mut out, button(slot, &tr(label), act, look, icons));
                }
            }
            // A1Compat's separate root command is the tenth logical cell. An
            // ordinary command in that cell takes the first free cell, as the
            // shared grid already does for an extra Evolve command.
            if page == Page::Top
                && let Some((key, remaining)) = officer_upgrade(feed)
                && let Some((_, mut item)) = model::build_buttons_by_slot(feed).into_iter()
                    .find(|(_, item)| matches!(item.act, Act::Build { what, .. } if what == key))
            {
                let occupant = out.iter().position(|b| b.cell == 9);
                let free = (0..usize::from(SLOTS)).find(|cell| !out.iter().any(|b| b.cell == *cell));
                if occupant.is_none() || free.is_some() {
                    if let (Some(index), Some(cell)) = (occupant, free) { out[index].cell = cell; }
                    item.cell = 9;
                    if let Act::Build { site, .. } = &mut item.act { *site = false; }
                    if remaining == 0 { item.look = Look::Locked; }
                    if costs::crew_shortfall(feed, key) && item.look == Look::Ready { item.look = Look::Poor; }
                    annotate(&mut item, feed, strings);
                    out.push(item);
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
    let mut line = costs::line(strings, &model::resource_model(feed), &needs, costs::shortfall(feed, *what));
    if costs::shortfall(feed, *what).is_none() && costs::crew_shortfall(feed, *what) {
        line.push_str(&format!(" ({})", crate::ui::i18n::trf("short of: {0}", &[&tr("crew")])));
    }
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
    let mut separator_drawn = false;
    for b in buttons {
        let rect = if b.cell >= speed::RAIL_BASE {
            // A speed rail position: the stock sheet places it, and its separator goes with it.
            let Some(rect) = speed::rect(layout, b.cell - speed::RAIL_BASE) else { continue };
            if !separator_drawn && let Some((r, name)) = speed::separator(layout) {
                commands.spawn((super::node(r), bevy::picking::Pickable::IGNORE, GuiImage::new(name), ChildOf(parent)));
                separator_drawn = true;
            }
            Some(rect)
        } else {
            rects.get(b.cell).copied().flatten()
        };
        let Some(rect) = rect else { continue };
        solid.push(rect);
        super::super::spawn_button(commands, parent, theme, rect, b, &state.native, hud, None);
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
