//! The campaign screens (UI-25): campaign select (Single Player's cards), the mission
//! list, the briefing and the debrief, over [`sta_mission::campaign`]'s book (LUA-08).
//!
//! ```text
//!   Single player --card--> Missions --Briefing--> Briefing --Start--> StartMission
//!        ^                      ^  ^                   |                     |
//!        +----------------------+  +--Mission list-----+       (game::mission_host)
//!                                  |                                       |
//!                              Debrief <-------- the mission's end ---------+
//! ```
//!
//! - **Select.** The card of a locked campaign is off and shows the install's locked text
//!   (`*_incomplete`); the others open that campaign's mission list.
//! - **Mission list.** Every mission of the campaign, each marked won, locked (not reached
//!   yet, or its campaign is locked) or blocked ([`CampaignBook::blocked`]: the mission cannot
//!   run to a win in STA64 yet; the picked mission's reason is shown and it cannot be started).
//! - **Briefing.** The text of the book (the objectives file the mission's script names: title,
//!   summary, goals). Start waits while a bridge scene is playing
//!   ([`MissionMovieLog::busy`](crate::media::campaign::MissionMovieLog::busy)).
//! - **Debrief.** After the mission ends (`game::mission_host`): won (next mission, or the
//!   campaign is complete and what it opened), lost or abandoned (retry, mission list). The
//!   result is recorded before the screen shows ([`Campaigns::finish`]) and saved.
//! - **Progress** is [`CampaignProgress::save`]'s text in the platform config directory
//!   ([`platform::config_dir`]), one file per campaign book (`campaign-<book digest>.txt`),
//!   written atomically after every recorded result and read when the front end starts.
//!
//! Regions: [`layout`] (each marked measured or arranged). Nothing here changes
//! [`AppState`]: [`StartMission`] is the flow's (`game::mission_host`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::math::{Rect, Vec2};
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use sta_mission::campaign::{Blocked, Campaign, CampaignBook, CampaignMission, CampaignProgress, CampaignRunner, MissionHost, MissionScript, Transition};

use crate::app::AppState;
use crate::frontend::screens::stock::{StockBox, StockBoxNode};
use crate::frontend::screens::{Screen, ScreenRoot};
use crate::media::campaign::MissionMovieLog;
use crate::platform;
use crate::ui::i18n::{tr, trf};
use crate::ui::sprite_font::{SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::{ButtonClicked, ListSelected, Theme, disabled_button, font_role, kit_button, kit_text, list};

pub mod layout;
#[cfg(test)]
pub(crate) mod tests;

use layout::{CSlot, region};

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// How a mission ended, as the debrief and the progress see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    Won,
    Lost,
    /// The player left the mission before its script ended it: a loss, progress unchanged.
    Abandoned,
}

/// Whether a mission of a campaign can be started now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionState {
    /// Its campaign is locked or the mission is not reached yet.
    Locked,
    /// Reached, but it cannot run to a win yet (the reason).
    Blocked(String),
    Ready,
}

/// What the debrief shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Debrief {
    pub campaign: String,
    pub mission: usize,
    pub result: RunResult,
    /// What follows: the next mission, the campaign's end or a retry.
    pub transition: Transition,
    /// Why the progress file could not be written, if it could not.
    pub save_error: Option<String>,
}

/// Where a campaign book's progress is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgressStore {
    path: Option<PathBuf>,
}

impl ProgressStore {
    /// Progress is neither read nor written.
    pub fn none() -> ProgressStore {
        ProgressStore::default()
    }

    /// `<dir>/campaign-<book digest>.txt`.
    pub fn in_dir(dir: &Path, book: &CampaignBook) -> ProgressStore {
        ProgressStore { path: Some(dir.join(format!("campaign-{:016x}.txt", book.digest()))) }
    }

