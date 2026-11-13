//! The stock-layout screens: each control stands in the region the stock screen
//! keeps it in ([`layout`](super::layout)), in the theme's own widgets (UI-21).
//! Texts come from the mounted `label.map` where it has a key for them, the
//! translated English otherwise.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::math::{Rect, Vec2};
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use sta_content::StartingResources;

use super::layout::{self, Slot, region};
use crate::app::AppState;
use crate::frontend::screens::data::SeatControl;
use crate::frontend::screens::view::{Ctx, ScreenRoot, SettingSlider, SettingToggle};
use crate::frontend::screens::{Act, Col, Screen, SettingKey};
use crate::ui::i18n::{tr, trf};
use crate::ui::theme::{disabled_button, dropdown, kit_button, kit_text, kit_toggle, label, slider_focusable, tab_bar, tab_panel};

/// A control placed by the layout: which slot, so tests and systems find it.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed(pub Slot);

/// A seat's dropdown: which row and column of the setup table it is.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatDrop {
    pub row: usize,
    pub col: Col,
}

/// The box a screen's regions are fractions of.
#[derive(Component, Debug, Clone, Copy)]
pub struct StockBoxNode;

/// The races a seat's side dropdown lists, in the install's skirmish order.
pub fn race_ids(ctx: &Ctx) -> Vec<String> {
    ctx.data.races()
}

/// A caption: the mounted `label.map`'s text for `key`, else the translated `fallback`.
pub fn cap(ctx: &Ctx, key: &str, fallback: &str) -> String {
    ctx.strings.and_then(|s| s.native_label(key)).filter(|t| !t.trim().is_empty()).map_or_else(|| tr(fallback), str::to_owned)
}

pub fn spawn(commands: &mut Commands, screen: Screen, ctx: &Ctx, camera: Option<Entity>) {
    let mut root = commands.spawn((
        ScreenRoot(screen),
        DespawnOnExit(AppState::FrontEnd),
        // Clear: the menu scene (UI-06) shows through.
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
    ));
    if let Some(c) = camera {
        root.insert(UiTargetCamera(c));
    }
    let b = ctx.stock_box;
    root.with_children(|p| {
        p.spawn((StockBoxNode, Node { width: Val::Px(b.x), height: Val::Px(b.y), ..default() })).with_children(|p| match screen {
            Screen::Main => main_menu(p, ctx, b),
            Screen::SinglePlayer => single_player(p, ctx, b),
            Screen::Setup => setup(p, ctx, b),
            Screen::Settings => settings(p, ctx, b),
            _ => {}
        });
    });
}

type Box_<'a> = RelatedSpawnerCommands<'a, ChildOf>;

/// `content` in the rectangle `r` (logical px in the box), centred vertically in it.
fn put(p: &mut Box_, slot: Slot, r: Rect, content: impl Bundle) {
    p.spawn((
        Placed(slot),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r.min.x),
            top: Val::Px(r.min.y),
            width: Val::Px(r.width()),
            height: Val::Px(r.height()),
            align_items: AlignItems::Center,
            ..default()
        },
        children![content],
    ));
}

/// A kit button of the region's width, with its `act`.
fn act_button(p: &mut Box_, ctx: &Ctx, b: Vec2, slot: Slot, text: &str, act: Act) {
    let r = region(slot).px(b);
    put(p, slot, r, (kit_button(ctx.theme, text, r.width()), act));
}

fn off_button(p: &mut Box_, ctx: &Ctx, b: Vec2, slot: Slot, text: &str) {
    let r = region(slot).px(b);
    put(p, slot, r, disabled_button(ctx.theme, text, r.width()));
}

