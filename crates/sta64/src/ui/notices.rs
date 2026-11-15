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

use super::sprite_font::{SpriteText, SpriteTextStyle, sprite_text_styled};
use super::theme::Theme;
use crate::app::GameSet;
use crate::sim_link::{ContentRes, SimLink, SimMsg};
use crate::ui::i18n::{tr, trf};
use crate::ui::profile::ProfileSync;

/// Seconds a notice shows, fade included.
pub const NOTICE_SECS: f32 = 5.0;
/// The last seconds of a notice, fading out.
pub const NOTICE_FADE: f32 = 1.0;
/// Notices on screen at once.
pub const NOTICE_MAX: usize = 4;
/// Widest a notice line gets, as a share of the screen width. Centred, so notices
/// stay between (100 - this) / 2 % and (100 + this) / 2 %, clear of the info text
/// (`map::HUD_TEXT_WIDTH_PCT`) at the top left. HUD.md puts event text in the middle
/// of the screen and chat down the left, so nothing of ours takes the left column.
pub const NOTICE_WIDTH_PCT: f32 = 30.0;
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

/// The horizontal span `(left, right)` of the notice column on a screen `screen_w` wide.
pub fn notice_span(screen_w: f32) -> (f32, f32) {
    let w = screen_w * NOTICE_WIDTH_PCT / 100.0;
    ((screen_w - w) / 2.0, (screen_w + w) / 2.0)
}

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
            let (owner, name) = unit(*id).unwrap_or((me, tr("Unit")));
            if owner == me {
                Notice::new(Severity::Bad, trf("{0} destroyed", &[&name]))
            } else {
                Notice::new(Severity::Info, trf("Enemy {0} destroyed", &[&name]))
            }
        }
        SimEvent::ShieldsDown { id, .. } => {
            let (owner, name) = unit(*id)?;
            (owner == me).then(|| Notice::new(Severity::Warn, trf("{0}: shields down", &[&name])))?
        }
        SimEvent::Built { id, .. } => {
            let (owner, name) = unit(*id)?;
            (owner == me).then(|| Notice::new(Severity::Good, trf("{0} complete", &[&name])))?
        }
        SimEvent::PlayerEliminated { player } => {
            if *player == me {
                Notice::new(Severity::Bad, tr("You have been eliminated"))
            } else {
                Notice::new(Severity::Info, trf("Player {0} eliminated", &[&player.0]))
            }
        }
        _ => return None,
    })
}

fn name_of(db: Option<&ContentRes>, u: &UnitView) -> String {
    // The ODF's own `unitName` caption (what the HUD's mouseover strip shows), the ODF name
    // only when it authors none.
    db.and_then(|d| {
        let caption = d.0.unit(u.def).and_then(super::hud::model::craft_names::unit_name);
        caption.or_else(|| d.0.name(u.def)).map(str::to_owned)
    })
    .unwrap_or_else(|| tr("Unit"))
}

fn events_to_notices(
    mut events: MessageReader<SimMsg>,
    link: Option<Res<SimLink>>,
    db: Option<Res<ContentRes>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    mut out: MessageWriter<Notice>,
) {
    // A cineractive shows no log: what the script does during it (replacing the
    // fleet, the staged fight of an intro) is not reported, and the lines would
    // otherwise appear the moment the player gets control back.
    let Some(link) = link.filter(|_| !crate::ui::presentation::cutscene_running(cine)) else {
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

/// Notice, chat and tooltip text (UI-16): the mod's sprite font where it has one.
fn text(theme: &Theme, s: String, size: f32, color: Color) -> impl Bundle {
    super::theme::kit_text(theme, s, size, color)
}

/// The tooltip wraps at the box's width.
fn tip_style(theme: &Theme) -> SpriteTextStyle {
    SpriteTextStyle::role(super::theme::font_role(theme, theme.small_size)).wrapped(theme.text_size * 20.0 - 2.0 * theme.pad - 4.0)
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
        children![(TipText, sprite_text_styled(String::new(), theme.small_size, theme.text, tip_style(&theme)))],
    ));
}

