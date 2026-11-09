//! Notices, the chat log and tooltips (UI-10), as Bevy UI nodes on the LCARS kit.
//!
//! - **Notices** are short lines at the top centre ("Cruiser destroyed"). Any
//!   system sends a [`Notice`]; [`SimMsg`] events become notices through
//!   [`notice_for`]. A notice stays [`NOTICE_SECS`], fading over the last
//!   [`NOTICE_FADE`]; at most [`NOTICE_MAX`] show, the oldest dropping first. The
//!   same text again restarts the line and counts it ("x2").
//! - **The chat log** keeps notices and chat lines ([`ChatMsg`]). The lines down the
//!   left show for [`CHAT_SECS`] each; the log keeps [`LOG_MAX`] for a screen that
//!   wants the history.
//! - **Tooltips**: put a [`Tooltip`] on any node. The short text shows after
//!   [`TIP_SHORT_SECS`] of rest, the verbose one (if any) after [`TIP_VERBOSE_SECS`]
//!   (Refit's `s_tooltipDelay` and `s_verboseTooltipDelay`, `ui/HUD.md`, "The tooltips
//!   at the pointer"), below and right of the pointer.
//!
//! Time is real time, so notices run on while the game is paused. Text is Bevy's, as
//! in the rest of the kit until the SDF font lands; no shader.

// Tooltips and chat are for the HUD and net packages; the binary only sends notices for now.
#![allow(dead_code)]

use std::collections::VecDeque;

use bevy::picking::events::{PointerOut, PointerOver};
use bevy::prelude::*;
use bevy::time::Real;
use bevy::window::PrimaryWindow;
use sta_sim::{PlayerId, SimEvent, UnitView};

use super::theme::Theme;
use crate::app::GameSet;
use crate::sim_link::{ContentRes, SimLink, SimMsg};

/// Seconds a notice shows, fade included.
pub const NOTICE_SECS: f32 = 5.0;
/// The last seconds of a notice, fading out.
pub const NOTICE_FADE: f32 = 1.0;
/// Notices on screen at once.
pub const NOTICE_MAX: usize = 4;
/// Seconds a chat line shows on the left.
pub const CHAT_SECS: f32 = 12.0;
/// Chat lines on screen at once.
pub const CHAT_SHOWN: usize = 6;
/// Lines the log keeps.
pub const LOG_MAX: usize = 100;
/// Rest before the short tooltip.
pub const TIP_SHORT_SECS: f32 = 0.35;
/// Rest before the verbose tooltip.
pub const TIP_VERBOSE_SECS: f32 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    #[default]
    Info,
    Good,
    Warn,
    Bad,
}

/// Ask for a notice.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct Notice {
    pub text: String,
    pub severity: Severity,
}

impl Notice {
    pub fn new(severity: Severity, text: impl Into<String>) -> Notice {
        Notice { text: text.into(), severity }
    }
}

/// A chat line from a player (the net lane sends these).
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ChatMsg {
    pub from: String,
    pub text: String,
}

/// One notice on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveNotice {
    pub text: String,
    pub severity: Severity,
    pub age: f32,
    pub count: u32,
}

impl ActiveNotice {
    /// 1 until the fade starts, then down to 0.
    pub fn alpha(&self) -> f32 {
        ((NOTICE_SECS - self.age) / NOTICE_FADE).clamp(0.0, 1.0)
    }

    pub fn shown(&self) -> String {
        if self.count > 1 { format!("{} x{}", self.text, self.count) } else { self.text.clone() }
    }
}

/// The notices on screen, newest last.
#[derive(Resource, Debug, Default)]
pub struct Notices {
    pub active: Vec<ActiveNotice>,
    dirty: bool,
}

impl Notices {
    pub fn push(&mut self, n: &Notice) {
        self.dirty = true;
        if let Some(a) = self.active.iter_mut().find(|a| a.text == n.text && a.severity == n.severity) {
            a.age = 0.0;
            a.count += 1;
            return;
        }
        self.active.push(ActiveNotice { text: n.text.clone(), severity: n.severity, age: 0.0, count: 1 });
        if self.active.len() > NOTICE_MAX {
            self.active.remove(0);
        }
    }

