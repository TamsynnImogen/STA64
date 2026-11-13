//! The multiplayer screens, drawn from the UI kit in the regions of [`layout`](super::layout).
//! Every part is a [`Placed`] node holding one widget; buttons carry an [`MpAct`], seat
//! dropdowns a [`LobbyDrop`], text fields a [`LobbyField`].

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::math::{Rect, Vec2};
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use sta_content::StartingResources;

use super::layout::{self, MSlot, ROWS, region};
use super::{Kind, LobbyView, MpFields, Multiplayer, Phase, SeatView, Session};
use crate::app::AppState;
use crate::frontend::screens::{Screen, ScreenData, ScreenRoot};
use crate::ui::i18n::{tr, trf};
use crate::ui::sprite_font::{SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::{TextFilter, Theme, disabled_button, dropdown, font_role, kit_button, kit_text, label, text_field};

/// A part placed by the layout: which slot, so tests and systems find it.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed(pub MSlot);

/// What a click on a button of these screens does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpAct {
    Goto(Screen),
    /// Open a room on the relay (host form).
    StartHost,
    /// Join the room named in the code field (join form).
    StartJoin,
    /// Take a seat back in a running game: off until NET-04.
    Rejoin,
    Kick(usize),
    /// Open or close the room to new players.
    ToggleOpen,
    /// The host starts the game.
    Launch,
    /// A joiner's Ready.
    ToggleReady,
    PickMap,
    CycleResources,
    CycleOfficers,
    /// Leave the room.
    Leave,
}

/// A text field of these screens.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LobbyField {
    Name,
    Relay,
    Code,
    Password,
    Chat,
}

/// Which column of the lobby table a dropdown is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropCol {
    Kind,
    Side,
    Team,
}

/// A seat's dropdown in the lobby table.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyDrop {
    pub slot: usize,
    pub col: DropCol,
}

/// The ready light of a row (green: ready).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadyLight {
    pub slot: usize,
    pub on: bool,
}

/// What a screen is drawn from.
pub struct Ctx<'a> {
    pub theme: &'a Theme,
    pub data: &'a ScreenData,
    pub strings: Option<&'a crate::ui::i18n::Strings>,
    pub mp: &'a Multiplayer,
    pub fields: &'a MpFields,
    /// The box the regions are fractions of, logical px.
    pub b: Vec2,
}

type Box_<'a> = RelatedSpawnerCommands<'a, ChildOf>;

/// A caption: the mounted `label.map`'s text for `key`, else the translated `fallback`.
fn cap(ctx: &Ctx, key: &str, fallback: &str) -> String {
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
    let b = ctx.b;
    root.with_children(|p| {
        p.spawn((crate::frontend::screens::stock::StockBoxNode, Node { width: Val::Px(b.x), height: Val::Px(b.y), ..default() })).with_children(|p| match screen {
            Screen::Multiplayer => connection(p, ctx),
            Screen::HostGame => host_form(p, ctx),
            Screen::JoinGame => join_form(p, ctx),
            Screen::Lobby => lobby(p, ctx),
            _ => {}
        });
    });
}

/// `content` in the rectangle `r`, centred vertically in it.
fn put(p: &mut Box_, slot: MSlot, r: Rect, content: impl Bundle) {
    p.spawn((
        Placed(slot),
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), align_items: AlignItems::Center, ..default() },
        children![content],
    ));
}

/// A block of wrapped lines in `r`, top aligned, clipped.
fn block(p: &mut Box_, slot: MSlot, r: Rect, fill: impl FnOnce(&mut Box_)) {
    p.spawn((
        Placed(slot),
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), flex_direction: FlexDirection::Column, overflow: Overflow::clip(), ..default() },
    ))
    .with_children(fill);
}

fn line(t: &Theme, text: impl Into<String>, size: f32, colour: Color) -> impl Bundle {
    (Node { margin: UiRect::bottom(Val::Px(t.gap * 0.5)), ..default() }, children![sprite_text_styled(text, size, colour, SpriteTextStyle::role(font_role(t, size)).wrap_to_parent())])
}

fn act_button(p: &mut Box_, ctx: &Ctx, slot: MSlot, text: &str, act: MpAct, on: bool) {
    let r = region(slot).px(ctx.b);
    if on {
        put(p, slot, r, (kit_button(ctx.theme, text, r.width()), act));
    } else {
        put(p, slot, r, disabled_button(ctx.theme, text, r.width()));
    }
}

