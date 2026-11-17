//! What a mission shows the player, and the answers it waits for (UI-30).
//!
//! A mission raises `SimEvent::Present(Presentation)` (see `sta_sim::present`); the
//! client consumes it here (text and fades), in `input/camera_cinematic.rs` (the
//! camera) and in `media/mission.rs` (movies, MED-03). Nothing in `ui/hud/` is
//! touched; the objectives list is an overlay of the UI kit, outside the HUD.
//!
//! | request | shown here | evidence |
//! |---|---|---|
//! | `Title`, `RemoveTitle`, A1 `ShowTitle` / `ShowSubtitle` placement, A2 `ShowTitle` / `ShowSubtitle` placement (x < 0 centred), A2 `UI.addTitle` / `addSubtitle` with x, y, seconds | text at a canvas position, until its seconds run out or it is removed | A2 canvas 1600x1200 and the default places (50,50) / (50,1000) from Fleet Ops Roots' Lua, A1 640x480 rect from the A1 mission DLLs; title colour: typed titles blue, subtitles white (read from the executable, static); stock A2 title font **not measured** |
//! | `Fade` | a full-screen black rectangle, alpha ramped over `seconds` | `ScreenFade` is a black rectangle in the original; the A1 argument meanings beyond seconds **not measured** |
//! | `Cineractive { skippable }` | state in [`CineractiveState`]; Space skips a skippable one ([`ScriptReply::CineractiveSkipped`]: the script's cancel), caps release it ([`ScriptReply::CineractiveReleased`]); the camera is `camera_cinematic` (UI-36 shot model) | `cineractiveCancel` (A2) / `switch_camera_shot` (A1) is Space in `Input.map`; no letterbox exists in the original's data or executables, so none is drawn |
//! | objectives and briefing (host state, not a `Presentation`) | the modal panel of [`ObjectivesView`], toggled with Alt+O | rectangles of `objectivesPanelArea` and the text areas in A2 stock `gui_glob16x12.cfg`, the A1 `gui_glob.cfg` panel; the completed-line tick **not measured** |
//! | a message box ([`MessageBox`]) | the same panel with an OK button | the original's message display reuses the objectives keys |
//! | `Movie`, `StopMovie` | MED-03; an in-panel movie goes to the HUD's cinematic window or viewscreen (UI-39, `ui/hud/cinematic.rs`) | the Armada II call raises the cinematic view (executable, static); the Armada I clip's rectangle inside the bridge scene is **not measured** |
//! | A1 `PreLoadBridgeScene`, `SetBridgeSceneRace`, `ShowBridgeScene`, `ShowBridgeOnScreen` | typed as [`BridgeScene`] and kept in [`BridgeState`]; the bridge backdrop and layers are not drawn (UI-39: the clip plays in the HUD's viewscreen) | race number 1 to 4 from the stock scripts; what `ShowBridgeOnScreen` selects is **not measured** |
//! | `Sound`, `StopSound` | [`PlayScriptSound`] (its own UI-bus stream, mixed with what plays, no queue or cut) / [`StopScriptSound`] (that stream alone; a stale id is ignored) (the audio lane resolves the name); the script's `IsAudioMessageDone` turns true when the clip's length has played ([`ScriptReply::SoundDone`], from `audio::SoundStarted`), at once if nothing plays it | clip length from the decoded file; the wait is measured for A1 only, for A2 and Fleet Ops **not measured** |
//! | `Music`, `StopMusic`, `Effect`, `WaitForKey`, other `Call`s | **not shown** (audio and effects lanes); each kind is listed once in [`Unhandled`] | |
//!
//! While a cineractive is on, the HUD and the notices are hidden and the fog layers are not drawn (the cinematic view's render list holds no HUD panel and the terrain pass draws fog and shroud only in the RTS view: Refit, read from the executable, static), and its end clears the fade.
//!
//! [`PresentationLog`] (opt in by inserting it) records every presentation request with the sim tick it was consumed on, for tests and mission timeline tools.
//!
//! **Answers.** The script learns that a movie ended, a message was dismissed or a
//! cineractive skipped through [`ScriptReply`] messages. There is no command kind to
//! carry them through the shared command stream (the "presentation done" command the
//! `WaitForKey` doc names was never added), so the host that runs the mission reads
//! them and calls the script (A1 `MovieDone()`, `AudioMessageDone(id)`); those calls
//! are not replay-safe until a seam PR adds the command. Reported in the UI-30 PR.

use std::collections::{BTreeMap, HashSet, VecDeque};

use bevy::prelude::*;
use bevy::time::Real;
use sta_mission::api::{Objective, ObjectiveBriefing, ObjectiveState};
use sta_sim::SimEvent;
use sta_sim::present::{Arg, Presentation};

use crate::app::GameSet;
use crate::media::{MovieFinished, MovieId, mission::Requested};
use crate::sim_link::{PlayScriptSound, SimMsg, StopScriptSound};
use crate::ui::i18n::Strings;
use crate::ui::sprite_font::{SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::{ButtonClicked, Theme, UiButton, button, font_role};

/// A1's bridge-scene calls as typed events (the sim raises them as untyped `Presentation::Call`s).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum BridgeScene {
    /// `PreLoadBridgeScene(type)`: load a race's bridge ahead of showing it.
    Preload { race: i32 },
    /// `SetBridgeSceneRace(type)`: the race of the bridge to show.
    SetRace { race: i32 },
    /// `ShowBridgeScene(flag)`: the bridge scene is shown or hidden.
    Show(bool),
    /// `ShowBridgeOnScreen(flag)`: the viewscreen of the scene is on or off.
    OnScreen(bool),
}

impl BridgeScene {
    /// The typed form of an `A1.<bridge call>` presentation call, `None` for any other call or
    /// for arguments of the wrong kind.
    pub fn from_call(name: &str, args: &[Arg]) -> Option<Self> {
        match (name, args) {
            ("A1.PreLoadBridgeScene", [Arg::Int(n), ..]) => Some(Self::Preload { race: *n as i32 }),
            ("A1.SetBridgeSceneRace", [Arg::Int(n), ..]) => Some(Self::SetRace { race: *n as i32 }),
            ("A1.ShowBridgeScene", [Arg::Bool(b)]) => Some(Self::Show(*b)),
            ("A1.ShowBridgeOnScreen", [Arg::Bool(b)]) => Some(Self::OnScreen(*b)),
            _ => None,
        }
    }
}

/// What the bridge calls last said. Nothing draws the bridge backdrop yet; the state is for the
/// HUD and tests to read.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct BridgeState {
    pub preloaded: Option<i32>,
    pub race: Option<i32>,
    pub shown: bool,
    pub on_screen: bool,
}

fn bridge_state(mut events: MessageReader<BridgeScene>, mut state: ResMut<BridgeState>) {
    for e in events.read() {
        match e {
            BridgeScene::Preload { race } => state.preloaded = Some(*race),
            BridgeScene::SetRace { race } => state.race = Some(*race),
            BridgeScene::Show(b) => state.shown = *b,
            BridgeScene::OnScreen(b) => state.on_screen = *b,
        }
    }
}

/// A2's layout space (stock `screenWidth` / `screenHeight` of `gui_glob16x12.cfg`).
pub const A2_CANVAS: Vec2 = Vec2::new(1600.0, 1200.0);
/// An A1 title fades in over this many seconds when the script's flag is set (all stock titles set it;
/// read from the executable, static; the unit of the engine's step is inferred, not measured).
pub const A1_TITLE_FADE_SECS: f32 = 1.0;
/// Typed titles (`ShowTitle`, both games) are drawn in this colour, hard-coded in each
/// executable (read from the executable, static); subtitles are white. Title colour and font should
/// become profile or mod data (TODO.md).
pub const TITLE_COLOUR: [f32; 3] = [0.0, 0.5, 1.0];

/// The text colour of a title or subtitle, before any fade level.
pub fn title_colour(subtitle: bool) -> [f32; 3] {
    if subtitle { [1.0; 3] } else { TITLE_COLOUR }
}

/// Where A1's `ShowTitle(x, y, w, h)` puts the text, in 480-unit-high canvas units from the window's
/// top-left: centred in the rectangle by the width and height the default font would give it
/// (5 units a letter and one between letters, 10 high; read from the executable, static). The stock title's size and font are not measured (a screenshot suggests about twice this, unconfirmed); ours uses the theme's heading size. The
/// window's aspect does not enter: the units are fractions of its height, so a wide window shows the
/// same margins in proportion to its height, with no 4:3 box around it.
pub fn a1_title_at(rect: [f32; 4], text: &str) -> Vec2 {
    let measured: f32 = text.chars().map(|c| if c == '\'' { 2.0 } else if c == ' ' { 4.0 } else { 6.0 }).sum();
    Vec2::new(rect[0] + (rect[2] - 1.0 - measured) * 0.5, rect[1] + (rect[3] - 1.0 - 10.0) * 0.5)
}

/// A1's layout space.
pub const A1_CANVAS: Vec2 = Vec2::new(640.0, 480.0);
/// Where a title with no placement goes (Roots' `addTitle(.., 50, 50, ..)`).
pub const TITLE_AT: Vec2 = Vec2::new(50.0, 50.0);
/// Where a subtitle with no placement goes (Roots' `addSubtitle(.., 50, 1000, ..)`).
pub const SUBTITLE_AT: Vec2 = Vec2::new(50.0, 1000.0);

/// Alt+O opens and closes the objectives panel (`objectives_toggle` of `Input.map`).
pub const OBJECTIVES_KEY: KeyCode = KeyCode::KeyO;

/// Request kinds nobody consumed, each counted, the first one logged.
#[derive(Resource, Default, Debug)]
pub struct Unhandled {
    seen: BTreeMap<String, u32>,
}

#[allow(dead_code)] // `count` and `list`: the bench and tests read them
impl Unhandled {
    /// Counts one request of `what`; true (and a log line) the first time.
    pub fn note(&mut self, what: &str) -> bool {
        let n = self.seen.entry(what.to_owned()).or_insert(0);
        *n += 1;
        if *n == 1 {
            warn!("mission presentation: '{what}' is not shown yet");
        }
        *n == 1
    }

    pub fn count(&self, what: &str) -> u32 {
        self.seen.get(what).copied().unwrap_or(0)
    }

    /// Every kind seen with its count, in name order.
    pub fn list(&self) -> Vec<(&str, u32)> {
        self.seen.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }
}

/// An answer for the script.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptReply {
    /// A movie the mission asked for is over (skipped or ended).
    MovieDone { id: MovieId, name: String },
    /// The player closed message `id` of a [`MessageBox`].
    MessageDismissed { id: u32 },
    /// The player skipped the cineractive (the skip key): the script hears
    /// `isCineractiveCancel` and ends it itself; MMM ends a skippable one with
    /// `cineractiveFinish(true)`.
    CineractiveSkipped,
    /// The cineractive is ended for the script (a cap of [`skip_cineractive`], a script fault,
    /// the mission's end): the camera is the player's again whatever the script does.
    CineractiveReleased,
    /// A message the mission started with `Presentation::Sound { id, .. }` has played to its end
    /// (the clip's length, from the audio layer) or could not be played: the script's
    /// `IsAudioMessageDone(id)` reads true from here on.
    SoundDone { id: u32 },
}

/// The mission messages (`Presentation::Sound`) that have not ended yet, oldest first. A message
/// ends when its clip has played for its length (reported by the audio layer as
/// [`crate::audio::SoundStarted`]), is stopped by the script, or at once when there is no audio
/// layer or the clip does not resolve. `left` is the playing time still to run, known once the
/// audio layer has taken the request.
#[derive(Resource, Default, Debug)]
pub struct ScriptSounds {
    pending: Vec<(u32, String, Option<f32>)>,
    /// The sim tick last seen, to notice a restart in place (the tick going back).
    last_tick: u64,
}

impl ScriptSounds {
    /// Notes the sim's tick; true when it went back, which is a restart of the mission in place.
    fn restarted(&mut self, tick: u64) -> bool {
        let back = tick < self.last_tick;
        self.last_tick = tick;
        back
    }

    /// Whether a script message or track is still playing: the event voices stay silent meanwhile
    /// ([`crate::audio`] voices).
    pub fn any_playing(&self) -> bool {
        !self.pending.is_empty()
    }

    /// A message `id` playing, as a test of another module needs one without an audio layer.
    #[cfg(test)]
    pub(crate) fn start_for_test(&mut self, id: u32) {
        self.pending.push((id, String::new(), Some(60.0)));
    }