fn main_menu(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    act_button(p, ctx, b, Slot::MainSinglePlayer, &cap(ctx, "single_player", "Single player"), Act::Goto(Screen::SinglePlayer));
    act_button(p, ctx, b, Slot::MainInstantAction, &cap(ctx, "instant_action", "Instant action"), Act::Goto(Screen::Setup));
    // The lobby (UI-11) opens this.
    off_button(p, ctx, b, Slot::MainMultiplayer, &cap(ctx, "multiplayer", "Multiplayer"));
    act_button(p, ctx, b, Slot::MainOptions, &cap(ctx, "options", "Options"), Act::Goto(Screen::Settings));
    // The intro (MED-05) and the credits have no screen yet.
    off_button(p, ctx, b, Slot::MainReplayIntro, &cap(ctx, "replay_intro", "Replay intro"));
    off_button(p, ctx, b, Slot::MainCredits, &cap(ctx, "credits", "Credits"));
    act_button(p, ctx, b, Slot::MainExit, &cap(ctx, "quit", "Exit"), Act::Exit);
    act_button(p, ctx, b, Slot::MainMods, &tr("Mods"), Act::PickMod);
}

fn single_player(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    // The cards stand where the stock panels do, in the stock order of the label keys. With the
    // installed campaigns (UI-25) each is the shell's slot: it opens its mission list, or stays off
    // with the locked text while the campaign before it is not complete.
    let cards = [("tutorials", "Tutorials"), ("federation_campaign", "Federation campaign"), ("klingon_campaign", "Klingon campaign"), ("borg_campaign", "Borg campaign")];
    let n = ctx.campaigns.map_or(cards.len(), |c| c.book.campaigns.len().max(1));
    for i in 0..n {
        let r = crate::frontend::campaign::layout::card_region(i, n).px(b);
        match crate::frontend::campaign::card(ctx.campaigns, i) {
            Some((text, true)) => put(p, Slot::Card(i), r, (kit_button(ctx.theme, &text, r.width()), crate::frontend::campaign::CampaignAct::Card(i))),
            Some((text, false)) => put(p, Slot::Card(i), r, disabled_button(ctx.theme, &text, r.width())),
            None => {
                let (key, fallback) = cards[i.min(cards.len() - 1)];
                put(p, Slot::Card(i), r, disabled_button(ctx.theme, &cap(ctx, key, fallback), r.width()));
            }
        }
    }
    off_button(p, ctx, b, Slot::SpLoad, &cap(ctx, "load_game", "Load game"));
    act_button(p, ctx, b, Slot::SpBack, &cap(ctx, "previous_menu", "Previous menu"), Act::Goto(Screen::Main));
}

