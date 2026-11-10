//! The screens' layouts, built from the UI-03 kit. Each screen is one LCARS
//! panel over the menu backdrop; the controls carry an [`Act`] (what a click
//! does) or a marker the systems in `mod.rs` read.

use bevy::ecs::hierarchy::ChildSpawner;
use bevy::ecs::spawn::SpawnWith;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;

use super::data::{Browse, ScreenData, SelectedMod, SettingKey, SetupDraft, Settings};
use super::{Col, Screen};
use crate::app::AppState;
use crate::ui::i18n::{tr, trf};
use crate::ui::theme::{Theme, button, disabled_button, label, list, panel, slider, toggle};

/// A screen's root node: everything of the screen hangs under it.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRoot(pub Screen);

/// What a click on a button does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Goto(Screen),
    /// Open map select with the draft's map picked.
    PickMap,
    /// Open mod select with the chosen mod picked.
    PickMod,
    /// Map select's Select Map: take the picked map into the draft.
    TakeMap,
    /// Mod select's Use Mod.
    UseMod,
    Cycle(usize, Col),
    Launch,
    Exit,
}

/// The slider of a setting.
#[derive(Component, Debug, Clone, Copy)]
pub struct SettingSlider(pub SettingKey);

/// The toggle of a setting.
#[derive(Component, Debug, Clone, Copy)]
pub struct SettingToggle(pub SettingKey);

/// Map select's list.
#[derive(Component, Debug, Clone, Copy)]
pub struct MapList;

/// Mod select's list.
#[derive(Component, Debug, Clone, Copy)]
pub struct ModList;

/// What a screen is drawn from.
pub struct Ctx<'a> {
    pub theme: &'a Theme,
    pub data: &'a ScreenData,
    pub draft: &'a SetupDraft,
    pub browse: &'a Browse,
    pub settings: &'a Settings,
    pub selected: &'a SelectedMod,
}

/// Spawn `screen`; `camera` is the UI camera when the host has one of its own.
pub fn spawn(commands: &mut Commands, screen: Screen, ctx: &Ctx, camera: Option<Entity>) {
    match screen {
        Screen::Main => frame(commands, ctx, screen, camera, &tr("Star Trek Armada"), 520.0, main_menu(ctx)),
        Screen::SinglePlayer => frame(commands, ctx, screen, camera, &tr("Single player"), 620.0, single_player(ctx)),
        Screen::Setup => frame(commands, ctx, screen, camera, &tr("Instant action"), 960.0, setup(ctx)),
        Screen::MapSelect => frame(commands, ctx, screen, camera, &tr("Select map"), 760.0, map_select(ctx)),
        Screen::Settings => frame(commands, ctx, screen, camera, &tr("Options"), 760.0, settings(ctx)),
        Screen::ModSelect => frame(commands, ctx, screen, camera, &tr("Mods"), 900.0, mod_select(ctx)),
    }
}

fn frame(commands: &mut Commands, ctx: &Ctx, screen: Screen, camera: Option<Entity>, title: &str, width: f32, content: impl Bundle) {
    let mut root = commands.spawn((
        ScreenRoot(screen),
        DespawnOnExit(AppState::FrontEnd),
        // Clear: the menu scene (UI-06) shows through.
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        children![(Node { width: Val::Px(width), flex_direction: FlexDirection::Column, ..default() }, children![panel(ctx.theme, title, content)])],
    ));
    if let Some(c) = camera {
        root.insert(UiTargetCamera(c));
    }
}

fn column(gap: f32) -> Node {
    Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(gap), ..default() }
}

fn row(gap: f32) -> Node {
    Node { column_gap: Val::Px(gap), align_items: AlignItems::Center, ..default() }
}

/// A label in a fixed-width cell.
fn cell(theme: &Theme, w: f32, s: &str) -> impl Bundle {
    (Node { width: Val::Px(w), ..default() }, children![label(theme, s)])
}