    /// The ids of the messages still playing (for tests).
    #[cfg(test)]
    pub fn playing(&self) -> Vec<u32> {
        self.pending.iter().map(|p| p.0).collect()
    }
}

/// A message for the player, with an OK button: the host sends it.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct MessageBox {
    pub id: u32,
    pub title: String,
    pub text: String,
}

/// How long (real seconds) a cineractive the script made non-skippable holds the player before
/// Space or Esc may end it anyway. A skippable one ends at once.
pub const SKIP_GRACE_SECS: f32 = 10.0;
/// The longest a cineractive may hold the player (real seconds): then it ends by itself, so a
/// script that never ends it (a lost reply, an aborted callback, a path of speed 0) cannot leave
/// the player without camera and input for good. The longest stock intro read is A2 fed10's
/// first shot, 184 s to its end (shot model, static); Roots intros run under a minute.
pub const MAX_HOLD_SECS: f32 = sta_mission::cineractive::MAX_HOLD_SECS as f32;
/// After a skip, how long (real seconds) the script has to end the cineractive itself (A1 fed1
/// plays a 5 s zoom-out first) before it is released for it.
pub const AFTER_SKIP_SECS: f32 = 30.0;

/// How long the current cineractive has held the player.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct CineractiveClock {
    /// Real seconds a cineractive has held the player; kept across a brief gap, so a script that
    /// raises it again as soon as it ends does not earn a fresh grace and cap each time.
    pub held: f32,
    /// How long no cineractive has been on.
    off_for: f32,
    /// Real seconds since the player skipped this cineractive, if they did.
    pub skipped_for: Option<f32>,
}

/// A cineractive that has been off this long (real seconds) counts as over: the next one starts a new hold.
pub const NEW_HOLD_AFTER_SECS: f32 = 5.0;

/// A host-level record of the mission's presentation requests, in arrival order, with the sim
/// tick of the view that carried them. Opt in: insert the resource (tests, timeline tools).
#[derive(Resource, Default, Debug, Clone)]
pub struct PresentationLog(pub Vec<(u64, Presentation)>);

/// Cineractive mode as the script set it.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CineractiveState {
    pub on: bool,
    pub skippable: bool,
}

/// The one "a cutscene is running" predicate: the announcement and acknowledgement voices
/// (`audio::voices`), the "construction complete" advisor lines (`audio::sfx`), the event
/// log's notices (`ui::notices::events_to_notices`) and the player's selection and orders
/// (`input`, the HUD's buttons) all ask it, so those cannot disagree. (The notice and HUD
/// roots' visibility follow `CineractiveState.on` directly, in `hide_notices_in_cineractive`
/// and `game::flow::show_match_ui`.) True while a script's cineractive is on.
/// Fleet Operations Roots' intros declare theirs the same way (`Cineractive.active`), so no
/// separate input-lock call exists to honour; whether the original also locks selection is
/// not measured (Cedric's playtest reports ships selectable and speaking in that intro).
pub fn cutscene_running(cine: Option<Res<CineractiveState>>) -> bool {
    cine.is_some_and(|c| c.on)
}

/// Event voice lines (unit acknowledgements and alerts, advisor lines) are dropped, not queued,
/// while a script's audio plays (any `AudioMessage` or `PlayMusic` stream), the cineractive view
/// is on, or a movie plays. Read from the Armada II executable, static: the event audio queue
/// refuses a request in each of those states. Applied to Armada I and the Fleet Operations family
/// as well: not established there. Not measured on the running original.
pub fn event_voices_silenced(cutscene: bool, sounds: Option<&ScriptSounds>, movies: Option<&crate::media::Movies>) -> bool {
    cutscene || sounds.is_some_and(ScriptSounds::any_playing) || movies.is_some_and(crate::media::Movies::busy)
}

/// Where a title goes: the canvas it is placed in and its top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub canvas: Vec2,
    pub at: Vec2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TitleItem {
    pub id: u32,
    pub text: String,
    pub subtitle: bool,
    pub place: Place,
    /// Seconds left; `None` until removed.
    pub left: Option<f32>,
    /// An A1 title's rectangle `[x, y, w, h]` on the 640x480 canvas, and seconds shown.
    pub a1_rect: Option<[f32; 4]>,
    /// An A2 `ShowTitle` / `ShowSubtitle` with a negative x: centred across the screen at `place.at.y`.
    pub centred: bool,
    /// An A1 title's fade level (RGB scale) when it is held: 0 to 1 over [`A1_TITLE_FADE_SECS`]
    /// while it fades in (the script's flag), then held; `age` counts seconds into the fade in.
    pub age: f32,
    /// Seconds since `RemoveTitle(id, true)` began the fade out, which takes the level down at
    /// the same rate and drops the title when it reaches 0 (read from the executable, static).
    pub out: Option<f32>,
}

impl TitleItem {
    /// The colour scale of an A1 title now: the fade in's level less what the fade out took.
    pub fn level(&self) -> f32 {
        (self.age.min(A1_TITLE_FADE_SECS) / A1_TITLE_FADE_SECS - self.out.unwrap_or(0.0) / A1_TITLE_FADE_SECS).max(0.0)
    }
}

/// The titles and subtitles on screen.
#[derive(Resource, Default, Debug)]
pub struct Titles {
    pub items: Vec<TitleItem>,
    dirty: bool,
}

impl Titles {
    /// Take every title down (the layer redraws).
    pub fn clear(&mut self) {
        self.items.clear();
        self.dirty = true;
    }

    fn add(&mut self, item: TitleItem) {
        self.items.retain(|t| t.id != item.id);
        self.items.push(item);
        self.dirty = true;
    }

    /// `RemoveTitle(id, true)`: fade out, then go. A title already going stays on its course.
    fn fade_out(&mut self, id: u32) {
        if let Some(t) = self.items.iter_mut().find(|t| t.id == id && t.out.is_none()) {
            t.out = Some(0.0);
            self.dirty = true;
        }
    }

    fn remove(&mut self, id: u32) {
        let n = self.items.len();
        self.items.retain(|t| t.id != id);
        self.dirty |= n != self.items.len();
    }

    fn tick(&mut self, dt: f32) {
        let n = self.items.len();
        for t in &mut self.items {
            if let Some(l) = &mut t.left {
                *l -= dt;
            }
        }
        self.items.retain(|t| t.left.is_none_or(|l| l > 0.0));
        self.dirty |= n != self.items.len();
        for t in self.items.iter_mut().filter(|t| t.a1_rect.is_some()) {
            if let Some(o) = &mut t.out {
                *o += dt;
                self.dirty = true;
            } else if t.age < A1_TITLE_FADE_SECS {
                t.age += dt;
                self.dirty = true;
            }
        }
        let n = self.items.len();
        self.items.retain(|t| t.out.is_none() || t.level() > 0.0);
        self.dirty |= n != self.items.len();
    }
}

/// The screen fade: alpha of black now, and where it is going.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct FadeState {
    pub alpha: f32,
    target: f32,
    rate: f32,
    pub rgb: [u8; 3],
    /// A fade was asked for since the cineractive began.
    touched: bool,
}

impl FadeState {
    fn start(&mut self, out: bool, seconds: f32, rgb: [u8; 3]) {
        self.rgb = rgb;
        self.target = if out { 1.0 } else { 0.0 };
        if seconds <= 0.0 {
            self.alpha = self.target;
            self.rate = 0.0;
        } else {
            // Runs on from the alpha it is at, so a fade never flashes the other way first.
            self.rate = 1.0 / seconds;
        }
    }

    fn tick(&mut self, dt: f32) {
        if self.rate > 0.0 {
            let step = self.rate * dt;
            self.alpha = if self.target > self.alpha { (self.alpha + step).min(self.target) } else { (self.alpha - step).max(self.target) };
            if self.alpha == self.target {
                self.rate = 0.0;
            }
        }
    }
}

/// Movie requests of the mission not yet reported over.
#[derive(Resource, Default)]
struct PendingMovies(HashSet<MovieId>);

/// The objectives panel: the host fills it from the mission and shows it.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct ObjectivesView {
    pub visible: bool,
    pub title: String,
    pub briefing: String,
    pub items: Vec<ObjectiveLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveLine {
    pub text: String,
    pub state: ObjectiveState,
}

#[allow(dead_code)] // `fill`: the host that runs the mission calls it
impl ObjectivesView {
    /// The panel's content from the mission runtime's objectives and file-backed
    /// briefing (`MissionRuntime::objectives`, `objective_briefing`); visibility kept.
    pub fn fill(&mut self, objectives: &[Objective], briefing: Option<&ObjectiveBriefing>) {
        self.title = briefing.map(|b| b.title.clone()).unwrap_or_default();
        self.briefing = briefing.map(|b| b.text.clone()).unwrap_or_default();
        self.items = objectives
            .iter()
            .map(|o| ObjectiveLine { text: if o.description.is_empty() { o.title.clone() } else { format!("{}: {}", o.title, o.description) }, state: o.state })
            .collect();
    }
}

/// Messages waiting for their OK; the front one is shown.
#[derive(Resource, Default)]
struct MessageQueue(VecDeque<MessageBox>);

fn text_of(strings: Option<&Strings>, text: &str) -> String {
    // A string id resolves in the mounted table; anything else is shown as given.
    strings.map_or(text, |s| s.game_text(text)).to_owned()
}

fn place_of(subtitle: bool) -> Place {
    Place { canvas: A2_CANVAS, at: if subtitle { SUBTITLE_AT } else { TITLE_AT } }
}

