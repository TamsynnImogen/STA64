//! Movie playback (MED-03) and the game's intro movies (MED-05, [`intro`]).
//!
//! A movie is asked for with a [`PlayMovie`] message and ends with a
//! [`MovieFinished`] message, whatever the reason: it played to its end, the
//! player skipped it, it was stopped, or it could not be played. A movie that
//! is missing, unsupported (AV1 in an MP4) or damaged never stops the flow that
//! asked for it: it is logged and reported as [`MovieOutcome::Failed`], which
//! callers treat as done ([`MovieOutcome::is_done`]).
//!
//! # Targets
//!
//! * [`MovieTarget::Fullscreen`]: a black overlay over the whole UI with the
//!   picture fitted to the window. One at a time; further requests wait their
//!   turn in order. Esc, Space, Enter or a click skips a `skippable` movie.
//! * [`MovieTarget::Panel`]: frames are written into the given image, for the
//!   bridge viewscreen (MED-04) or any UI panel to show. [`PanelScreen`] holds
//!   the image a mission's in-panel movie goes to.
//!
//! # Threads and sync
//!
//! The file is read on the main thread (a `Vfs` read) and everything else
//! happens on a decoder thread ([`decoder`]): decode, YUV to RGBA, audio
//! into a [`PcmStream`]. The audio backend pulls the stream at the device's
//! pace (`AudioBackend::play_stream`) and the number of frames it has pulled
//! is the movie's clock, so the picture follows the sound. A backend that takes
//! no streams (a missing device) leaves the wall clock in charge.
//!
//! # Mission events
//!
//! [`mission`] turns the `Presentation::Movie` / `StopMovie` events a mission
//! raises (`Media:playMovie`, `PlayCinematicMovie`, `PlayBridgeMovie`) into
//! these messages.

// The API is for MED-04, UI-30 and the bridge display; the binary uses the intro and mission paths.
#![allow(dead_code)]

mod decoder;
pub mod intro;
pub mod locate;
pub mod mission;
mod pcm;

use std::collections::VecDeque;
use std::time::Duration;

use bevy::asset::RenderAssetUsages;
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;

use crate::app::GameSet;
use crate::audio::{Mixer, Volumes};
use crate::sim_link::Bus;
use crate::viewer::Mounted;
use decoder::{Decoder, Frame, Msg};
pub use intro::{IntroPlugin, IntroSettings};
pub use pcm::PcmStream;

/// The bus movie sound plays on.
pub const MOVIE_BUS: Bus = Bus::Voice;
/// Frames decoded before playback starts (a smooth start without a long wait).
const START_FRAMES: usize = 6;
/// Audio queued before playback starts, seconds.
const START_AUDIO_SECS: f64 = 0.2;
/// Start with whatever is ready after this long.
const START_TIMEOUT: f64 = 2.0;
/// Give up on a movie that shows no frame this long after it started.
const STALL_TIMEOUT: f64 = 10.0;

/// Names one request, chosen by whoever asks (see [`MovieIds`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MovieId(pub u64);

/// Hands out ids.
#[derive(Resource, Default)]
pub struct MovieIds(u64);

impl MovieIds {
    pub fn next(&mut self) -> MovieId {
        self.0 += 1;
        MovieId(self.0)
    }
}

#[derive(Debug, Clone)]
pub enum MovieTarget {
    Fullscreen,
    /// Frames go into this image.
    Panel(Handle<Image>),
}

/// Ask for a movie. `name` is what the game's data says (`Intro.bik`).
#[derive(Message, Debug, Clone)]
pub struct PlayMovie {
    pub id: MovieId,
    pub name: String,
    pub target: MovieTarget,
    /// The player may skip it (Esc and the like); only for fullscreen.
    pub skippable: bool,
}

/// Stop one movie, or every movie (`None`), playing or waiting. Each reports [`MovieOutcome::Stopped`].
#[derive(Message, Debug, Clone, Copy)]
pub struct StopMovie(pub Option<MovieId>);

