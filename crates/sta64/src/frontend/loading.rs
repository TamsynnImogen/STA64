//! The loading screen (UI-08), after Refit's `ui/LOADING.md` and the "Screens"
//! section of its `STYLE.md`: the map's own sky edge to edge, the mission's
//! kicker, title and briefing down the left, and across the foot a ring of dots
//! turning, what the load is doing, the percentage and the theme's bar.
//!
//! # Driving it (INT-M3 / INT-M4)
//!
//! [`LoadingPlugin`] is for a game mode to add. The screen shows while a load
//! is under way ([`LoadProgress::is_active`]) and goes away by itself a few
//! frames after the last stage finishes. A load is told to the screen as
//! stages the game's load really has ([`LoadStage`]): mount the install,
//! build the content database, convert the map, load models and textures, set
//! up the sim. Each stage reports `done` of `total` steps, either by writing
//! a [`LoadReport`] message or by calling [`LoadProgress::report`] directly:
//!
//! ```ignore
//! app.add_plugins(LoadingPlugin);
//! commands.insert_resource(LoadInfo::from_map(&map_def)); // title and sky
//! reports.write(LoadReport::step(LoadStage::Assets, 12, 80, "fgalaxy.sod"));
//! reports.write(LoadReport::finished(LoadStage::Assets));
//! ```
//!
//! The first report begins a load; [`LoadProgress::begin`] does so explicitly
//! (and starts over after an earlier load). A stage that is never reported
//! counts as not done: report every stage the load has, and
//! [`LoadReport::finished`] one that was skipped.
//!
//! # Marks
//!
//! The load's progress is also a list of bench marks ([`LoadProgress::marks`],
//! in the format `sta_bench::marks` writes, stamped with the frame since the
//! load began): `load.begin`, `load.stage stage=<name> pct=<n>`, `load.progress
//! pct=<tens>`, `load.stage.end stage=<name> pct=<n>` and `load.done`. A frame
//! run prints each as a `mark` line and appends it to `$STA64_BENCH_OUT/load.marks`
//! (the run's `marks` file is the sim's, written whole at the end of the run).
//!
//! # The sky
//!
//! The faces of the background the map names are put up with the map's own
//! [`space::sky_faces`](crate::render::space::sky_faces) and looked at from
//! inside by a slowly turning camera of this screen. Without a mounted install,
//! or without the faces, the screen draws the theme's gradient and a fixed
//! field of faint stars, so it works without assets.

// Wired into the game by INT-M4; until then only the frame run and tests use it.
#![allow(dead_code)]

use std::f32::consts::TAU;

use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::ui::UiTargetCamera;
use bevy::window::PrimaryWindow;
use sta_bench::marks::{Mark, Val as MarkVal};
use sta_content::MapDef;

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::textures::{Loader, TextureCache};
use crate::ui::theme::{Fill, ProgressBar, Theme, ThemePlugin, mix};
use crate::viewer::Mounted;

// ---------------------------------------------------------------------------
// The load as stages
// ---------------------------------------------------------------------------

/// The parts of a game load, in the order the game does them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LoadStage {
    /// Mount the install and the mod chain (`Vfs::mount_install`).
    Mount,
    /// Read the game's data into the content database (`ContentDb::load`).
    Content,
    /// Convert the map (`.bzn`) into a `MapDef` and its scene.
    MapConvert,
    /// Models and textures of everything the map names.
    Assets,
    /// Build the sim from the `GameSetup` and spawn the map's objects.
    SimSetup,
}

impl LoadStage {
    pub const ALL: [LoadStage; 5] = [LoadStage::Mount, LoadStage::Content, LoadStage::MapConvert, LoadStage::Assets, LoadStage::SimSetup];

