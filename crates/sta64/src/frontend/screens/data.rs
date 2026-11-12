//! What the screens show: the maps, races and mods to choose from, and the
//! player's choices so far.

use std::path::Path;
use std::sync::Arc;

use bevy::prelude::*;
use sta_content::{ContentDb, Fixed, MapDef, StartPos, StartingResources, Vec3 as SimVec3};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{PlayerId, TeamId};

/// The registry name of the computer opponent (`sta_ai::Registry::builtin`).
pub const SKIRMISH_AI: &str = "skirmish";

/// Most seats a game setup offers, whatever the map lists.
pub const MAX_SEATS: usize = 8;

/// A map the player can pick.
#[derive(Debug, Clone)]
pub struct MapEntry {
    pub def: Arc<MapDef>,
}

impl MapEntry {
    pub fn new(def: MapDef) -> MapEntry {
        MapEntry { def: Arc::new(def) }
    }

    /// The map's display name, or its file stem.
    pub fn title(&self) -> &str {
        self.def.label.as_deref().unwrap_or(&self.def.name)
    }

    /// Seats the map has start positions for.
    pub fn slots(&self) -> usize {
        self.def.starts.len().clamp(1, MAX_SEATS)
    }
}

/// An installed mod, as the mod select screen lists it (`sta_vfs::mods::ModInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModEntry {
    /// The folder name: what `--mod` and `mount_install` take.
    pub folder: String,
    pub name: String,
    pub description: String,
    pub authors: Vec<String>,
    pub version: Option<String>,
    pub parent: Option<String>,
}

impl From<&sta_vfs::mods::ModInfo> for ModEntry {
    fn from(m: &sta_vfs::mods::ModInfo) -> ModEntry {
        ModEntry {
            folder: m.folder.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            authors: m.authors.clone(),
            version: m.version.clone(),
            parent: m.parent.clone(),
        }
    }
}

/// The lists the screens choose from. The host fills it: [`ScreenData::fake`]
/// for tests and the frame run, a real mount's content, maps and mods in the
/// game (INT-M4).
#[derive(Resource, Debug, Clone)]
pub struct ScreenData {
    pub content: Arc<ContentDb>,
    pub races: Arc<sta_content::Races>,
    pub maps: Vec<MapEntry>,
    pub mods: Vec<ModEntry>,
    /// What the mod select screen calls the plain install (its folder's name);
    /// `None` shows "Base game".
    pub base_name: Option<String>,
}

impl ScreenData {
    /// Races in the mounted install's skirmish setup order.
    pub fn races(&self) -> Vec<String> {
        sta_content::race::skirmish_races(&self.races, &self.content)
    }

    /// User-facing caption, separate from the canonical ID used by setup/simulation.
    pub fn race_label(&self, id: &str, strings: Option<&crate::ui::i18n::Strings>) -> String {
        if id.is_empty() { return crate::ui::i18n::tr("none"); }
        let key = self.races.get_playable(id).or_else(|| self.races.get(id))
            .and_then(|race| race.display_key.as_deref());
        match key {
            Some(key) => strings.map_or(key, |strings| strings.game_text(key)).to_owned(),
            None => id.to_owned(),
        }
    }

    /// Every mod in a `Data` folder (CON-13), and a note for each folder skipped.
    pub fn mods_in(data: &Path) -> (Vec<ModEntry>, Vec<String>) {
        match sta_vfs::mods::list_mods(data) {
            Ok((mods, notes)) => (mods.iter().map(ModEntry::from).collect(), notes),
            Err(e) => (Vec::new(), vec![e.to_string()]),
        }
    }

    /// Made-up content, two maps (two and four seats) and two mods, for a
    /// front end with nothing mounted: tests and the frame run.
    pub fn fake() -> ScreenData {
        let two = sta_content::fake::two_bases();
        let mut four = two.clone();
        four.name = "fx_four_corners".into();
        four.label = Some("Four Corners".into());
        four.starts = [(-4000, -4000), (4000, -4000), (4000, 4000), (-4000, 4000)]
            .iter()
            .enumerate()
            .map(|(i, &(x, z))| StartPos {
                slot: i as u8,
                pos: SimVec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) },
                heading: (Fixed::ONE, Fixed::ZERO),
            })
            .collect();
        let m = |folder: &str, name: &str, desc: &str, parent: Option<&str>| ModEntry {
            folder: folder.into(),
            name: name.into(),
            description: desc.into(),
            authors: vec!["Fake Author".into()],
            version: Some("1.0".into()),
            parent: parent.map(str::to_owned),
        };
        ScreenData {
            base_name: Some("Fake Install".into()),
            content: Arc::new(sta_content::fake::db()),
            races: Arc::new(sta_content::Races::default()),
            maps: vec![MapEntry::new(two), MapEntry::new(four)],
            mods: vec![m("fx_balance", "Balance Pack", "Rebalances every ship.", None), m("fx_ships", "Extra Ships", "Adds a cruiser.", Some("fx_balance"))],
        }
    }
}

