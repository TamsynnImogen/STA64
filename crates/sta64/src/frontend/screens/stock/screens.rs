//! The stock-layout screens: each control stands in the region the stock screen
//! keeps it in ([`layout`](super::layout)), in the theme's own widgets (UI-21).
//! Texts come from the mounted `label.map` where it has a key for them, the
//! translated English otherwise.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::math::{Rect, Vec2};
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use sta_content::StartingResources;
use sta_sim::PlayerId;

use super::layout::{self, Slot, region};
use crate::app::AppState;
use crate::frontend::screens::data::SeatControl;
use crate::frontend::screens::preview::AuthoredPreview;
use crate::frontend::screens::view::{Ctx, ScreenRoot, SettingSlider, SettingToggle};
use crate::frontend::screens::{Act, Col, Screen, SettingKey};
use crate::ui::i18n::{tr, trf};
use crate::ui::theme::{UiButton, WidgetState, disabled_button, dropdown, kit_button, kit_text, label, slider_focusable, tab_bar, tab_panel};

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
            Screen::Main => super::main_menu::spawn(p, ctx, b),
            Screen::SinglePlayer => single_player(p, ctx, b),
            Screen::Setup => setup(p, ctx, b),
            Screen::Advanced => advanced(p, ctx, b),
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

fn compact_button(p: &mut Box_, ctx: &Ctx, b: Vec2, slot: Slot, text: &str, act: Act) {
    let r = region(slot).px(b);
    let mut theme = ctx.theme.clone();
    theme.text_size = theme.micro_size;
    put(p, slot, r, (kit_button(&theme, text, r.width()), act));
}

fn off_button(p: &mut Box_, ctx: &Ctx, b: Vec2, slot: Slot, text: &str) {
    let r = region(slot).px(b);
    put(p, slot, r, disabled_button(ctx.theme, text, r.width()));
}

/// Dark framed regions keep the setup's table and map readable over the menu scene.
/// They are inert; interactive children are placed separately above them.
fn frame(p: &mut Box_, r: Rect, colour: Color, border: Color) {
    p.spawn((
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r.min.x), top: Val::Px(r.min.y),
            width: Val::Px(r.width()), height: Val::Px(r.height()),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(colour), BorderColor::all(border),
    ));
}

fn static_label(p: &mut Box_, r: Rect, theme: &crate::ui::theme::Theme, text: String) {
    p.spawn((
        Pickable::IGNORE,
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), align_items: AlignItems::Center, ..default() },
        children![kit_text(theme, text, theme.heading_size, theme.text)],
    ));
}

fn setup_frames(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    let k = b / layout::DESIGN;
    let scaled = |x: f32, y: f32, w: f32, h: f32| Rect::new(x * k.x, y * k.y, (x + w) * k.x, (y + h) * k.y);
    let body = Color::srgba(0.02, 0.025, 0.055, 0.76);
    let left_bottom = layout::setup_region(Slot::RandomColour, ctx.draft.seats.len()).y + 56.0;
    frame(p, scaled(24.0, 54.0, 430.0, left_bottom - 54.0), body, ctx.theme.accent2);
    frame(p, scaled(466.0, 54.0, 310.0, 412.0), body, ctx.theme.accent2);
    frame(p, scaled(24.0, 54.0, 430.0, 9.0), ctx.theme.accent2, ctx.theme.accent2);
    frame(p, scaled(466.0, 54.0, 310.0, 9.0), ctx.theme.accent3, ctx.theme.accent3);
}

fn map_preview(p: &mut Box_, ctx: &Ctx, b: Vec2, placed: bool) {
    let t = ctx.theme;
    let pv = layout::placed_rect(Slot::Preview, b);
    let mut preview = p.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(pv.min.x), top: Val::Px(pv.min.y),
            width: Val::Px(pv.width()), height: Val::Px(pv.height()),
            border: UiRect::all(Val::Px(1.0)), ..default()
        },
        BackgroundColor(t.track), BorderColor::all(t.accent2),
    ));
    if placed { preview.insert(Placed(Slot::Preview)); }
    preview.with_children(|c| {
        c.spawn((
            AuthoredPreview,
            if ctx.draft.preview_visible { Visibility::Inherited } else { Visibility::Hidden },
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        )).with_children(|c| {
            for (i, (fx, fy)) in preview_dots(ctx).into_iter().enumerate() {
                if ctx.draft.show_start_marker(i) {
                    c.spawn((
                        Node { position_type: PositionType::Absolute, left: Val::Percent(fx * 100.0), top: Val::Percent(fy * 100.0), ..default() },
                        children![kit_text(t, (i + 1).to_string(), t.small_size, t.accent)],
                    ));
                }
            }
        });
    });
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
    let load = region(Slot::SpLoad).px(b);
    put(p, Slot::SpLoad, load, (kit_button(ctx.theme, &cap(ctx, "load_game", "Load game"), load.width()), crate::ui::save_menu::OpenSaveButton(crate::ui::save_menu::SaveMode::Load)));
    act_button(p, ctx, b, Slot::SpBack, &cap(ctx, "previous_menu", "Previous menu"), Act::Goto(Screen::Main));
}