/// The roots and tooltip persist across a theme switch. Rebuild only their
/// displayed children; the active notice and chat history resources survive.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn retheme(
    theme: Res<Theme>,
    mut notices: ResMut<Notices>,
    mut log: ResMut<ChatLog>,
    mut notice_roots: Query<&mut Node, (With<NoticeRoot>, Without<ChatRoot>, Without<TipBox>)>,
    mut chat_roots: Query<&mut Node, (With<ChatRoot>, Without<NoticeRoot>, Without<TipBox>)>,
    mut tips: Query<(&mut Node, &mut BackgroundColor, &mut BorderColor), (With<TipBox>, Without<NoticeRoot>, Without<ChatRoot>)>,
    mut tip_texts: Query<(&mut SpriteText, &mut SpriteTextStyle), With<TipText>>,
) {
    if !theme.is_changed() {
        return;
    }
    for mut root in &mut notice_roots {
        root.top = Val::Px(theme.pad * 4.0);
        root.row_gap = Val::Px(theme.gap * 0.5);
    }
    for mut root in &mut chat_roots {
        root.top = Val::Px(theme.pad * 4.0);
        root.left = Val::Px(theme.pad);
    }
    for (mut node, mut background, mut border) in &mut tips {
        node.max_width = Val::Px(theme.text_size * 20.0);
        node.padding = UiRect::axes(Val::Px(theme.pad), Val::Px(theme.gap * 0.5));
        background.0 = theme.panel;
        *border = BorderColor::all(theme.accent2);
    }
    for (mut words, mut style) in &mut tip_texts {
        words.size = theme.small_size;
        words.color = theme.text;
        *style = tip_style(&theme);
    }
    notices.dirty = true;
    log.dirty = true;
}