    pub fn age(&mut self, dt: f32) {
        for a in &mut self.active {
            a.age += dt;
        }
        let before = self.active.len();
        self.active.retain(|a| a.age < NOTICE_SECS);
        self.dirty |= self.active.len() != before;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogLine {
    /// `None` for a notice (the game speaking).
    pub from: Option<String>,
    pub text: String,
    pub severity: Severity,
    pub age: f32,
}

/// Notices and chat, oldest first.
#[derive(Resource, Debug, Default)]
pub struct ChatLog {
    pub lines: VecDeque<LogLine>,
    dirty: bool,
}

impl ChatLog {
    pub fn push(&mut self, from: Option<String>, text: String, severity: Severity) {
        self.lines.push_back(LogLine { from, text, severity, age: 0.0 });
        while self.lines.len() > LOG_MAX {
            self.lines.pop_front();
        }
        self.dirty = true;
    }

    /// The lines still on screen, oldest first, at most [`CHAT_SHOWN`].
    pub fn recent(&self) -> impl Iterator<Item = &LogLine> {
        let live = self.lines.iter().filter(|l| l.age < CHAT_SECS).count();
        self.lines.iter().filter(|l| l.age < CHAT_SECS).skip(live.saturating_sub(CHAT_SHOWN))
    }
}

/// Words for a thing under the pointer.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Tooltip {
    pub short: String,
    pub verbose: Option<String>,
}

impl Tooltip {
    pub fn new(short: impl Into<String>) -> Tooltip {
        Tooltip { short: short.into(), verbose: None }
    }

    pub fn verbose(mut self, text: impl Into<String>) -> Tooltip {
        self.verbose = Some(text.into());
        self
    }