fn title(p: &mut Box_, ctx: &Ctx, text: &str) {
    let t = ctx.theme;
    put(p, MSlot::Title, region(MSlot::Title).px(ctx.b), kit_text(t, t.caps(text, t.caps_titles), t.title_size, t.title));
}

fn caption(p: &mut Box_, ctx: &Ctx, slot: MSlot, text: &str) {
    put(p, slot, region(slot).px(ctx.b), label(ctx.theme, text));
}

#[allow(clippy::too_many_arguments)]
fn field(p: &mut Box_, ctx: &Ctx, slot: MSlot, which: LobbyField, initial: &str, placeholder: &str, max: usize, filter: TextFilter) {
    let r = region(slot).px(ctx.b);
    put(p, slot, r, (text_field(ctx.theme, initial, placeholder, r.width(), max, filter), which));
}

/// The notice under a form: why the last try failed, or that one is running.
fn notice(p: &mut Box_, ctx: &Ctx, slot: MSlot) {
    let t = ctx.theme;
    let (text, colour) = match (&ctx.mp.notice, ctx.mp.connecting()) {
        (_, true) => (tr("Connecting to the relay..."), t.text),
        (Some(n), _) => (n.clone(), t.bad),
        _ => return,
    };
    block(p, slot, region(slot).px(ctx.b), |d| {
        d.spawn(line(t, text, t.text_size, colour));
    });
}

fn connection(p: &mut Box_, ctx: &Ctx) {
    title(p, ctx, &cap(ctx, "multiplayer", "Multiplayer"));
    caption(p, ctx, MSlot::NameLabel, &cap(ctx, "enter_name", "Your name"));
    field(p, ctx, MSlot::NameField, LobbyField::Name, &ctx.fields.name, &tr("Player name"), 24, TextFilter::Name);
    caption(p, ctx, MSlot::RelayLabel, &tr("Relay address (host:port)"));
    field(p, ctx, MSlot::RelayField, LobbyField::Relay, &ctx.fields.relay, "127.0.0.1:7864", 80, TextFilter::Any);
    notice(p, ctx, MSlot::ConnNotice);
    let idle = !ctx.mp.busy();
    act_button(p, ctx, MSlot::HostBtn, &cap(ctx, "create_game", "Host a game"), MpAct::Goto(Screen::HostGame), idle);
    act_button(p, ctx, MSlot::JoinBtn, &cap(ctx, "join_game", "Join a game"), MpAct::Goto(Screen::JoinGame), idle);
    act_button(p, ctx, MSlot::Back, &cap(ctx, "previous_menu", "Previous menu"), MpAct::Goto(Screen::Main), true);
}

fn host_form(p: &mut Box_, ctx: &Ctx) {
    title(p, ctx, &cap(ctx, "create_game", "Host a game"));
    caption(p, ctx, MSlot::HostPassLabel, &tr("Password (leave empty for none)"));
    field(p, ctx, MSlot::HostPassField, LobbyField::Password, &ctx.fields.password, &tr("No password"), 32, TextFilter::Any);
    act_button(p, ctx, MSlot::CreateBtn, &cap(ctx, "create", "Create room"), MpAct::StartHost, !ctx.mp.busy());
    notice(p, ctx, MSlot::HostNotice);
    act_button(p, ctx, MSlot::Back, &cap(ctx, "previous_menu", "Previous menu"), MpAct::Goto(Screen::Multiplayer), !ctx.mp.connecting());
}

fn join_form(p: &mut Box_, ctx: &Ctx) {
    title(p, ctx, &cap(ctx, "join_game", "Join a game"));
    caption(p, ctx, MSlot::CodeLabel, &tr("Room code"));
    field(p, ctx, MSlot::CodeField, LobbyField::Code, &ctx.fields.code, "K7M-Q2X", 12, TextFilter::Any);
    caption(p, ctx, MSlot::JoinPassLabel, &tr("Password (if the room has one)"));
    field(p, ctx, MSlot::JoinPassField, LobbyField::Password, &ctx.fields.password, &tr("No password"), 32, TextFilter::Any);
    act_button(p, ctx, MSlot::JoinNowBtn, &cap(ctx, "join", "Join"), MpAct::StartJoin, !ctx.mp.busy());
    // Taking a seat back in a running game is NET-04's.
    act_button(p, ctx, MSlot::RejoinBtn, &tr("Rejoin a running game"), MpAct::Rejoin, false);
    block(p, MSlot::RejoinNote, region(MSlot::RejoinNote).px(ctx.b), |d| {
        d.spawn(line(ctx.theme, super::rejoin_unavailable(), ctx.theme.small_size, ctx.theme.text_dim));
    });
    notice(p, ctx, MSlot::JoinNotice);
    act_button(p, ctx, MSlot::Back, &cap(ctx, "previous_menu", "Previous menu"), MpAct::Goto(Screen::Multiplayer), !ctx.mp.connecting());
}