fn setup(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    setup_frames(p, ctx, b);
    let t = ctx.theme;
    let draft = ctx.draft;
    let races = race_ids(ctx);
    let race_labels: Vec<String> = std::iter::once(tr("Random")).chain(races.iter().map(|r| ctx.data.race_label(r, ctx.strings))).collect();
    let race_refs: Vec<&str> = race_labels.iter().map(String::as_str).collect();
    let control_choices = SeatControl::choices(draft.observing);
    let controls: Vec<String> = control_choices.iter().map(|c| c.label(ctx.strings)).collect();
    let control_refs: Vec<&str> = controls.iter().map(String::as_str).collect();
    let teams: Vec<String> = (0..draft.seats.len().max(1)).map(|i| cap(ctx, &format!("team_{}", i + 1), &trf("Team {0}", &[&(i + 1)]))).collect();
    let team_refs: Vec<&str> = teams.iter().map(String::as_str).collect();
    let colours = draft.effective_colours(ctx.data);
    let starts = ctx.data.maps.get(draft.map).map(|m| m.playable_starts()).unwrap_or_default();

    let title = region(Slot::SetupTitle).px(b);
    put(p, Slot::SetupTitle, title, kit_text(t, t.caps(&cap(ctx, "game_setup", "Game setup"), t.caps_titles), t.title_size * 1.15, t.title));
    for (slot, text) in [(Slot::HeadControl, tr("Player")), (Slot::HeadSide, tr("Side")), (Slot::HeadTeam, tr("Team"))] {
        put(p, slot, region(slot).px(b), label(t, &text));
    }
    for (i, seat) in draft.seats.iter().enumerate().take(layout::TABLE_ROWS) {
        let at = |slot: Slot| region(slot).px(b);
        if let Some(start) = starts.get(i) {
            let chip = layout::placed_rect(Slot::SeatColour(i), b);
            let colour = colours.owner_colour(PlayerId(start.slot)).unwrap_or(t.text_dim);
            put(p, Slot::SeatColour(i), chip, (
                Node { width: Val::Px(chip.width()), height: Val::Px(chip.height()), border: UiRect::all(Val::Px(1.0)), ..default() },
                BackgroundColor(colour), BorderColor::all(t.accent2),
            ));
        }
        let control = control_choices.iter().position(|c| *c == seat.control).unwrap_or(0);
        let r = at(Slot::SeatControl(i));
        put(p, Slot::SeatControl(i), r, (dropdown(t, &control_refs, control, r.width()), SeatDrop { row: i, col: Col::Control }));
        let side = races.iter().position(|id| id.eq_ignore_ascii_case(&seat.race)).map_or(0, |i| i + 1);
        let r = at(Slot::SeatSide(i));
        put(p, Slot::SeatSide(i), r, (dropdown(t, &race_refs, side, r.width()), SeatDrop { row: i, col: Col::Race }));
        let r = at(Slot::SeatTeam(i));
        if seat.control == SeatControl::Human || draft.observer_row() == Some(i) {
            let mut choices = team_refs.clone();
            let observer = tr("Observer");
            choices.push(observer.as_str());
            let selected = if draft.observing { choices.len() - 1 } else { usize::from(seat.team) };
            put(p, Slot::SeatTeam(i), r, (dropdown(t, &choices, selected, r.width()), SeatDrop { row: i, col: Col::Team }));
        } else {
            put(p, Slot::SeatTeam(i), r, (dropdown(t, &team_refs, usize::from(seat.team), r.width()), SeatDrop { row: i, col: Col::Team }));
        }
    }
    let random = layout::setup_region(Slot::RandomColour, draft.seats.len()).px(b);
    let random_label = cap(ctx, "random_color", "Random Color");
    let mut compact = t.clone();
    compact.text_size = compact.micro_size;
    if draft.colour_enabled() {
        put(p, Slot::RandomColour, random, (kit_button(&compact, &random_label, random.width()), Act::RandomTeamColour));
    } else {
        put(p, Slot::RandomColour, random, disabled_button(&compact, &random_label, random.width()));
    }
    let selected = draft.active_swatch(ctx.data);
    for (i, colour) in ctx.data.colours.swatches().into_iter().enumerate() {
        let r = layout::setup_rect(Slot::Palette(i), b, draft.seats.len());
        let edge = if selected == Some(i) { t.accent } else { t.accent2 };
        let mut chip = p.spawn((
            Name::new(trf("Player colour {0}", &[&(i + 1)])),
            Placed(Slot::Palette(i)),
            Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), border: UiRect::all(Val::Px(1.0)), ..default() },
            BorderColor::all(edge),
            children![(Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(colour), Pickable::IGNORE)],
        ));
        if draft.colour_enabled() { chip.insert((UiButton, WidgetState::Normal, Act::PickTeamColour(i))); }
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
    map_preview(p, ctx, b, true);
    if map.is_some() {
        compact_button(p, ctx, b, Slot::SelectMap, &cap(ctx, "select_map", "Select map"), Act::PickMap);
    } else {
        off_button(p, ctx, b, Slot::SelectMap, &cap(ctx, "select_map", "Select map"));
    }
    let visibility = if draft.preview_visible { "Minimap Shown" } else { "Minimap Hidden" };
    if map.is_some() {
        compact_button(p, ctx, b, Slot::PreviewVisibility, &tr(visibility), Act::TogglePreview);
        compact_button(p, ctx, b, Slot::StartMarkers, &tr(draft.start_markers.label()), Act::CycleStartMarkers);
    } else {
        off_button(p, ctx, b, Slot::PreviewVisibility, &tr(visibility));
        off_button(p, ctx, b, Slot::StartMarkers, &tr(draft.start_markers.label()));
    }
    let (key, fallback) = match draft.options.starting_resources {
        StartingResources::Normal => ("sr_normal", "Resources: Normal"),
        StartingResources::Lots => ("sr_lots", "Resources: Lots"),
    };
    compact_button(p, ctx, b, Slot::Resources, &cap(ctx, key, fallback), Act::CycleResources);
    compact_button(p, ctx, b, Slot::DefaultSettings, &tr("Default Settings"), Act::DefaultOptions);
    compact_button(p, ctx, b, Slot::SaveSettings, &tr("Save Settings"), Act::SaveSetup);
    compact_button(p, ctx, b, Slot::LoadSettings, &tr("Load Settings"), Act::LoadSetup);
    let placement = if draft.random_placement { "Random Placement" } else { "Fixed Placement" };
    if map.is_some() {
        compact_button(p, ctx, b, Slot::Fog, &tr(draft.fog.label()), Act::CycleFog);
        compact_button(p, ctx, b, Slot::RandomPlacement, &tr(placement), Act::ToggleRandomPlacement);
    } else {
        off_button(p, ctx, b, Slot::Fog, &tr(draft.fog.label()));
        off_button(p, ctx, b, Slot::RandomPlacement, &tr(placement));
    }
    act_button(p, ctx, b, Slot::Advanced, &tr("Advanced"), Act::Goto(Screen::Advanced));

    act_button(p, ctx, b, Slot::SetupBack, &cap(ctx, "previous_menu", "Previous menu"), Act::Goto(Screen::Main));
    if draft.to_setup(ctx.data).is_some() {
        act_button(p, ctx, b, Slot::Launch, &cap(ctx, "launch", "Launch"), Act::Launch);
    } else {
        off_button(p, ctx, b, Slot::Launch, &cap(ctx, "launch", "Launch"));
    }
}