fn draw_notices(
    mut commands: Commands,
    theme: Res<Theme>,
    mut notices: ResMut<Notices>,
    root: Single<(Entity, Option<&Children>), With<NoticeRoot>>,
    mut colours: Query<(&NoticeLine, &mut SpriteText)>,
) {
    let (root, kids) = *root;
    if notices.dirty {
        notices.dirty = false;
        for k in kids.into_iter().flatten() {
            commands.entity(*k).despawn();
        }
        for (i, a) in notices.active.iter().enumerate() {
            commands.entity(root).with_child((
                Node { padding: UiRect::axes(Val::Px(theme.pad), Val::Px(2.0)), max_width: Val::Percent(NOTICE_WIDTH_PCT), ..default() },
                BackgroundColor(theme.panel.with_alpha(0.6)),
                children![(NoticeLine(i), sprite_text_styled(a.shown(), theme.text_size, colour(&theme, a.severity), SpriteTextStyle::role(super::theme::font_role(&theme, theme.text_size)).wrap_to_parent()))],
            ));
        }
        return;
    }
    for (line, mut c) in &mut colours {
        if let Some(a) = notices.active.get(line.0) {
            c.color = colour(&theme, a.severity).with_alpha(a.alpha());
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
        commands.entity(root).with_child(text(&theme, s, theme.small_size, c));
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

#[allow(clippy::too_many_arguments)]
fn draw_tip(
    hover: Res<TipHover>,
    tips: Query<&Tooltip>,
    strings: Option<Res<crate::ui::i18n::Strings>>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    ui_scale: Option<Res<UiScale>>,
    measured: Option<Single<&ComputedNode, With<TipBox>>>,
    mut boxed: Single<&mut Node, With<TipBox>>,
    mut words: Single<&mut SpriteText, With<TipText>>,
) {
    let shown = hover.target.and_then(|e| tips.get(e).ok()).and_then(|t| t.text_at(hover.rest));
    let shown = shown.map(|text| strings.as_ref().map_or(text, |s| s.tooltip_text(text)))
        .filter(|text| !text.is_empty());
    let Some(shown) = shown else {
        boxed.display = Display::None;
        return;
    };
    if words.text != shown {
        words.text = shown.to_owned();
    }
    boxed.display = Display::Flex;
    if let Some(w) = window
        && let Some(p) = w.cursor_position()
    {
        // The cursor, the window and the measured box are in logical window pixels, while a `Val::Px`
        // is multiplied by `UiScale` when the UI is laid out: place in window pixels, then divide.
        let k = ui_scale.map_or(1.0, |s| s.0).max(1e-3);
        let size = measured.map_or(Vec2::ZERO, |c| tip_size(&c, k));
        let at = place_tip(p, size, Vec2::new(w.width(), w.height()), k);
        boxed.left = Val::Px(at.x / k);
        boxed.top = Val::Px(at.y / k);
        boxed.right = Val::Auto;
    }
}

/// The box's size in logical window pixels. `ComputedNode::size` is physical; its `inverse_scale_factor` is
/// 1 / (window scale factor x `UiScale`) (bevy_ui 0.20 `update_ui_context_system` and `ui_layout_system`), so
/// physical x inverse is in UI units, and `UiScale` (`k`) takes it to window pixels.
pub(crate) fn tip_size(node: &ComputedNode, k: f32) -> Vec2 {
    node.size() * node.inverse_scale_factor() * k
}

/// Where the tooltip's top-left corner goes, in logical window pixels. STA64's rule (quality of life;
/// no sheet of any install states a tooltip placement): below and right of the pointer, `k` being the
/// UI scale that the offsets (UI units) are multiplied by; shifted inside the window when it would cross an
/// edge; flipped above or left of the pointer when it would run off the bottom or right edge, and when
/// the shift alone would leave it over the pointer. Only the position changes, never the box.
pub(crate) fn place_tip(cursor: Vec2, size: Vec2, window: Vec2, k: f32) -> Vec2 {
    let (dx, dy, gap) = (16.0 * k, 20.0 * k, 8.0 * k);
    let mut at = cursor + Vec2::new(dx, dy);
    if at.x + size.x > window.x {
        at.x = cursor.x - gap - size.x;
    }
    if at.y + size.y > window.y {
        at.y = cursor.y - gap - size.y;
    }
    let inside = |at: Vec2| Vec2::new(at.x.clamp(0.0, (window.x - size.x).max(0.0)), at.y.clamp(0.0, (window.y - size.y).max(0.0)));
    let covers = |at: Vec2| cursor.x >= at.x && cursor.x <= at.x + size.x && cursor.y >= at.y && cursor.y <= at.y + size.y;
    let shifted = inside(at);
    if !covers(shifted) {
        return shifted;
    }
    // The window leaves no room on the flipped side: try the other sides of the pointer.
    [
        Vec2::new(cursor.x + dx, cursor.y - gap - size.y),
        Vec2::new(cursor.x - gap - size.x, cursor.y + dy),
        Vec2::new(cursor.x - gap - size.x, cursor.y - gap - size.y),
        Vec2::new(cursor.x + dx, cursor.y + dy),
    ]
    .into_iter()
    .map(inside)
    .find(|c| !covers(*c))
    .unwrap_or(shifted)
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
            .add_systems(Update, (events_to_notices.after(crate::ui::presentation::consume), collect, age, retheme, (draw_notices, draw_chat, draw_tip)).chain().in_set(GameSet::Ui).after(ProfileSync));
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
        app.world_mut().query::<&SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect()
    }

    #[test]
    fn no_notice_is_raised_during_a_cineractive() {
        let mut app = rig();
        app.init_resource::<crate::ui::presentation::CineractiveState>();
        app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = true;
        for _ in 0..10 {
            app.update();
        }
        let shown: Vec<_> = texts(&mut app).into_iter().filter(|t| !t.is_empty()).collect();
        assert!(shown.is_empty(), "{shown:?}");
        assert!(app.world().resource::<ChatLog>().lines.is_empty());
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
    fn name_of_prefers_the_unit_name_caption_over_the_odf_name() {
        let unit = |odf: &str, text: &str| {
            let mut def = sta_content::UnitDef::new(odf, "cruiser");
            def.raw = std::sync::Arc::new(sta_odf::resolve("unit", &mut |_| Some(text.to_owned())).unwrap());
            def
        };
        let db = sta_content::ContentDb::builder()
            .unit(unit("fx_captioned", "[GameObjectClass]\nunitName = \"Test Cruiser\"\n"))
            .unit(unit("fx_plain", "[GameObjectClass]\nclassLabel = \"cruiser\"\n"))
            .build()
            .unwrap();
        let view = |odf: &str| UnitView {
            id: sta_sim::EntityId(1),
            def: db.key(odf).unwrap(),
            owner: PlayerId(0),
            pos: sta_sim::Vec3::ZERO,
            heading: (sta_sim::Fixed::ZERO, sta_sim::Fixed::ONE),
            speed: sta_sim::Fixed::ZERO,
            hull: [sta_sim::Fixed::ONE; 2],
            shields: [[sta_sim::Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None,
            autonomy: None,
            flags: sta_sim::view::UnitFlags::default(),
            progress: None,
        };
        let res = ContentRes(std::sync::Arc::new(db.clone()));
        assert_eq!(name_of(Some(&res), &view("fx_captioned")), "Test Cruiser");
        assert_eq!(name_of(Some(&res), &view("fx_plain")), "fx_plain");
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

    fn rect(at: Vec2, size: Vec2) -> Rect {
        Rect::from_corners(at, at + size)
    }

    /// Authored sizes only: a box of 20 text units wide and a few lines tall, at the three window
    /// aspects, with the pointer at every edge and corner, centre and a spot far from the edges.
    /// The placement does not depend on the HUD fit (centred or stretched): it sees window pixels only.
    #[test]
    fn tooltip_stays_inside_the_window_and_off_the_pointer() {
        for (w, h) in [(1440.0_f32, 1080.0_f32), (1920.0, 1080.0), (2520.0, 1080.0), (3440.0, 1440.0)] {
            let k = h / 1080.0;
            let window = Vec2::new(w, h);
            for size in [Vec2::new(300.0, 40.0), Vec2::new(300.0, 160.0)].map(|s| s * k) {
                let xs = [0.0, 1.0, w * 0.5, w - 1.0, w];
                let ys = [0.0, 1.0, h * 0.5, h - 1.0, h];
                for x in xs {
                    for y in ys {
                        let cursor = Vec2::new(x, y);
                        let at = place_tip(cursor, size, window, k);
                        let r = rect(at, size);
                        let why = format!("{w}x{h} cursor {cursor:?} size {size:?} at {at:?}");
                        assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= w && r.max.y <= h, "outside: {why}");
                        assert!(!(r.min.x < cursor.x && cursor.x < r.max.x && r.min.y < cursor.y && cursor.y < r.max.y), "covers the pointer: {why}");
                    }
                }
            }
        }
    }

    /// The measuring path: a node laid out by bevy_ui carries its physical size and
    /// `inverse_scale_factor = 1 / (window scale x UiScale)`; its extent in window pixels must come out as
    /// UI units x UiScale, and the placed box must lie inside the window.
    #[test]
    fn measured_box_lies_inside_the_window_at_any_scale() {
        for ws in [1.0_f32, 2.0] {
            for k in [1.0_f32, 1.333, 2.0] {
                let ui_units = Vec2::new(300.0, 120.0);
                let node = ComputedNode { size: ui_units * ws * k, inverse_scale_factor: 1.0 / (ws * k), ..default() };
                let size = tip_size(&node, k);
                assert!((size - ui_units * k).abs().max_element() < 1e-3, "ws {ws} k {k}: {size:?}");
                let window = Vec2::new(2520.0, 1080.0) * k;
                for cursor in [window - Vec2::splat(2.0), Vec2::new(window.x - 2.0, window.y * 0.5), Vec2::new(window.x * 0.5, window.y - 2.0), Vec2::new(10.0, window.y - 2.0)] {
                    let at = place_tip(cursor, size, window, k);
                    assert!(at.x >= 0.0 && at.y >= 0.0 && at.x + size.x <= window.x + 1e-3 && at.y + size.y <= window.y + 1e-3, "ws {ws} k {k} cursor {cursor:?}: {at:?}");
                    assert!(!(at.x < cursor.x && cursor.x < at.x + size.x && at.y < cursor.y && cursor.y < at.y + size.y), "covers pointer");
                }
            }
        }
    }

    #[test]
    fn tooltip_far_from_the_edges_sits_below_right_of_the_pointer() {
        // The unchanged case, the stock Armada II one: window 1440x1080 (4:3, UI scale 1).
        let at = place_tip(Vec2::new(400.0, 300.0), Vec2::new(300.0, 60.0), Vec2::new(1440.0, 1080.0), 1.0);
        assert_eq!(at, Vec2::new(416.0, 320.0));
    }

    #[test]
    fn tooltip_opens_upward_near_the_bottom_and_leftward_near_the_right_edge() {
        let window = Vec2::new(2520.0, 1080.0);
        let size = Vec2::new(300.0, 100.0);
        let up = place_tip(Vec2::new(500.0, 1060.0), size, window, 1.0);
        assert!(up.y + size.y <= 1060.0 && up.x == 516.0, "{up:?}");
        let left = place_tip(Vec2::new(2500.0, 500.0), size, window, 1.0);
        assert!(left.x + size.x <= 2500.0 && left.y == 520.0, "{left:?}");
    }

    #[test]
    fn tooltip_box_lands_beside_the_pointer_at_any_ui_scale() {
        use bevy::window::PrimaryWindow;
        for scale in [1.0_f32, 1.5] {
            let mut app = rig();
            app.insert_resource(UiScale(scale));
            let mut window = Window::default();
            window.resolution.set(2520.0, 1080.0);
            window.set_cursor_position(Some(Vec2::new(800.0, 400.0)));
            app.world_mut().spawn((window, PrimaryWindow));
            app.update();
            let e = app.world_mut().spawn(Tooltip::new("Metal")).id();
            *app.world_mut().resource_mut::<TipHover>() = TipHover { target: Some(e), rest: 5.0 };
            app.update();
            let node = app.world_mut().query_filtered::<&Node, With<TipBox>>().single(app.world()).unwrap().clone();
            let (Val::Px(left), Val::Px(top)) = (node.left, node.top) else { panic!("{node:?}") };
            // The node is laid out at left * scale window pixels: that is the pointer plus the offset.
            assert!((left * scale - (800.0 + 16.0 * scale)).abs() < 0.01, "scale {scale}: left {left}");
            assert!((top * scale - (400.0 + 20.0 * scale)).abs() < 0.01, "scale {scale}: top {top}");
        }
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
        let words = app.world_mut().query_filtered::<&SpriteText, With<TipText>>().single(app.world()).unwrap().text.clone();
        assert_eq!(words, "Metal");
    }

    #[test]
    fn tooltip_draw_resolves_short_verbose_and_reload_without_rebuilding_node() {
        use crate::ui::i18n::Strings;
        let mut app = rig();
        app.update();
        let e = app.world_mut().spawn(Tooltip::new(crate::ui::i18n::tooltip_key("Source tip")).verbose("GUI_DETAILS")).id();
        app.insert_resource(Strings::from_pairs("english", &[("Source tip", "Short tip"), ("GUI_DETAILS", "Detailed tip")]));
        let words = |app: &mut App| app.world_mut().query_filtered::<&SpriteText, With<TipText>>().single(app.world()).unwrap().text.clone();
        *app.world_mut().resource_mut::<TipHover>() = TipHover { target: Some(e), rest: TIP_SHORT_SECS };
        app.update();
        assert_eq!(words(&mut app), "Short tip");
        app.insert_resource(Strings::from_pairs("german", &[("Source tip", "Kurzer Hinweis"), ("GUI_DETAILS", "Detailed tip")]));
        app.update();
        assert_eq!(words(&mut app), "Kurzer Hinweis");
        app.world_mut().resource_mut::<TipHover>().rest = TIP_VERBOSE_SECS;
        app.update();
        assert_eq!(words(&mut app), "Detailed tip");
        app.insert_resource(Strings::from_pairs("german", &[("GUI_DETAILS", "Neue Details")]));
        app.update();
        assert_eq!(words(&mut app), "Neue Details");
        app.insert_resource(Strings::default());
        app.update();
        assert_eq!(words(&mut app), "GUI_DETAILS");
        app.insert_resource(Strings::from_pairs("english", &[("GUI_DETAILS", "")]));
        app.update();
        assert_eq!(app.world_mut().query_filtered::<&Node, With<TipBox>>().single(app.world()).unwrap().display, Display::None);
    }

    /// Installed assets are local-only. Exercise the exact stop-command key
    /// reported in a screenshot through mounted I18nPlugin and tooltip drawing.
    #[test]
    fn installed_command_tooltips_resolve_from_the_mounted_header() {
        if std::env::var("STA64_TOOLTIP_TESTS").as_deref() != Ok("1") {
            eprintln!("SKIP: set STA64_TOOLTIP_TESTS=1 STA64_TEST_INSTALL and optional STA64_TEST_MOD");
            return;
        }
        let install = std::env::var("STA64_TEST_INSTALL").expect("STA64_TEST_INSTALL");
        let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|s| !s.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let odf = sta_assets::a2fmt::odf::Odf::load_layered("stop.odf", |name, version| {
            index.load(&vfs, name, version).map(String::into_bytes)
        }).expect("installed stop command");
        let short = odf.get_str("tooltip").expect("stop tooltip").to_owned();
        let verbose = odf.get_str("verbose").expect("stop verbose tooltip").to_owned();
        assert_eq!(verbose, "GUI_CP_STOP_VTOOLTIP", "fixture must cover the reported key");
        let table = sta_content::strings::StringTable::load(&vfs);
        let expected = [short.as_str(), verbose.as_str()].map(|key| {
            let text = table.get(key).expect("installed header entry");
            assert!(!text.is_empty() && text != key, "fixture needs localized text for {key}");
            text.to_owned()
        });
        let mut app = rig();
        app.insert_resource(crate::viewer::Mounted(vfs))
            .add_plugins(crate::ui::i18n::I18nPlugin);
        app.update();
        let e = app.world_mut().spawn(Tooltip::new(short).verbose(verbose)).id();
        for (rest, expected) in [TIP_SHORT_SECS, TIP_VERBOSE_SECS].into_iter().zip(expected) {
            *app.world_mut().resource_mut::<TipHover>() = TipHover { target: Some(e), rest };
            app.update();
            let text = app.world_mut().query_filtered::<&Text, With<TipText>>().single(app.world()).unwrap();
            assert_eq!(text.0, expected);
            eprintln!("mounted stop tooltip ({rest}s): {}", text.0);
            assert_eq!(app.world_mut().query_filtered::<&Node, With<TipBox>>().single(app.world()).unwrap().display, Display::Flex);
        }
    }

    #[test]
    fn theme_switch_restyles_roots_without_losing_messages() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        configure_sets(&mut app);
        app.add_plugins(NoticePlugin);
        app.update();
        app.world_mut().write_message(Notice::new(Severity::Warn, "Shields low"));
        app.world_mut().write_message(ChatMsg { from: "Ada".into(), text: "Ready".into() });
        app.update();
        assert_eq!(app.world().resource::<Notices>().active.len(), 1);
        assert_eq!(app.world().resource::<ChatLog>().lines.len(), 2);

        let new_theme = crate::ui::profile::UiProfile::FleetOps.theme();
        app.world_mut().insert_resource(new_theme.clone());
        app.update();
        assert_eq!(app.world().resource::<Notices>().active[0].text, "Shields low");
        assert_eq!(app.world().resource::<ChatLog>().lines.len(), 2);
        let tip = app.world_mut().query_filtered::<(&Node, &BackgroundColor, &BorderColor), With<TipBox>>().single(app.world()).unwrap();
        assert_eq!(tip.0.padding.left, Val::Px(new_theme.pad));
        assert_eq!(tip.1.0, new_theme.panel);
        assert_eq!(tip.2.top, new_theme.accent2);
        let notice_root = app.world_mut().query_filtered::<&Node, With<NoticeRoot>>().single(app.world()).unwrap();
        assert_eq!(notice_root.row_gap, Val::Px(new_theme.gap * 0.5));
        let lines = app.world_mut().query_filtered::<&SpriteText, With<NoticeLine>>().iter(app.world()).map(|t| (t.text.clone(), t.color)).collect::<Vec<_>>();
        assert_eq!(lines, vec![("Shields low".to_owned(), new_theme.warn)]);
    }
}