    /// In the platform's config directory (`platform::config_dir`); none without one.
    pub fn platform(book: &CampaignBook) -> ProgressStore {
        platform::config_dir().map_or_else(ProgressStore::none, |d| ProgressStore::in_dir(&d, book))
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The saved progress, or a new player's; the second value says why a file was not used.
    pub fn load(&self, book: &CampaignBook) -> (CampaignProgress, Option<String>) {
        let Some(path) = &self.path else { return (CampaignProgress::new(book), None) };
        match std::fs::read(path) {
            Ok(bytes) => match CampaignProgress::restore_for(book, &String::from_utf8_lossy(&bytes)) {
                Ok(mut p) => {
                    p.fit(book);
                    p.apply_give_all(book);
                    (p, None)
                }
                Err(e) => (CampaignProgress::new(book), Some(format!("{}: {e}", path.display()))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (CampaignProgress::new(book), None),
            Err(e) => (CampaignProgress::new(book), Some(format!("{}: {e}", path.display()))),
        }
    }

    /// Write `progress`; a store with no path writes nothing.
    pub fn save(&self, progress: &CampaignProgress) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        crate::ui::settings::write_atomic(path, &progress.save()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The installed campaigns, how far the player is, and what the screens show.
#[derive(Resource)]
pub struct Campaigns {
    pub book: CampaignBook,
    pub progress: CampaignProgress,
    pub store: ProgressStore,
    /// Resolves each mission's script (`MissionHost::script`) when one starts.
    pub host: Arc<dyn MissionHost + Send + Sync>,
    blocked: Vec<Blocked>,
    /// The campaign whose missions are listed.
    pub campaign: Option<String>,
    /// The picked mission of that list.
    pub mission: usize,
    /// The last mission's result, for the debrief.
    pub debrief: Option<Debrief>,
    /// Why saved progress was not used (shown once in the log).
    pub load_warning: Option<String>,
    /// Missions whose script is a converted Armada II DLL mission (lower-case `.bzn`
    /// names): experimental, marked as such on the screens.
    pub experimental: std::collections::BTreeSet<String>,
    /// What to tell the player when missions wait for a conversion that has not run
    /// (the command to run); `None` when nothing is waiting or the conversion is mounted.
    pub convert_hint: Option<String>,
}

impl Campaigns {
    /// A player's campaigns: progress read from `store` when it has a file for this book.
    pub fn new(book: CampaignBook, host: Arc<dyn MissionHost + Send + Sync>, store: ProgressStore) -> Campaigns {
        let (progress, load_warning) = store.load(&book);
        let blocked = book.blocked();
        Campaigns { book, progress, store, host, blocked, campaign: None, mission: 0, debrief: None, load_warning, experimental: Default::default(), convert_hint: None }
    }

    /// Mark the missions whose script file exists under `converted` (a mounted conversion,
    /// [`sta_mission::converted`]) as experimental, except those whose path is in `own` (lower
    /// case; the install's own scripts, which shadow the conversion).
    pub fn mark_converted(&mut self, converted: &Path, own: &std::collections::BTreeSet<String>) {
        // File names compared in lower case: the mount is case-insensitive, the disk may not be.
        let held: std::collections::BTreeSet<String> = std::fs::read_dir(converted.join("missions"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| format!("missions/{}", e.file_name().to_string_lossy().to_ascii_lowercase()))
            .collect();
        for c in &self.book.campaigns {
            for m in &c.missions {
                if let MissionScript::Lua { path } = &m.script
                    && held.contains(&path.to_ascii_lowercase())
                    && !own.contains(&path.to_ascii_lowercase())
                {
                    self.experimental.insert(m.bzn.to_ascii_lowercase());
                }
            }
        }
    }

    /// Whether the mission runs a converted (experimental) script.
    pub fn is_experimental(&self, m: &CampaignMission) -> bool {
        self.experimental.contains(&m.bzn.to_ascii_lowercase())
    }

    /// Whether some mission is blocked only for want of a Lua conversion.
    pub fn awaits_conversion(&self) -> bool {
        self.blocked.iter().any(|b| b.reason.starts_with("legacy DLL only"))
    }

    /// The campaign shown on card `i` of Single Player: the book's `i`-th (the shell's slot order).
    pub fn card(&self, i: usize) -> Option<&Campaign> {
        self.book.campaigns.get(i)
    }

    /// Whether the campaign is open (it has missions and what it requires is complete).
    pub fn is_open(&self, campaign: &str) -> bool {
        self.progress.is_open(&self.book, campaign)
    }

    /// The text of a campaign's card: its name, or what the shell says while it is locked.
    pub fn card_text(&self, c: &Campaign) -> String {
        if self.is_open(&c.id) || c.missions.is_empty() {
            campaign_name(c)
        } else {
            c.locked_text.clone().unwrap_or_else(|| campaign_name(c))
        }
    }

    /// Whether `mission` of `campaign` can be started.
    pub fn state(&self, campaign: &str, mission: usize) -> MissionState {
        if !self.progress.available(&self.book, campaign).contains(&mission) {
            return MissionState::Locked;
        }
        match self.blocked.iter().find(|b| b.campaign == campaign && b.index == mission) {
            Some(b) => MissionState::Blocked(b.reason.clone()),
            None => MissionState::Ready,
        }
    }

    /// Whether `mission` has been won (it lies behind the furthest reached, or its campaign is complete).
    pub fn is_won(&self, campaign: &str, mission: usize) -> bool {
        let st = self.progress.state(campaign);
        st.complete || mission < st.reached
    }

    pub fn mission(&self, campaign: &str, mission: usize) -> Option<(&Campaign, &CampaignMission)> {
        let c = self.book.campaign(campaign)?;
        Some((c, c.missions.get(mission)?))
    }

    /// The game seed of a mission: fixed, so a mission starts the same way every time.
    pub fn seed(&self, campaign: &str, mission: usize) -> u64 {
        let slot = self.book.campaign(campaign).map_or(0, |c| c.slot as u64);
        slot * 1000 + mission as u64 + 1
    }

    /// Record a finished mission: the result goes into the progress, the progress is saved
    /// and the debrief is set. `None` when the book has no such mission.
    pub fn finish(&mut self, campaign: &str, mission: usize, result: RunResult) -> Option<&Debrief> {
        // The runner is only used for its rule (`record`: the stock shell's), never to run.
        let mut runner = CampaignRunner::with_progress(self.book.clone(), self.progress.clone());
        let transition = runner.record(campaign, mission, result == RunResult::Won).ok()?;
        self.progress = runner.progress;
        if self.load_warning.take().is_some()
            && let Some(p) = self.store.path()
        {
            // A file that could not be used is kept aside, not overwritten.
            let _ = std::fs::rename(p, p.with_extension("bad"));
        }
        let save_error = self.store.save(&self.progress).err();
        if let Some(e) = &save_error {
            warn!("campaign: progress not saved: {e}");
        }
        self.campaign = Some(campaign.to_owned());
        self.mission = match &transition {
            Transition::Next { mission, .. } => *mission,
            _ => mission,
        };
        self.debrief = Some(Debrief { campaign: campaign.to_owned(), mission, result, transition, save_error });
        self.debrief.as_ref()
    }
}

/// A campaign's display name: the install's label, else its id.
pub fn campaign_name(c: &Campaign) -> String {
    c.name.clone().unwrap_or_else(|| {
        let mut s = c.id.clone();
        if let Some(f) = s.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        s
    })
}

/// A mission's display name: the install's title, else its map.
pub fn mission_name(m: &CampaignMission) -> String {
    m.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| m.stem().to_owned())
}

/// Start this mission: written by the briefing's Start, read by `game::mission_host`.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct StartMission {
    pub campaign: String,
    pub mission: usize,
}

/// What a campaign control does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignAct {
    /// Single Player's card `i`: open that campaign's mission list.
    Card(usize),
    /// The mission list's Briefing.
    ToBriefing,
    /// Back to the mission list.
    ToMissions,
    /// Back to Single Player.
    ToSinglePlayer,
    /// The briefing's Start.
    Start,
    /// The debrief's Next mission (or Retry): pick that mission and open its briefing.
    Pick(usize),
}

/// The mission list's rows.
#[derive(Component, Debug, Clone, Copy)]
pub struct MissionList;

/// A control of a campaign screen, by its region.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CPlaced(pub CSlot);

/// Whether a campaign screen draws `screen`.
pub fn covers(screen: Screen) -> bool {
    matches!(screen, Screen::Missions | Screen::Briefing | Screen::Debrief)
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Clicks on campaign controls and on the mission list.
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_buttons(
    mut clicks: MessageReader<ButtonClicked>,
    mut picks: MessageReader<ListSelected>,
    acts: Query<&CampaignAct>,
    lists: Query<(), With<MissionList>>,
    campaigns: Option<ResMut<Campaigns>>,
    log: Option<Res<MissionMovieLog>>,
    mut next: ResMut<NextState<Screen>>,
    mut start: MessageWriter<StartMission>,
) {
    let Some(mut campaigns) = campaigns else {
        clicks.clear();
        picks.clear();
        return;
    };
    for p in picks.read() {
        if lists.contains(p.list) && campaigns.mission != p.index {
            campaigns.mission = p.index;
        }
    }
    for ButtonClicked(e) in clicks.read() {
        let Ok(&act) = acts.get(*e) else { continue };
        match act {
            CampaignAct::Card(i) => {
                let Some(c) = campaigns.card(i) else { continue };
                if !campaigns.is_open(&c.id) {
                    continue;
                }
                let (id, furthest) = (c.id.clone(), campaigns.progress.state(&c.id).reached.min(c.missions.len().saturating_sub(1)));
                campaigns.campaign = Some(id);
                campaigns.mission = furthest;
                next.set(Screen::Missions);
            }
            CampaignAct::ToBriefing => {
                if let Some(id) = campaigns.campaign.clone()
                    && campaigns.state(&id, campaigns.mission) != MissionState::Locked
                {
                    next.set(Screen::Briefing);
                }
            }
            CampaignAct::ToMissions => {
                next.set(Screen::Missions);
            }
            CampaignAct::ToSinglePlayer => {
                next.set(Screen::SinglePlayer);
            }
            CampaignAct::Pick(m) => {
                campaigns.mission = m;
                next.set(Screen::Briefing);
            }
            CampaignAct::Start => {
                let Some(id) = campaigns.campaign.clone() else { continue };
                if campaigns.state(&id, campaigns.mission) == MissionState::Ready && !log.as_ref().is_some_and(|l| l.busy()) {
                    start.write(StartMission { campaign: id, mission: campaigns.mission });
                }
            }
        }
    }
}

/// Draw the campaign screen when it is shown, or what it shows changed. The screens'
/// own `rebuild` has taken the old roots down when the screen changed (and draws nothing
/// for these screens); a root already there is replaced when this screen's data changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<ScreenRoot>>,
    screen: Res<State<Screen>>,
    theme: Res<Theme>,
    campaigns: Option<Res<Campaigns>>,
    log: Option<Res<MissionMovieLog>>,
    stock_box: Res<StockBox>,
    camera: Option<Res<crate::frontend::screens::ScreensCamera>>,
    mut busy: Local<bool>,
) {
    let (screen, Some(campaigns)) = (*screen.get(), campaigns) else { return };
    if !covers(screen) {
        return;
    }
    let wait = log.as_ref().is_some_and(|l| l.busy());
    let changed = campaigns.is_changed() || theme.is_changed() || stock_box.is_changed() || std::mem::replace(&mut *busy, wait) != wait;
    if !roots.is_empty() && !changed {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    spawn(&mut commands, screen, &Ctx { theme: &theme, campaigns: &campaigns, wait, b: stock_box.0 }, camera.map(|c| c.0));
}

// ---------------------------------------------------------------------------
// The screens
// ---------------------------------------------------------------------------

/// What a campaign screen is drawn from.
pub struct Ctx<'a> {
    pub theme: &'a Theme,
    pub campaigns: &'a Campaigns,
    /// A bridge scene is playing: Start and Next wait.
    pub wait: bool,
    /// The box the regions are fractions of, logical px.
    pub b: Vec2,
}

type Box_<'a> = RelatedSpawnerCommands<'a, ChildOf>;

fn spawn(commands: &mut Commands, screen: Screen, ctx: &Ctx, camera: Option<Entity>) {
    let mut root = commands.spawn((
        ScreenRoot(screen),
        DespawnOnExit(AppState::FrontEnd),
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
    ));
    if let Some(c) = camera {
        root.insert(UiTargetCamera(c));
    }
    let b = ctx.b;
    root.with_children(|p| {
        p.spawn((StockBoxNode, Node { width: Val::Px(b.x), height: Val::Px(b.y), ..default() })).with_children(|p| match screen {
            Screen::Missions => missions(p, ctx),
            Screen::Briefing => briefing(p, ctx),
            Screen::Debrief => debrief(p, ctx),
            _ => {}
        });
    });
}

fn put(p: &mut Box_, slot: CSlot, r: Rect, content: impl Bundle) {
    p.spawn((
        CPlaced(slot),
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), align_items: AlignItems::Center, ..default() },
        children![content],
    ));
}

/// A block region: a column for paragraphs.
fn block(p: &mut Box_, slot: CSlot, r: Rect, fill: impl FnOnce(&mut Box_)) {
    p.spawn((
        CPlaced(slot),
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), flex_direction: FlexDirection::Column, overflow: Overflow::clip(), ..default() },
    ))
    .with_children(fill);
}