fn setup(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    let t = ctx.theme;
    let draft = ctx.draft;
    let races = race_ids(ctx);
    let race_labels: Vec<String> = races.iter().map(|r| ctx.data.race_label(r, ctx.strings)).collect();
    let race_refs: Vec<&str> = race_labels.iter().map(String::as_str).collect();
    let controls = [SeatControl::Human, SeatControl::Computer, SeatControl::Idle].map(|c| tr(c.label()));
    let control_refs: Vec<&str> = controls.iter().map(String::as_str).collect();
    let teams: Vec<String> = (0..draft.seats.len().max(1)).map(|i| cap(ctx, &format!("team_{}", i + 1), &trf("Team {0}", &[&(i + 1)]))).collect();
    let team_refs: Vec<&str> = teams.iter().map(String::as_str).collect();

    let title = region(Slot::SetupTitle).px(b);
    put(p, Slot::SetupTitle, title, kit_text(t, t.caps(&cap(ctx, "instant_action", "Instant action"), t.caps_titles), t.title_size, t.title));
    for (slot, text) in [(Slot::HeadControl, tr("Player")), (Slot::HeadSide, tr("Side")), (Slot::HeadTeam, tr("Team"))] {
        put(p, slot, region(slot).px(b), label(t, &text));
    }
    for (i, seat) in draft.seats.iter().enumerate().take(layout::TABLE_ROWS) {
        let at = |slot: Slot| region(slot).px(b);
        let control = [SeatControl::Human, SeatControl::Computer, SeatControl::Idle].iter().position(|c| *c == seat.control).unwrap_or(0);
        let r = at(Slot::SeatControl(i));
        put(p, Slot::SeatControl(i), r, (dropdown(t, &control_refs, control, r.width()), SeatDrop { row: i, col: Col::Control }));
        let side = races.iter().position(|id| id.eq_ignore_ascii_case(&seat.race)).unwrap_or(0);
        let r = at(Slot::SeatSide(i));
        put(p, Slot::SeatSide(i), r, (dropdown(t, &race_refs, side, r.width()), SeatDrop { row: i, col: Col::Race }));
        let r = at(Slot::SeatTeam(i));
        put(p, Slot::SeatTeam(i), r, (dropdown(t, &team_refs, usize::from(seat.team), r.width()), SeatDrop { row: i, col: Col::Team }));
    }

    // The map window: its labels above the preview, the preview, the settings beside it.
    let map = ctx.data.maps.get(draft.map);
    let info = region(Slot::MapInfo).px(b);
    put(
        p,
        Slot::MapInfo,
        info,
        match map {
            Some(m) => label(t, &trf("Map: {0}  ({1} players)", &[&m.title(), &m.slots()])),
            None => label(t, &tr("No maps found.")),
        },
    );
    let pv = region(Slot::Preview).px(b);
    p.spawn((
        Placed(Slot::Preview),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(pv.min.x),
            top: Val::Px(pv.min.y),
            width: Val::Px(pv.width()),
            height: Val::Px(pv.height()),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(t.track),
        BorderColor::all(t.accent2),
    ))
    .with_children(|c| {
        for (i, (fx, fy)) in preview_dots(ctx).into_iter().enumerate() {
            c.spawn((
                Node { position_type: PositionType::Absolute, left: Val::Percent(fx * 100.0), top: Val::Percent(fy * 100.0), ..default() },
                children![kit_text(t, (i + 1).to_string(), t.small_size, t.accent)],
            ));
        }
    });
    if map.is_some() {
        act_button(p, ctx, b, Slot::SelectMap, &cap(ctx, "select_map", "Select map"), Act::PickMap);
    } else {
        off_button(p, ctx, b, Slot::SelectMap, &cap(ctx, "select_map", "Select map"));
    }
    let (key, fallback) = match draft.options.starting_resources {
        StartingResources::Normal => ("sr_normal", "Resources: Normal"),
        StartingResources::Lots => ("sr_lots", "Resources: Lots"),
    };
    act_button(p, ctx, b, Slot::Resources, &cap(ctx, key, fallback), Act::CycleResources);
    let officer = ctx.data.content.rules().officer_limits.get(draft.options.officer_limit as usize).map(|choice| {
        let caption = ctx
            .strings
            .and_then(|s| s.native_label(&choice.key))
            .filter(|c| !c.trim().is_empty())
            .or_else(|| (!choice.key.trim().is_empty()).then_some(choice.key.as_str()))
            .map_or_else(|| tr("Officer limit unavailable"), str::to_owned);
        format!("{caption}: {}", choice.value)
    });
    match officer {
        Some(caption) => act_button(p, ctx, b, Slot::Officers, &caption, Act::CycleOfficerLimit),
        None => off_button(p, ctx, b, Slot::Officers, &tr("Officer limit unavailable")),
    }

    act_button(p, ctx, b, Slot::SetupBack, &cap(ctx, "previous_menu", "Previous menu"), Act::Goto(Screen::Main));
    if draft.to_setup(ctx.data).is_some() {
        act_button(p, ctx, b, Slot::Launch, &cap(ctx, "launch", "Launch"), Act::Launch);
    } else {
        off_button(p, ctx, b, Slot::Launch, &cap(ctx, "launch", "Launch"));
    }
}