    /// The name a mark carries.
    pub fn name(self) -> &'static str {
        match self {
            LoadStage::Mount => "mount",
            LoadStage::Content => "content",
            LoadStage::MapConvert => "map",
            LoadStage::Assets => "assets",
            LoadStage::SimSetup => "sim",
        }
    }

    /// What the screen says while the stage runs, in the player's words.
    pub fn doing(self) -> &'static str {
        match self {
            LoadStage::Mount => "Mounting the game files",
            LoadStage::Content => "Reading the game's data",
            LoadStage::MapConvert => "Building the map",
            LoadStage::Assets => "Loading models and textures",
            LoadStage::SimSetup => "Setting up the simulation",
        }
    }

    /// Share of the whole bar, in percent; the five add up to 100. Models and
    /// textures are the long part of a real load, mounting the short one.
    pub fn weight(self) -> u32 {
        match self {
            LoadStage::Mount => 5,
            LoadStage::Content => 15,
            LoadStage::MapConvert => 15,
            LoadStage::Assets => 50,
            LoadStage::SimSetup => 15,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One stage's progress, to the screen: `done` of `total` steps, and the step
/// being done (a file name, say) when the stage knows it. Write it as a message,
/// or hand it to [`LoadProgress::report`].
#[derive(Message, Debug, Clone, PartialEq)]
pub struct LoadReport {
    pub stage: LoadStage,
    pub done: u32,
    pub total: u32,
    pub step: Option<String>,
}

impl LoadReport {
    pub fn step(stage: LoadStage, done: u32, total: u32, step: impl Into<String>) -> LoadReport {
        LoadReport { stage, done, total, step: Some(step.into()) }
    }

    /// `done` of `total` with no step text (the stage's own words show).
    pub fn counts(stage: LoadStage, done: u32, total: u32) -> LoadReport {
        LoadReport { stage, done, total, step: None }
    }

    /// The stage is over (also for one the load skipped).
    pub fn finished(stage: LoadStage) -> LoadReport {
        LoadReport { stage, done: 1, total: 1, step: None }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct StageState {
    started: bool,
    finished: bool,
    /// 0..=1, never goes down.
    fraction: f32,
}

/// Frames the finished screen stays up before it goes (a blink at 60 a second).
const HOLD_FRAMES: u32 = 20;

/// Where the load is: the five stages, the step in hand and the marks so far.
#[derive(Resource, Debug, Default)]
pub struct LoadProgress {
    stages: [StageState; 5],
    step: Option<String>,
    active: bool,
    /// Frames since the load began; the marks' tick.
    frame: u32,
    done_at: Option<u32>,
    last_tens: u32,
    marks: Vec<Mark>,
}

impl LoadProgress {
    /// Start a load: the screen comes up, the stages are all to do, and the
    /// marks start afresh.
    pub fn begin(&mut self) {
        *self = LoadProgress { active: true, ..LoadProgress::default() };
        self.mark("load.begin", vec![]);
    }

    /// Whether the screen is to be up.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Every stage is finished.
    pub fn is_complete(&self) -> bool {
        self.stages.iter().all(|s| s.finished)
    }

    /// Overall progress in percent, 0..=100: the stages' weights times how far
    /// each is.
    pub fn percent(&self) -> u32 {
        let sum: f32 = LoadStage::ALL.iter().map(|s| s.weight() as f32 * self.stages[s.index()].fraction).sum();
        (sum.round() as u32).min(100)
    }

    /// 0..=1 for a bar.
    pub fn fraction(&self) -> f32 {
        self.percent() as f32 / 100.0
    }

    /// One stage's progress, 0..=1.
    pub fn stage_fraction(&self, stage: LoadStage) -> f32 {
        self.stages[stage.index()].fraction
    }

    /// The stage the screen says is under way: the first one not finished that
    /// has started, else the first not finished.
    pub fn current(&self) -> Option<LoadStage> {
        let open = |s: &&LoadStage| !self.stages[s.index()].finished;
        LoadStage::ALL.iter().find(|s| self.stages[s.index()].started && open(s)).or_else(|| LoadStage::ALL.iter().find(open)).copied()
    }

    /// What the screen says it is doing: the step's text after the stage's
    /// words, when the stage reported one.
    pub fn doing(&self) -> String {
        let Some(stage) = self.current() else { return "Ready".into() };
        match &self.step {
            Some(step) if !step.is_empty() => format!("{} - {step}", stage.doing()),
            _ => stage.doing().to_owned(),
        }
    }

    /// Take one stage's report. The first report of a load, or the first after
    /// a finished one has gone, begins a load.
    pub fn report(&mut self, r: &LoadReport) {
        if !self.active {
            self.begin();
        }
        let i = r.stage.index();
        if !self.stages[i].started {
            self.stages[i].started = true;
            let pct = self.percent().to_string();
            self.mark("load.stage", vec![("stage", MarkVal::Text(r.stage.name().into())), ("pct", MarkVal::Text(pct))]);
        }
        if let Some(s) = &r.step {
            self.step = Some(s.clone());
        }
        let f = if r.total == 0 { 1.0 } else { (r.done as f32 / r.total as f32).clamp(0.0, 1.0) };
        if f > self.stages[i].fraction {
            self.stages[i].fraction = f;
        }
        if f >= 1.0 && !self.stages[i].finished {
            self.stages[i].finished = true;
            self.step = None;
            let pct = self.percent().to_string();
            self.mark("load.stage.end", vec![("stage", MarkVal::Text(r.stage.name().into())), ("pct", MarkVal::Text(pct))]);
        }
        let tens = self.percent() / 10 * 10;
        if tens > self.last_tens && tens < 100 {
            self.last_tens = tens;
            self.mark("load.progress", vec![("pct", MarkVal::Text(tens.to_string()))]);
        }
        if self.is_complete() && self.done_at.is_none() {
            self.done_at = Some(self.frame);
            self.mark("load.done", vec![]);
        }
    }

    /// One frame of the screen: the marks' clock, and the end of a finished
    /// load's hold, after which the screen goes.
    pub fn tick(&mut self) {
        if !self.active {
            return;
        }
        self.frame += 1;
        if self.done_at.is_some_and(|at| self.frame >= at + HOLD_FRAMES) {
            self.active = false;
        }
    }

    /// Take the screen down now.
    pub fn dismiss(&mut self) {
        self.active = false;
    }

    fn mark(&mut self, kind: &'static str, fields: Vec<(&'static str, MarkVal)>) {
        self.marks.push(Mark { tick: self.frame.into(), kind, fields });
    }

    /// The marks of this load so far.
    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// The marks as the bench's marks file writes them (`TICK KIND k=v ...`).
    pub fn marks_text(&self) -> String {
        marks_text(&self.marks)
    }
}

/// Marks as lines of the marks file.
pub fn marks_text(marks: &[Mark]) -> String {
    sta_bench::marks::render_with(marks, |id| format!("#{}", id.0))
}

// ---------------------------------------------------------------------------
// What the screen shows
// ---------------------------------------------------------------------------

/// The mission's own words and the sky behind them. Set before or while the
/// load runs; the screen is rebuilt when it changes.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct LoadInfo {
    /// The campaign and mission number, Instant Action or Multiplayer.
    pub kicker: String,
    pub title: String,
    pub briefing: Vec<String>,
    pub objectives: Vec<String>,
    /// The sky: the background the map names (`mbgborg.sod`, `mbg02`).
    pub background: String,
}

impl LoadInfo {
    /// Title and sky from a converted map (`MapDef`): its label, else its name.
    pub fn from_map(map: &MapDef) -> LoadInfo {
        LoadInfo {
            title: map.label.clone().unwrap_or_else(|| map.name.clone()),
            background: map.scenery.sky.clone().unwrap_or_default(),
            ..LoadInfo::default()
        }
    }
}

/// How the screen fits where it is drawn.
#[derive(Resource, Debug, Clone)]
pub struct LoadingConfig {
    /// Scale the UI to the primary window (`UiScale` = height / 1080). Off
    /// when something else sets the scale (the frame run).
    pub fit_scale: bool,
    /// Print the layout when it changes (`loading layout:` lines).
    pub log: bool,
}

impl Default for LoadingConfig {
    fn default() -> Self {
        LoadingConfig { fit_scale: true, log: false }
    }
}

/// Draw into this image instead of the window (the frame run).
#[derive(Resource, Debug, Clone, Default)]
pub struct LoadingTarget(pub Option<Handle<Image>>);

/// Theme units (pixels of a 1080-line screen) the layout is built from.
pub mod layout {
    pub const SCREEN_H: f32 = 1080.0;
    /// Space left and right of the text and the bar.
    pub const MARGIN: f32 = 96.0;
    /// Below the bar.
    pub const BOTTOM: f32 = 64.0;
    pub const BAR_H: f32 = 10.0;
    /// The status row (ring, words, percentage) above the bar.
    pub const STATUS_H: f32 = 48.0;
    pub const STATUS_GAP: f32 = 18.0;
    pub const RING: f32 = 40.0;
    pub const DOTS: usize = 12;
    /// The left text column's width.
    pub const TEXT_W: f32 = 760.0;
    /// The top of the title block.
    pub const TOP: f32 = 150.0;
}

/// Everything the screen spawns at the top level.
type ScreenParts = Or<(With<LoadingRoot>, With<LoadingCamera>, With<LoadingSky>)>;

/// The render layer of the sky, so nothing else of the app is drawn with it.
pub const LOADING_LAYER: usize = 29;

#[derive(Component)]
struct LoadingCamera;

#[derive(Component)]
struct LoadingRoot;

#[derive(Component)]
struct LoadingSky;

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct PercentText;

#[derive(Component)]
struct RingDot(usize);

#[derive(Component)]
struct LoadingBar;

/// What the screen put up, for the layout log and for tests.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ScreenSky {
    /// Sky faces found (of 6); 0 means the gradient and stars stand in.
    pub faces: usize,
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        app.add_message::<LoadReport>()
            .init_resource::<LoadProgress>()
            .init_resource::<LoadInfo>()
            .init_resource::<LoadingConfig>()
            .init_resource::<LoadingTarget>()
            .init_resource::<ScreenSky>()
            .init_resource::<UiScale>()
            .add_systems(
                Update,
                (
                    apply_reports,
                    rebuild_on_info_change,
                    show_screen,
                    layer_sky,
                    hide_screen,
                    fit_scale,
                    update_screen,
                    turn_sky,
                    tick_progress,
                    log_marks,
                )
                    .chain()
                    .in_set(GameSet::Ui),
            );
    }
}