fn para(t: &Theme, text: impl Into<String>, size: f32, colour: Color) -> impl Bundle {
    (Node { margin: UiRect::bottom(Val::Px(t.gap)), ..default() }, children![sprite_text_styled(text, size, colour, SpriteTextStyle::role(font_role(t, size)).wrap_to_parent())])
}

fn act_button(p: &mut Box_, ctx: &Ctx, slot: CSlot, text: &str, act: CampaignAct, on: bool) {
    let r = region(slot).px(ctx.b);
    if on {
        put(p, slot, r, (kit_button(ctx.theme, text, r.width()), act));
    } else {
        put(p, slot, r, disabled_button(ctx.theme, text, r.width()));
    }
}

fn title(p: &mut Box_, ctx: &Ctx, slot: CSlot, text: &str) {
    let t = ctx.theme;
    put(p, slot, region(slot).px(ctx.b), kit_text(t, t.caps(text, t.caps_titles), t.title_size, t.title));
}

fn current<'a>(ctx: &'a Ctx) -> Option<(&'a Campaign, &'a CampaignMission)> {
    let id = ctx.campaigns.campaign.as_deref()?;
    ctx.campaigns.mission(id, ctx.campaigns.mission)
}

fn missions(p: &mut Box_, ctx: &Ctx) {
    let t = ctx.theme;
    let cs = ctx.campaigns;
    let Some(c) = cs.campaign.as_deref().and_then(|id| cs.book.campaign(id)) else {
        title(p, ctx, CSlot::ListTitle, &tr("Missions"));
        act_button(p, ctx, CSlot::ListBack, &tr("Previous menu"), CampaignAct::ToSinglePlayer, true);
        return;
    };
    title(p, ctx, CSlot::ListTitle, &campaign_name(c));
    let rows: Vec<String> = c
        .missions
        .iter()
        .map(|m| {
            let mark = match cs.state(&c.id, m.index) {
                MissionState::Locked => tr(" (locked)"),
                MissionState::Blocked(_) => tr(" (cannot run yet)"),
                MissionState::Ready if cs.is_won(&c.id, m.index) && cs.is_experimental(m) => tr(" (won, experimental)"),
                MissionState::Ready if cs.is_won(&c.id, m.index) => tr(" (won)"),
                MissionState::Ready if cs.is_experimental(m) => tr(" (experimental)"),
                MissionState::Ready => String::new(),
            };
            format!("{}. {}{mark}", m.index + 1, mission_name(m))
        })
        .collect();
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let pick = cs.mission.min(c.missions.len().saturating_sub(1));
    let r = region(CSlot::ListRows).px(ctx.b);
    p.spawn((
        CPlaced(CSlot::ListRows),
        Node { position_type: PositionType::Absolute, left: Val::Px(r.min.x), top: Val::Px(r.min.y), width: Val::Px(r.width()), height: Val::Px(r.height()), flex_direction: FlexDirection::Column, overflow: Overflow::scroll_y(), ..default() },
        children![(list(t, &refs, (!refs.is_empty()).then_some(pick)), MissionList)],
    ));
    let state = cs.state(&c.id, pick);
    let picked = c.missions.get(pick);
    block(p, CSlot::ListDetails, region(CSlot::ListDetails).px(ctx.b), |d| {
        let Some(m) = picked else { return };
        d.spawn(para(t, mission_name(m), t.heading_size, t.title));
        match &state {
            MissionState::Locked => {
                let why = if cs.is_open(&c.id) { tr("Win the missions before it to open it.") } else { c.locked_text.clone().unwrap_or_else(|| tr("This campaign is locked.")) };
                d.spawn(para(t, why, t.text_size, t.warn));
            }
            MissionState::Blocked(reason) => {
                d.spawn(para(t, tr("This mission cannot be started yet."), t.text_size, t.bad));
                d.spawn(para(t, reason.clone(), t.small_size, t.text_dim));
                if let (Some(hint), true) = (&cs.convert_hint, reason.starts_with("legacy DLL only")) {
                    d.spawn(para(t, hint.clone(), t.small_size, t.warn));
                }
            }
            MissionState::Ready => {
                if cs.is_experimental(m) {
                    d.spawn(para(t, tr("Experimental: converted from the original mission file. It may stop early or misbehave."), t.small_size, t.warn));
                }
                if let Some(b) = &m.briefing {
                    d.spawn(para(t, first_paragraph(&b.text), t.text_size, t.text));
                }
            }
        }
    });
    act_button(p, ctx, CSlot::ListBack, &tr("Previous menu"), CampaignAct::ToSinglePlayer, true);
    act_button(p, ctx, CSlot::ListPlay, &tr("Briefing"), CampaignAct::ToBriefing, state != MissionState::Locked);
}