/// A request is over.
#[derive(Message, Debug, Clone)]
pub struct MovieFinished {
    pub id: MovieId,
    pub name: String,
    pub outcome: MovieOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MovieOutcome {
    /// Played to its end.
    Completed,
    /// The player skipped it.
    Skipped,
    /// Stopped by a [`StopMovie`].
    Stopped,
    /// Could not be played (or broke off); the reason was logged.
    Failed(String),
}

impl MovieOutcome {
    /// Whatever the reason, the request is over: a script waiting for it goes on.
    pub fn is_done(&self) -> bool {
        true
    }
}

/// How names are opened, when not from the mounted install (tests supply synthetic movies).
#[derive(Resource)]
pub(crate) struct MovieOpener(pub Box<OpenFn>);

pub(crate) type OpenFn = dyn Fn(&str) -> Result<locate::Opened, String> + Send + Sync;

/// The image a mission's in-panel movie (the bridge viewscreen) is written to.
#[derive(Resource, Clone)]
pub struct PanelScreen(pub Handle<Image>);

/// The player a movie is fitted into: its picture's size, to place it in the window.
#[derive(Component)]
struct MovieOverlay {
    id: MovieId,
}

#[derive(Component)]
struct MovieFit {
    size: (u32, u32),
}

enum Phase {
    /// Waiting for the decoder to have enough ready.
    Buffering,
    Playing,
}

struct Player {
    id: MovieId,
    name: String,
    skippable: bool,
    image: Handle<Image>,
    overlay: Option<Entity>,
    decoder: Decoder,
    stream: Option<PcmStream>,
    has_video: bool,
    size: (u32, u32),
    phase: Phase,
    /// Wall seconds in the current phase.
    waited: f64,
    clock: Duration,
    frames: VecDeque<Frame>,
    shown: u64,
    ended: bool,
    failure: Option<String>,
    /// Wall seconds since the video ran out.
    tail: f64,
    tail_budget: f64,
}

/// Requests waiting and movies playing.
#[derive(Resource, Default)]
pub struct Movies {
    pending: VecDeque<PlayMovie>,
    fullscreen: Option<Player>,
    panels: Vec<Player>,
    skip: bool,
}

impl Movies {
    /// A movie is on screen or waiting for the screen.
    pub fn busy(&self) -> bool {
        self.fullscreen.is_some() || !self.pending.is_empty() || !self.panels.is_empty()
    }

    /// A fullscreen movie is playing or queued.
    pub fn fullscreen_busy(&self) -> bool {
        self.fullscreen.is_some() || self.pending.iter().any(|p| matches!(p.target, MovieTarget::Fullscreen))
    }

    /// Ids of every movie playing or queued, oldest first.
    pub fn active_ids(&self) -> Vec<MovieId> {
        self.fullscreen.iter().map(|p| p.id).chain(self.panels.iter().map(|p| p.id)).chain(self.pending.iter().map(|p| p.id)).collect()
    }
}

pub struct MediaPlugin;

impl Plugin for MediaPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlayMovie>()
            .add_message::<StopMovie>()
            .add_message::<MovieFinished>()
            .add_message::<mission::Requested>()
            .add_message::<crate::sim_link::SimMsg>()
            .init_resource::<Movies>()
            .init_resource::<MovieIds>()
            .add_systems(Startup, init_panel)
            .add_systems(PreUpdate, skip_keys.after(InputSystems))
            .add_systems(
                Update,
                (mission::bridge, intake, drive, fit_overlays, duck_music).chain().in_set(GameSet::Ui),
            );
    }
}

fn init_panel(mut commands: Commands, images: Option<ResMut<Assets<Image>>>) {
    if let Some(mut images) = images {
        commands.insert_resource(PanelScreen(images.add(blank(16, 9))));
    }
}