fn apply_reports(mut reports: MessageReader<LoadReport>, mut progress: ResMut<LoadProgress>) {
    for r in reports.read() {
        progress.report(r);
    }
}

fn tick_progress(mut progress: ResMut<LoadProgress>) {
    progress.tick();
}

/// The marks of this load, as they come, to stdout (the bench's log) and, when
/// `$STA64_BENCH_OUT` names a directory, appended to its `load.marks` file (the run's own `marks` file is the sim's and is written whole at the end).
fn log_marks(progress: Res<LoadProgress>, mut seen: Local<usize>) {
    let marks = progress.marks();
    if marks.len() < *seen {
        *seen = 0;
    }
    if marks.len() == *seen {
        return;
    }
    let text = marks_text(&marks[*seen..]);
    for line in text.lines() {
        println!("mark {line}");
    }
    *seen = marks.len();
    if let Some(dir) = std::env::var_os("STA64_BENCH_OUT").filter(|d| !d.is_empty()) {
        use std::io::Write;
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("load.marks")) {
            let _ = f.write_all(text.as_bytes());
        }
    }
}

fn fit_scale(
    config: Res<LoadingConfig>,
    progress: Res<LoadProgress>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut ui_scale: ResMut<UiScale>,
) {
    if !config.fit_scale || !progress.is_active() {
        return;
    }
    if let Some(w) = window
        && w.height() > 0.0
    {
        let s = w.height() / layout::SCREEN_H;
        if (ui_scale.0 - s).abs() > 1e-4 {
            ui_scale.0 = s;
        }
    }
}

fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s.into()),
        TextFont { font_size: bevy::text::FontSize::Px(size), ..default() },
        TextColor(color),
        Pickable::IGNORE,
    )
}

fn stop(color: Color, at: f32) -> bevy::ui::ColorStop {
    bevy::ui::ColorStop::new(color, Val::Percent(at))
}

fn gradient(angle: f32, stops: Vec<bevy::ui::ColorStop>) -> BackgroundGradient {
    BackgroundGradient::from(bevy::ui::LinearGradient::new(angle, stops))
}

fn fill_node() -> Node {
    Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }
}

/// Small deterministic generator for the stand-in stars.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

/// The theme's gradient and a fixed field of faint stars, for a screen with no sky.
fn spawn_backdrop(p: &mut ChildSpawnerCommands, theme: &Theme) {
    let top = mix(theme.panel, theme.accent3, 0.10);
    let bottom = mix(theme.panel, theme.accent2, 0.04);
    p.spawn((fill_node(), gradient(bevy::ui::LinearGradient::TO_BOTTOM, vec![stop(top, 0.0), stop(bottom, 100.0)]), Pickable::IGNORE)).with_children(|s| {
        let mut rng = Lcg(0x4c4f_4144);
        for _ in 0..140 {
            let (x, y, size, a) = (rng.next() * 100.0, rng.next() * 100.0, 1.0 + (rng.next() * 2.0).floor(), 0.18 + rng.next() * 0.5);
            s.spawn((
                Node { position_type: PositionType::Absolute, left: Val::Percent(x), top: Val::Percent(y), width: Val::Px(size), height: Val::Px(size), ..default() },
                BackgroundColor(theme.text.with_alpha(a)),
                Pickable::IGNORE,
            ));
        }
    });
}

/// Dark at the left, where the text sits, and at the foot, where the bar is.
fn spawn_shades(p: &mut ChildSpawnerCommands, theme: &Theme) {
    let dark = theme.panel;
    p.spawn((
        fill_node(),
        gradient(bevy::ui::LinearGradient::TO_RIGHT, vec![stop(dark.with_alpha(0.86), 0.0), stop(dark.with_alpha(0.55), 40.0), stop(dark.with_alpha(0.0), 78.0)]),
        Pickable::IGNORE,
    ));
    p.spawn((fill_node(), gradient(bevy::ui::LinearGradient::TO_BOTTOM, vec![stop(dark.with_alpha(0.0), 62.0), stop(dark.with_alpha(0.88), 100.0)]), Pickable::IGNORE));
}

/// The kicker, the title with its rule, the briefing and the objectives.
fn spawn_info(p: &mut ChildSpawnerCommands, t: &Theme, info: &LoadInfo) {
    p.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(layout::MARGIN),
            top: Val::Px(layout::TOP),
            width: Val::Px(layout::TEXT_W),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(t.gap * 1.5),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        if !info.kicker.is_empty() {
            c.spawn(text(info.kicker.to_uppercase(), t.heading_size, t.accent));
        }
        if !info.title.is_empty() {
            c.spawn(text(info.title.clone(), t.display_size, t.text));
            c.spawn((Node { width: Val::Px(120.0), height: Val::Px(4.0), ..default() }, BackgroundColor(t.accent)));
        }
        for para in &info.briefing {
            c.spawn(text(para.clone(), t.text_size, t.text_dim));
        }
        if !info.objectives.is_empty() {
            c.spawn(text("OBJECTIVES", t.small_size, t.accent));
            for o in &info.objectives {
                c.spawn((Node { column_gap: Val::Px(t.gap), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE)).with_children(|row| {
                    row.spawn((Node { width: Val::Px(8.0), height: Val::Px(8.0), border_radius: BorderRadius::MAX, ..default() }, BackgroundColor(t.accent)));
                    row.spawn(text(o.clone(), t.text_size, t.text));
                });
            }
        }
    });
}