/// What the lobby shows when there is no room yet (still connecting) or it is over.
fn empty_lobby(p: &mut Box_, ctx: &Ctx) {
    title(p, ctx, &cap(ctx, "multiplayer", "Multiplayer"));
    block(p, MSlot::Status, region(MSlot::Status).px(ctx.b), |d| {
        d.spawn(line(ctx.theme, tr("Connecting to the relay..."), ctx.theme.text_size, ctx.theme.text));
    });
    act_button(p, ctx, MSlot::Leave, &cap(ctx, "previous_menu", "Leave"), MpAct::Leave, true);
}

fn kind_label(ctx: &Ctx, k: Kind) -> String {
    match k {
        Kind::Open => cap(ctx, "empty", "Open"),
        Kind::Computer => tr("Computer"),
        Kind::Idle => tr("Idle"),
        Kind::Human => tr("Player"),
    }
}

/// The side's caption: the install's, else its id.
fn race_caption(ctx: &Ctx, id: &str) -> String {
    if id.is_empty() { String::new() } else { ctx.data.race_label(id, ctx.strings) }
}

fn lobby(p: &mut Box_, ctx: &Ctx) {
    let t = ctx.theme;
    let Some(s) = ctx.mp.session.as_ref() else { return empty_lobby(p, ctx) };
    let host = s.is_host();
    let view = s.view();
    title(p, ctx, &cap(ctx, "multiplayer", "Multiplayer"));
    for (slot, text) in [(MSlot::HeadPlayer, tr("Player")), (MSlot::HeadSide, tr("Side")), (MSlot::HeadTeam, tr("Team"))] {
        put(p, slot, region(slot).px(ctx.b), label(t, &text));
    }
    let races = s.races().to_vec();
    let race_labels: Vec<String> = races.iter().map(|r| race_caption(ctx, r)).collect();
    let race_refs: Vec<&str> = race_labels.iter().map(String::as_str).collect();
    let n = view.seats.len().clamp(1, ROWS);
    let teams: Vec<String> = (0..n).map(|i| cap(ctx, &format!("team_{}", i + 1), &trf("Team {0}", &[&(i + 1)]))).collect();
    let team_refs: Vec<&str> = teams.iter().map(String::as_str).collect();
    let kinds = [Kind::Open, Kind::Computer, Kind::Idle].map(|k| kind_label(ctx, k));
    let kind_refs: Vec<&str> = kinds.iter().map(String::as_str).collect();
    for seat in view.seats.iter().take(ROWS) {
        let i = usize::from(seat.slot);
        let at = |slot: MSlot| region(slot).px(ctx.b);
        // The light: green for ready people and computers, red for people not ready, dull for nobody.
        let (on, colour) = match (seat.kind, seat.ready) {
            (Kind::Human | Kind::Computer, true) => (true, t.good),
            (Kind::Human, false) => (false, t.bad),
            _ => (false, t.track),
        };
        let lr = at(MSlot::Light(i));
        p.spawn((
            Placed(MSlot::Light(i)),
            ReadyLight { slot: i, on },
            Node { position_type: PositionType::Absolute, left: Val::Px(lr.min.x), top: Val::Px(lr.min.y), width: Val::Px(lr.width()), height: Val::Px(lr.height()), ..default() },
            BackgroundColor(colour),
        ));
        // The player: a name, or (for the host) what an empty slot is.
        let r = at(MSlot::Player(i));
        if seat.kind == Kind::Human {
            let name = if seat.host { trf("{0} (host)", &[&seat.name]) } else { seat.name.clone() };
            p.spawn((
                Placed(MSlot::Player(i)),
                Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), align_items: AlignItems::Center, overflow: Overflow::clip(), ..default() },
                children![label(t, &name)],
            ));
            if host && i != 0 {
                let kr = at(MSlot::Kick(i));
                put(p, MSlot::Kick(i), kr, (kit_button(t, &cap(ctx, "kick", "Kick"), kr.width()), MpAct::Kick(i)));
            }
        } else if host {
            let selected = match seat.kind {
                Kind::Computer => 1,
                Kind::Idle => 2,
                _ => 0,
            };
            // The player column's whole width (Kick has no place on an empty slot).
            let full = Rect::new(r.min.x, r.min.y, at(MSlot::Kick(i)).max.x, r.max.y);
            p.spawn((
                Placed(MSlot::Player(i)),
                Node { position_type: PositionType::Absolute, left: Val::Px(full.min.x), top: Val::Px(full.min.y), width: Val::Px(full.width()), height: Val::Px(full.height()), align_items: AlignItems::Center, ..default() },
                children![(dropdown(t, &kind_refs, selected, full.width()), LobbyDrop { slot: i, col: DropCol::Kind })],
            ));
        } else {
            put(p, MSlot::Player(i), r, label(t, &kind_label(ctx, seat.kind)));
        }
        // Side and team: a dropdown where the viewer may choose, else the value.
        let editable = if host { seat.kind != Kind::Human || i == 0 } else { seat.me };
        let side_r = at(MSlot::Side(i));
        if seat.kind == Kind::Human || seat.kind == Kind::Computer || host {
            if editable {
                let sel = races.iter().position(|id| id.eq_ignore_ascii_case(&seat.race)).unwrap_or(0);
                put(p, MSlot::Side(i), side_r, (dropdown(t, &race_refs, sel, side_r.width()), LobbyDrop { slot: i, col: DropCol::Side }));
                let team_r = at(MSlot::Team(i));
                put(p, MSlot::Team(i), team_r, (dropdown(t, &team_refs, usize::from(seat.team).min(n - 1), team_r.width()), LobbyDrop { slot: i, col: DropCol::Team }));
            } else {
                put(p, MSlot::Side(i), side_r, label(t, &race_caption(ctx, &seat.race)));
                put(p, MSlot::Team(i), at(MSlot::Team(i)), label(t, &teams[usize::from(seat.team).min(n - 1)]));
            }
        }
    }
    // The log and the chat line.
    let log_r = region(MSlot::Log).px(ctx.b);
    let rows = ((log_r.height() / (t.small_size * 1.5)).floor() as usize).max(1);
    let lines: Vec<String> = s.chat().iter().map(|c| format!("{}: {}", c.from, c.text)).collect();
    block(p, MSlot::Log, log_r, |d| {
        for l in lines.iter().skip(lines.len().saturating_sub(rows)) {
            d.spawn(line(t, l.clone(), t.small_size, t.text));
        }
    });
    let chat_on = matches!(s.phase, Phase::Open);
    if chat_on {
        field(p, ctx, MSlot::ChatLine, LobbyField::Chat, &ctx.fields.chat, &tr("Say something to the room"), proto_chat_len(), TextFilter::Any);
    }
    map_window(p, ctx, s, &view);
    // The room.
    let code = format!("{}", s.code());
    put(p, MSlot::RoomInfo, region(MSlot::RoomInfo).px(ctx.b), label(t, &trf("Room code: {0}", &[&code])));
    if host {
        let text = if view.closed { tr("Room closed to new players: click to open") } else { tr("Room open: click to close") };
        act_button(p, ctx, MSlot::OpenToggle, &text, MpAct::ToggleOpen, matches!(s.phase, Phase::Open));
    }
    // What to do next, in plain words.
    let status = status_lines(ctx, s, &view);
    block(p, MSlot::Status, region(MSlot::Status).px(ctx.b), |d| {
        for (text, colour) in &status {
            d.spawn(line(t, text.clone(), t.text_size, *colour));
        }
    });
    act_button(p, ctx, MSlot::Leave, &cap(ctx, "previous_menu", "Leave"), MpAct::Leave, true);
    if host {
        act_button(p, ctx, MSlot::Launch, &cap(ctx, "launch", "Launch"), MpAct::Launch, s.launch_block().is_none());
    } else {
        let me = view.seats.iter().find(|x| x.me);
        let ready = me.is_some_and(|x| x.ready);
        let text = if ready { tr("Not ready") } else { tr("Ready") };
        act_button(p, ctx, MSlot::Launch, &text, MpAct::ToggleReady, s.ready_block().is_none() && matches!(s.phase, Phase::Open));
    }
}

