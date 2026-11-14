//! What a mission shows the player, and the answers it waits for (UI-30).
//!
//! A mission raises `SimEvent::Present(Presentation)` (see `sta_sim::present`); the
//! client consumes it here (text and fades), in `input/camera_cinematic.rs` (the
//! camera) and in `media/mission.rs` (movies, MED-03). Nothing in `ui/hud/` is
//! touched; the objectives list is an overlay of the UI kit, outside the HUD.
//!
//! | request | shown here | evidence |
//! |---|---|---|
//! | `Title`, `RemoveTitle`, A1 `ShowTitle` / `ShowSubtitle` placement, A2 `UI.addTitle` / `addSubtitle` with x, y, seconds | text at a canvas position, until its seconds run out or it is removed | A2 canvas 1600x1200 and the default places (50,50) / (50,1000) from Fleet Ops Roots' Lua, A1 640x480 rect from the A1 mission DLLs; stock A2 title colour and font **not measured** |
//! | `Fade` | a full-screen black rectangle, alpha ramped over `seconds` | `ScreenFade` is a black rectangle in the original; the A1 argument meanings beyond seconds **not measured** |
//! | `Cineractive { skippable }` | state in [`CineractiveState`]; Space skips a skippable one ([`ScriptReply::CineractiveSkipped`]: the script's cancel), caps release it ([`ScriptReply::CineractiveReleased`]); the camera is `camera_cinematic` (UI-36 shot model) | `cineractiveCancel` (A2) / `switch_camera_shot` (A1) is Space in `Input.map`; no letterbox exists in the original's data or executables, so none is drawn |
//! | objectives and briefing (host state, not a `Presentation`) | the modal panel of [`ObjectivesView`], toggled with Alt+O | rectangles of `objectivesPanelArea` and the text areas in A2 stock `gui_glob16x12.cfg`, the A1 `gui_glob.cfg` panel; the completed-line tick **not measured** |
//! | a message box ([`MessageBox`]) | the same panel with an OK button | the original's message display reuses the objectives keys |
//! | `Movie`, `StopMovie` | MED-03 | |
//! | `Sound`, `StopSound` | `PlaySound` on the UI bus (no admission limit) / `StopSounds` (the audio lane resolves the name); the script's `IsAudioMessageDone` turns true when the clip's length has played ([`ScriptReply::SoundDone`], from `audio::SoundStarted`), at once if nothing plays it | clip length from the decoded file |
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
use crate::sim_link::{Bus, PlaySound, SimMsg, StopSounds};
use crate::ui::i18n::Strings;
use crate::ui::sprite_font::{SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::{ButtonClicked, Theme, UiButton, button, font_role};

/// A2's layout space (stock `screenWidth` / `screenHeight` of `gui_glob16x12.cfg`).
pub const A2_CANVAS: Vec2 = Vec2::new(1600.0, 1200.0);
/// An A1 title fades in over this many seconds when the script's flag is set (all stock titles set it;
/// read from the executable, static; the unit of the engine's step is inferred, not measured).
pub const A1_TITLE_FADE_SECS: f32 = 1.0;
/// A1 titles are drawn in the title font's colour, RGB (0, 0.5, 1) (hard-coded in the executable, static).
pub const A1_TITLE_COLOUR: [f32; 3] = [0.0, 0.5, 1.0];

/// Where A1's `ShowTitle(x, y, w, h)` puts the text, in 480-unit-high canvas units from the window's
/// top-left: centred in the rectangle by the width and height the default font would give it
/// (5 units a letter and one between letters, 10 high; read from the executable, static). The stock title draws about twice that size (fitted to a screenshot, not found in the executable); ours uses the theme's heading size. The
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
}