/// The foot: the ring, the words, the percentage, and the bar under them.
fn spawn_foot(p: &mut ChildSpawnerCommands, t: &Theme, progress: &LoadProgress) {
    p.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(layout::MARGIN),
            right: Val::Px(layout::MARGIN),
            bottom: Val::Px(layout::BOTTOM + layout::BAR_H + layout::STATUS_GAP),
            height: Val::Px(layout::STATUS_H),
            align_items: AlignItems::Center,
            column_gap: Val::Px(t.gap * 2.0),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|row| {
        row.spawn((Node { width: Val::Px(layout::RING), height: Val::Px(layout::RING), ..default() }, Pickable::IGNORE)).with_children(|ring| {
            let (c, r, d) = (layout::RING * 0.5, layout::RING * 0.5 - 4.5, 7.0);
            for i in 0..layout::DOTS {
                let a = i as f32 / layout::DOTS as f32 * TAU;
                ring.spawn((
                    RingDot(i),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(c + r * a.sin() - d * 0.5),
                        top: Val::Px(c - r * a.cos() - d * 0.5),
                        width: Val::Px(d),
                        height: Val::Px(d),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(t.track),
                    Pickable::IGNORE,
                ));
            }
        });
        row.spawn((Node { flex_grow: 1.0, ..default() }, Pickable::IGNORE)).with_children(|w| {
            w.spawn((StatusText, text(progress.doing(), t.heading_size, t.text)));
        });
        row.spawn((PercentText, text(format!("{}%", progress.percent()), t.title_size + 6.0, t.title)));
    });
    p.spawn((
        LoadingBar,
        ProgressBar(progress.fraction()),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(layout::MARGIN),
            right: Val::Px(layout::MARGIN),
            bottom: Val::Px(layout::BOTTOM),
            height: Val::Px(layout::BAR_H),
            border_radius: BorderRadius::MAX,
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(t.track),
        Pickable::IGNORE,
        children![(
            Fill,
            Node { width: Val::Percent(progress.fraction() * 100.0), height: Val::Percent(100.0), ..default() },
            BackgroundColor(t.fill),
            Pickable::IGNORE,
        )],
    ));
}

/// Put the screen up when a load is under way and none is shown.
#[allow(clippy::too_many_arguments)]
fn show_screen(
    mut commands: Commands,
    progress: Res<LoadProgress>,
    info: Res<LoadInfo>,
    theme: Res<Theme>,
    target: Res<LoadingTarget>,
    roots: Query<(), With<LoadingRoot>>,
    vfs: Option<Res<Mounted>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    device: Option<Res<RenderDevice>>,
    mut cache: Local<TextureCache>,
    mut screen_sky: ResMut<ScreenSky>,
) {
    if !progress.is_active() || !roots.is_empty() {
        return;
    }
    let t = &*theme;
    let mut cam = commands.spawn((
        LoadingCamera,
        Camera3d::default(),
        Camera { order: 20, clear_color: ClearColorConfig::Custom(t.panel), ..default() },
        Msaa::Off,
        RenderLayers::layer(LOADING_LAYER),
        Transform::IDENTITY,
    ));
    if let Some(image) = &target.0 {
        cam.insert(RenderTarget::from(image.clone()));
    }
    let cam = cam.id();

    // The map's sky, six faces about the camera, when the install has them.
    let mut faces = 0;
    if let (Some(vfs), Some(mut meshes), Some(mut materials), Some(mut images)) = (vfs, meshes, materials, images) {
        let prefix = info.background.trim_end_matches(".sod").trim_end_matches(".SOD").to_owned();
        let has = |n: &str| sta_assets::find_texture(&vfs.0, n).is_some();
        if !prefix.is_empty() && (has(&format!("{prefix}0")) || has(&format!("{prefix}.1"))) {
            let formats = device.map(|d| CompressedImageFormats::from_features(d.features())).unwrap_or(CompressedImageFormats::BC);
            let sky = commands.spawn((LoadingSky, Transform::from_scale(Vec3::splat(5.0)), Visibility::default())).id();
            let mut loader = Loader { vfs: &vfs.0, cache: &mut cache, images: &mut images, formats };
            faces = crate::render::space::sky_faces(&mut commands, &mut meshes, &mut materials, &mut loader, &prefix, sky);
            if faces == 0 {
                commands.entity(sky).despawn();
            }
        }
    }
    screen_sky.faces = faces;

    commands.spawn((LoadingRoot, fill_node(), UiTargetCamera(cam), GlobalZIndex(i32::MAX - 8), Pickable::IGNORE)).with_children(|p| {
        if faces == 0 {
            spawn_backdrop(p, t);
        }
        spawn_shades(p, t);
        spawn_info(p, t, &info);
        spawn_foot(p, t, &progress);
    });
}

/// Put the sky's faces on the screen's own render layer, so no other camera draws
/// them (the layer is set here, not propagated: the map plugin owns the
/// propagation plugin).
fn layer_sky(mut commands: Commands, sky: Query<&Children, With<LoadingSky>>, layered: Query<(), With<RenderLayers>>) {
    for kids in &sky {
        for k in kids.iter() {
            if !layered.contains(k) {
                commands.entity(k).insert(RenderLayers::layer(LOADING_LAYER));
            }
        }
    }
}

/// The mission's words changed under a shown screen: build it again.
fn rebuild_on_info_change(commands: Commands, info: Res<LoadInfo>, parts: Query<Entity, ScreenParts>) {
    if info.is_changed() && !info.is_added() {
        despawn_all(commands, &parts);
    }
}