/// The picked map's start positions as fractions of the preview frame.
pub fn preview_dots(ctx: &Ctx) -> Vec<(f32, f32)> {
    let Some(m) = ctx.data.maps.get(ctx.draft.map) else { return Vec::new() };
    let f = |v: sta_content::Fixed| v.to_f32();
    let (lo, hi) = (&m.def.bounds.0, &m.def.bounds.1);
    let mut starts: Vec<_> = m.def.starts.iter().collect();
    starts.sort_by_key(|s| s.slot);
    starts
        .iter()
        .take(layout::TABLE_ROWS)
        .map(|s| layout::preview_point((f(s.pos.x), f(s.pos.z)), (f(lo.x), f(lo.z)), (f(hi.x), f(hi.z))))
        .collect()
}

fn settings(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    let t = ctx.theme;
    let title = region(Slot::OptionsTitle).px(b);
    put(p, Slot::OptionsTitle, title, kit_text(t, t.caps(&cap(ctx, "options", "Options"), t.caps_titles), t.title_size, t.title));
    let body = region(Slot::OptionsBody).px(b);
    let s = ctx.settings.clone();
    let theme = t.clone();
    let tabs = [tr("Sound"), tr("Game"), tr("Graphics")];
    let tab = ctx.options_tab.min(2);
    p.spawn((
        Placed(Slot::OptionsBody),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(body.min.x),
            top: Val::Px(body.min.y),
            width: Val::Px(body.width()),
            height: Val::Px(body.height()),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(t.gap),
            ..default()
        },
    ))
    .with_children(move |c| {
            let refs: Vec<&str> = tabs.iter().map(String::as_str).collect();
            let shown = |i: usize| if i == tab { Display::Flex } else { Display::None };
            let bar = c.spawn(tab_bar(&theme, &refs, tab)).id();
            let slider_row = |c: &mut RelatedSpawnerCommands<ChildOf>, name: &str, key: SettingKey| {
                let w = body.width() * 0.5;
                c.spawn(Node { column_gap: Val::Px(theme.gap), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                    r.spawn((Node { width: Val::Px(body.width() * 0.35), ..default() }, children![label(&theme, name)]));
                    r.spawn((slider_focusable(&theme, w, 0.0, 100.0, s.number(key).unwrap_or(0.0), 5.0), SettingSlider(key)));
                });
            };
            c.spawn(tab_panel(bar, 0)).insert(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(theme.gap), display: shown(0), ..default() }).with_children(|c| {
                slider_row(c, &tr("Master volume"), SettingKey::MasterVolume);
                slider_row(c, &tr("Music"), SettingKey::MusicVolume);
                slider_row(c, &tr("Effects"), SettingKey::EffectsVolume);
                slider_row(c, &tr("Voices"), SettingKey::VoiceVolume);
            });
            c.spawn(tab_panel(bar, 1)).insert(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(theme.gap), display: shown(1), ..default() }).with_children(|c| {
                slider_row(c, &tr("Scroll speed"), SettingKey::ScrollSpeed);
            });
            c.spawn(tab_panel(bar, 2)).insert(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(theme.gap), display: shown(2), ..default() }).with_children(|c| {
                for (name, key, on) in [
                    (tr("Fullscreen"), SettingKey::Fullscreen, s.fullscreen),
                    (tr("Vertical sync"), SettingKey::Vsync, s.vsync),
                    (tr("Show FPS"), SettingKey::ShowFps, s.show_fps),
                    (tr("LCARS interface"), SettingKey::LcarsUi, s.lcars_ui),
                ] {
                    c.spawn((kit_toggle(&theme, &name, on, 320.0), SettingToggle(key)));
                }
                c.spawn(label(&theme, &tr("Off uses the game/mod default.")));
            });
    });
    act_button(p, ctx, b, Slot::OptionsBack, &cap(ctx, "previous_menu", "Previous menu"), Act::Goto(Screen::Main));
}