fn advanced(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    // Static setup context behind the modal. It has no actions or focusable
    // dropdowns, so pointer and keyboard input cannot reach setup controls.
    setup_frames(p, ctx, b);
    let t = ctx.theme;
    map_preview(p, ctx, b, false);
    let k = b / layout::DESIGN;
    let scaled = |x: f32, y: f32, w: f32, h: f32| Rect::new(x * k.x, y * k.y, (x + w) * k.x, (y + h) * k.y);
    static_label(p, scaled(27.0, 8.0, 430.0, 40.0), t, tr("Game setup"));
    static_label(p, scaled(42.0, 75.0, 390.0, 35.0), t, tr("Players"));
    if let Some(map) = ctx.data.maps.get(ctx.draft.map) {
        static_label(p, scaled(483.0, 75.0, 280.0, 58.0), t, trf("Map: {0}  ({1} players)", &[&map.title(), &map.slots()]));
    }
    frame(p, scaled(151.0, 101.0, 498.0, 394.0), t.panel, t.accent);
    frame(p, scaled(168.0, 177.0, 225.0, 232.0), t.panel, t.accent2);
    frame(p, scaled(407.0, 177.0, 225.0, 232.0), t.panel, t.accent2);
    static_label(p, scaled(177.0, 162.0, 205.0, 31.0), t, tr("Match settings"));
    static_label(p, scaled(415.0, 162.0, 205.0, 31.0), t, tr("Game speed"));
    let title = region(Slot::AdvancedTitle).px(b);
    put(p, Slot::AdvancedTitle, title, kit_text(t, t.caps(&tr("Advanced"), t.caps_titles), t.title_size, t.title));
    let (key, fallback) = match ctx.draft.options.starting_resources {
        StartingResources::Normal => ("sr_normal", "Resources: Normal"),
        StartingResources::Lots => ("sr_lots", "Resources: Lots"),
    };
    act_button(p, ctx, b, Slot::AdvancedResources, &cap(ctx, key, fallback), Act::CycleResources);
    match super::super::view::officer_caption(ctx) {
        Some(caption) => act_button(p, ctx, b, Slot::AdvancedOfficers, &caption, Act::CycleOfficerLimit),
        None => off_button(p, ctx, b, Slot::AdvancedOfficers, &tr("Officer limit unavailable")),
    }
    act_button(p, ctx, b, Slot::AdvancedSpeed, &trf("Game speed: {0}", &[&ctx.draft.speed.label()]), Act::CycleMatchSpeed);
    if let Some(caption) = super::super::view::directors_caption(ctx) {
        act_button(p, ctx, b, Slot::AdvancedDirectorsCut, &caption, Act::ToggleDirectorsCut);
    }
    // The same slot: Director's Cut and warp belong to different profiles and never show together.
    if let Some(caption) = super::super::view::warp_caption(ctx) {
        act_button(p, ctx, b, Slot::AdvancedDirectorsCut, &caption, Act::ToggleWarp);
    }
    act_button(p, ctx, b, Slot::AdvancedDefault, &tr("Default"), Act::DefaultOptions);
    act_button(p, ctx, b, Slot::AdvancedClose, &tr("OK"), Act::CloseAdvanced(true));
    act_button(p, ctx, b, Slot::AdvancedCancel, &tr("Cancel"), Act::CloseAdvanced(false));
}