fn blank(w: u32, h: u32) -> Image {
    Image::new_fill(
        Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Esc, Space, Enter or a click asks to skip the fullscreen movie, and the press is taken
/// so the front end underneath does not also act on it.
fn skip_keys(
    keys: Option<ResMut<ButtonInput<KeyCode>>>,
    mouse: Option<ResMut<ButtonInput<MouseButton>>>,
    mut movies: ResMut<Movies>,
) {
    let skippable = movies.fullscreen.as_ref().is_some_and(|p| p.skippable);
    if !skippable {
        return;
    }
    if let Some(mut keys) = keys {
        for k in [KeyCode::Escape, KeyCode::Space, KeyCode::Enter, KeyCode::NumpadEnter] {
            if keys.just_pressed(k) {
                keys.clear_just_pressed(k);
                movies.skip = true;
            }
        }
    }
    if let Some(mut mouse) = mouse
        && mouse.just_pressed(MouseButton::Left)
    {
        mouse.clear_just_pressed(MouseButton::Left);
        movies.skip = true;
    }
}

/// Take new requests and stop requests.
fn intake(
    mut play: MessageReader<PlayMovie>,
    mut stop: MessageReader<StopMovie>,
    mut movies: ResMut<Movies>,
    mut done: MessageWriter<MovieFinished>,
    mut commands: Commands,
) {
    for r in play.read() {
        movies.pending.push_back(r.clone());
    }
    for StopMovie(which) in stop.read() {
        let matches = |id: MovieId| which.is_none_or(|w| w == id);
        let mut gone = Vec::new();
        movies.pending.retain(|p| {
            let hit = matches(p.id);
            if hit {
                gone.push((p.id, p.name.clone()));
            }
            !hit
        });
        if movies.fullscreen.as_ref().is_some_and(|p| matches(p.id)) {
            let p = movies.fullscreen.take().unwrap();
            gone.push(finish(p, &mut commands));
        }
        let mut i = 0;
        while i < movies.panels.len() {
            if matches(movies.panels[i].id) {
                let p = movies.panels.remove(i);
                gone.push(finish(p, &mut commands));
            } else {
                i += 1;
            }
        }
        for (id, name) in gone {
            done.write(MovieFinished { id, name, outcome: MovieOutcome::Stopped });
        }
    }
}

/// Wind a player down: stop its thread and audio, take its overlay away.
fn finish(p: Player, commands: &mut Commands) -> (MovieId, String) {
    p.decoder.cancel();
    if let Some(s) = &p.stream {
        s.cancel();
    }
    if let Some(e) = p.overlay {
        commands.entity(e).despawn();
    }
    (p.id, p.name)
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time<Real>>,
    mut movies: ResMut<Movies>,
    vfs: Option<Res<Mounted>>,
    opener: Option<Res<MovieOpener>>,
    images: Option<ResMut<Assets<Image>>>,
    mut mixer: Option<NonSendMut<Mixer>>,
    mut done: MessageWriter<MovieFinished>,
    mut commands: Commands,
) {
    let dt = time.delta_secs_f64();
    let Some(mut images) = images else {
        // No renderer to show a movie in: every request is over at once.
        let m = &mut *movies;
        for p in m.pending.drain(..) {
            done.write(MovieFinished { id: p.id, name: p.name, outcome: MovieOutcome::Failed("no renderer".into()) });
        }
        return;
    };

    // Start what is waiting: panels at once, fullscreen when the screen is free.
    let mut waiting = std::mem::take(&mut movies.pending);
    while let Some(req) = waiting.pop_front() {
        let fullscreen = matches!(req.target, MovieTarget::Fullscreen);
        if fullscreen && movies.fullscreen.is_some() {
            movies.pending.push_back(req);
            continue;
        }
        let opened = match (&opener, &vfs) {
            (Some(o), _) => (o.0)(&req.name),
            (None, Some(v)) => locate::open(&v.0, &req.name),
            (None, None) => Err("no install mounted".to_owned()),
        };
        let started = opened.map(|o| start(&req, o, &mut images, &mut commands));
        match started {
            Ok(p) if fullscreen => movies.fullscreen = Some(p),
            Ok(p) => movies.panels.push(p),
            Err(why) => {
                warn!("movie {:?}: {why}; skipped", req.name);
                done.write(MovieFinished { id: req.id, name: req.name, outcome: MovieOutcome::Failed(why) });
            }
        }
    }

    let skip = std::mem::take(&mut movies.skip);
    let mut ended: Vec<(Player, MovieOutcome)> = Vec::new();
    if let Some(mut p) = movies.fullscreen.take() {
        let skipped = skip && p.skippable;
        match step(&mut p, dt, skipped, &mut images, mixer.as_deref_mut()) {
            Some(o) => ended.push((p, o)),
            None => movies.fullscreen = Some(p),
        }
    }
    for mut p in std::mem::take(&mut movies.panels) {
        match step(&mut p, dt, false, &mut images, mixer.as_deref_mut()) {
            Some(o) => ended.push((p, o)),
            None => movies.panels.push(p),
        }
    }
    for (p, outcome) in ended {
        if let MovieOutcome::Failed(why) = &outcome {
            warn!("movie {:?}: {why}", p.name);
        }
        let (id, name) = finish(p, &mut commands);
        done.write(MovieFinished { id, name, outcome });
    }
}

/// Open the movie and set up its thread, audio queue, image and overlay.
fn start(req: &PlayMovie, opened: locate::Opened, images: &mut Assets<Image>, commands: &mut Commands) -> Player {
    let info = opened.movie.info().clone();
    let size = info.video.as_ref().map_or((0, 0), |v| (v.width, v.height));
    info!("movie {:?}: {} ({}x{}, {})", req.name, opened.path, size.0, size.1, info.video.as_ref().map_or("audio only", |v| v.codec.as_str()));
    let stream = match (&opened.sidecar, info.audio.first()) {
        (Some((rate, _)), _) => Some(PcmStream::new(*rate)),
        (None, Some(a)) => Some(PcmStream::new(a.sample_rate)),
        _ => None,
    };
    let image = match &req.target {
        MovieTarget::Panel(h) => h.clone(),
        MovieTarget::Fullscreen => images.add(blank(size.0, size.1)),
    };
    if let (MovieTarget::Panel(h), true) = (&req.target, size.0 > 0)
        && let Some(mut img) = images.get_mut(h)
    {
        *img = blank(size.0, size.1);
    }
    let overlay = matches!(req.target, MovieTarget::Fullscreen).then(|| spawn_overlay(commands, req.id, image.clone(), size));
    let decoder = Decoder::spawn(opened.movie, stream.clone(), opened.sidecar);
    Player {
        id: req.id,
        name: req.name.clone(),
        skippable: req.skippable,
        image,
        overlay,
        decoder,
        stream,
        has_video: info.video.is_some(),
        size,
        phase: Phase::Buffering,
        waited: 0.0,
        clock: Duration::ZERO,
        frames: VecDeque::new(),
        shown: 0,
        ended: false,
        failure: None,
        tail: 0.0,
        tail_budget: 1.0,
    }
}

fn spawn_overlay(commands: &mut Commands, id: MovieId, image: Handle<Image>, size: (u32, u32)) -> Entity {
    commands
        .spawn((
            MovieOverlay { id },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(i32::MAX - 4),
            FocusPolicy::Block,
            Pickable { should_block_lower: true, is_hoverable: true },
            children![(
                MovieFit { size },
                ImageNode::new(image),
                Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                Pickable::IGNORE,
            )],
        ))
        .id()
}

/// Size each overlay's picture to the largest that fits the window at its own aspect.
fn fit_overlays(window: Query<&Window, With<PrimaryWindow>>, mut fits: Query<(&MovieFit, &mut Node)>) {
    let Ok(w) = window.single() else { return };
    let (ww, wh) = (w.width(), w.height());
    for (fit, mut node) in &mut fits {
        let (pw, ph) = (fit.size.0 as f32, fit.size.1 as f32);
        if pw <= 0.0 || ph <= 0.0 || ww <= 0.0 || wh <= 0.0 {
            continue;
        }
        let scale = (ww / pw).min(wh / ph);
        let (nw, nh) = (Val::Px(pw * scale), Val::Px(ph * scale));
        if node.width != nw || node.height != nh {
            node.width = nw;
            node.height = nh;
        }
    }
}

/// One frame of one movie; `Some` when it is over.
fn step(p: &mut Player, dt: f64, skip: bool, images: &mut Assets<Image>, mixer: Option<&mut Mixer>) -> Option<MovieOutcome> {
    if skip {
        return Some(MovieOutcome::Skipped);
    }
    while let Some(m) = p.decoder.try_recv() {
        match m {
            Msg::Frame(f) => p.frames.push_back(f),
            Msg::End => p.ended = true,
            Msg::Failed(e) => {
                p.failure = Some(e);
                p.ended = true;
            }
        }
    }
    p.waited += dt;
    match p.phase {
        Phase::Buffering => {
            let audio_ready = p.stream.as_ref().is_none_or(|s| s.queued_secs() >= START_AUDIO_SECS);
            let video_ready = !p.has_video || p.frames.len() >= START_FRAMES;
            if p.ended || p.waited > START_TIMEOUT || (video_ready && audio_ready) {
                if p.ended && p.frames.is_empty() && p.shown == 0 && p.failure.is_some() && p.stream.as_ref().is_none_or(|s| s.queued_frames() == 0) {
                    return Some(MovieOutcome::Failed(p.failure.take().unwrap_or_default()));
                }
                if let (Some(s), Some(m)) = (&p.stream, mixer) {
                    m.backend.play_stream(s, MOVIE_BUS);
                }
                p.phase = Phase::Playing;
                p.waited = 0.0;
            }
            None
        }
        Phase::Playing => {
            // The sound's position is the clock while a backend is pulling it; otherwise wall time.
            match &p.stream {
                Some(s) if s.is_attached() && !s.is_drained() => p.clock = p.clock.max(s.position()),
                _ => p.clock += Duration::from_secs_f64(dt),
            }
            p.decoder.set_clock(p.clock);
            let mut latest = None;
            while p.frames.front().is_some_and(|f| f.pts <= p.clock) {
                latest = p.frames.pop_front();
            }
            if let Some(f) = latest {
                show(p, f, images);
            }
            if p.shown == 0 && p.waited > STALL_TIMEOUT && p.has_video {
                return Some(MovieOutcome::Failed("no picture decoded".into()));
            }
            if p.ended && p.frames.is_empty() {
                // The picture is over; the sound may run a little longer.
                if p.tail == 0.0 {
                    p.tail_budget = 1.0 + p.stream.as_ref().map_or(0.0, PcmStream::queued_secs);
                }
                p.tail += dt.max(1e-6);
                let sound_over = p.stream.as_ref().is_none_or(|s| !s.is_attached() || s.is_drained());
                if sound_over || p.tail > p.tail_budget {
                    return Some(match p.failure.take() {
                        Some(e) => MovieOutcome::Failed(e),
                        None => MovieOutcome::Completed,
                    });
                }
            }
            None
        }
    }
}

fn show(p: &mut Player, f: Frame, images: &mut Assets<Image>) {
    let Some(mut img) = images.get_mut(&p.image) else { return };
    if (img.width(), img.height()) != (f.width, f.height) {
        *img = blank(f.width, f.height);
    }
    img.data = Some(f.rgba);
    p.size = (f.width, f.height);
    p.shown += 1;
}

/// Movie sound replaces the music while a fullscreen movie is up.
fn duck_music(movies: Res<Movies>, volumes: Option<ResMut<Volumes>>, mut saved: Local<Option<f32>>) {
    let Some(mut volumes) = volumes else { return };
    let up = movies.fullscreen_busy();
    match (*saved, up) {
        (None, true) => {
            *saved = Some(volumes.music);
            volumes.music = 0.0;
        }
        (Some(v), false) => {
            if volumes.music == 0.0 {
                volumes.music = v;
            }
            *saved = None;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