fn despawn_all(mut commands: Commands, parts: &Query<Entity, ScreenParts>) {
    for e in parts {
        commands.entity(e).despawn();
    }
}

/// Take the screen down once the load is over.
fn hide_screen(commands: Commands, progress: Res<LoadProgress>, parts: Query<Entity, ScreenParts>) {
    if !progress.is_active() {
        despawn_all(commands, &parts);
    }
}

/// The screen's animation time in seconds: the bench clock's frames in a bench
/// run (so frames repeat), real time otherwise.
fn anim_time(clock: Option<&AnimClock>, time: &Time) -> f32 {
    clock.map_or_else(|| time.elapsed_secs(), |c| c.wall() as f32)
}

/// Words, percentage, bar and the ring's dots from the progress.
#[allow(clippy::too_many_arguments)]
fn update_screen(
    progress: Res<LoadProgress>,
    theme: Res<Theme>,
    config: Res<LoadingConfig>,
    screen_sky: Res<ScreenSky>,
    clock: Option<Res<AnimClock>>,
    time: Res<Time>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut status: Query<&mut Text, (With<StatusText>, Without<PercentText>)>,
    mut percent: Query<&mut Text, (With<PercentText>, Without<StatusText>)>,
    mut bar: Query<&mut ProgressBar, With<LoadingBar>>,
    mut dots: Query<(&RingDot, &mut BackgroundColor)>,
    mut logged: Local<String>,
) {
    if !progress.is_active() {
        return;
    }
    let words = progress.doing();
    for mut t in &mut status {
        if t.0 != words {
            t.0.clone_from(&words);
        }
    }
    let pct = format!("{}%", progress.percent());
    for mut t in &mut percent {
        if t.0 != pct {
            t.0.clone_from(&pct);
        }
    }
    for mut b in &mut bar {
        if (b.0 - progress.fraction()).abs() > 1e-4 {
            b.0 = progress.fraction();
        }
    }
    // A tail of dots chasing round the ring, one turn in 1.5 seconds.
    let head = anim_time(clock.as_deref(), &time) / 1.5 * layout::DOTS as f32;
    for (d, mut c) in &mut dots {
        let behind = (head - d.0 as f32).rem_euclid(layout::DOTS as f32);
        let k = (1.0 - behind / (layout::DOTS as f32 * 0.7)).clamp(0.0, 1.0);
        c.0 = mix(theme.track, theme.accent, k * k);
    }
    if config.log {
        let (w, h) = window.map_or((1920.0, layout::SCREEN_H), |w| (w.width() / w.height() * layout::SCREEN_H, layout::SCREEN_H));
        let stage = progress.current().map_or("none", LoadStage::name);
        let sky = if screen_sky.faces > 0 { format!("faces:{}", screen_sky.faces) } else { "stars".into() };
        let line = format!(
            "loading layout: screen={w:.0}x{h:.0} pct={} stage={stage} sky={sky} bar={:.0},{:.0},{:.0},{:.0} ring={:.0},{:.0},{:.0},{:.0}",
            progress.percent(),
            layout::MARGIN,
            h - layout::BOTTOM - layout::BAR_H,
            w - 2.0 * layout::MARGIN,
            layout::BAR_H,
            layout::MARGIN,
            h - layout::BOTTOM - layout::BAR_H - layout::STATUS_GAP - layout::STATUS_H * 0.5 - layout::RING * 0.5,
            layout::RING,
            layout::RING,
        );
        if *logged != line {
            println!("{line}");
            *logged = line;
        }
    }
}

/// The sky turns slowly about the camera: a full turn in about three minutes.
fn turn_sky(clock: Option<Res<AnimClock>>, time: Res<Time>, mut camera: Query<&mut Transform, With<LoadingCamera>>) {
    let t = anim_time(clock.as_deref(), &time);
    for mut tr in &mut camera {
        tr.rotation = Quat::from_euler(EulerRot::YXZ, t * TAU / 180.0, 0.12, 0.0);
    }
}

// ---------------------------------------------------------------------------
// The frame run
// ---------------------------------------------------------------------------

/// `STA64_UI_LOADING=1`: draw the loading screen over the bench's frame.
pub fn frame_requested() -> bool {
    std::env::var("STA64_UI_LOADING").as_deref() == Ok("1")
}

/// `STA64_UI_LOADING_AT=<stage>:<done>/<total>` (default `assets:3/6`): how far
/// the scripted load has got; the stages before it are finished.
pub fn frame_script() -> Vec<LoadReport> {
    let at = std::env::var("STA64_UI_LOADING_AT").unwrap_or_else(|_| "assets:3/6".into());
    parse_script(&at).unwrap_or_else(|| parse_script("assets:3/6").expect("the default parses"))
}

fn parse_script(at: &str) -> Option<Vec<LoadReport>> {
    let (name, counts) = at.split_once(':')?;
    let (done, total) = counts.split_once('/')?;
    let stage = LoadStage::ALL.into_iter().find(|s| s.name() == name.trim())?;
    let (done, total) = (done.trim().parse().ok()?, total.trim().parse().ok()?);
    let mut out: Vec<LoadReport> = LoadStage::ALL.into_iter().filter(|s| *s < stage).map(LoadReport::finished).collect();
    out.push(LoadReport::step(stage, done, total, "fgalaxy.sod"));
    Some(out)
}