fn first_paragraph(text: &str) -> String {
    let t = text.trim();
    t.split("\n\n").next().unwrap_or(t).trim().to_owned()
}

fn briefing(p: &mut Box_, ctx: &Ctx) {
    let t = ctx.theme;
    let Some((c, m)) = current(ctx) else {
        title(p, ctx, CSlot::BriefTitle, &tr("Briefing"));
        act_button(p, ctx, CSlot::BriefBack, &tr("Mission list"), CampaignAct::ToMissions, true);
        return;
    };
    let name = m.briefing.as_ref().map(|b| b.title.clone()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| mission_name(m));
    title(p, ctx, CSlot::BriefTitle, &name);
    block(p, CSlot::BriefText, region(CSlot::BriefText).px(ctx.b), |d| match &m.briefing {
        Some(b) if !b.text.trim().is_empty() => {
            for chunk in b.text.trim().split("\n\n") {
                d.spawn(para(t, chunk.trim().to_owned(), t.text_size, t.text));
            }
        }
        _ => {
            d.spawn(para(t, trf("{0}: no briefing text in the install.", &[&mission_name(m)]), t.text_size, t.text_dim));
        }
    });
    block(p, CSlot::BriefGoals, region(CSlot::BriefGoals).px(ctx.b), |d| {
        if let Some(b) = m.briefing.as_ref().filter(|b| !b.goals.is_empty()) {
            d.spawn(para(t, tr("Objectives"), t.heading_size, t.title));
            for g in &b.goals {
                d.spawn(para(t, format!("* {g}"), t.text_size, t.text));
            }
        }
    });
    let state = ctx.campaigns.state(&c.id, m.index);
    act_button(p, ctx, CSlot::BriefBack, &tr("Mission list"), CampaignAct::ToMissions, true);
    let (label, on) = match &state {
        MissionState::Ready if ctx.wait => (tr("Please wait"), false),
        MissionState::Ready => (tr("Start mission"), true),
        _ => (tr("Cannot start"), false),
    };
    act_button(p, ctx, CSlot::BriefStart, &label, CampaignAct::Start, on);
}