    /// What shows after `rest` seconds under the pointer.
    pub fn text_at(&self, rest: f32) -> Option<&str> {
        if rest >= TIP_VERBOSE_SECS
            && let Some(v) = &self.verbose
        {
            return Some(v);
        }
        (rest >= TIP_SHORT_SECS).then_some(self.short.as_str())
    }
}

/// What the pointer rests on.
#[derive(Resource, Debug, Default)]
pub struct TipHover {
    pub target: Option<Entity>,
    pub rest: f32,
}

/// The notice for a sim event, from the point of view of `me`. `unit` gives a
/// unit's owner and name as the views show it.
pub fn notice_for(ev: &SimEvent, me: PlayerId, unit: impl Fn(sta_sim::EntityId) -> Option<(PlayerId, String)>) -> Option<Notice> {
    Some(match ev {
        SimEvent::Destroyed { id, .. } => {
            let (owner, name) = unit(*id).unwrap_or((me, "Unit".into()));
            if owner == me {
                Notice::new(Severity::Bad, format!("{name} destroyed"))
            } else {
                Notice::new(Severity::Info, format!("Enemy {name} destroyed"))
            }
        }
        SimEvent::ShieldsDown { id, .. } => {
            let (owner, name) = unit(*id)?;
            (owner == me).then(|| Notice::new(Severity::Warn, format!("{name}: shields down")))?
        }
        SimEvent::Built { id, .. } => {
            let (owner, name) = unit(*id)?;
            (owner == me).then(|| Notice::new(Severity::Good, format!("{name} complete")))?
        }
        SimEvent::PlayerEliminated { player } => {
            if *player == me {
                Notice::new(Severity::Bad, "You have been eliminated")
            } else {
                Notice::new(Severity::Info, format!("Player {} eliminated", player.0))
            }
        }
        _ => return None,
    })
}

fn name_of(db: Option<&ContentRes>, u: &UnitView) -> String {
    db.and_then(|d| d.0.name(u.def)).map(str::to_owned).unwrap_or_else(|| "Unit".into())
}

fn events_to_notices(
    mut events: MessageReader<SimMsg>,
    link: Option<Res<SimLink>>,
    db: Option<Res<ContentRes>>,
    mut out: MessageWriter<Notice>,
) {
    let Some(link) = link else {
        events.clear();
        return;
    };
    let lookup = |id| {
        // A unit that just died is only in the view before; one just built, only in this.
        let u = link.prev().unit(id).or_else(|| link.curr().unit(id))?;
        Some((u.owner, name_of(db.as_deref(), u)))
    };
    for m in events.read() {
        if let Some(n) = notice_for(&m.0, link.player(), lookup) {
            out.write(n);
        }
    }
}

fn collect(mut notices: MessageReader<Notice>, mut chats: MessageReader<ChatMsg>, mut active: ResMut<Notices>, mut log: ResMut<ChatLog>) {
    for n in notices.read() {
        active.push(n);
        log.push(None, n.text.clone(), n.severity);
    }
    for c in chats.read() {
        log.push(Some(c.from.clone()), c.text.clone(), Severity::Info);
    }
}

fn age(time: Res<Time<Real>>, mut active: ResMut<Notices>, mut log: ResMut<ChatLog>, mut hover: ResMut<TipHover>) {
    let dt = time.delta_secs();
    active.age(dt);
    let mut left = false;
    for l in &mut log.lines {
        l.age += dt;
        left |= l.age >= CHAT_SECS && l.age - dt < CHAT_SECS;
    }
    // Lines that left the screen change what is drawn once.
    log.dirty |= left;
    if hover.target.is_some() {
        hover.rest += dt;
    }
}

#[derive(Component)]
pub(crate) struct NoticeRoot;
#[derive(Component)]
struct NoticeLine(usize);
#[derive(Component)]
struct ChatRoot;
#[derive(Component)]
struct TipBox;
#[derive(Component)]
struct TipText;

fn colour(theme: &Theme, s: Severity) -> Color {
    match s {
        Severity::Info => theme.text,
        Severity::Good => theme.good,
        Severity::Warn => theme.warn,
        Severity::Bad => theme.bad,
    }
}

fn text(s: String, size: f32, color: Color) -> impl Bundle {
    (Text::new(s), TextFont { font_size: bevy::text::FontSize::Px(size), ..default() }, TextColor(color), Pickable::IGNORE)
}

fn spawn_roots(mut commands: Commands, theme: Res<Theme>) {
    commands.spawn((
        NoticeRoot,
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(theme.pad * 4.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(theme.gap * 0.5),
            ..default()
        },
    ));
    commands.spawn((
        ChatRoot,
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(theme.pad * 4.0),
            left: Val::Px(theme.pad),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        },
    ));
    commands.spawn((
        TipBox,
        Pickable::IGNORE,
        GlobalZIndex(1000),
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            max_width: Val::Px(theme.text_size * 20.0),
            padding: UiRect::axes(Val::Px(theme.pad), Val::Px(theme.gap * 0.5)),
            border: UiRect::left(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(theme.panel),
        BorderColor::all(theme.accent2),
        children![(TipText, text(String::new(), theme.small_size, theme.text))],
    ));
}

fn draw_notices(
    mut commands: Commands,
    theme: Res<Theme>,
    mut notices: ResMut<Notices>,
    root: Single<(Entity, Option<&Children>), With<NoticeRoot>>,
    mut colours: Query<(&NoticeLine, &mut TextColor)>,
) {
    let (root, kids) = *root;
    if notices.dirty {
        notices.dirty = false;
        for k in kids.into_iter().flatten() {
            commands.entity(*k).despawn();
        }
        for (i, a) in notices.active.iter().enumerate() {
            commands.entity(root).with_child((
                Node { padding: UiRect::axes(Val::Px(theme.pad), Val::Px(2.0)), ..default() },
                BackgroundColor(theme.panel.with_alpha(0.6)),
                children![(NoticeLine(i), text(a.shown(), theme.text_size, colour(&theme, a.severity)))],
            ));
        }
        return;
    }
    for (line, mut c) in &mut colours {
        if let Some(a) = notices.active.get(line.0) {
            c.0 = colour(&theme, a.severity).with_alpha(a.alpha());
        }
    }
}

fn draw_chat(
    mut commands: Commands,
    theme: Res<Theme>,
    mut log: ResMut<ChatLog>,
    root: Single<(Entity, Option<&Children>), With<ChatRoot>>,
) {
    if !log.dirty {
        return;
    }
    log.dirty = false;
    let (root, kids) = *root;
    for k in kids.into_iter().flatten() {
        commands.entity(*k).despawn();
    }
    for l in log.recent() {
        let s = match &l.from {
            Some(from) => format!("{from}: {}", l.text),
            None => l.text.clone(),
        };
        let c = if l.from.is_some() { theme.accent2 } else { colour(&theme, l.severity) };
        commands.entity(root).with_child(text(s, theme.small_size, c));
    }
}

fn on_over(ev: On<PointerOver>, tips: Query<(), With<Tooltip>>, mut hover: ResMut<TipHover>) {
    if tips.contains(ev.entity) && hover.target != Some(ev.entity) {
        *hover = TipHover { target: Some(ev.entity), rest: 0.0 };
    }
}

fn on_out(ev: On<PointerOut>, mut hover: ResMut<TipHover>) {
    if hover.target == Some(ev.entity) {
        *hover = TipHover::default();
    }
}

fn draw_tip(
    hover: Res<TipHover>,
    tips: Query<&Tooltip>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut boxed: Single<&mut Node, With<TipBox>>,
    mut words: Single<&mut Text, With<TipText>>,
) {
    let shown = hover.target.and_then(|e| tips.get(e).ok()).and_then(|t| t.text_at(hover.rest));
    let Some(shown) = shown else {
        boxed.display = Display::None;
        return;
    };
    if words.0 != shown {
        words.0 = shown.to_owned();
    }
    boxed.display = Display::Flex;
    if let Some(w) = window
        && let Some(p) = w.cursor_position()
    {
        // Below and right of the pointer; left of it near the right edge.
        let flip = p.x > w.width() - 320.0;
        boxed.top = Val::Px(p.y + 20.0);
        boxed.left = if flip { Val::Auto } else { Val::Px(p.x + 16.0) };
        boxed.right = if flip { Val::Px(w.width() - p.x + 8.0) } else { Val::Auto };
    }
}

pub struct NoticePlugin;

impl Plugin for NoticePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Theme>()
            .add_message::<SimMsg>()
            .add_message::<Notice>()
            .add_message::<ChatMsg>()
            .init_resource::<Notices>()
            .init_resource::<ChatLog>()
            .init_resource::<TipHover>()
            .add_observer(on_over)
            .add_observer(on_out)
            .add_systems(Startup, spawn_roots)
            .add_systems(Update, (events_to_notices, collect, age, (draw_notices, draw_chat, draw_tip)).chain().in_set(GameSet::Ui));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::sim_link::SimLinkPlugin;
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;
    use std::time::Duration;