/// A column of content: `fill` puts the rows in.
fn content(theme: &Theme, fill: impl FnOnce(&mut ChildSpawner) + Send + Sync + 'static) -> impl Bundle {
    (column(theme.gap), Children::spawn(SpawnWith(fill)))
}

fn main_menu(ctx: &Ctx) -> impl Bundle {
    let t = ctx.theme.clone();
    let w = 440.0;
    content(ctx.theme, move |p| {
        p.spawn((button(&t, &tr("Single player"), w), Act::Goto(Screen::SinglePlayer)));
        p.spawn((button(&t, &tr("Instant action"), w), Act::Goto(Screen::Setup)));
        // The lobby (UI-11) opens this.
        p.spawn(disabled_button(&t, &tr("Multiplayer"), w));
        p.spawn((button(&t, &tr("Mods"), w), Act::PickMod));
        p.spawn((button(&t, &tr("Options"), w), Act::Goto(Screen::Settings)));
        p.spawn((button(&t, &tr("Exit"), w), Act::Exit));
    })
}

fn single_player(ctx: &Ctx) -> impl Bundle {
    let t = ctx.theme.clone();
    let w = 520.0;
    content(ctx.theme, move |p| {
        // The campaigns need the mission scripts (LUA lane); the cards stand
        // where Refit's `single_player.rs` has them.
        for name in ["Tutorials", "Federation campaign", "Klingon campaign", "Borg campaign"] {
            p.spawn(disabled_button(&t, &tr(name), w));
        }
        p.spawn(disabled_button(&t, &tr("Load game"), w));
        p.spawn((button(&t, &tr("Previous menu"), w), Act::Goto(Screen::Main)));
    })
}

fn setup(ctx: &Ctx) -> impl Bundle {
    let t = ctx.theme.clone();
    let draft = ctx.draft.clone();
    let map = ctx.data.maps.get(draft.map).map(|m| (m.title().to_owned(), m.slots()));
    content(ctx.theme, move |p| {
        p.spawn((
            row(t.gap),
            children![cell(&t, 130.0, &tr("Seat")), cell(&t, 200.0, &tr("Control")), cell(&t, 260.0, &tr("Side")), cell(&t, 160.0, &tr("Team"))],
        ));
        for (i, seat) in draft.seats.iter().enumerate() {
            p.spawn((
                row(t.gap),
                children![
                    cell(&t, 130.0, &trf("Player {0}", &[&(i + 1)])),
                    (button(&t, seat.control.label(), 200.0 - t.gap), Act::Cycle(i, Col::Control)),
                    (button(&t, if seat.race.is_empty() { "none" } else { &seat.race }, 260.0 - t.gap), Act::Cycle(i, Col::Race)),
                    (button(&t, &trf("Team {0}", &[&(seat.team + 1)]), 160.0 - t.gap), Act::Cycle(i, Col::Team)),
                ],
            ));
        }
        match &map {
            Some((title, slots)) => {
                p.spawn((row(t.gap), children![label(&t, &trf("Map: {0}  ({1} players)", &[&title, &slots])), (button(&t, &tr("Select map"), 220.0), Act::PickMap)]));
            }
            None => {
                p.spawn(label(&t, &tr("No maps found.")));
            }
        }
        p.spawn(row(t.gap)).with_children(|r| {
            if map.is_some() {
                r.spawn((button(&t, &tr("Launch"), 220.0), Act::Launch));
            } else {
                r.spawn(disabled_button(&t, &tr("Launch"), 220.0));
            }
            r.spawn((button(&t, &tr("Previous menu"), 220.0), Act::Goto(Screen::Main)));
        });
    })
}