impl ScriptSounds {
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
    mut play: MessageWriter<PlaySound>,
    mut stop: MessageWriter<StopSounds>,
    mut log: Option<ResMut<PresentationLog>>,
    link: Option<Res<crate::sim_link::SimLink>>,
    mut sounds: ResMut<ScriptSounds>,
    reports: Option<Res<crate::audio::ReportsSoundLengths>>,
    mut replies: MessageWriter<ScriptReply>,
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
                age: 0.0,
                out: None,
            }),
            Presentation::RemoveTitle { id } => titles.remove(*id),
            Presentation::Fade { out, seconds, rgb } => {
                // The first fade a cineractive asks for, when it is a fade in, comes up from black:
                // the stock openings call `fadeIn` at their first tick and begin black, so nothing of
                // the RTS view shows before the first shot (the original's start colour is inferred:
                // not measured; a fade in on a screen already faded out runs from there as before).
                if !*out && cineractive.on && !fade.touched && fade.alpha == 0.0 {
                    fade.alpha = 1.0;
                }
                fade.touched = true;
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
                    titles.add(TitleItem { id, text: text_of(strings.as_deref(), text), subtitle, place: Place { canvas: A2_CANVAS, at: Vec2::new(x, y) }, left: (secs > 0.0).then_some(secs), a1_rect: None, age: 0.0, out: None });
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
            Presentation::Call { name, .. } if matches!(name.split_once('.').map_or("", |x| x.1), "CenterCamera" | "SetCameraZoom" | "CineractiveConstantSpeed" | "CineractiveCameraOffset" | "CineractiveShot") => {}
            Presentation::Call { name, .. } => {
                unhandled.note(name);
            }
            // A script's voice or message sound goes to the audio layer by name (the resolver
            // finds `Data/sounds/...` by stem, events by name); the host answers the script.
            Presentation::Sound { id, name, .. } => {
                // The UI bus: the voice bus drops a line while another plays (a unit's acknowledgement,
                // or the script's previous message).
                play.write(PlaySound { name: name.clone(), at: None, bus: Bus::Ui });
                // The message ends when its clip does (`sound_replies`); with no audio layer to
                // say how long it is, at once.
                if reports.is_some() {
                    sounds.pending.push((*id, name.clone(), None));
                } else {
                    replies.write(ScriptReply::SoundDone { id: *id });
                }
            }
            // `Media:stopSounds()` stops every script sound: that bus (not per sound). A stopped
            // message is over for the script, which has already forgotten it.
            Presentation::StopSound { id } => {
                stop.write(StopSounds { bus: Bus::Ui });
                sounds.pending.retain(|p| p.0 != *id);
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

/// Ends the mission messages: the audio layer says how long each clip is when it takes the request
/// ([`crate::audio::SoundStarted`], matched by name in request order), and the message is done
/// that long after, in real time, as the player hears it. A clip that did not resolve ends at once.
fn sound_replies(mut started: MessageReader<crate::audio::SoundStarted>, time: Res<Time<Real>>, mut sounds: ResMut<ScriptSounds>, mut out: MessageWriter<ScriptReply>) {
    let dt = time.delta_secs();
    for p in &mut sounds.pending {
        if let Some(left) = p.2.as_mut() {
            *left -= dt;
        }
    }
    for s in started.read() {
        if s.bus != Bus::Ui {
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
        if let Some(rect) = t.a1_rect {
            let at = a1_title_at(rect, &t.text);
            let b = t.level();
            let base = if t.subtitle { [1.0; 3] } else { A1_TITLE_COLOUR };
            let [r, g, bl] = base.map(|c| c * b);
            commands.entity(root).with_child((
                Node { position_type: PositionType::Absolute, left: Val::Vh(at.x / A1_CANVAS.y * 100.0), top: Val::Vh(at.y / A1_CANVAS.y * 100.0), ..default() },
                children![sprite_text_styled(t.text.clone(), size, Color::srgb(r, g, bl), SpriteTextStyle::role(font_role(&theme, size)).wrap_to_parent())],
            ));
            continue;
        }
        commands.entity(root).with_child((
            Node { position_type: PositionType::Absolute, left: Val::Percent(p.at.x / p.canvas.x * 100.0), top: Val::Percent(p.at.y / p.canvas.y * 100.0), max_width: Val::Percent(80.0), ..default() },
            children![sprite_text_styled(t.text.clone(), size, Color::WHITE, SpriteTextStyle::role(font_role(&theme, size)).wrap_to_parent())],
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

/// The panel is A2 stock's `objectivesPanelArea=192 80 1280 1024` of 1600x1200, the
/// title and text 30/50 px in, the OK button at (952, 948) of 252x60 inside it.
fn draw_panel(
    mut commands: Commands,
    theme: Res<Theme>,
    strings: Option<Res<Strings>>,
    view: Res<ObjectivesView>,
    queue: Res<MessageQueue>,
    roots: Query<Entity, With<PanelRoot>>,
    mut shown: Local<Option<PanelContent>>,
) {
    let want = panel_content(&view, &queue);
    if want == *shown && !theme.is_changed() {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    *shown = want.clone();
    let Some(c) = want else { return };
    let s = strings.as_deref();
    let (pw, ph) = (1280.0, 1024.0);
    let pct = |v: f32, of: f32| Val::Percent(v / of * 100.0);
    let head = |t: String, colour: Color| (Node { margin: UiRect::bottom(Val::Px(theme.gap)), ..default() }, children![sprite_text_styled(t, theme.heading_size, colour, SpriteTextStyle::role(font_role(&theme, theme.heading_size)).wrap_to_parent())]);
    let line = |t: String, colour: Color| (Node { margin: UiRect::bottom(Val::Px(theme.gap)), ..default() }, children![sprite_text_styled(t, theme.text_size, colour, SpriteTextStyle::role(font_role(&theme, theme.text_size)).wrap_to_parent())]);
    let summary_colour = Color::srgb(0.8, 1.0, 1.0);
    let ok = fixed(s, "GUI_POP_OBJ_BUTTON", "OK");
    let mut panel = commands.spawn((
        PanelRoot,
        Node { position_type: PositionType::Absolute, left: pct(192.0, 1600.0), top: pct(80.0, 1200.0), width: pct(pw, 1600.0), height: pct(ph, 1200.0), flex_direction: FlexDirection::Column, padding: UiRect { left: pct(50.0, pw), right: pct(30.0, pw), top: pct(50.0, ph), bottom: Val::Px(0.0) }, ..default() },
        BackgroundColor(theme.panel.with_alpha(0.92)),
        GlobalZIndex(920),
    ));
    panel.with_children(|p| {
        if !c.title.is_empty() {
            p.spawn(head(c.title.clone(), theme.title));
        }
        if c.message.is_none() && !c.body.is_empty() {
            p.spawn(head(fixed(s, "GUI_OD_BRIEFING_SUMMARY", "BRIEFING SUMMARY:"), summary_colour));
        }
        if !c.body.is_empty() {
            p.spawn(line(c.body.clone(), theme.text));
        }
        if c.message.is_none() && !c.lines.is_empty() {
            p.spawn(head(fixed(s, "GUI_OD_OBJECTIVES", "OBJECTIVES:"), summary_colour));
            for l in &c.lines {
                let (mark, colour) = match l.state {
                    ObjectiveState::Active => ("* ", theme.text),
                    ObjectiveState::Complete => ("* ", theme.good),
                    ObjectiveState::Failed => ("* ", theme.bad),
                };
                p.spawn(line(format!("{mark}{}", l.text), colour));
            }
        }
        p.spawn((Node { position_type: PositionType::Absolute, left: pct(952.0, pw), top: pct(948.0, ph), ..default() }, children![(OkButton, button(&theme, &ok, 252.0))]));
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

/// Back at the front end (UI-34): titles, fades, queued message boxes, pending movie replies and the
/// objectives panel of the match are gone, whatever ended it (a mission's debrief, F10, the in-game menu).
fn reset_presentation(
    mut titles: ResMut<Titles>,
    mut fade: ResMut<FadeState>,
    mut cine: ResMut<CineractiveState>,
    mut movies: ResMut<PendingMovies>,
    mut view: ResMut<ObjectivesView>,
    mut queue: ResMut<MessageQueue>,
    mut sounds: ResMut<ScriptSounds>,
) {
    sounds.pending.clear();
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
            .init_resource::<PendingMovies>()
            .init_resource::<ScriptSounds>()
            .add_message::<crate::audio::SoundStarted>()
            .init_resource::<ObjectivesView>()
            .init_resource::<MessageQueue>()
            .add_systems(OnEnter(crate::app::AppState::FrontEnd), reset_presentation)
            .add_message::<ScriptReply>()
            .add_message::<MessageBox>()
            .add_message::<crate::media::mission::Requested>()
            .add_message::<MovieFinished>()
            .add_message::<SimMsg>()
            .add_message::<PlaySound>()
            .add_message::<StopSounds>()
            .add_message::<ButtonClicked>()
            .add_systems(Startup, spawn_layers.run_if(resource_exists::<Theme>))
            .add_systems(
                Update,
                (consume, sound_replies, hide_notices_in_cineractive, run_clock, movie_replies, skip_cineractive, apply_replies, toggle_objectives, queue_messages, ok_pressed, draw_titles.run_if(resource_exists::<Theme>), draw_fade, draw_panel.run_if(resource_exists::<Theme>)).chain().in_set(GameSet::Ui),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::sprite_font::SpriteText;
    use sta_core::Fixed;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<ButtonInput<KeyCode>>().add_plugins(PresentationPlugin);
        crate::app::configure_sets(&mut app);
        app
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
        let mut app = app();
        send(&mut app, Presentation::Sound { id: 1, name: "Voice.mp3".into(), at: None });
        let heard: Vec<String> = app.world_mut().resource_mut::<Messages<PlaySound>>().drain().map(|m| m.name).collect();
        assert_eq!(heard, ["Voice.mp3"]);
        send(&mut app, Presentation::StopSound { id: 1 });
        assert_eq!(app.world_mut().resource_mut::<Messages<StopSounds>>().drain().count(), 1);
        assert_eq!(app.world().resource::<Unhandled>().count("Sound"), 0);
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
        send(app, Presentation::Title { id: 1, text: "'PREMONITIONS'".into(), subtitle: false, seconds: None });
        send(app, Presentation::Call { name: "A1.ShowTitle".into(), args: vec![Arg::Int(1), Arg::Int(0), Arg::Int(0), Arg::Int(200), Arg::Int(70), Arg::Bool(true)] });
    }

    /// Federation 1's `ShowTitle(0, 0, 200, 70, "'PREMONITIONS'", true)`: blue, top-left, placed in
    /// fractions of the window's height (so a 21:9 window has the same margins in proportion to its
    /// height, no 4:3 box and no bars), fading in over a second.
    #[test]
    fn an_a1_title_is_blue_top_left_anchored_to_the_height_and_fades_in() {
        let mut app = app();
        app.insert_resource(Theme::default());
        app.update();
        a1_title(&mut app);
        let at = a1_title_at([0.0, 0.0, 200.0, 70.0], "'PREMONITIONS'");
        assert!((at.x - 63.0).abs() < 4.0 && (at.y - 29.5).abs() < 1.0, "{at:?}");
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
        a1_title(&mut app);
        app.world_mut().resource_mut::<Titles>().tick(3.0);
        assert_eq!(title_colours(&mut app).first().map(|c| (c.red, c.green, c.blue)), Some((0.0, 0.5, 1.0)), "held at full level");
        send(&mut app, Presentation::Call { name: "A1.RemoveTitleFade".into(), args: vec![Arg::Int(1)] });
        assert_eq!(app.world().resource::<Titles>().items.len(), 1, "still there");
        app.world_mut().resource_mut::<Titles>().tick(0.5);
        let c = title_colours(&mut app)[0];
        assert!((c.blue - 0.5).abs() < 0.02 && (c.green - 0.25).abs() < 0.02, "half way down: {c:?}");
        app.world_mut().resource_mut::<Titles>().tick(0.6);
        app.update();
        assert!(app.world().resource::<Titles>().items.is_empty(), "gone after the second");
        assert!(title_colours(&mut app).is_empty());
        // A fade out begun while still fading in goes down from the level reached.
        a1_title(&mut app);
        app.world_mut().resource_mut::<Titles>().tick(0.5);
        send(&mut app, Presentation::Call { name: "A1.RemoveTitleFade".into(), args: vec![Arg::Int(1)] });
        app.world_mut().resource_mut::<Titles>().tick(0.4);
        assert!((app.world().resource::<Titles>().items[0].level() - 0.1).abs() < 0.02);
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
        app.init_resource::<ReportsSoundLengths>().add_message::<SoundStarted>();
        send(&mut app, Presentation::Sound { id: 7, name: "log.wav".into(), at: None });
        assert!(replies(&mut app).is_empty(), "not done when it starts");
        app.world_mut().write_message(SoundStarted { name: "log.wav".into(), bus: Bus::Ui, seconds: Some(0.0) });
        app.update();
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 7 }], "a clip of no length is over at once");
        send(&mut app, Presentation::Sound { id: 8, name: "log.wav".into(), at: None });
        app.world_mut().write_message(SoundStarted { name: "log.wav".into(), bus: Bus::Ui, seconds: Some(3600.0) });
        app.update();
        assert!(replies(&mut app).is_empty() && app.world().resource::<ScriptSounds>().playing() == [8], "a long clip is still playing");
        send(&mut app, Presentation::StopSound { id: 8 });
        assert!(app.world().resource::<ScriptSounds>().playing().is_empty(), "stopped by the script");
    }

    #[test]
    fn without_an_audio_layer_a_message_ends_at_once() {
        let mut app = app();
        send(&mut app, Presentation::Sound { id: 3, name: "x.wav".into(), at: None });
        assert_eq!(replies(&mut app), [ScriptReply::SoundDone { id: 3 }]);
    }

    #[test]
    fn objectives_fill_from_the_runtime_types() {
        let mut v = ObjectivesView::default();
        let o = |id, state| Objective { id, title: format!("t{id}"), description: "d".into(), state };
        v.fill(&[o(1, ObjectiveState::Active), o(2, ObjectiveState::Complete)], Some(&ObjectiveBriefing { file: "f".into(), title: "Mission".into(), text: "Go.".into(), notes: String::new() }));
        assert_eq!((v.title.as_str(), v.items.len(), v.items[1].state), ("Mission", 2, ObjectiveState::Complete));
    }
}