fn num(a: &Arg) -> Option<f32> {
    match a {
        Arg::Num(n) => Some(n.to_f32()),
        Arg::Int(i) => Some(*i as f32),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn consume(
    mut events: MessageReader<SimMsg>,
    strings: Option<Res<Strings>>,
    mut titles: ResMut<Titles>,
    mut fade: ResMut<FadeState>,
    mut cineractive: ResMut<CineractiveState>,
    mut clock: ResMut<CineractiveClock>,
    mut unhandled: ResMut<Unhandled>,
    mut play: MessageWriter<PlayScriptSound>,
    mut stop: MessageWriter<StopScriptSound>,
    mut log: Option<ResMut<PresentationLog>>,
    link: Option<Res<crate::sim_link::SimLink>>,
    mut sounds: ResMut<ScriptSounds>,
    reports: Option<Res<crate::audio::ReportsSoundLengths>>,
    mut replies: MessageWriter<ScriptReply>,
    mut bridge: MessageWriter<BridgeScene>,
    mut mute: ResMut<crate::audio::SoundMute>,
) {
    for SimMsg(e) in events.read() {
        let SimEvent::Present(p) = e else { continue };
        if let Some(log) = log.as_mut() {
            log.0.push((link.as_deref().map_or(0, |l| l.curr().tick), p.clone()));
        }
        match p {
            Presentation::Title { id, text, subtitle, seconds } => titles.add(TitleItem {
                id: *id,
                text: text_of(strings.as_deref(), text),
                subtitle: *subtitle,
                place: place_of(*subtitle),
                left: seconds.map(|s| s.to_f32()),
                a1_rect: None,
                centred: false,
                age: 0.0,
                out: None,
            }),
            Presentation::RemoveTitle { id } => titles.remove(*id),
            Presentation::Fade { out, seconds, rgb } => {
                // The first fade a cineractive asks for, when it is a fade in, comes up from black:
                // the stock openings call `fadeIn` at their first tick and begin black, so nothing of
                // the RTS view shows before the first shot (the original's start colour is inferred:
                // not measured; a fade in on a screen already faded out runs from there as before).
                // An instant fade in on a clear screen (a script clearing a held fade) is neither a
                // first fade nor a start from black.
                let instant_clear = !*out && seconds.to_f32() <= 0.0 && fade.alpha == 0.0;
                if !*out && cineractive.on && !fade.touched && fade.alpha == 0.0 && !instant_clear {
                    fade.alpha = 1.0;
                }
                fade.touched |= !instant_clear;
                fade.start(*out, seconds.to_f32(), *rgb);
            }
            Presentation::Cineractive { on, skippable } => {
                // A new cineractive (even one started in the frame the last one ended) can be skipped anew.
                if !(*on && cineractive.on) {
                    clock.skipped_for = None;
                }
                // The fade is the cinematic view's own (`ScreenFade` is in that view's render list
                // only, read from the executable by Refit, static): the view that comes back
                // starts clear, so a script that fades out and ends the cineractive in the same
                // breath (the Roots missions do) hands over a visible game, not a black one.
                if !*on && cineractive.on {
                    *fade = FadeState::default();
                }
                *cineractive = CineractiveState { on: *on, skippable: *on && *skippable };
            }
            // A2 `UI.addTitle(text, x, y, seconds, flag)`: the placement the typed form lacks.
            Presentation::Call { name, args } if name == "UI.addTitle" || name == "UI.addSubtitle" => {
                let subtitle = name == "UI.addSubtitle";
                if let [Arg::Int(id), Arg::Text(text), x, y, secs, _] = args.as_slice()
                    && let (Some(x), Some(y), Some(secs), Ok(id)) = (num(x), num(y), num(secs), u32::try_from(*id))
                {
                    titles.add(TitleItem { id, text: text_of(strings.as_deref(), text), subtitle, place: Place { canvas: A2_CANVAS, at: Vec2::new(x, y) }, left: (secs > 0.0).then_some(secs), a1_rect: None, centred: false, age: 0.0, out: None });
                } else {
                    unhandled.note(name);
                }
            }
            // A1 `ShowTitle(x, y, w, h, text, flag)`, queued after the typed title with its id.
            Presentation::Call { name, args } if name == "A1.ShowTitle" || name == "A1.ShowSubtitle" => {
                if let [Arg::Int(id), x, y, ..] = args.as_slice()
                    && let (Some(x), Some(y), Ok(id)) = (num(x), num(y), u32::try_from(*id))
                    && let Some(t) = titles.items.iter_mut().find(|t| t.id == id)
                {
                    t.place = Place { canvas: A1_CANVAS, at: Vec2::new(x, y) };
                    if let [_, _, _, Arg::Int(w), Arg::Int(h), flag] = args.as_slice() {
                        t.a1_rect = Some([x, y, *w as f32, *h as f32]);
                        // The flag: fade in over a second (true) or show at once (false).
                        t.age = if matches!(flag, Arg::Bool(false)) { A1_TITLE_FADE_SECS } else { 0.0 };
                    }
                    titles.dirty = true;
                } else {
                    unhandled.note(name);
                }
            }
            // A2 `ShowTitle(id, x, y, fade)` / `ShowSubtitle`, queued after the typed title with its id:
            // the canvas place; a negative x is centred (read from the executable, static). The
            // fade flag and the title font are not applied (not measured); wrapped lines
            // of a centred title are not re-centred; the title layer is under the fade overlay.
            Presentation::Call { name, args } if name == "A2.ShowTitle" || name == "A2.ShowSubtitle" => {
                if let [Arg::Int(id), x, y, ..] = args.as_slice()
                    && let (Some(x), Some(y), Ok(id)) = (num(x), num(y), u32::try_from(*id))
                    && let Some(t) = titles.items.iter_mut().find(|t| t.id == id)
                {
                    t.centred = x < 0.0;
                    t.place = Place { canvas: A2_CANVAS, at: Vec2::new(x.max(0.0), y) };
                    titles.dirty = true;
                } else {
                    unhandled.note(name);
                }
            }
            // A1 `RemoveTitle(id, true)` / `RemoveSubtitle(id, true)`: the 1 s fade out (the plain
            // form is the typed `RemoveTitle`, which goes at once).
            Presentation::Call { name, args } if name == "A1.RemoveTitleFade" => {
                if let [Arg::Int(id)] = args.as_slice() && let Ok(id) = u32::try_from(*id) {
                    titles.fade_out(id);
                } else {
                    unhandled.note(name);
                }
            }
            // Shown elsewhere (movies, MED-03; the camera, camera_cinematic).
            Presentation::Movie { .. } | Presentation::StopMovie | Presentation::Shot { .. } | Presentation::CameraOffset { .. } => {}
            Presentation::Call { name, .. } if name == "Camera.center" => {}
            // A2 `MuteSounds(f)`: the master multiplier of everything but the script's own streams
            // (see [`crate::audio::SoundMute`]).
            Presentation::Call { name, args } if name == "A2.MuteSounds" => {
                if let Some(f) = args.first().and_then(num) {
                    mute.set(f);
                } else {
                    unhandled.note(name);
                }
            }
            Presentation::Call { name, args } if name.starts_with("A1.") && BridgeScene::from_call(name, args).is_some() => {
                bridge.write(BridgeScene::from_call(name, args).unwrap());
            }
            Presentation::Call { name, .. } if matches!(name.split_once('.').map_or("", |x| x.1), "CenterCamera" | "SetCameraZoom" | "CineractiveConstantSpeed" | "CineractiveCameraOffset" | "CineractiveShot") => {}
            // A2 `PlayMusic(script, file, dont_wait)`: the engine's audio overlay numbers music and
            // voice lines from one counter, and a wait on the track ends with it (read from the
            // executable, static). STA64 plays the track as one more script sound (the A1 route),
            // so the wait ends with the clip instead of never.
            Presentation::Call { name, args } if name == "A2.PlayMusic" => {
                if let [Arg::Int(id), Arg::Text(file)] = args.as_slice() && let Ok(id) = u32::try_from(*id) {
                    start_script_sound(id, file, &mut play, &mut sounds, reports.is_some(), &mut replies);
                } else {
                    unhandled.note(name);
                }
            }
            Presentation::Call { name, .. } => {
                unhandled.note(name);
            }
            // A script's voice or message sound goes to the audio layer by name (the resolver
            // finds `Data/sounds/...` by stem, events by name); the host answers the script.
            Presentation::Sound { id, name, .. } => {
                start_script_sound(*id, name, &mut play, &mut sounds, reports.is_some(), &mut replies);
            }
            // `StopAudioMessage(id)` stops that one stream, whatever else plays (read from the
            // executable, static: a stream per call, stopped by id). An id that is no longer
            // playing is ignored, so a stop of a finished (or never started) message cuts nothing.
            Presentation::StopSound { id } => {
                let live = sounds.pending.iter().any(|p| p.0 == *id);
                sounds.pending.retain(|p| p.0 != *id);
                if live {
                    stop.write(StopScriptSound { id: *id });
                }
            }
            Presentation::Music { .. } => {
                unhandled.note("Music");
            }
            Presentation::StopMusic => {
                unhandled.note("StopMusic");
            }
            Presentation::Effect { kind, .. } => {
                unhandled.note(&format!("Effect {kind}"));
            }
            Presentation::WaitForKey { .. } => {
                unhandled.note("WaitForKey");
            }
            _ => {
                unhandled.note("(a request kind newer than this client)");
            }
        }
    }
}

/// Starts a script's sound (a voice line, or an A2 music track) on the UI bus; the script's wait on
/// it ends when the clip does (`sound_replies`), at once when no audio layer says how long it is.
fn start_script_sound(
    id: u32,
    name: &str,
    play: &mut MessageWriter<PlayScriptSound>,
    sounds: &mut ScriptSounds,
    audio_layer: bool,
    replies: &mut MessageWriter<ScriptReply>,
) {
    // The script's own stream (the UI bus, not the voice bus, which drops a line while another
    // plays): it mixes with whatever plays and is stopped by its id alone.
    play.write(PlayScriptSound { id, name: name.to_owned() });
    if audio_layer {
        sounds.pending.push((id, name.to_owned(), None));
    } else {
        replies.write(ScriptReply::SoundDone { id });
    }
}

/// Ends the mission messages: the audio layer says how long each clip is when it takes the request
/// ([`crate::audio::SoundStarted`], matched by name in request order), and the message is done
/// that long after, in real time, as the player hears it. A clip that did not resolve ends at once.
fn sound_replies(
    mut started: MessageReader<crate::audio::SoundStarted>,
    time: Res<Time<Real>>,
    mut sounds: ResMut<ScriptSounds>,
    mut out: MessageWriter<ScriptReply>,
) {
    // Real time, as the player hears the clip: the audio is not paused with the game, and a paused
    // sim holds the script anyway.
    let dt = time.delta_secs();
    for p in &mut sounds.pending {
        if let Some(left) = p.2.as_mut() {
            *left -= dt;
        }
    }
    for s in started.read() {
        if s.bus != crate::sim_link::Bus::Ui {
            continue;
        }
        if let Some(p) = sounds.pending.iter_mut().find(|p| p.2.is_none() && p.1 == s.name) {
            p.2 = Some(s.seconds.unwrap_or(0.0));
        }
    }
    sounds.pending.retain(|(id, _, left)| {
        let done = left.is_some_and(|l| l <= 0.0);
        if done {
            out.write(ScriptReply::SoundDone { id: *id });
        }
        !done
    });
}

/// A restart of the mission in place (the sim's tick going back) drops the messages of the run
/// before it: their ids restart at 1, so a stale entry would end the new run's wait or stop the
/// new run's stream of the same id. What still plays is stopped, stream by stream.
fn clear_sounds_on_restart(link: Option<Res<crate::sim_link::SimLink>>, mut sounds: ResMut<ScriptSounds>, mut stop: MessageWriter<StopScriptSound>, mut mute: ResMut<crate::audio::SoundMute>) {
    let Some(link) = link else { return };
    if sounds.restarted(link.curr().tick) {
        // The new run's intro mutes (and un-mutes) for itself.
        *mute = default();
        for (id, _, _) in std::mem::take(&mut sounds.pending) {
            stop.write(StopScriptSound { id });
        }
    }
}

/// A skip or a release of the cineractive ends what the script was playing: the UI bus is cut
/// and every pending message is over for the script, so nothing plays on into the game.
fn end_messages_on_skip(
    mut replies: ParamSet<(MessageReader<ScriptReply>, MessageWriter<ScriptReply>)>,
    mut sounds: ResMut<ScriptSounds>,
    mut stop: MessageWriter<StopScriptSound>,
) {
    if !replies.p0().read().any(|r| matches!(r, ScriptReply::CineractiveSkipped | ScriptReply::CineractiveReleased)) {
        return;
    }
    for (id, _, _) in std::mem::take(&mut sounds.pending) {
        stop.write(StopScriptSound { id });
        replies.p1().write(ScriptReply::SoundDone { id });
    }
}

/// The notices are hidden over a cutscene (the HUD and radar roots follow the same state in
/// `game::flow::show_match_ui`, their single owner).
fn hide_notices_in_cineractive(cine: Res<CineractiveState>, mut roots: Query<&mut Visibility, With<crate::ui::notices::NoticeRoot>>) {
    let want = if cine.on { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut roots {
        v.set_if_neq(want);
    }
}

fn run_clock(time: Res<Time<Real>>, mut titles: ResMut<Titles>, mut fade: ResMut<FadeState>) {
    let dt = time.delta_secs();
    if !titles.items.is_empty() {
        titles.tick(dt);
    }
    if fade.rate > 0.0 {
        fade.tick(dt);
    }
}

fn movie_replies(mut requested: MessageReader<Requested>, mut finished: MessageReader<MovieFinished>, mut pending: ResMut<PendingMovies>, mut out: MessageWriter<ScriptReply>) {
    for r in requested.read() {
        pending.0.insert(r.id);
    }
    for f in finished.read() {
        if pending.0.remove(&f.id) {
            out.write(ScriptReply::MovieDone { id: f.id, name: f.name.clone() });
        }
    }
}

/// The rules by which a cineractive ends besides the script ending it (the script's own end is
/// `Presentation::Cineractive { on: false }`, handled in `consume`):
///
/// - Space or Esc on a skippable one is the player's skip ([`ScriptReply::CineractiveSkipped`]):
///   `cineractiveCancel` / A1 `switch_camera_shot` is Space in `Input.map`, and the original only
///   sets `isCineractiveCancel` for the script, which ends the cineractive itself; if it has not
///   after [`AFTER_SKIP_SECS`], it is released;
/// - a non-skippable one (MMM `skippable = false`) is released by Space or Esc after [`SKIP_GRACE_SECS`];
/// - any one is released after [`MAX_HOLD_SECS`];
/// - the host releases it when the script faults or the mission ends.
///
/// A release is a [`ScriptReply::CineractiveReleased`], which the camera and the script hear.
fn skip_cineractive(keys: Res<ButtonInput<KeyCode>>, time: Res<Time<Real>>, mut state: ResMut<CineractiveState>, mut clock: ResMut<CineractiveClock>, mut out: MessageWriter<ScriptReply>) {
    if !state.on {
        clock.skipped_for = None;
        clock.off_for += time.delta_secs();
        if clock.off_for >= NEW_HOLD_AFTER_SECS {
            clock.held = 0.0;
        }
        return;
    }
    let dt = time.delta_secs();
    clock.off_for = 0.0;
    clock.held += dt;
    if let Some(s) = clock.skipped_for.as_mut() {
        *s += dt;
    }
    let pressed = keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::Escape);
    let release = (pressed && !state.skippable && clock.held >= SKIP_GRACE_SECS) || clock.held >= MAX_HOLD_SECS || clock.skipped_for.is_some_and(|s| s >= AFTER_SKIP_SECS);
    if release {
        // `held` stays: a cineractive raised again at once is released again at once.
        *state = CineractiveState::default();
        clock.skipped_for = None;
        out.write(ScriptReply::CineractiveReleased);
    } else if pressed && state.skippable && clock.skipped_for.is_none() {
        clock.skipped_for = Some(0.0);
        out.write(ScriptReply::CineractiveSkipped);
    }
}

/// An end that came from outside (the host's fault or mission-end release) clears the state too.
pub(crate) fn apply_replies(mut replies: MessageReader<ScriptReply>, mut state: ResMut<CineractiveState>) {
    if replies.read().any(|r| *r == ScriptReply::CineractiveReleased) {
        *state = CineractiveState::default();
    }
}

fn toggle_objectives(keys: Res<ButtonInput<KeyCode>>, mut view: ResMut<ObjectivesView>) {
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    if alt && keys.just_pressed(OBJECTIVES_KEY) {
        view.visible = !view.visible;
    }
}

fn queue_messages(mut incoming: MessageReader<MessageBox>, mut queue: ResMut<MessageQueue>) {
    queue.0.extend(incoming.read().cloned());
}

// --- drawing ---------------------------------------------------------------

#[derive(Component)]
struct TitleLayer;

#[derive(Component)]
struct FadeOverlay;

#[derive(Component)]
struct PanelRoot;

#[derive(Component)]
struct OkButton;

fn spawn_layers(mut commands: Commands) {
    commands.spawn((TitleLayer, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, GlobalZIndex(900), Pickable::IGNORE));
    commands.spawn((
        FadeOverlay,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)),
        GlobalZIndex(950),
        Pickable::IGNORE,
    ));
}