/// Add the loading screen to a frame run, over `target`: the UI scaled to the
/// 1080-unit screen the theme is written for, the title and sky of the fake
/// `two_bases()` map, and a scripted load ([`frame_script`]). The sky's faces
/// come from the directory `STA64_UI_LOADING_SKY` names (a data directory with
/// `textures/<sky>0..5.png`), else the gradient and stars stand in.
pub fn add_frame_overlay(app: &mut App, target: Handle<Image>, size: (u32, u32)) {
    let map = sta_content::fake::two_bases();
    let mut info = LoadInfo::from_map(&map);
    info.kicker = "Instant Action".into();
    info.briefing = vec!["Two starbases face each other across a mineable moon.".into(), "A nebula lies on one flank and an asteroid field on the other.".into()];
    info.objectives = vec!["Destroy the enemy base".into(), "Protect your own".into()];
    if let Some(dir) = std::env::var_os("STA64_UI_LOADING_SKY") {
        let mut vfs = sta_vfs::Vfs::default();
        match vfs.push_dir("loading-sky".into(), dir.into()) {
            Ok(()) => {
                app.insert_resource(Mounted(vfs));
            }
            Err(e) => eprintln!("loading sky: {e}"),
        }
    }
    let script = frame_script();
    app.insert_resource(UiScale(size.1 as f32 / layout::SCREEN_H))
        .insert_resource(LoadingConfig { fit_scale: false, log: true })
        .insert_resource(LoadingTarget(Some(target)))
        .insert_resource(info)
        .add_plugins(LoadingPlugin)
        .add_systems(
            Update,
            (move |mut reports: MessageWriter<LoadReport>, mut sent: Local<bool>| {
                if !*sent {
                    *sent = true;
                    for r in &script {
                        reports.write(r.clone());
                    }
                }
            })
            .before(GameSet::Ui),
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;

    fn marks(p: &LoadProgress) -> Vec<String> {
        p.marks_text().lines().map(str::to_owned).collect()
    }

    #[test]
    fn weights_add_up_to_the_whole_bar() {
        assert_eq!(LoadStage::ALL.iter().map(|s| s.weight()).sum::<u32>(), 100);
    }

    #[test]
    fn stages_move_the_bar_by_their_weights() {
        let mut p = LoadProgress::default();
        p.report(&LoadReport::finished(LoadStage::Mount));
        assert_eq!(p.percent(), 5);
        p.report(&LoadReport::counts(LoadStage::Content, 1, 2));
        assert_eq!(p.percent(), 5 + 8); // half of 15 is 7.5, rounded up
        p.report(&LoadReport::finished(LoadStage::Content));
        p.report(&LoadReport::finished(LoadStage::MapConvert));
        p.report(&LoadReport::counts(LoadStage::Assets, 25, 100));
        assert_eq!(p.percent(), 5 + 15 + 15 + 13); // a quarter of 50 is 12.5
        assert_eq!(p.current(), Some(LoadStage::Assets));
        assert!(!p.is_complete());
    }

    #[test]
    fn progress_never_goes_back() {
        let mut p = LoadProgress::default();
        p.report(&LoadReport::counts(LoadStage::Assets, 60, 100));
        let before = p.percent();
        p.report(&LoadReport::counts(LoadStage::Assets, 10, 100));
        assert_eq!(p.percent(), before);
        assert_eq!(p.stage_fraction(LoadStage::Assets), 0.6);
    }

    #[test]
    fn a_stage_with_no_steps_is_done() {
        let mut p = LoadProgress::default();
        p.report(&LoadReport::counts(LoadStage::Mount, 0, 0));
        assert!(p.stage_fraction(LoadStage::Mount) >= 1.0);
    }

    #[test]
    fn marks_follow_the_stages_in_the_bench_format() {
        let mut p = LoadProgress::default();
        p.begin();
        p.tick();
        p.report(&LoadReport::step(LoadStage::Mount, 0, 2, "Data"));
        p.tick();
        p.report(&LoadReport::finished(LoadStage::Mount));
        p.tick();
        p.report(&LoadReport::counts(LoadStage::Content, 1, 1));
        assert_eq!(
            marks(&p),
            [
                "     0 load.begin",
                "     1 load.stage stage=mount pct=0",
                "     2 load.stage.end stage=mount pct=5",
                "     3 load.stage stage=content pct=5",
                "     3 load.stage.end stage=content pct=20",
                "     3 load.progress pct=20",
            ]
        );
    }

    #[test]
    fn a_whole_load_ends_in_load_done_once() {
        let mut p = LoadProgress::default();
        for s in LoadStage::ALL {
            p.report(&LoadReport::finished(s));
        }
        p.report(&LoadReport::finished(LoadStage::SimSetup));
        assert!(p.is_complete());
        assert_eq!(p.percent(), 100);
        let m = marks(&p);
        assert_eq!(m.iter().filter(|l| l.contains("load.done")).count(), 1, "{m:?}");
        assert_eq!(m.iter().filter(|l| l.contains("load.stage.end")).count(), 5);
        assert!(m.last().unwrap().ends_with("load.done"));
        assert!(m.iter().all(|l| !l.contains("load.progress pct=100")));
    }

    #[test]
    fn the_screen_goes_when_the_hold_is_over_and_a_new_load_starts_afresh() {
        let mut p = LoadProgress::default();
        for s in LoadStage::ALL {
            p.report(&LoadReport::finished(s));
        }
        assert!(p.is_active());
        for _ in 0..HOLD_FRAMES {
            p.tick();
        }
        assert!(!p.is_active());
        p.report(&LoadReport::counts(LoadStage::Mount, 1, 4));
        assert!(p.is_active() && !p.is_complete());
        assert_eq!(p.percent(), 1);
        assert_eq!(marks(&p).first().map(String::as_str), Some("     0 load.begin"));
    }

    #[test]
    fn what_it_is_doing_names_the_stage_and_the_step() {
        let mut p = LoadProgress::default();
        p.report(&LoadReport::finished(LoadStage::Mount));
        assert_eq!(p.doing(), "Reading the game's data");
        p.report(&LoadReport::step(LoadStage::Assets, 1, 9, "fgalaxy.sod"));
        assert!(p.doing().starts_with("Loading models and textures"));
        assert!(p.doing().ends_with("fgalaxy.sod"));
    }

    #[test]
    fn the_script_finishes_the_stages_before_it() {
        let s = parse_script("assets:3/6").unwrap();
        assert_eq!(s.iter().map(|r| r.stage).collect::<Vec<_>>(), [LoadStage::Mount, LoadStage::Content, LoadStage::MapConvert, LoadStage::Assets]);
        let mut p = LoadProgress::default();
        s.iter().for_each(|r| p.report(r));
        assert_eq!(p.percent(), 5 + 15 + 15 + 25);
        assert!(parse_script("nonsense").is_none());
    }

    #[test]
    fn info_comes_from_the_map_def() {
        let i = LoadInfo::from_map(&sta_content::fake::two_bases());
        assert_eq!((i.title.as_str(), i.background.as_str()), ("Two Bases", "fx_sky"));
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        configure_sets(&mut app);
        app.insert_resource(LoadInfo::from_map(&sta_content::fake::two_bases())).add_plugins(LoadingPlugin);
        app
    }

    fn count<C: Component>(app: &mut App) -> usize {
        app.world_mut().query_filtered::<(), With<C>>().iter(app.world()).count()
    }

    #[test]
    fn messages_drive_the_screen_up_and_down() {
        let mut app = app();
        app.update();
        assert_eq!(count::<LoadingRoot>(&mut app), 0, "no load, no screen");
        app.world_mut().write_message(LoadReport::counts(LoadStage::Assets, 1, 2));
        app.update();
        assert_eq!(count::<LoadingRoot>(&mut app), 1);
        assert_eq!(count::<LoadingCamera>(&mut app), 1);
        assert_eq!(count::<RingDot>(&mut app), layout::DOTS);
        assert_eq!(app.world().resource::<ScreenSky>().faces, 0, "no install: the stand-in sky");
        assert_eq!(app.world().resource::<LoadProgress>().percent(), 25);
        let words = app.world_mut().query_filtered::<&Text, With<StatusText>>().single(app.world()).unwrap().0.clone();
        assert!(words.starts_with("Loading models and textures"), "{words}");
        let pct = app.world_mut().query_filtered::<&Text, With<PercentText>>().single(app.world()).unwrap().0.clone();
        assert_eq!(pct, "25%");
        let bar = app.world_mut().query_filtered::<&ProgressBar, With<LoadingBar>>().single(app.world()).unwrap().0;
        assert!((bar - 0.25).abs() < 1e-4);

        for s in LoadStage::ALL {
            app.world_mut().write_message(LoadReport::finished(s));
        }
        app.update();
        assert!(app.world().resource::<LoadProgress>().is_complete());
        assert_eq!(count::<LoadingRoot>(&mut app), 1, "held a moment on 100%");
        for _ in 0..=HOLD_FRAMES {
            app.update();
        }
        assert_eq!(count::<LoadingRoot>(&mut app), 0);
        assert_eq!(count::<LoadingCamera>(&mut app), 0);
    }

    #[test]
    fn a_new_title_rebuilds_the_screen() {
        let mut app = app();
        app.world_mut().write_message(LoadReport::counts(LoadStage::Mount, 0, 1));
        app.update();
        app.world_mut().resource_mut::<LoadInfo>().title = "Another".into();
        app.update();
        app.update();
        assert_eq!(count::<LoadingRoot>(&mut app), 1);
        let texts: Vec<String> = app.world_mut().query::<&Text>().iter(app.world()).map(|t| t.0.clone()).collect();
        assert!(texts.iter().any(|t| t == "Another"), "{texts:?}");
    }
}