fn debrief(p: &mut Box_, ctx: &Ctx) {
    let t = ctx.theme;
    let Some(d) = &ctx.campaigns.debrief else {
        title(p, ctx, CSlot::DebTitle, &tr("Debrief"));
        act_button(p, ctx, CSlot::DebList, &tr("Previous menu"), CampaignAct::ToSinglePlayer, true);
        return;
    };
    let cs = ctx.campaigns;
    let mission = cs.mission(&d.campaign, d.mission);
    let name = mission.map_or_else(String::new, |(_, m)| mission_name(m));
    title(p, ctx, CSlot::DebTitle, &name);
    let (headline, colour) = match d.result {
        RunResult::Won => (tr("Mission accomplished"), t.good),
        RunResult::Lost => (tr("Mission failed"), t.bad),
        RunResult::Abandoned => (tr("Mission abandoned"), t.warn),
    };
    put(p, CSlot::DebResult, region(CSlot::DebResult).px(ctx.b), kit_text(t, t.caps(&headline, t.caps_titles), t.title_size, colour));
    block(p, CSlot::DebText, region(CSlot::DebText).px(ctx.b), |b| {
        match &d.transition {
            Transition::Next { campaign, mission } => {
                let next = cs.mission(campaign, *mission).map_or_else(String::new, |(_, m)| mission_name(m));
                b.spawn(para(t, trf("Next mission: {0}", &[&next]), t.text_size, t.text));
            }
            Transition::CampaignComplete { campaign, unlocked } => {
                let name = cs.book.campaign(campaign).map_or_else(String::new, campaign_name);
                b.spawn(para(t, trf("{0} is complete.", &[&name]), t.text_size, t.good));
                for u in unlocked {
                    let name = cs.book.campaign(u).map_or_else(|| u.clone(), campaign_name);
                    b.spawn(para(t, trf("{0} is now open.", &[&name]), t.text_size, t.text));
                }
            }
            Transition::Lost { .. } => {
                b.spawn(para(t, tr("Your progress is unchanged."), t.text_size, t.text));
            }
        }
        if let Some(e) = &d.save_error {
            b.spawn(para(t, trf("Progress could not be saved: {0}", &[e]), t.small_size, t.bad));
        }
    });
    act_button(p, ctx, CSlot::DebList, &tr("Mission list"), CampaignAct::ToMissions, true);
    match &d.transition {
        Transition::Next { mission, .. } => {
            act_button(p, ctx, CSlot::DebRetry, &tr("Play again"), CampaignAct::Pick(d.mission), !ctx.wait);
            act_button(p, ctx, CSlot::DebNext, &tr("Next mission"), CampaignAct::Pick(*mission), !ctx.wait);
        }
        Transition::CampaignComplete { .. } => {
            act_button(p, ctx, CSlot::DebNext, &tr("Single player"), CampaignAct::ToSinglePlayer, !ctx.wait);
        }
        Transition::Lost { mission, .. } => {
            act_button(p, ctx, CSlot::DebNext, &tr("Retry"), CampaignAct::Pick(*mission), !ctx.wait);
        }
    }
}

/// Single Player's card for campaign `i`: the text and whether it opens. `None`: no such campaign
/// (the card stays off with the stock caption).
pub fn card(campaigns: Option<&Campaigns>, i: usize) -> Option<(String, bool)> {
    let cs = campaigns?;
    let c = cs.card(i)?;
    Some((cs.card_text(c), cs.is_open(&c.id)))
}