fn draw_titles(mut commands: Commands, theme: Res<Theme>, mut titles: ResMut<Titles>, layer: Query<(Entity, Option<&Children>), With<TitleLayer>>) {
    if !titles.dirty {
        return;
    }
    titles.dirty = false;
    let Ok((root, kids)) = layer.single() else { return };
    for k in kids.into_iter().flatten() {
        commands.entity(*k).despawn();
    }
    for t in &titles.items {
        let size = if t.subtitle { theme.text_size } else { theme.heading_size };
        let p = t.place;
        let colour = title_colour(t.subtitle);
        if let Some(rect) = t.a1_rect {
            let at = a1_title_at(rect, &t.text);
            let b = t.level();
            let [r, g, bl] = title_colour(t.subtitle).map(|c| c * b);
            commands.entity(root).with_child((
                Node { position_type: PositionType::Absolute, left: Val::Vh(at.x / A1_CANVAS.y * 100.0), top: Val::Vh(at.y / A1_CANVAS.y * 100.0), ..default() },
                children![sprite_text_styled(t.text.clone(), size, Color::srgb(r, g, bl), SpriteTextStyle::role(font_role(&theme, size)).wrap_to_parent())],
            ));
            continue;
        }
        if t.centred {
            commands.entity(root).with_child((
                Node { position_type: PositionType::Absolute, left: Val::Percent(0.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, top: Val::Percent(p.at.y / p.canvas.y * 100.0), ..default() },
                children![sprite_text_styled(t.text.clone(), size, Color::srgb(colour[0], colour[1], colour[2]), SpriteTextStyle::role(font_role(&theme, size)).wrap_to_parent())],
            ));
            continue;
        }
        commands.entity(root).with_child((
            Node { position_type: PositionType::Absolute, left: Val::Percent(p.at.x / p.canvas.x * 100.0), top: Val::Percent(p.at.y / p.canvas.y * 100.0), max_width: Val::Percent(80.0), ..default() },
            children![sprite_text_styled(t.text.clone(), size, Color::srgb(colour[0], colour[1], colour[2]), SpriteTextStyle::role(font_role(&theme, size)).wrap_to_parent())],
        ));
    }
}

fn draw_fade(fade: Res<FadeState>, mut q: Query<&mut BackgroundColor, With<FadeOverlay>>) {
    if !fade.is_changed() {
        return;
    }
    let [r, g, b] = fade.rgb.map(|c| f32::from(c) / 255.0);
    for mut c in &mut q {
        c.0 = Color::srgba(r, g, b, fade.alpha);
    }
}

/// What the modal panel shows now: the front message, else the objectives.
#[derive(Debug, Clone, PartialEq)]
struct PanelContent {
    message: Option<u32>,
    title: String,
    body: String,
    lines: Vec<ObjectiveLine>,
}

fn panel_content(view: &ObjectivesView, queue: &MessageQueue) -> Option<PanelContent> {
    if let Some(m) = queue.0.front() {
        return Some(PanelContent { message: Some(m.id), title: m.title.clone(), body: m.text.clone(), lines: Vec::new() });
    }
    view.visible.then(|| PanelContent { message: None, title: view.title.clone(), body: view.briefing.clone(), lines: view.items.clone() })
}

fn fixed(strings: Option<&Strings>, key: &str, fallback: &str) -> String {
    strings.map(|s| s.game_text(key)).filter(|t| *t != key).unwrap_or(fallback).to_owned()
}

/// A heading as the game's string table defines it. Rule: with a game table loaded, a key the table lacks
/// means the game has no such heading and none is drawn (a game that does not define
/// `GUI_OD_BRIEFING_SUMMARY` shows no summary heading). With no game table (an empty or absent one) there is
/// nothing to say otherwise and `fallback` is used.
fn heading_text(strings: Option<&Strings>, key: &str, fallback: &str) -> Option<String> {
    match strings {
        Some(s) if s.has_game_table() => Some(s.game_text(key)).filter(|t| *t != key).map(str::to_owned),
        _ => Some(fallback.to_owned()),
    }
}

/// Whether the briefing text already closes with `heading` as a line of its own (case and a trailing colon
/// ignored), so the panel must not draw it a second time.
fn text_ends_with_heading(body: &str, heading: &str) -> bool {
    let norm = |t: &str| t.trim().trim_end_matches(':').trim_end().to_lowercase();
    let want = norm(heading);
    !want.is_empty() && body.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| norm(l) == want)
}

/// Where the objectives panel draws, from the race's GUI cfg (UI-48): `objectivesPanelArea`
/// on the cfg's canvas, and inside it the title, summary heading, text, objectives heading
/// and OK button rectangles with their colours, sprites and string keys. A key the cfg leaves
/// out takes A2 stock's `gui_glob16x12.cfg` value, which is what the panel drew before.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ObjectivesLayout {
    pub canvas: (f32, f32),
    pub panel: [f32; 4],
    pub title: [f32; 4],
    pub summary: [f32; 4],
    pub text: [f32; 4],
    pub header: [f32; 4],
    pub ok: [f32; 4],
    pub title_colour: Option<Color>,
    pub summary_colour: Option<Color>,
    pub text_colour: Option<Color>,
    pub header_colour: Option<Color>,
    pub ok_image: Option<String>,
    pub ok_text: String,
    pub ok_tooltip: Option<String>,
}

impl ObjectivesLayout {
    pub(crate) fn from_cfg(cfg: Option<&sta_content::GuiLayout>) -> ObjectivesLayout {
        let rect = |key: &str, stock: [f32; 4]| {
            cfg.and_then(|c| c.rects.get(key)).filter(|r| r.w > 0 && r.h > 0).map_or(stock, |r| [r.x as f32, r.y as f32, r.w as f32, r.h as f32])
        };
        let colour = |key: &str| cfg.and_then(|c| c.colours.get(key)).map(|c| Color::srgba(c.r, c.g, c.b, c.a));
        let name = |key: &str| cfg.and_then(|c| c.names.get(key)).cloned();
        let canvas = cfg.map(|c| c.canvas).filter(|c| c.0 > 0 && c.1 > 0).map_or((1600.0, 1200.0), |c| (c.0 as f32, c.1 as f32));
        ObjectivesLayout {
            canvas,
            // `objectivesPanelArea` is a panel key: the cfg keeps it with the panel.
            panel: cfg
                .and_then(|c| c.panels.iter().find(|(k, _)| k.eq_ignore_ascii_case("objectives")).and_then(|(_, p)| p.area))
                .filter(|r| r.w > 0 && r.h > 0)
                .map_or([192.0, 80.0, 1280.0, 1024.0], |r| [r.x as f32, r.y as f32, r.w as f32, r.h as f32]),
            title: rect("objectivesTitleInformationArea", [30.0, 50.0, 1200.0, 50.0]),
            summary: rect("objectivesSummaryInformationArea", [30.0, 120.0, 1200.0, 40.0]),
            text: rect("objectivesTextInformationArea", [50.0, 160.0, 1200.0, 485.0]),
            header: rect("objectivesHeaderInformationArea", [30.0, 392.0, 1200.0, 45.0]),
            ok: rect("objectivesOkayButtonArea", [952.0, 948.0, 252.0, 60.0]),
            title_colour: colour("objectivesTitleColor"),
            summary_colour: colour("objectivesSummaryColor"),
            text_colour: colour("objectivesTextColor"),
            header_colour: colour("objectivesHeaderColor"),
            ok_image: name("objectivesOkayButtonImage"),
            ok_text: name("objectivesOkayButtonText").unwrap_or_else(|| "GUI_POP_OBJ_BUTTON".to_owned()),
            ok_tooltip: name("objectivesOkayButtonTooltip"),
        }
    }
}