fn map_select(ctx: &Ctx) -> impl Bundle {
    let t = ctx.theme.clone();
    let titles: Vec<String> = ctx.data.maps.iter().map(|m| format!("{}  ({} players)", m.title(), m.slots())).collect();
    let picked = (!titles.is_empty()).then_some(ctx.browse.map.min(titles.len() - 1));
    content(ctx.theme, move |p| {
        let rows: Vec<&str> = titles.iter().map(String::as_str).collect();
        p.spawn((list(&t, &rows, picked), MapList));
        p.spawn(row(t.gap)).with_children(|r| {
            if picked.is_some() {
                r.spawn((button(&t, &tr("Select map"), 220.0), Act::TakeMap));
            } else {
                r.spawn(disabled_button(&t, &tr("Select map"), 220.0));
            }
            r.spawn((button(&t, &tr("Cancel"), 220.0), Act::Goto(Screen::Setup)));
        });
    })
}

fn settings(ctx: &Ctx) -> impl Bundle {
    let t = ctx.theme.clone();
    let s = ctx.settings.clone();
    content(ctx.theme, move |p| {
        p.spawn(label(&t, &tr("Sound")));
        for (name, key) in [
            ("Master volume", SettingKey::MasterVolume),
            ("Music", SettingKey::MusicVolume),
            ("Effects", SettingKey::EffectsVolume),
            ("Voices", SettingKey::VoiceVolume),
        ] {
            p.spawn((row(t.gap), children![cell(&t, 240.0, &tr(name)), (slider(&t, 360.0, 0.0, 100.0, s.number(key).unwrap_or(0.0)), SettingSlider(key))]));
        }
        p.spawn(label(&t, &tr("Game")));
        p.spawn((row(t.gap), children![cell(&t, 240.0, &tr("Scroll speed")), (slider(&t, 360.0, 0.0, 100.0, s.scroll_speed), SettingSlider(SettingKey::ScrollSpeed))]));
        p.spawn(label(&t, &tr("Graphics")));
        p.spawn((
            row(t.gap),
            children![
                (toggle(&t, &tr("Fullscreen"), s.fullscreen, 220.0), SettingToggle(SettingKey::Fullscreen)),
                (toggle(&t, &tr("Vertical sync"), s.vsync, 220.0), SettingToggle(SettingKey::Vsync)),
                (toggle(&t, &tr("Show FPS"), s.show_fps, 220.0), SettingToggle(SettingKey::ShowFps)),
            ],
        ));
        p.spawn((button(&t, &tr("Previous menu"), 220.0), Act::Goto(Screen::Main)));
    })
}

fn mod_select(ctx: &Ctx) -> impl Bundle {
    let t = ctx.theme.clone();
    let mut names = vec![tr("Base game")];
    names.extend(ctx.data.mods.iter().map(|m| m.name.clone()));
    let picked = ctx.browse.mod_row.min(names.len() - 1);
    let mut details = Vec::new();
    match picked.checked_sub(1).and_then(|i| ctx.data.mods.get(i)) {
        None => details.push(tr("The game as installed, without a mod.")),
        Some(m) => {
            details.push(format!("{}{}", m.name, m.version.as_deref().map(|v| format!("  v{v}")).unwrap_or_default()));
            if !m.authors.is_empty() {
                details.push(trf("By {0}", &[&m.authors.join(", ")]));
            }
            if let Some(parent) = &m.parent {
                details.push(trf("Needs {0}", &[parent]));
            }
            if !m.description.is_empty() {
                details.push(m.description.clone());
            }
        }
    }
    let current = match &ctx.selected.0 {
        Some(f) => trf("In use: {0}", &[f]),
        None => tr("In use: base game"),
    };
    content(ctx.theme, move |p| {
        let rows: Vec<&str> = names.iter().map(String::as_str).collect();
        p.spawn((list(&t, &rows, Some(picked)), ModList));
        for d in &details {
            p.spawn(label(&t, d));
        }
        p.spawn(label(&t, &current));
        p.spawn((row(t.gap), children![(button(&t, &tr("Use mod"), 220.0), Act::UseMod), (button(&t, &tr("Previous menu"), 220.0), Act::Goto(Screen::Main))]));
    })
}