fn proto_chat_len() -> usize {
    sta_net::proto::MAX_CHAT_BYTES.min(120)
}

fn status_lines(ctx: &Ctx, s: &Session, view: &LobbyView) -> Vec<(String, Color)> {
    let t = ctx.theme;
    let mut out = Vec::new();
    match &s.phase {
        Phase::Joining => out.push((tr("Waiting for the host to let you in..."), t.text)),
        Phase::Launched => out.push((tr("The game is starting."), t.good)),
        Phase::Ended(e) => out.push((e.text(), t.bad)),
        Phase::Open => {
            if s.is_host() {
                match s.launch_block() {
                    Some(why) => out.push((why, t.text_dim)),
                    None => out.push((tr("Everyone is ready. Launch when you are."), t.good)),
                }
            } else {
                match s.ready_block() {
                    Some(why) => out.push((why, t.bad)),
                    None => {
                        let ready = view.seats.iter().find(|x| x.me).is_some_and(|x| x.ready);
                        out.push((if ready { tr("You are ready. Waiting for the host to launch.") } else { tr("Press Ready when you are set.") }, t.text_dim));
                    }
                }
            }
        }
    }
    if let Some(note) = &ctx.mp.lobby_note {
        out.push((note.clone(), t.warn));
    }
    out
}