/// The panel on the cfg's rectangles (see [`ObjectivesLayout`]). Text blocks are placed at
/// their rectangles' tops: the title, the briefing heading, the briefing in the text area,
/// the objectives heading and the objective lines under it.
#[allow(clippy::too_many_arguments)]
fn draw_panel(
    mut commands: Commands,
    theme: Res<Theme>,
    strings: Option<Res<Strings>>,
    hud: Option<Res<crate::ui::hud::HudConfig>>,
    view: Res<ObjectivesView>,
    queue: Res<MessageQueue>,
    roots: Query<Entity, With<PanelRoot>>,
    mut shown: Local<Option<(PanelContent, ObjectivesLayout)>>,
) {
    let layout = ObjectivesLayout::from_cfg(hud.as_deref().and_then(|h| h.gui.as_ref()));
    let want = panel_content(&view, &queue);
    let same = match (&want, shown.as_ref()) {
        (None, None) => true,
        (Some(w), Some((c, l))) => w == c && *l == layout,
        _ => false,
    };
    if same && !theme.is_changed() {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    *shown = want.clone().map(|c| (c, layout.clone()));
    let Some(c) = want else { return };
    let s = strings.as_deref();
    let l = &layout;
    let [px, py, pw, ph] = l.panel;
    let pct = |v: f32, of: f32| Val::Percent(v / of * 100.0);
    let at = |r: [f32; 4]| Node { position_type: PositionType::Absolute, left: pct(r[0], pw), top: pct(r[1], ph), width: pct(r[2], pw), flex_direction: FlexDirection::Column, ..default() };
    let text = |t: String, size: f32, colour: Color| sprite_text_styled(t, size, colour, SpriteTextStyle::role(font_role(&theme, size)).wrap_to_parent());
    let line = |t: String, colour: Color| (Node { margin: UiRect::bottom(Val::Px(theme.gap)), ..default() }, children![text(t, theme.text_size, colour)]);
    let summary_colour = l.summary_colour.unwrap_or(Color::srgb(0.8, 1.0, 1.0));
    let header_colour = l.header_colour.unwrap_or(summary_colour);
    let text_colour = l.text_colour.unwrap_or(theme.text);
    let ok = fixed(s, &l.ok_text, "OK");
    let mut panel = commands.spawn((
        PanelRoot,
        Node { position_type: PositionType::Absolute, left: pct(px, l.canvas.0), top: pct(py, l.canvas.1), width: pct(pw, l.canvas.0), height: pct(ph, l.canvas.1), ..default() },
        BackgroundColor(theme.panel.with_alpha(0.92)),
        GlobalZIndex(920),
    ));
    panel.with_children(|p| {
        if !c.title.is_empty() {
            p.spawn((at(l.title), children![text(c.title.clone(), theme.heading_size, l.title_colour.unwrap_or(theme.title))]));
        }
        if c.message.is_none()
            && !c.body.is_empty()
            && let Some(h) = heading_text(s, "GUI_OD_BRIEFING_SUMMARY", "BRIEFING SUMMARY:")
        {
            p.spawn((at(l.summary), children![text(h, theme.heading_size, summary_colour)]));
        }
        if !c.body.is_empty() {
            p.spawn((at(l.text), children![line(c.body.clone(), text_colour)]));
        }
        if c.message.is_none() && !c.lines.is_empty() {
            // Objective files may carry the heading themselves as the last line of the briefing text; then it is
            // not drawn twice. Limit: the comparison is with the table's heading, or the English fallback when
            // the table has none, so a translated game without the string but with a translated text line can
            // still show two.
            let heading = fixed(s, "GUI_OD_OBJECTIVES", "OBJECTIVES:");
            if !text_ends_with_heading(&c.body, &heading) {
                p.spawn((at(l.header), children![text(heading, theme.heading_size, header_colour)]));
            }
            let below = [l.text[0], l.header[1] + l.header[3], l.text[2], 0.0];
            p.spawn(at(below)).with_children(|list| {
                for o in &c.lines {
                    let colour = match o.state {
                        ObjectiveState::Active => text_colour,
                        ObjectiveState::Complete => theme.good,
                        ObjectiveState::Failed => theme.bad,
                    };
                    list.spawn(line(format!("* {}", o.text), colour));
                }
            });
        }
        let mut ok_button = p.spawn((Node { position_type: PositionType::Absolute, left: pct(l.ok[0], pw), top: pct(l.ok[1], ph), ..default() },));
        ok_button.with_children(|b| {
            let mut e = b.spawn((OkButton, button(&theme, &ok, l.ok[2])));
            if let Some(img) = &l.ok_image {
                // The cfg's okay border image (the hover frame) is not drawn: the sprite button has no hover art.
                e.insert(crate::ui::gui_sprites::GuiButton { names: vec![img.clone()], keep_label: true });
            }
            if let Some(tip) = &l.ok_tooltip {
                e.insert(crate::ui::notices::Tooltip::new(fixed(s, tip, "")));
            }
        });
    });
}

fn ok_pressed(
    mut clicks: MessageReader<ButtonClicked>,
    buttons: Query<(), (With<OkButton>, With<UiButton>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mut view: ResMut<ObjectivesView>,
    mut queue: ResMut<MessageQueue>,
    mut out: MessageWriter<ScriptReply>,
) {
    let clicked = clicks.read().any(|c| buttons.contains(c.0));
    // Enter closes it too (Refit's `ui/HUD.md`).
    let open = !queue.0.is_empty() || view.visible;
    if !(open && (clicked || keys.just_pressed(KeyCode::Enter))) {
        return;
    }
    if let Some(m) = queue.0.pop_front() {
        out.write(ScriptReply::MessageDismissed { id: m.id });
    } else {
        view.visible = false;
    }
}

pub struct PresentationPlugin;

fn reset_bridge(mut bridge: ResMut<BridgeState>) {
    *bridge = default();
}

/// Back at the front end (UI-34): titles, fades, queued message boxes, pending movie replies and the
/// objectives panel of the match are gone, whatever ended it (a mission's debrief, F10, the in-game menu).
#[allow(clippy::too_many_arguments)]
fn reset_presentation(
    mut titles: ResMut<Titles>,
    mut fade: ResMut<FadeState>,
    mut cine: ResMut<CineractiveState>,
    mut movies: ResMut<PendingMovies>,
    mut view: ResMut<ObjectivesView>,
    mut queue: ResMut<MessageQueue>,
    mut sounds: ResMut<ScriptSounds>,
    mut stop: MessageWriter<StopScriptSound>,
    mut mute: ResMut<crate::audio::SoundMute>,
) {
    // The menus are not muted by the match's script.
    *mute = default();
    // A script's stream does not play on into the menus.
    for (id, _, _) in std::mem::take(&mut sounds.pending) {
        stop.write(StopScriptSound { id });
    }
    sounds.last_tick = 0;
    titles.items.clear();
    // `Titles::default()` is not dirty, so the layer would keep what it drew.
    titles.dirty = true;
    *fade = default();
    *cine = default();
    movies.0.clear();
    *view = default();
    queue.0.clear();
}

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Titles>()
            .init_resource::<FadeState>()
            .init_resource::<CineractiveState>()
            .init_resource::<CineractiveClock>()
            .init_resource::<Unhandled>()
            .init_resource::<crate::audio::SoundMute>()
            .init_resource::<PendingMovies>()
            .init_resource::<BridgeState>()
            .add_message::<BridgeScene>()
            .init_resource::<ScriptSounds>()
            .add_message::<crate::audio::SoundStarted>()
            .init_resource::<ObjectivesView>()
            .init_resource::<MessageQueue>()
            .add_systems(OnEnter(crate::app::AppState::FrontEnd), (reset_presentation, reset_bridge))
            .add_message::<ScriptReply>()
            .add_message::<MessageBox>()
            .add_message::<crate::media::mission::Requested>()
            .add_message::<MovieFinished>()
            .add_message::<SimMsg>()
            .add_message::<StopScriptSound>()
            .add_message::<PlayScriptSound>()
            .add_message::<ButtonClicked>()
            .add_systems(Startup, spawn_layers.run_if(resource_exists::<Theme>))
            .add_systems(
                Update,
                (clear_sounds_on_restart, consume, bridge_state, sound_replies, hide_notices_in_cineractive, run_clock, movie_replies, skip_cineractive, apply_replies, end_messages_on_skip, toggle_objectives, queue_messages, ok_pressed, draw_titles.run_if(resource_exists::<Theme>), draw_fade, draw_panel.run_if(resource_exists::<Theme>)).chain().in_set(GameSet::Ui),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_link::Bus;
    use crate::ui::sprite_font::SpriteText;
    use sta_core::Fixed;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<ButtonInput<KeyCode>>().add_plugins(PresentationPlugin);
        crate::app::configure_sets(&mut app);
        app
    }

    /// Time stands still between the test's own steps (`Time<Real>` advances only when a test says so).
    fn frozen(app: &mut App) {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO));
        app.update();
    }

    fn step(app: &mut App, seconds: f32) {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(seconds)));
        app.update();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO));
    }

    fn send(app: &mut App, p: Presentation) {
        app.world_mut().write_message(SimMsg(SimEvent::Present(p)));
        app.update();
    }

    fn secs(s: i32) -> Fixed {
        Fixed::from_int(s)
    }

    fn replies(app: &mut App) -> Vec<ScriptReply> {
        app.world_mut().resource_mut::<Messages<ScriptReply>>().drain().collect()
    }

    #[test]
    fn titles_use_the_title_colour_and_subtitles_stay_white() {
        assert_eq!(title_colour(false), TITLE_COLOUR);
        assert_eq!(title_colour(true), [1.0; 3]);
        assert_ne!(title_colour(false), title_colour(true));
    }

    #[test]
    fn an_instant_clear_on_a_clear_screen_is_not_a_cineractives_first_fade() {
        let mut app = app();
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        send(&mut app, Presentation::Fade { out: false, seconds: Fixed::ZERO, rgb: [0; 3] });
        assert_eq!(app.world().resource::<FadeState>().alpha, 0.0);
        // The first real fade in still opens from black.
        send(&mut app, Presentation::Fade { out: false, seconds: secs(2), rgb: [0; 3] });
        assert!(app.world().resource::<FadeState>().alpha > 0.99);
        // A clear of a held black does clear it.
        send(&mut app, Presentation::Fade { out: true, seconds: Fixed::ZERO, rgb: [0; 3] });
        assert_eq!(app.world().resource::<FadeState>().alpha, 1.0);
        send(&mut app, Presentation::Fade { out: false, seconds: Fixed::ZERO, rgb: [0; 3] });
        assert_eq!(app.world().resource::<FadeState>().alpha, 0.0);
    }

    #[test]
    fn an_a2_show_title_places_the_typed_title_and_centres_on_a_negative_x() {
        let mut app = app();
        send(&mut app, Presentation::Title { id: 4, text: "Caption".into(), subtitle: false, seconds: None });
        send(&mut app, Presentation::Call { name: "A2.ShowTitle".into(), args: vec![Arg::Int(4), Arg::Int(-1), Arg::Int(200), Arg::Bool(true)] });
        send(&mut app, Presentation::Title { id: 5, text: "Heading".into(), subtitle: false, seconds: None });
        send(&mut app, Presentation::Call { name: "A2.ShowTitle".into(), args: vec![Arg::Int(5), Arg::Int(100), Arg::Int(120), Arg::Bool(true)] });
        let t = app.world().resource::<Titles>();
        assert!(t.items[0].centred);
        assert_eq!(t.items[0].place, Place { canvas: A2_CANVAS, at: Vec2::new(0.0, 200.0) });
        assert!(!t.items[1].centred);
        assert_eq!(t.items[1].place.at, Vec2::new(100.0, 120.0));
        assert_eq!(app.world().resource::<Unhandled>().count("A2.ShowTitle"), 0);
    }

    #[test]
    fn titles_show_at_their_places_and_go_when_removed_or_timed_out() {
        let mut app = app();
        send(&mut app, Presentation::Title { id: 1, text: "Hello".into(), subtitle: false, seconds: None });
        send(&mut app, Presentation::Title { id: 2, text: "Sub".into(), subtitle: true, seconds: Some(secs(2)) });
        let t = app.world().resource::<Titles>();
        assert_eq!(t.items.len(), 2);
        assert_eq!(t.items[0].place.at, TITLE_AT);
        assert_eq!(t.items[1].place.at, SUBTITLE_AT);
        // A1 places a title by its id, in its own canvas.
        send(&mut app, Presentation::Call { name: "A1.ShowTitle".into(), args: vec![Arg::Int(1), Arg::Int(0), Arg::Int(380), Arg::Int(640), Arg::Int(40), Arg::Bool(true)] });
        assert_eq!(app.world().resource::<Titles>().items[0].place, Place { canvas: A1_CANVAS, at: Vec2::new(0.0, 380.0) });
        // A2 with a placement and seconds.
        send(&mut app, Presentation::Call { name: "UI.addSubtitle".into(), args: vec![Arg::Int(3), Arg::Text("Press spacebar to skip".into()), Arg::Num(secs(50)), Arg::Num(secs(1000)), Arg::Num(secs(3)), Arg::Bool(true)] });
        let t = app.world().resource::<Titles>();
        assert_eq!(t.items[2].place.at, Vec2::new(50.0, 1000.0));
        assert!(t.items[2].left.is_some_and(|l| l > 2.9 && l <= 3.0));
        send(&mut app, Presentation::RemoveTitle { id: 1 });
        assert!(app.world().resource::<Titles>().items.iter().all(|t| t.id != 1));
        app.world_mut().resource_mut::<Titles>().tick(2.5);
        let ids: Vec<_> = app.world().resource::<Titles>().items.iter().map(|t| t.id).collect();
        assert_eq!(ids, [3]);
        assert_eq!(app.world().resource::<Unhandled>().list(), Vec::<(&str, u32)>::new());
    }

    #[test]
    fn a_script_sound_reaches_the_audio_layer_and_stop_sounds_stops_it() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        send(&mut app, Presentation::Sound { id: 1, name: "Voice.mp3".into(), at: None });
        let heard: Vec<String> = app.world_mut().resource_mut::<Messages<PlayScriptSound>>().drain().map(|m| m.name).collect();
        assert_eq!(heard, ["Voice.mp3"]);
        send(&mut app, Presentation::StopSound { id: 1 });
        assert_eq!(app.world_mut().resource_mut::<Messages<StopScriptSound>>().drain().count(), 1);
        assert_eq!(app.world().resource::<Unhandled>().count("Sound"), 0);
    }

    /// Plays `name` (as `id`) through the host and tells it the clip is `length` seconds long.
    fn start(app: &mut App, p: Presentation, name: &str, length: f32) {
        use crate::audio::SoundStarted;
        send(app, p);
        app.world_mut().write_message(SoundStarted { name: name.into(), bus: Bus::Ui, seconds: Some(length) });
        app.update();
    }

    fn a2_music(id: i64, file: &str) -> Presentation {
        Presentation::Call { name: "A2.PlayMusic".into(), args: vec![Arg::Int(id), Arg::Text(file.into())] }
    }

    /// The ids of the streams stopped since the last call.
    fn stops(app: &mut App) -> Vec<u32> {
        app.world_mut().resource_mut::<Messages<StopScriptSound>>().drain().map(|m| m.id).collect()
    }

    fn bus_cuts(app: &mut App) -> usize {
        stops(app).len()
    }

    #[test]
    fn stopping_a_script_stream_stops_that_stream_alone() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        // The music is message 1, a long voice line message 2 (the A2 counter is shared).
        start(&mut app, a2_music(1, "clip_t.wav"), "clip_t.wav", 10.0);
        start(&mut app, Presentation::Sound { id: 2, name: "clip_b.wav".into(), at: None }, "clip_b.wav", 20.0);
        let _ = bus_cuts(&mut app);
        send(&mut app, Presentation::StopSound { id: 1 });
        assert_eq!(stops(&mut app), [1], "the stop names the track and nothing else");
        assert_eq!(app.world().resource::<ScriptSounds>().playing(), [2], "the voice line is still playing");
        assert!(replies(&mut app).is_empty());
        send(&mut app, Presentation::StopSound { id: 2 });
        assert_eq!(stops(&mut app), [2]);
        assert!(app.world().resource::<ScriptSounds>().playing().is_empty());
    }

    #[test]
    fn a_skip_stops_every_pending_stream_and_ends_its_wait() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        start(&mut app, a2_music(1, "clip_t.wav"), "clip_t.wav", 10.0);
        start(&mut app, Presentation::Sound { id: 2, name: "clip_a.wav".into(), at: None }, "clip_a.wav", 20.0);
        let _ = bus_cuts(&mut app);
        let _ = replies(&mut app);
        app.world_mut().write_message(ScriptReply::CineractiveSkipped);
        app.update();
        let mut cut = stops(&mut app);
        cut.sort_unstable();
        assert_eq!(cut, [1, 2], "the skip stops the streams the script had playing");
        let done = replies(&mut app);
        assert!(done.contains(&ScriptReply::SoundDone { id: 1 }) && done.contains(&ScriptReply::SoundDone { id: 2 }), "{done:?}");
        assert!(app.world().resource::<ScriptSounds>().playing().is_empty());
        // The script then stops its current line: nothing is left to stop, nothing is cut twice.
        send(&mut app, Presentation::StopSound { id: 2 });
        assert_eq!(bus_cuts(&mut app), 0);
    }

    #[test]
    fn a_restart_in_place_drops_the_old_runs_messages() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        use crate::sim_link::SimLink;
        use sta_sim::command::LocalRouter;
        use sta_sim::WorldSource;
        let link_at = |ticks: u32| {
            let mut sim = sta_sim::Sim::new(1);
            for _ in 0..ticks {
                sim.step(&[]);
            }
            SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), sta_core::PlayerId(0))
        };
        let mut app = app();
        frozen(&mut app);
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>().insert_resource(link_at(5));
        start(&mut app, Presentation::Sound { id: 1, name: "clip_c.wav".into(), at: None }, "clip_c.wav", 20.0);
        let _ = bus_cuts(&mut app);
        let _ = replies(&mut app);
        // The mission restarts in place: the sim's clock goes back.
        app.insert_resource(link_at(0));
        app.update();
        assert!(app.world().resource::<ScriptSounds>().playing().is_empty(), "the old run's message is gone");
        assert_eq!(bus_cuts(&mut app), 1, "and what still played is cut");
        // The new run's message 1 is not ended by anything left of the old one.
        start(&mut app, Presentation::Sound { id: 1, name: "clip_d.wav".into(), at: None }, "clip_d.wav", 20.0);
        assert!(replies(&mut app).is_empty());
        assert_eq!(app.world().resource::<ScriptSounds>().playing(), [1]);
    }

    #[test]
    fn a_wait_on_a_message_ends_with_its_clip_in_real_time_even_while_the_game_is_paused() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 0));
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        start(&mut app, Presentation::Sound { id: 4, name: "clip_a.wav".into(), at: None }, "clip_a.wav", 5.0);
        app.world_mut().resource_mut::<crate::clock::AnimClock>().set_paused(true);
        step(&mut app, 6.0);
        // The audio is not paused with the game, so the clip has ended and the wait with it (the paused sim holds the script).
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 4 }]);
    }

    #[test]
    fn stopping_a_message_that_has_ended_does_nothing() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        start(&mut app, Presentation::Sound { id: 3, name: "short.wav".into(), at: None }, "short.wav", 1.0);
        start(&mut app, Presentation::Sound { id: 4, name: "long.wav".into(), at: None }, "long.wav", 60.0);
        step(&mut app, 1.5);
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 3 }], "the short clip is over");
        let _ = bus_cuts(&mut app);
        // A skip stops the "current" message, which has ended: the engine ignores the id.
        send(&mut app, Presentation::StopSound { id: 3 });
        assert_eq!(bus_cuts(&mut app), 0);
        assert_eq!(app.world().resource::<ScriptSounds>().playing(), [4], "the other line plays on");
    }

    /// Script streams overlay: a second message or track neither queues behind nor cuts the first,
    /// and a stop of a stale id (never started, or ended) stops nothing.
    #[test]
    fn script_streams_overlay_and_only_a_live_id_can_be_stopped() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        start(&mut app, Presentation::Sound { id: 1, name: "clip_a.wav".into(), at: None }, "clip_a.wav", 30.0);
        start(&mut app, a2_music(2, "clip_m.wav"), "clip_m.wav", 60.0);
        start(&mut app, Presentation::Sound { id: 3, name: "clip_b.wav".into(), at: None }, "clip_b.wav", 5.0);
        let played: Vec<(u32, String)> = app.world_mut().resource_mut::<Messages<PlayScriptSound>>().drain().map(|m| (m.id, m.name)).collect();
        assert_eq!(played, [(1, "clip_a.wav".to_owned()), (2, "clip_m.wav".to_owned()), (3, "clip_b.wav".to_owned())], "all three start at once, none waits for another");
        assert!(stops(&mut app).is_empty(), "a new track does not stop the one before it");
        assert!(app.world().resource::<ScriptSounds>().any_playing());
        send(&mut app, Presentation::StopSound { id: 9 });
        assert!(stops(&mut app).is_empty(), "an id that never played is ignored");
        send(&mut app, Presentation::StopSound { id: 2 });
        assert_eq!(stops(&mut app), [2], "the track stops and the voice lines play on");
        assert_eq!(app.world().resource::<ScriptSounds>().playing(), [1, 3]);
    }

    /// Each condition of the event-voice suppression holds on its own.
    #[test]
    fn event_voices_are_silenced_by_a_cutscene_a_script_stream_or_a_movie_each_alone() {
        let mut busy = ScriptSounds::default();
        busy.start_for_test(5);
        let movie = crate::media::Movies::busy_for_test();
        assert!(!event_voices_silenced(false, None, None));
        assert!(!event_voices_silenced(false, Some(&ScriptSounds::default()), Some(&crate::media::Movies::default())));
        assert!(event_voices_silenced(true, None, None), "the cinematic view");
        assert!(event_voices_silenced(false, Some(&busy), None), "a script stream");
        assert!(event_voices_silenced(false, None, Some(&movie)), "a movie");
    }

    /// Entering the front end stops the script's streams, so a track does not play into the menu.
    #[test]
    fn leaving_the_match_stops_the_script_streams() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.add_plugins(bevy::state::app::StatesPlugin).init_state::<crate::app::AppState>();
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        app.update();
        start(&mut app, a2_music(1, "clip_m.wav"), "clip_m.wav", 60.0);
        start(&mut app, Presentation::Sound { id: 2, name: "clip_a.wav".into(), at: None }, "clip_a.wav", 30.0);
        let _ = stops(&mut app);
        app.world_mut().resource_mut::<NextState<crate::app::AppState>>().set(crate::app::AppState::FrontEnd);
        app.update();
        let mut cut = stops(&mut app);
        cut.sort_unstable();
        assert_eq!(cut, [1, 2]);
        assert!(!app.world().resource::<ScriptSounds>().any_playing());
    }

    fn mute_call(f: Arg) -> Presentation {
        Presentation::Call { name: "A2.MuteSounds".into(), args: vec![f] }
    }

    /// `MuteSounds(f)` sets the master multiplier (held to 0..1, a non-number ignored and listed),
    /// and the match's end, the front end and a restart in place put it back to 1.
    #[test]
    fn mute_sounds_sets_the_multiplier_and_it_is_reset_when_the_match_is_left_or_restarted() {
        use crate::sim_link::SimLink;
        use sta_sim::command::LocalRouter;
        use sta_sim::WorldSource;
        let link_at = |ticks: u32| {
            let mut sim = sta_sim::Sim::new(1);
            for _ in 0..ticks {
                sim.step(&[]);
            }
            SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), sta_core::PlayerId(0))
        };
        let mut app = app();
        frozen(&mut app);
        app.add_plugins(bevy::state::app::StatesPlugin).init_state::<crate::app::AppState>().insert_resource(link_at(5));
        app.update();
        let level = |app: &App| app.world().resource::<crate::audio::SoundMute>().0;
        assert_eq!(level(&app), 1.0);
        send(&mut app, mute_call(Arg::Num(Fixed::from_f64(0.37))));
        assert!((level(&app) - 0.37).abs() < 1e-6);
        send(&mut app, mute_call(Arg::Num(Fixed::from_int(7))));
        assert_eq!(level(&app), 1.0, "held to full level");
        send(&mut app, mute_call(Arg::Num(Fixed::from_f64(0.25))));
        send(&mut app, mute_call(Arg::Bool(true)));
        assert!((level(&app) - 0.25).abs() < 1e-6, "a non-number leaves it");
        assert_eq!(app.world().resource::<Unhandled>().count("A2.MuteSounds"), 1);
        // A retry starts the intro again: the sim's clock goes back and the multiplier with it.
        app.insert_resource(link_at(0));
        app.update();
        assert_eq!(level(&app), 1.0, "a restart in place un-mutes");
        send(&mut app, mute_call(Arg::Num(Fixed::from_f64(0.29))));
        app.world_mut().resource_mut::<NextState<crate::app::AppState>>().set(crate::app::AppState::FrontEnd);
        app.update();
        assert_eq!(level(&app), 1.0, "leaving the match un-mutes");
    }

    #[test]
    fn an_a2_music_wait_ends_with_its_clip() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        send(&mut app, a2_music(14, "clip_t.wav"));
        let heard: Vec<String> = app.world_mut().resource_mut::<Messages<PlayScriptSound>>().drain().map(|m| m.name).collect();
        assert_eq!(heard, ["clip_t.wav"], "the track is played, not left unhandled");
        assert_eq!(app.world().resource::<Unhandled>().count("A2.PlayMusic"), 0);
        app.world_mut().write_message(SoundStarted { name: "clip_t.wav".into(), bus: Bus::Ui, seconds: Some(10.0) });
        app.update();
        step(&mut app, 9.0);
        assert!(replies(&mut app).is_empty(), "a script that waits for its track is still waiting at 9 s");
        step(&mut app, 1.5);
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 14 }], "and is released when the clip has played");
    }

    #[test]
    fn a_cineractive_opens_black_and_its_end_clears_the_fade() {
        let mut app = app();
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        send(&mut app, Presentation::Fade { out: false, seconds: Fixed::from_int(2), rgb: [0; 3] });
        assert!(app.world().resource::<FadeState>().alpha > 0.99, "the first fade in starts from black");
        send(&mut app, Presentation::Fade { out: true, seconds: Fixed::ZERO, rgb: [0; 3] });
        assert_eq!(app.world().resource::<FadeState>().alpha, 1.0);
        send(&mut app, Presentation::Cineractive { on: false, skippable: true });
        assert_eq!(app.world().resource::<FadeState>().alpha, 0.0, "the game is not left black");
        // Outside a cineractive a fade in does not jump to black.
        send(&mut app, Presentation::Fade { out: false, seconds: Fixed::from_int(2), rgb: [0; 3] });
        assert_eq!(app.world().resource::<FadeState>().alpha, 0.0);
    }

    #[test]
    fn the_notices_are_hidden_over_a_cineractive() {
        let mut app = app();
        let root = app.world_mut().spawn((crate::ui::notices::NoticeRoot, Visibility::Inherited)).id();
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Hidden);
        send(&mut app, Presentation::Cineractive { on: false, skippable: true });
        assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Inherited);
    }

    #[test]
    fn a_fade_ramps_over_its_seconds() {
        let mut f = FadeState::default();
        f.start(true, 2.0, [0, 0, 0]);
        f.tick(1.0);
        assert!((f.alpha - 0.5).abs() < 1e-6);
        f.tick(5.0);
        assert_eq!(f.alpha, 1.0);
        f.start(false, 1.0, [0, 0, 0]);
        f.tick(0.25);
        assert!((f.alpha - 0.75).abs() < 1e-6);
        f.start(true, 0.0, [0, 0, 0]);
        assert_eq!(f.alpha, 1.0);
    }

    #[test]
    fn a_non_skippable_cineractive_yields_after_the_grace_and_ends_by_itself_in_the_end() {
        let mut app = app();
        send(&mut app, Presentation::Cineractive { on: true, skippable: false });
        let press = |app: &mut App, k: KeyCode| {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(k);
            keys.clear();
            keys.press(k);
            app.update();
        };
        press(&mut app, KeyCode::Escape);
        assert!(replies(&mut app).is_empty(), "inside the grace Esc does nothing");
        app.world_mut().resource_mut::<CineractiveClock>().held = SKIP_GRACE_SECS;
        press(&mut app, KeyCode::Escape);
        assert_eq!(replies(&mut app), [ScriptReply::CineractiveReleased]);
        assert!(!app.world().resource::<CineractiveState>().on);
        // Nothing is pressed: the hold ends at the cap.
        send(&mut app, Presentation::Cineractive { on: true, skippable: false });
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.world_mut().resource_mut::<CineractiveClock>().held = MAX_HOLD_SECS;
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::CineractiveReleased]);
        assert!(!app.world().resource::<CineractiveState>().on);
        // A script that raises it again at once earns no new grace: it is released again at once.
        send(&mut app, Presentation::Cineractive { on: true, skippable: false });
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::CineractiveReleased]);
        assert!(!app.world().resource::<CineractiveState>().on);
    }

    #[test]
    fn a_release_from_the_host_clears_the_state_and_a_skip_does_not() {
        let mut app = app();
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        app.world_mut().write_message(ScriptReply::CineractiveSkipped);
        app.update();
        assert!(app.world().resource::<CineractiveState>().on, "a skip leaves the end to the script");
        app.world_mut().write_message(ScriptReply::CineractiveReleased);
        app.update();
        assert!(!app.world().resource::<CineractiveState>().on);
    }

    #[test]
    fn space_skips_a_skippable_cineractive_once_and_the_script_gets_time_to_end_it() {
        let mut app = app();
        send(&mut app, Presentation::Cineractive { on: true, skippable: false });
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
        app.update();
        assert!(replies(&mut app).is_empty());
        assert!(app.world().resource::<CineractiveState>().on);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::Space);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::CineractiveSkipped]);
        assert!(app.world().resource::<CineractiveState>().on, "the script ends it");
        // A second press is not a second skip.
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::Space);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
        app.update();
        assert!(replies(&mut app).is_empty());
        // The script ends it: off.
        send(&mut app, Presentation::Cineractive { on: false, skippable: true });
        assert!(!app.world().resource::<CineractiveState>().on);
        // A script that does not end it after a skip is released for it.
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::Space);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        app.world_mut().resource_mut::<CineractiveClock>().skipped_for = Some(AFTER_SKIP_SECS);
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::CineractiveReleased]);
    }

    #[test]
    fn a_cineractive_started_in_the_frame_the_skipped_one_ended_can_be_skipped_again() {
        let mut app = app();
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::CineractiveSkipped]);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::Space);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        // End and start in one frame.
        app.world_mut().write_message(SimMsg(SimEvent::Present(Presentation::Cineractive { on: false, skippable: true })));
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        assert_eq!(app.world().resource::<CineractiveClock>().skipped_for, None);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::CineractiveSkipped]);
    }

    #[test]
    fn a_requested_movie_is_answered_when_it_finishes() {
        let mut app = app();
        let id = MovieId(7);
        app.world_mut().write_message(Requested { id, name: "intro".into() });
        app.world_mut().write_message(MovieFinished { id: MovieId(99), name: "other".into(), outcome: crate::media::MovieOutcome::Completed });
        app.update();
        assert!(replies(&mut app).is_empty(), "a movie the mission did not ask for");
        app.world_mut().write_message(MovieFinished { id, name: "intro".into(), outcome: crate::media::MovieOutcome::Completed });
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::MovieDone { id, name: "intro".into() }]);
    }

    #[test]
    fn a_message_waits_for_ok_and_the_script_hears_it() {
        let mut app = app();
        app.world_mut().write_message(MessageBox { id: 4, title: "Orders".into(), text: "Hold.".into() });
        app.update();
        assert_eq!(app.world().resource::<MessageQueue>().0.len(), 1);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Enter);
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::MessageDismissed { id: 4 }]);
        assert!(app.world().resource::<MessageQueue>().0.is_empty());
    }

    #[test]
    fn the_briefing_panel_closes_on_enter_and_never_holds_a_message_back() {
        let mut app = app();
        app.world_mut().resource_mut::<ObjectivesView>().visible = true;
        app.world_mut().write_message(MessageBox { id: 9, title: "t".into(), text: "x".into() });
        app.update();
        // The message is in front; Enter dismisses it, the next Enter closes the panel.
        for want_visible in [true, false] {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::Enter);
            keys.clear();
            keys.press(KeyCode::Enter);
            app.update();
            assert_eq!(app.world().resource::<ObjectivesView>().visible, want_visible);
        }
        assert_eq!(replies(&mut app), [ScriptReply::MessageDismissed { id: 9 }]);
        // Nothing the panel shows is a camera hold: the cineractive state is independent of it.
        assert!(!app.world().resource::<CineractiveState>().on);
    }

    #[test]
    fn alt_o_toggles_the_objectives_and_requests_without_a_consumer_are_listed() {
        let mut app = app();
        {
            let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            k.press(KeyCode::AltLeft);
            k.press(OBJECTIVES_KEY);
        }
        app.update();
        assert!(app.world().resource::<ObjectivesView>().visible);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        send(&mut app, Presentation::Music { name: "m".into(), looped: true });
        send(&mut app, Presentation::Music { name: "n".into(), looped: true });
        send(&mut app, Presentation::Call { name: "A1.StartCredits".into(), args: vec![] });
        assert_eq!(app.world().resource::<Unhandled>().list(), [("A1.StartCredits", 1), ("Music", 2)]);
    }

    #[test]
    fn titles_the_fade_and_the_panel_are_drawn() {
        let mut app = app();
        app.insert_resource(Theme::default());
        app.update(); // Startup: the layers
        send(&mut app, Presentation::Title { id: 1, text: "Stardate 1".into(), subtitle: false, seconds: None });
        send(&mut app, Presentation::Fade { out: true, seconds: Fixed::ZERO, rgb: [0, 0, 0] });
        let texts = |app: &mut App| app.world_mut().query::<&SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&mut app), ["Stardate 1"]);
        let alpha = app.world_mut().query_filtered::<&BackgroundColor, With<FadeOverlay>>().single(app.world()).unwrap().0.alpha();
        assert_eq!(alpha, 1.0);
        let obj = [Objective { id: 1, title: "Find".into(), description: String::new(), state: ObjectiveState::Active }];
        app.world_mut().resource_mut::<ObjectivesView>().fill(&obj, Some(&ObjectiveBriefing { file: "f".into(), title: "Mission 1".into(), text: "Go.".into(), notes: String::new() }));
        app.world_mut().resource_mut::<ObjectivesView>().visible = true;
        app.update();
        let t = texts(&mut app);
        assert!(["Mission 1", "BRIEFING SUMMARY:", "Go.", "OBJECTIVES:", "* Find"].iter().all(|w| t.iter().any(|x| x == w)), "{t:?}");
        send(&mut app, Presentation::RemoveTitle { id: 1 });
        assert!(!texts(&mut app).contains(&"Stardate 1".to_string()));
        app.world_mut().resource_mut::<ObjectivesView>().visible = false;
        app.update();
        assert!(app.world_mut().query::<&PanelRoot>().iter(app.world()).next().is_none());
    }

    fn a1_title(app: &mut App) {
        send(app, Presentation::Title { id: 1, text: "'TITLE TEXT'".into(), subtitle: false, seconds: None });
        send(app, Presentation::Call { name: "A1.ShowTitle".into(), args: vec![Arg::Int(1), Arg::Int(0), Arg::Int(0), Arg::Int(200), Arg::Int(70), Arg::Bool(true)] });
    }

    /// An A1 `ShowTitle(0, 0, 200, 70, text, true)`: blue, top-left, placed in
    /// fractions of the window's height (so a 21:9 window has the same margins in proportion to its
    /// height, no 4:3 box and no bars), fading in over a second.
    #[test]
    fn an_a1_title_is_blue_top_left_anchored_to_the_height_and_fades_in() {
        let mut app = app();
        app.insert_resource(Theme::default());
        app.update();
        frozen(&mut app);
        a1_title(&mut app);
        let at = a1_title_at([0.0, 0.0, 200.0, 70.0], "'TITLE TEXT'");
        // Measured by the default font: 62 units wide, 10 high, centred in 200 x 70.
        assert_eq!((at.x, at.y), (68.5, 29.5));
        let node = |app: &mut App| {
            let (n, c) = app.world_mut().query_filtered::<(&Node, &Children), Without<TitleLayer>>().iter(app.world()).find(|(n, _)| matches!(n.left, Val::Vh(_))).map(|(n, c)| (n.clone(), c.to_vec())).expect("the title node");
            (n, c)
        };
        let (n, kids) = node(&mut app);
        // Units of the window's height, never of its width: the same on any aspect ratio.
        assert_eq!((n.left, n.top), (Val::Vh(at.x / 480.0 * 100.0), Val::Vh(at.y / 480.0 * 100.0)));
        assert!(n.max_width == Val::Auto, "no width box that would depend on the aspect");
        let colour = |app: &App, e: Entity| app.world().get::<SpriteText>(e).map(|t| t.color);
        let first = colour(&app, kids[0]);
        assert!(first.is_some_and(|c| c.to_srgba().blue < 0.1), "starts dark (fading in): {first:?}");
        app.world_mut().resource_mut::<Titles>().tick(2.0);
        app.update();
        let (_, kids) = node(&mut app);
        let c = colour(&app, kids[0]).unwrap().to_srgba();
        assert_eq!((c.red, c.green, c.blue), (0.0, 0.5, 1.0));
        assert!(app.world_mut().query::<&Node>().iter(app.world()).all(|n| !matches!((n.left, n.right), (Val::Vw(_), Val::Vw(_)))), "no bars");
    }

    fn title_colours(app: &mut App) -> Vec<Srgba> {
        app.update();
        app.world_mut().query::<&SpriteText>().iter(app.world()).map(|t| t.color.to_srgba()).collect()
    }

    /// `RemoveTitle(id, true)` takes the title down over a second, from the level it has, then it
    /// is gone; `RemoveTitle(id, false)` takes it down at once (read from the executable, static).
    #[test]
    fn an_a1_title_removed_with_the_flag_fades_out_over_a_second() {
        let mut app = app();
        app.insert_resource(Theme::default());
        app.update();
        frozen(&mut app);
        a1_title(&mut app);
        app.world_mut().resource_mut::<Titles>().tick(3.0);
        assert_eq!(title_colours(&mut app).first().map(|c| (c.red, c.green, c.blue)), Some((0.0, 0.5, 1.0)), "held at full level");
        send(&mut app, Presentation::Call { name: "A1.RemoveTitleFade".into(), args: vec![Arg::Int(1)] });
        assert_eq!(app.world().resource::<Titles>().items.len(), 1, "still there");
        app.world_mut().resource_mut::<Titles>().tick(0.5);
        let c = title_colours(&mut app)[0];
        assert!((c.blue - 0.5).abs() < 1e-6 && (c.green - 0.25).abs() < 1e-6, "half way down: {c:?}");
        app.world_mut().resource_mut::<Titles>().tick(0.6);
        app.update();
        assert!(app.world().resource::<Titles>().items.is_empty(), "gone after the second");
        assert!(title_colours(&mut app).is_empty());
        // A fade out begun while still fading in goes down from the level reached.
        a1_title(&mut app);
        app.world_mut().resource_mut::<Titles>().tick(0.5);
        send(&mut app, Presentation::Call { name: "A1.RemoveTitleFade".into(), args: vec![Arg::Int(1)] });
        app.world_mut().resource_mut::<Titles>().tick(0.4);
        assert!((app.world().resource::<Titles>().items[0].level() - 0.1).abs() < 1e-6);
        app.world_mut().resource_mut::<Titles>().tick(0.2);
        assert!(app.world().resource::<Titles>().items.is_empty());
        // The plain form removes at once.
        a1_title(&mut app);
        send(&mut app, Presentation::RemoveTitle { id: 1 });
        assert!(app.world().resource::<Titles>().items.is_empty());
        assert_eq!(app.world().resource::<Unhandled>().list(), Vec::<(&str, u32)>::new());
    }

    /// A1 subtitles (`ShowSubtitle(0, 380, 640, 40, ..)`) are white and centred in their rectangle
    /// on both axes by the default font's measure, like titles (read from the executable, static);
    /// a false flag shows them at full level at once.
    #[test]
    fn an_a1_subtitle_is_white_and_centred_in_its_rectangle() {
        let mut app = app();
        app.insert_resource(Theme::default());
        app.update();
        frozen(&mut app);
        send(&mut app, Presentation::Title { id: 4, text: "Hi".into(), subtitle: true, seconds: None });
        send(&mut app, Presentation::Call { name: "A1.ShowSubtitle".into(), args: vec![Arg::Int(4), Arg::Int(0), Arg::Int(380), Arg::Int(640), Arg::Int(40), Arg::Bool(false)] });
        let at = a1_title_at([0.0, 380.0, 640.0, 40.0], "Hi");
        // Two 6-unit letters: text starts at (639 - 12) / 2 and its 10 units centre on y 399.5.
        assert_eq!((at.x, at.y + 5.0), (313.5, 399.5));
        let (n, _) = app.world_mut().query_filtered::<(&Node, &Children), Without<TitleLayer>>().iter(app.world()).find(|(n, _)| matches!(n.left, Val::Vh(_))).map(|(n, c)| (n.clone(), c.to_vec())).expect("the subtitle node");
        assert_eq!((n.left, n.top), (Val::Vh(at.x / 480.0 * 100.0), Val::Vh(at.y / 480.0 * 100.0)));
        let c = title_colours(&mut app)[0];
        assert_eq!((c.red, c.green, c.blue), (1.0, 1.0, 1.0), "white, full level at once");
    }

    #[test]
    fn a_mission_message_ends_when_its_clip_has_played_and_not_before() {
        use crate::audio::{ReportsSoundLengths, SoundStarted};
        let mut app = app();
        frozen(&mut app);
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        send(&mut app, Presentation::Sound { id: 7, name: "log.wav".into(), at: None });
        assert!(replies(&mut app).is_empty(), "not done when it starts");
        app.world_mut().write_message(SoundStarted { name: "log.wav".into(), bus: Bus::Ui, seconds: Some(0.0) });
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 7 }], "a clip of no length is over at once");
        // A one-second clip: not done after 0.9 s of playing, done after 1.1 s.
        send(&mut app, Presentation::Sound { id: 8, name: "log.wav".into(), at: None });
        app.world_mut().write_message(SoundStarted { name: "log.wav".into(), bus: Bus::Ui, seconds: Some(1.0) });
        app.update();
        step(&mut app, 0.9);
        assert!(replies(&mut app).is_empty() && app.world().resource::<ScriptSounds>().playing() == [8], "still playing at 0.9 s");
        step(&mut app, 0.2);
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 8 }], "done at 1.1 s");
        send(&mut app, Presentation::Sound { id: 9, name: "log.wav".into(), at: None });
        app.world_mut().write_message(SoundStarted { name: "log.wav".into(), bus: Bus::Ui, seconds: Some(3600.0) });
        app.update();
        assert!(replies(&mut app).is_empty() && app.world().resource::<ScriptSounds>().playing() == [9], "a long clip is still playing");
        send(&mut app, Presentation::StopSound { id: 9 });
        assert!(app.world().resource::<ScriptSounds>().playing().is_empty(), "stopped by the script");
    }

    #[test]
    fn without_an_audio_layer_a_message_ends_at_once() {
        let mut app = app();
        send(&mut app, Presentation::Sound { id: 3, name: "x.wav".into(), at: None });
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 3 }]);
    }

    #[test]
    fn an_objectives_heading_in_the_briefing_text_is_not_drawn_twice() {
        assert!(text_ends_with_heading("TITLE TEXT\n\nOBJECTIVES:\n", "OBJECTIVES:"));
        assert!(text_ends_with_heading("Go.\nobjectives", "OBJECTIVES:"));
        assert!(!text_ends_with_heading("Go.", "OBJECTIVES:"));
        assert!(!text_ends_with_heading("OBJECTIVES:\nGo.", "OBJECTIVES:"));
        assert!(!text_ends_with_heading("", "OBJECTIVES:"));

        let mut app = app();
        app.insert_resource(Theme::default());
        app.update();
        let texts = |app: &mut App| app.world_mut().query::<&SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect::<Vec<_>>();
        let obj = [Objective { id: 1, title: "Find".into(), description: String::new(), state: ObjectiveState::Active }];
        // An A1-shaped briefing: the text ends with the heading, then the goals follow.
        app.world_mut().resource_mut::<ObjectivesView>().fill(&obj, Some(&ObjectiveBriefing { file: "f".into(), title: "Mission 1".into(), text: "Go.\n\nOBJECTIVES:".into(), notes: String::new() }));
        app.world_mut().resource_mut::<ObjectivesView>().visible = true;
        app.update();
        let t = texts(&mut app);
        assert_eq!(t.iter().filter(|x| x.contains("OBJECTIVES:")).count(), 1, "{t:?}");
        assert!(t.iter().any(|x| x == "* Find"));
        // An A2-shaped briefing (no heading in the text): the panel draws it.
        app.world_mut().resource_mut::<ObjectivesView>().fill(&obj, Some(&ObjectiveBriefing { file: "f".into(), title: "Mission 1".into(), text: "Go.".into(), notes: String::new() }));
        app.update();
        let t = texts(&mut app);
        assert_eq!(t.iter().filter(|x| x.contains("OBJECTIVES:")).count(), 1, "{t:?}");
    }

    #[test]
    fn the_briefing_heading_is_drawn_only_when_the_game_has_a_string_for_it() {
        let a2 = Strings::from_game("en", "\"GUI_OD_BRIEFING_SUMMARY\", \"SUMMARY:\",\n\"GUI_OD_OBJECTIVES\", \"GOALS:\"");
        let a1 = Strings::from_game("en", "\"GUI_POP_OBJ_BUTTON\", \"Close\"");
        assert_eq!(heading_text(Some(&a2), "GUI_OD_BRIEFING_SUMMARY", "X"), Some("SUMMARY:".into()));
        assert_eq!(heading_text(Some(&a1), "GUI_OD_BRIEFING_SUMMARY", "X"), None);
        assert_eq!(heading_text(None, "GUI_OD_BRIEFING_SUMMARY", "X"), Some("X".into()));
        assert_eq!(heading_text(Some(&Strings::default()), "GUI_OD_BRIEFING_SUMMARY", "BRIEFING SUMMARY:"), Some("BRIEFING SUMMARY:".into()), "an empty table is no table");
        let texts = |app: &mut App| app.world_mut().query::<&SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect::<Vec<_>>();
        let obj = [Objective { id: 1, title: "Find".into(), description: String::new(), state: ObjectiveState::Active }];
        for (strings, summary) in [(a2, true), (a1, false), (Strings::default(), false)] {
            let strings_is_empty = !strings.has_game_table();
            let mut app = app();
            app.insert_resource(Theme::default());
            app.insert_resource(strings);
            app.update();
            app.world_mut().resource_mut::<ObjectivesView>().fill(&obj, Some(&ObjectiveBriefing { file: "f".into(), title: "M".into(), text: "Go.".into(), notes: String::new() }));
            app.world_mut().resource_mut::<ObjectivesView>().visible = true;
            app.update();
            let t = texts(&mut app);
            assert!(t.iter().any(|x| x == "Go."), "{t:?}");
            assert_eq!(t.iter().any(|x| x == "SUMMARY:"), summary, "{t:?}");
            // An empty table is no table: the English fallback heading shows.
            assert_eq!(t.iter().any(|x| x == "BRIEFING SUMMARY:"), strings_is_empty, "{t:?}");
        }
    }

    #[test]
    fn objectives_fill_from_the_runtime_types() {
        let mut v = ObjectivesView::default();
        let o = |id, state| Objective { id, title: format!("t{id}"), description: "d".into(), state };
        v.fill(&[o(1, ObjectiveState::Active), o(2, ObjectiveState::Complete)], Some(&ObjectiveBriefing { file: "f".into(), title: "Mission".into(), text: "Go.".into(), notes: String::new() }));
        assert_eq!((v.title.as_str(), v.items.len(), v.items[1].state), ("Mission", 2, ObjectiveState::Complete));
    }

    #[test]
    fn the_four_bridge_calls_are_typed_and_tracked_and_other_calls_are_not_taken() {
        let mut app = app();
        let call = |name: &str, args: Vec<Arg>| SimMsg(SimEvent::Present(Presentation::Call { name: name.into(), args }));
        for m in [
            call("A1.PreLoadBridgeScene", vec![Arg::Int(2)]),
            call("A1.SetBridgeSceneRace", vec![Arg::Int(3), Arg::Text("romulan".into())]),
            call("A1.ShowBridgeScene", vec![Arg::Bool(true)]),
            call("A1.ShowBridgeOnScreen", vec![Arg::Bool(true)]),
            call("A1.ShowBridgeOnScreen", vec![Arg::Bool(false)]),
        ] {
            app.world_mut().write_message(m);
        }
        step(&mut app, 0.0);
        let state = app.world().resource::<BridgeState>().clone();
        assert_eq!(state, BridgeState { preloaded: Some(2), race: Some(3), shown: true, on_screen: false });
        assert!(app.world().resource::<Unhandled>().list().is_empty(), "typed, so no longer listed as not shown");
        // A call that is not one of the four, or with the wrong argument kind, is still listed.
        app.world_mut().write_message(call("A1.ShowBridgeScene", vec![Arg::Int(1)]));
        step(&mut app, 0.0);
        assert_eq!(app.world().resource::<Unhandled>().count("A1.ShowBridgeScene"), 1);
        assert_eq!(BridgeScene::from_call("A1.Nothing", &[Arg::Bool(true)]), None);
    }

    #[test]
    fn the_objectives_panel_takes_its_rectangles_colours_and_button_from_the_cfg() {
        let stock = ObjectivesLayout::from_cfg(None);
        assert_eq!((stock.panel, stock.ok, stock.ok_text.as_str()), ([192.0, 80.0, 1280.0, 1024.0], [952.0, 948.0, 252.0, 60.0], "GUI_POP_OBJ_BUTTON"));
        let text = "screenWidth = 800\nscreenHeight = 600\nobjectivesPanelArea = 10 20 700 500\nobjectivesTitleInformationArea = 5 6 300 20\nobjectivesHeaderInformationArea = 0 0 0 0\nobjectivesTitleColor = 1 0 0\nobjectivesOkayButtonArea = 400 450 120 30\nobjectivesOkayButtonImage = \"ok_spr\"\nobjectivesOkayButtonText = \"MY_OK\"\n";
        let (cfg, warnings) = sta_content::GuiCfg::parse("obj.cfg", text);
        assert!(warnings.is_empty(), "{warnings:?}");
        let l = ObjectivesLayout::from_cfg(Some(&cfg.layout_auto()));
        assert_eq!(l.canvas, (800.0, 600.0));
        assert_eq!((l.panel, l.title, l.ok), ([10.0, 20.0, 700.0, 500.0], [5.0, 6.0, 300.0, 20.0], [400.0, 450.0, 120.0, 30.0]));
        assert_eq!(l.header, stock.header, "a zero rectangle keeps the stock place");
        assert_eq!(l.title_colour, Some(Color::srgba(1.0, 0.0, 0.0, 1.0)));
        assert_eq!((l.ok_image.as_deref(), l.ok_text.as_str()), (Some("ok_spr"), "MY_OK"));
        assert_eq!(l.text_colour, None, "an absent colour keeps the theme's");
    }
}