/// The picked map's start positions as fractions of the preview frame.
pub fn preview_dots(ctx: &Ctx) -> Vec<(f32, f32)> {
    let Some(m) = ctx.data.maps.get(ctx.draft.map) else { return Vec::new() };
    let f = |v: sta_content::Fixed| v.to_f32();
    let (lo, hi) = (&m.def.bounds.0, &m.def.bounds.1);
    m.playable_starts()
        .iter()
        .take(layout::TABLE_ROWS)
        .map(|s| layout::preview_point((f(s.pos.x), f(s.pos.z)), (f(lo.x), f(lo.z)), (f(hi.x), f(hi.z))))
        .collect()
}

fn settings(p: &mut Box_, ctx: &Ctx, b: Vec2) {
    let t = ctx.theme;
    let title = region(Slot::OptionsTitle).px(b);
    put(p, Slot::OptionsTitle, title, kit_text(t, t.caps(&cap(ctx, "options", "Options"), t.caps_titles), t.title_size, t.title));
    options_body(p, t, ctx.settings, ctx.controls, ctx.options_tab, ctx.graphics, region(Slot::OptionsBody).px(b), Placed(Slot::OptionsBody));
    act_button(p, ctx, b, Slot::OptionsBack, &cap(ctx, "previous_menu", "Previous menu"), Act::Goto(Screen::Main));
}

/// The options panel: the tabs and their sliders and toggles in the rectangle `body` (logical
/// px in its parent), for the options screen and for the in-game menu (UI-34), which shows the
/// same panel over a running match. The sliders and toggles carry `SettingSlider` and
/// `SettingToggle`; `on_settings` writes them to the `Settings` resource, which the settings
/// store (UI-29) saves. `tab` is the selected tab; `mark` tags the panel's node.
#[allow(clippy::too_many_arguments)]
pub fn options_body(p: &mut Box_, t: &crate::ui::theme::Theme, settings: &crate::frontend::screens::Settings, controls: &super::controls::ControlsDraft, tab: usize, info: Option<&super::GraphicsInfo>, body: Rect, mark: impl Bundle) {
    let s = settings.clone();
    let theme = t.clone();
    let info = info.cloned().unwrap_or_default();
    let tabs = [tr("Sound"), tr("Game"), tr("Graphics"), tr("Controls")];
    let tab = tab.min(3);
    let controls = controls.clone();
    p.spawn((
        mark,
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
                super::graphics::body(c, &theme, &s, &info, body.width());
            });
            c.spawn(tab_panel(bar, 3)).insert(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(theme.gap), display: shown(3), ..default() }).with_children(|c| {
                super::controls::body(c, &theme, &controls, body.width());
            });
    });
}