/// The map window: its labels, the preview of the start positions, the settings beside it.
fn map_window(p: &mut Box_, ctx: &Ctx, s: &Session, view: &LobbyView) {
    let t = ctx.theme;
    let host = s.is_host();
    let info = match (view.map_index, view.map_known) {
        (Some(i), true) => {
            let m = &s.maps()[i];
            trf("Map: {0}  ({1} players)", &[&ctx.data.maps.get(i).map_or(m.name.as_str(), |e| e.title()), &m.slots])
        }
        (Some(_), false) => trf("Map: {0}  (your copy is not the host's)", &[&view.map]),
        (None, _) if view.map.is_empty() => tr("No maps found."),
        (None, _) => trf("Map: {0}  (you do not have this map)", &[&view.map]),
    };
    put(p, MSlot::MapInfo, region(MSlot::MapInfo).px(ctx.b), label(t, &info));
    let pv = region(MSlot::Preview).px(ctx.b);
    p.spawn((
        Placed(MSlot::Preview),
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
        for (i, (fx, fy)) in preview_dots(s, view).into_iter().enumerate() {
            c.spawn((Node { position_type: PositionType::Absolute, left: Val::Percent(fx * 100.0), top: Val::Percent(fy * 100.0), ..default() }, children![kit_text(t, (i + 1).to_string(), t.small_size, t.accent)]));
        }
    });
    let open = matches!(s.phase, Phase::Open);
    act_button(p, ctx, MSlot::SelectMap, &cap(ctx, "select_map", "Select map"), MpAct::PickMap, host && open && !ctx.data.maps.is_empty());
    let (key, fallback) = match view.options.starting_resources {
        StartingResources::Normal => ("sr_normal", "Resources: Normal"),
        StartingResources::Lots => ("sr_lots", "Resources: Lots"),
    };
    act_button(p, ctx, MSlot::Resources, &cap(ctx, key, fallback), MpAct::CycleResources, host && open);
    let officer = ctx.data.content.rules().officer_limits.get(view.options.officer_limit as usize).map(|choice| {
        let caption = ctx
            .strings
            .and_then(|st| st.native_label(&choice.key))
            .filter(|c| !c.trim().is_empty())
            .or_else(|| (!choice.key.trim().is_empty()).then_some(choice.key.as_str()))
            .map_or_else(|| tr("Officer limit unavailable"), str::to_owned);
        format!("{caption}: {}", choice.value)
    });
    match officer {
        Some(c) => act_button(p, ctx, MSlot::Officers, &c, MpAct::CycleOfficers, host && open),
        None => act_button(p, ctx, MSlot::Officers, &tr("Officer limit unavailable"), MpAct::CycleOfficers, false),
    }
}

/// The map's start positions as fractions of the preview frame.
pub fn preview_dots(s: &Session, view: &LobbyView) -> Vec<(f32, f32)> {
    let Some(i) = view.map_index.filter(|_| view.map_known) else { return Vec::new() };
    let m = &s.maps()[i].def;
    let f = |v: sta_content::Fixed| v.to_f32();
    let (lo, hi) = (&m.bounds.0, &m.bounds.1);
    let mut starts: Vec<_> = m.starts.iter().collect();
    starts.sort_by_key(|x| x.slot);
    starts.iter().take(ROWS).map(|x| layout::preview_point((f(x.pos.x), f(x.pos.z)), (f(lo.x), f(lo.z)), (f(hi.x), f(hi.z)))).collect()
}