    const TIMELINE: &str = "{ units: [
        { id: 1, owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] },
        { id: 2, owner: 1, hull: 100, until: 3, rails: [{ at: 0, pos: { x: 9, y: 0, z: 0 } }] },
        { id: 3, owner: 2, hull: 100, until: 6, rails: [{ at: 0, pos: { x: 90, y: 0, z: 0 } }] },
    ] }";

    fn rig() -> App {
        let db = sta_content::ContentDb::builder().build().unwrap();
        let source = ScriptedSource::from_json5(TIMELINE, &db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 30.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(1)));
        configure_sets(&mut app);
        app.add_plugins(NoticePlugin);
        app
    }

    fn texts(app: &mut App) -> Vec<String> {
        app.world_mut().query_filtered::<&Text, With<TextColor>>().iter(app.world()).map(|t| t.0.clone()).collect()
    }

    #[test]
    fn destroyed_shows_a_notice() {
        let mut app = rig();
        // 30 Hz frames make a tick each; the units go at ticks 3 and 6.
        for _ in 0..10 {
            app.update();
        }
        let shown = texts(&mut app);
        assert!(shown.contains(&"Unit destroyed".to_string()), "{shown:?}");
        assert!(shown.contains(&"Enemy Unit destroyed".to_string()), "{shown:?}");
        let log = app.world().resource::<ChatLog>();
        assert_eq!(log.lines.len(), 2);
        assert!(log.lines.iter().all(|l| l.from.is_none()));
        assert_eq!(app.world().resource::<Notices>().active[0].severity, Severity::Bad);
    }

    #[test]
    fn notice_for_covers_events() {
        let me = PlayerId(1);
        let mine = |_| Some((me, "Cruiser".to_string()));
        let theirs = |_| Some((PlayerId(2), "Cruiser".to_string()));
        let id = sta_sim::EntityId(5);
        let n = notice_for(&SimEvent::Destroyed { id, by: None }, me, mine).unwrap();
        assert_eq!((n.severity, n.text.as_str()), (Severity::Bad, "Cruiser destroyed"));
        let n = notice_for(&SimEvent::Destroyed { id, by: None }, me, theirs).unwrap();
        assert_eq!(n.text, "Enemy Cruiser destroyed");
        assert!(notice_for(&SimEvent::ShieldsDown { id, facing: 0 }, me, theirs).is_none());
        assert_eq!(notice_for(&SimEvent::ShieldsDown { id, facing: 0 }, me, mine).unwrap().severity, Severity::Warn);
        assert!(notice_for(&SimEvent::Spawned { id }, me, mine).is_none());
    }

    #[test]
    fn notices_expire_repeat_and_cap() {
        let mut n = Notices::default();
        let a = Notice::new(Severity::Info, "a");
        n.push(&a);
        n.push(&a);
        assert_eq!(n.active.len(), 1);
        assert_eq!(n.active[0].shown(), "a x2");
        n.age(NOTICE_SECS - NOTICE_FADE / 2.0);
        assert!((n.active[0].alpha() - 0.5).abs() < 1e-4);
        n.push(&a);
        assert_eq!(n.active[0].age, 0.0);
        for i in 0..6 {
            n.push(&Notice::new(Severity::Info, format!("n{i}")));
        }
        assert_eq!(n.active.len(), NOTICE_MAX);
        assert_eq!(n.active[0].text, "n2");
        n.age(NOTICE_SECS);
        assert!(n.active.is_empty());
    }

    #[test]
    fn chat_lines_leave_the_screen_but_stay_in_the_log() {
        let mut app = rig();
        app.world_mut().write_message(ChatMsg { from: "Ada".into(), text: "hello".into() });
        app.update();
        assert_eq!(app.world().resource::<ChatLog>().recent().count(), 1);
        for l in &mut app.world_mut().resource_mut::<ChatLog>().lines {
            l.age = CHAT_SECS;
        }
        assert_eq!(app.world().resource::<ChatLog>().recent().count(), 0);
        assert_eq!(app.world().resource::<ChatLog>().lines.len(), 1);
    }

    #[test]
    fn tooltip_waits_then_gets_verbose() {
        let t = Tooltip::new("Hull").verbose("Hull integrity at 100%");
        assert_eq!(t.text_at(0.1), None);
        assert_eq!(t.text_at(TIP_SHORT_SECS), Some("Hull"));
        assert_eq!(t.text_at(TIP_VERBOSE_SECS), Some("Hull integrity at 100%"));
        assert_eq!(Tooltip::new("Hull").text_at(5.0), Some("Hull"));
    }

    #[test]
    fn tooltip_box_follows_hover() {
        let mut app = rig();
        app.update();
        let e = app.world_mut().spawn(Tooltip::new("Metal")).id();
        *app.world_mut().resource_mut::<TipHover>() = TipHover { target: Some(e), rest: 0.0 };
        let display = |app: &mut App| app.world_mut().query_filtered::<&Node, With<TipBox>>().single(app.world()).unwrap().display;
        app.update();
        assert_eq!(display(&mut app), Display::None);
        for _ in 0..12 {
            app.update();
        }
        assert_eq!(display(&mut app), Display::Flex);
        let words = app.world_mut().query_filtered::<&Text, With<TipText>>().single(app.world()).unwrap().0.clone();
        assert_eq!(words, "Metal");
    }
}