/// Who plays a seat in game setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatControl {
    /// This machine's player; one seat only.
    Human,
    /// The `skirmish` computer player (AI-02).
    Computer,
    /// Nobody: the seat exists and never sends a command.
    Idle,
}

impl SeatControl {
    pub fn label(self) -> &'static str {
        match self {
            SeatControl::Human => "Player",
            SeatControl::Computer => "Computer",
            SeatControl::Idle => "Idle",
        }
    }

    fn next(self) -> SeatControl {
        match self {
            SeatControl::Human => SeatControl::Computer,
            SeatControl::Computer => SeatControl::Idle,
            SeatControl::Idle => SeatControl::Human,
        }
    }

    pub fn control(self) -> Control {
        match self {
            SeatControl::Human => Control::Human,
            SeatControl::Computer => Control::Ai(SKIRMISH_AI.into()),
            SeatControl::Idle => Control::Idle,
        }
    }

    fn of(c: &Control) -> SeatControl {
        match c {
            Control::Human => SeatControl::Human,
            Control::Idle => SeatControl::Idle,
            // A remote seat is a lobby's; Instant Action has none.
            _ => SeatControl::Computer,
        }
    }
}

/// One start slot's choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    pub control: SeatControl,
    pub race: String,
    /// Team number from 0.
    pub team: u8,
}

/// Game setup's choices so far; [`SetupDraft::to_setup`] makes the
/// [`GameSetup`] a match starts from, [`SetupDraft::from_setup`] reads one back.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct SetupDraft {
    /// Index into [`ScreenData::maps`].
    pub map: usize,
    pub seed: u64,
    pub seats: Vec<Seat>,
    pub options: GameOptions,
}

impl SetupDraft {
    /// The first map, seat 0 the player, the others computers, every seat its
    /// own team, the races in turn.
    pub fn new(data: &ScreenData) -> SetupDraft {
        let mut d = SetupDraft { map: 0, seed: 1, seats: Vec::new(), options: GameOptions::default() };
        d.set_map(data, 0);
        d
    }

    /// Choose a map: the seats keep their choices as far as the new map has
    /// start slots, new ones are computers.
    pub fn set_map(&mut self, data: &ScreenData, map: usize) {
        self.map = map.min(data.maps.len().saturating_sub(1));
        if self.options.officer_limit as usize >= data.content.rules().officer_limits.len() {
            self.options.officer_limit = 0;
        }
        let slots = data.maps.get(self.map).map_or(1, MapEntry::slots);
        let races = data.races();
        self.seats.truncate(slots);
        while self.seats.len() < slots {
            let i = self.seats.len();
            self.seats.push(Seat {
                control: if i == 0 { SeatControl::Human } else { SeatControl::Computer },
                race: races.get(i % races.len().max(1)).cloned().unwrap_or_default(),
                team: i as u8,
            });
        }
        if !self.seats.iter().any(|s| s.control == SeatControl::Human) {
            self.seats[0].control = SeatControl::Human;
        }
    }

    /// Step a seat's control. The player is on one seat: taking it hands the
    /// old seat to a computer; stepping off the only player hands the role to
    /// the next seat.
    pub fn cycle_control(&mut self, row: usize) {
        let Some(old) = self.seats.get(row).map(|s| s.control) else { return };
        let mut new = old.next();
        if old == SeatControl::Human {
            // Stepping off the player: the next seat takes the role.
            let other = (row + 1) % self.seats.len();
            if other == row {
                return;
            }
            self.seats[other].control = SeatControl::Human;
            new = SeatControl::Computer;
        } else if new == SeatControl::Human {
            for s in &mut self.seats {
                if s.control == SeatControl::Human {
                    s.control = SeatControl::Computer;
                }
            }
        }
        self.seats[row].control = new;
    }

    pub fn cycle_race(&mut self, data: &ScreenData, row: usize) {
        let races = data.races();
        let Some(seat) = self.seats.get_mut(row) else { return };
        if let Some(i) = races.iter().position(|r| *r == seat.race) {
            seat.race = races[(i + 1) % races.len()].clone();
        } else if let Some(r) = races.first() {
            seat.race = r.clone();
        }
    }

    pub fn cycle_team(&mut self, row: usize) {
        let n = self.seats.len() as u8;
        if let Some(seat) = self.seats.get_mut(row) {
            seat.team = (seat.team + 1) % n.max(1);
        }
    }

    pub fn cycle_resources(&mut self) {
        self.options.starting_resources = match self.options.starting_resources {
            StartingResources::Normal => StartingResources::Lots,
            StartingResources::Lots => StartingResources::Normal,
        };
    }

    pub fn cycle_officer_limit(&mut self, data: &ScreenData) {
        let count = data.content.rules().officer_limits.len();
        if count > 0 {
            self.options.officer_limit = ((self.options.officer_limit as usize + 1) % count) as u32;
        }
    }

    /// The setup a match starts from: seat `i` is `PlayerId(i)` on the map's
    /// start slot `i`. `None` when the draft names no map.
    pub fn to_setup(&self, data: &ScreenData) -> Option<GameSetup> {
        let map = data.maps.get(self.map)?;
        let races = data.races();
        if races.is_empty() || self.seats.iter().any(|s| !races.iter().any(|r| r.eq_ignore_ascii_case(&s.race))) {
            return None;
        }
        Some(GameSetup {
            seed: self.seed,
            map: map.def.clone(),
            content: data.content.clone(),
            players: self
                .seats
                .iter()
                .enumerate()
                .map(|(i, s)| PlayerSetup { id: PlayerId(i as u8), team: TeamId(s.team), race: s.race.clone(), control: s.control.control() })
                .collect(),
            options: self.options.clone(),
        })
    }

    /// The draft a setup came from; `None` when its map is not in the list.
    pub fn from_setup(setup: &GameSetup, data: &ScreenData) -> Option<SetupDraft> {
        let map = data.maps.iter().position(|m| m.def.name == setup.map.name)?;
        Some(SetupDraft {
            map,
            seed: setup.seed,
            seats: setup.players.iter().map(|p| Seat { control: SeatControl::of(&p.control), race: p.race.clone(), team: p.team.0 }).collect(),
            options: setup.options.clone(),
        })
    }
}

/// The mod the player chose on the mod select screen (`None`: the base game).
/// Mounting it is the host's: it reads [`ModChosen`](super::ModChosen).
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectedMod(pub Option<String>);

/// Which row a list screen has picked, before the player confirms.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct Browse {
    /// Map select: index into [`ScreenData::maps`].
    pub map: usize,
    /// Mod select: 0 is the base game, `n` is `mods[n - 1]`.
    pub mod_row: usize,
}

/// A setting the settings screen edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    MasterVolume,
    MusicVolume,
    EffectsVolume,
    VoiceVolume,
    ScrollSpeed,
    Fullscreen,
    Vsync,
    ShowFps,
    LcarsUi,
}

/// The player's settings. The screen edits them; what reads them (audio, the
/// window, the camera) is each lane's to wire, by watching this resource.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Settings {
    /// Volumes and scroll speed, `0..=100`.
    pub master: f32,
    pub music: f32,
    pub effects: f32,
    pub voice: f32,
    pub scroll_speed: f32,
    pub fullscreen: bool,
    pub vsync: bool,
    pub show_fps: bool,
    /// Explicit LCARS presentation override; false leaves the game/mod default in control.
    pub lcars_ui: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { master: 80.0, music: 60.0, effects: 80.0, voice: 80.0, scroll_speed: 50.0, fullscreen: false, vsync: true, show_fps: false, lcars_ui: false }
    }
}

impl Settings {
    pub fn number(&self, k: SettingKey) -> Option<f32> {
        Some(match k {
            SettingKey::MasterVolume => self.master,
            SettingKey::MusicVolume => self.music,
            SettingKey::EffectsVolume => self.effects,
            SettingKey::VoiceVolume => self.voice,
            SettingKey::ScrollSpeed => self.scroll_speed,
            _ => return None,
        })
    }

    pub fn flag(&self, k: SettingKey) -> Option<bool> {
        Some(match k {
            SettingKey::Fullscreen => self.fullscreen,
            SettingKey::Vsync => self.vsync,
            SettingKey::ShowFps => self.show_fps,
            SettingKey::LcarsUi => self.lcars_ui,
            _ => return None,
        })
    }

    pub fn set_number(&mut self, k: SettingKey, v: f32) {
        let v = v.clamp(0.0, 100.0);
        match k {
            SettingKey::MasterVolume => self.master = v,
            SettingKey::MusicVolume => self.music = v,
            SettingKey::EffectsVolume => self.effects = v,
            SettingKey::VoiceVolume => self.voice = v,
            SettingKey::ScrollSpeed => self.scroll_speed = v,
            _ => {}
        }
    }

    pub fn set_flag(&mut self, k: SettingKey, on: bool) {
        match k {
            SettingKey::Fullscreen => self.fullscreen = on,
            SettingKey::Vsync => self.vsync = on,
            SettingKey::ShowFps => self.show_fps = on,
            SettingKey::LcarsUi => self.lcars_ui = on,
            _ => {}
        }
    }
}
