//! The match flow (INT-M3, docs/WORKPLAN.md S10): a skirmish between the local
//! player and one opponent seat, built on the map view.
//!
//! `--skirmish <map>` runs the map view ([`crate::map`]) with a [`SkirmishCfg`]:
//!
//! - [`seats`] picks two seats from the map's start positions (the local one,
//!   `--seat`, and the lowest other), their races (`--race`,
//!   `--opponent-race`, else the playable races of the install in setup order)
//!   and who runs the opponent (`Control::Ai("skirmish")` or `Control::Idle`);
//! - the sim is built with `Sim::from_setup_with_starts`, so each seat starts
//!   with its race's start units and stock (`sta_content::race`);
//! - [`controllers`] builds the AI seats' controllers from `sta_ai`'s registry;
//!   the sim link thinks them after every tick;
//! - [`apply_race`] points the HUD at the local race's GUI cfg and the music at
//!   its playlist once the map has loaded;
//! - [`outcome`] says when a side has been eliminated: a notice, a log line and
//!   a banner;
//! - [`flow`] is the front end around it (INT-M4): launch, loading, the match, back.

pub mod flow;
pub mod mission_host;

use std::sync::Arc;

use bevy::prelude::*;
use sta_content::{ContentDb, MapDef, Races};
use sta_core::{PlayerId, TeamId};
use sta_sim::setup::{Control, Controller, GameSetup, PlayerSetup};

use crate::app::{LaunchConfig, Opponent};
use crate::sim_link::SimLink;
use crate::ui::notices::{Notice, Severity};

/// What the command line asked of the match.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct SkirmishCfg {
    /// The local player's start slot.
    pub seat: u8,
    pub opponent: Opponent,
    pub race: Option<String>,
    pub opponent_race: Option<String>,
}

impl SkirmishCfg {
    pub fn from_launch(cfg: &LaunchConfig) -> SkirmishCfg {
        SkirmishCfg { seat: cfg.seat.unwrap_or(0), opponent: cfg.opponent, race: cfg.race.clone(), opponent_race: cfg.opponent_race.clone() }
    }
}

/// The local player's race, once the match is built: what the HUD and the
/// music are set from.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct LocalRace {
    pub name: String,
    pub interface: Option<String>,
    pub interface_sprites: Option<String>,
    pub music: Option<String>,
}

/// The seats of a skirmish, and what was decided on the way.
pub struct Seats {
    pub players: Vec<PlayerSetup>,
    pub local: LocalRace,
    /// One line for the map's info panel and the log.
    pub summary: String,
}

/// Two seats from the map's start slots.
pub fn seats(map: &MapDef, cfg: &SkirmishCfg, races: &Races, db: &ContentDb) -> Result<Seats, String> {
    let mut slots: Vec<u8> = map.starts.iter().map(|s| s.slot).collect();
    slots.sort_unstable();
    slots.dedup();
    if slots.len() < 2 {
        return Err(format!("{} has {} start position(s); a skirmish needs two (try a multiplayer map)", map.name, slots.len()));
    }
    if !slots.contains(&cfg.seat) {
        return Err(format!("seat {} has no start position on {}; its start slots are {slots:?}", cfg.seat, map.name));
    }
    let opponent_slot = *slots.iter().find(|&&s| s != cfg.seat).expect("two slots");

    let available = sta_content::race::skirmish_races(races, db);
    if available.is_empty() {
        return Err("this install has no playable races declared for Instant Action".into());
    }
    let playable = races.playable();
    let pick = |wanted: &Option<String>, taken: Option<&str>, skip: usize| -> String {
        if let Some(w) = wanted {
            if let Some(name) = available.iter().find(|name| name.eq_ignore_ascii_case(w.trim())) {
                return name.clone();
            }
            match races.get(w) {
                Some(r) if playable.iter().any(|p| p.stem == r.stem) => return r.name.clone(),
                _ => warn!("race {w:?} is not playable in this install ({})", available.join(", ")),
            }
        }
        let others: Vec<&String> = available.iter().filter(|r| Some(r.as_str()) != taken).collect();
        others.get(skip % others.len().max(1)).map(|r| (*r).clone()).or_else(|| available.first().cloned()).unwrap_or_default()
    };
    let mine = pick(&cfg.race, None, 0);
    let theirs = pick(&cfg.opponent_race, Some(&mine), 0);

    let player = |slot: u8, race: &str, control: Control| PlayerSetup { id: PlayerId(slot), team: TeamId(slot), race: race.to_owned(), control };
    let ai = match cfg.opponent {
        Opponent::Skirmish => Control::Ai(sta_ai::SKIRMISH.into()),
        Opponent::Idle => Control::Idle,
    };
    let mut players = vec![player(cfg.seat, &mine, Control::Human), player(opponent_slot, &theirs, ai)];
    players.sort_by_key(|p| p.id);

    let def = races.get_playable(&mine).or_else(|| races.get(&mine));
    let local = LocalRace {
        name: mine.clone(),
        interface: def.and_then(|r| r.interface.clone()),
        interface_sprites: def.and_then(|r| r.interface_sprites.clone()),
        music: def.and_then(|r| r.multiplayer_music.clone().or_else(|| r.single_player_music.clone())),
    };
    let summary = format!(
        "skirmish: you are seat {} ({mine}, {}), opponent seat {opponent_slot} ({theirs}, {})",
        cfg.seat,
        races.loadout(db, &mine).source,
        match cfg.opponent {
            Opponent::Skirmish => "skirmish AI",
            Opponent::Idle => "idle",
        }
    );
    Ok(Seats { players, local, summary })
}

/// The controllers of the setup's AI seats, by the registry's names.
pub fn controllers(setup: &GameSetup) -> Vec<Box<dyn Controller>> {
    let registry = sta_ai::Registry::builtin();
    let mut out = Vec::new();
    for p in &setup.players {
        if let Control::Ai(name) = &p.control {
            match registry.create_for_seat(name, setup, p.id) {
                Ok(c) => out.push(c),
                Err(e) => warn!("seat {}: {e}; it stays idle", p.id.0),
            }
        }
    }
    out
}

/// The sim of a skirmish: starting forces placed, the AI seats' controllers
/// ready for the link.
pub fn start(setup: &GameSetup, races: &Races) -> (sta_sim::Sim, Vec<sta_content::Warning>, Vec<Box<dyn Controller>>) {
    let (sim, warnings) = sta_sim::Sim::from_setup_with_starts(setup, races);
    {
        use sta_sim::WorldSource;
        let view = sim.view();
        for p in &setup.players {
            let units: Vec<&str> = view.units.iter().filter(|u| u.owner == p.id).filter_map(|u| setup.content.name(u.def)).collect();
            let stock = view.players.iter().find(|v| v.id == p.id).map(|v| v.resources.iter().map(|r| r.to_f32() as i64).collect::<Vec<_>>());
            info!("start: seat {} ({}, {:?}): {units:?}, stock {stock:?}", p.id.0, p.race, p.control);
        }
    }
    (sim, warnings, controllers(setup))
}

/// Read a GUI cfg of the mount (`misc/<name>`, includes by name) as the HUD's layout.
pub fn gui_layout(vfs: &sta_vfs::Vfs, name: &str) -> Option<sta_content::GuiLayout> {
    let mut load = |n: &str| {
        let p = format!("misc/{}", n.rsplit(['/', '\\']).next()?);
        vfs.read(&p)?.ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    let (cfg, warnings) = sta_content::GuiCfg::load(name, &mut load);
    if cfg.files().is_empty() {
        warn!("hud: {name} not found; stock sizes");
        return None;
    }
    for w in warnings.iter().take(5) {
        debug!("hud cfg: {w}");
    }
    Some(cfg.layout_auto())
}

#[derive(Default)]
pub struct GuiCandidates {
    source: Option<String>,
    mounted: bool,
    layouts: Vec<(String, sta_content::GuiLayout)>,
}

fn gui_names(name: &str) -> Vec<String> {
    let Some(stem) = name.get(..name.len().saturating_sub(4)).filter(|_| name.to_ascii_lowercase().ends_with(".cfg")) else {
        return vec![name.to_owned()];
    };
    const ASPECTS: [&str; 4] = ["4x3", "5x4", "16x10", "16x9"];
    if ASPECTS.iter().any(|suffix| stem.to_ascii_lowercase().ends_with(suffix)) {
        return vec![name.to_owned()];
    }
    let mut names = vec![name.to_owned()];
    names.extend(ASPECTS.map(|suffix| format!("{stem}{suffix}.cfg")));
    names
}

fn closest_gui(layouts: &[(String, sta_content::GuiLayout)], aspect: f32) -> Option<(&str, &sta_content::GuiLayout)> {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
    layouts.iter()
        .filter_map(|(name, layout)| {
            let (w, h) = layout.canvas;
            (w > 0 && h > 0).then_some((name.as_str(), layout, (w as f32 / h as f32 - aspect).abs()))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(name, layout, _)| (name, layout))
}

/// Point the HUD and the music at the local race, when the match is (re)built.
pub fn apply_race(
    race: Option<Res<LocalRace>>,
    vfs: Option<Res<crate::viewer::Mounted>>,
    hud: Option<ResMut<crate::ui::hud::HudConfig>>,
    music: Option<ResMut<crate::audio::MusicConfig>>,
    skin: Option<ResMut<crate::ui::gui_sprites::GuiSkin>>,
    mut candidates: Local<GuiCandidates>,
) {
    let Some(race) = race else {
        candidates.source = None;
        candidates.mounted = false;
        candidates.layouts.clear();
        if let Some(mut skin) = skin
            && skin.table.is_some()
        {
            skin.table = None;
        }
        if let Some(mut hud) = hud && hud.gui.is_some() {
            hud.gui = None;
        }
        return;
    };
    if let Some(mut skin) = skin && skin.table != race.interface_sprites {
        skin.table.clone_from(&race.interface_sprites);
    }
    let source = race.interface.as_ref();
    if race.is_changed()
        || vfs.as_ref().is_some_and(|v| v.is_changed())
        || candidates.source.as_ref() != source
        || candidates.mounted != vfs.is_some()
    {
        candidates.source = source.cloned();
        candidates.mounted = vfs.is_some();
        candidates.layouts.clear();
        if let (Some(vfs), Some(cfg)) = (&vfs, source) {
            for name in gui_names(cfg) {
                let basename = name.rsplit(['/', '\\']).next().unwrap_or(&name);
                if vfs.0.read(&format!("misc/{basename}")).is_some()
                    && let Some(layout) = gui_layout(&vfs.0, &name)
                {
                    candidates.layouts.push((name, layout));
                }
            }
        }
    }
    if let Some(mut hud) = hud {
        let chosen = closest_gui(&candidates.layouts, hud.aspect);
        if hud.gui.as_ref() != chosen.map(|(_, layout)| layout) {
            hud.gui = chosen.map(|(_, layout)| layout.clone());
            if let Some((name, _)) = chosen {
                info!("hud: {} for {}", name, race.name);
            }
        }
    }
    if race.is_changed() && let Some(mut music) = music {
        music.calm = race.music.iter().cloned().collect();
        info!("music: {:?} for {}", music.calm, race.name);
    }
}

/// The match's result, once it is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Victory,
    Defeat,
}

/// Who has won, from a view: the game is over when one team or none has a seat
/// left and at least one seat has been eliminated (SIM-13's rule). `me`'s team
/// wins when it is the one left.
pub fn verdict(view: &sta_sim::WorldView, me: PlayerId) -> Option<Verdict> {
    if !view.players.iter().any(|p| p.eliminated) {
        return None;
    }
    let mut alive: Vec<TeamId> = view.players.iter().filter(|p| !p.eliminated).map(|p| p.team).collect();
    alive.sort();
    alive.dedup();
    if alive.len() > 1 {
        return None;
    }
    let mine = view.players.iter().find(|p| p.id == me)?;
    Some(if mine.eliminated { Verdict::Defeat } else { Verdict::Victory })
}

#[derive(Component)]
pub struct EndBanner;

/// Shows the result once: a notice, a log line and a banner. A new match
/// (a new view that is undecided again) clears it.
pub fn outcome(
    mut commands: Commands,
    link: Option<Res<SimLink>>,
    mut shown: Local<Option<Verdict>>,
    mut notices: MessageWriter<Notice>,
    banner: Query<Entity, With<EndBanner>>,
) {
    let Some(link) = link else {
        for e in &banner {
            commands.entity(e).despawn();
        }
        *shown = None;
        return;
    };
    let now = verdict(link.curr(), link.player());
    if now == *shown {
        return;
    }
    for e in &banner {
        commands.entity(e).despawn();
    }
    *shown = now;
    let Some(v) = now else { return };
    let (title, line, severity) = match v {
        Verdict::Victory => ("VICTORY", "All enemy forces are destroyed.", Severity::Good),
        Verdict::Defeat => ("DEFEAT", "Your forces are destroyed.", Severity::Bad),
    };
    info!("skirmish over at tick {}: {title} ({line})", link.curr().tick);
    notices.write(Notice::new(severity, format!("{title}: {line}")));
    let colour = match v {
        Verdict::Victory => Color::srgb(0.95, 0.8, 0.3),
        Verdict::Defeat => Color::srgb(0.9, 0.3, 0.3),
    };
    commands
        .spawn((
            EndBanner,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                top: Val::Percent(30.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(50),
        ))
        .with_children(|p| {
            p.spawn((Text::new(title), TextFont { font_size: bevy::text::FontSize::Px(72.0), ..default() }, TextColor(colour)));
            p.spawn((Text::new(line), TextFont { font_size: bevy::text::FontSize::Px(24.0), ..default() }, TextColor(Color::WHITE)));
        });
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        // Headless frame runs have no notice plugin; the match still posts its notices.
        app.add_message::<Notice>()
            .add_systems(Update, (apply_race.before(crate::ui::radar::RadarUi).before(crate::ui::gui_sprites::paint), outcome).in_set(crate::app::GameSet::Ui));
    }
}

/// Keeps `Arc<Races>` handy for the map view.
pub type SharedRaces = Arc<Races>;

#[cfg(test)]
mod gui_selection_tests {
    use super::*;

    #[test]
    fn suffixes_are_only_probed_for_unsuffixed_names() {
        assert_eq!(gui_names("gui_fed.cfg").iter().map(String::as_str).collect::<Vec<_>>(), vec![
            "gui_fed.cfg", "gui_fed4x3.cfg", "gui_fed5x4.cfg",
            "gui_fed16x10.cfg", "gui_fed16x9.cfg",
        ]);
        assert_eq!(gui_names("gui_fed16x9.cfg"), vec!["gui_fed16x9.cfg".to_string()]);
    }

    #[test]
    fn race_cfg_tracks_canvas_aspect_mount_and_absence() {
        use crate::ui::gui_sprites::GuiSkin;
        use crate::ui::hud::HudConfig;
        use crate::viewer::Mounted;
        use std::fs;

        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("sta64-race-gui-{}-{nonce}", std::process::id()));
        let misc = root.join("Data/misc");
        fs::create_dir_all(&misc).unwrap();
        fs::write(misc.join("gui_test.cfg"), b"screenWidth=1600\nscreenHeight=1200\n").unwrap();
        fs::write(misc.join("gui_test16x9.cfg"), b"screenWidth=2134\nscreenHeight=1200\n").unwrap();
        let mut app = App::new();
        app.insert_resource(HudConfig { aspect: 4.0 / 3.0, ..default() })
            .init_resource::<GuiSkin>()
            .insert_resource(Mounted(sta_vfs::Vfs::mount_install(&root, None).unwrap()))
            .add_systems(Update, apply_race);
        app.insert_resource(LocalRace {
            name: "test".into(), interface: Some("gui_test.cfg".into()),
            interface_sprites: Some("gui_test.spr".into()), music: None,
        });
        app.update();
        assert_eq!(app.world().resource::<HudConfig>().gui.as_ref().unwrap().canvas, (1600, 1200));
        app.world_mut().resource_mut::<HudConfig>().aspect = 16.0 / 9.0;
        app.update();
        assert_eq!(app.world().resource::<HudConfig>().gui.as_ref().unwrap().canvas, (2134, 1200));
        app.world_mut().resource_mut::<LocalRace>().interface = Some("gui_test16x9.cfg".into());
        app.world_mut().resource_mut::<HudConfig>().aspect = 4.0 / 3.0;
        app.update();
        assert_eq!(app.world().resource::<HudConfig>().gui.as_ref().unwrap().canvas, (2134, 1200));
        app.world_mut().remove_resource::<Mounted>();
        app.update();
        assert!(app.world().resource::<HudConfig>().gui.is_none());
        app.world_mut().resource_mut::<LocalRace>().interface = None;
        app.update();
        assert!(app.world().resource::<HudConfig>().gui.is_none());
        app.world_mut().remove_resource::<LocalRace>();
        app.update();
        assert!(app.world().resource::<HudConfig>().gui.is_none());
        assert!(app.world().resource::<GuiSkin>().table.is_none());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod mission_tests;
#[cfg(test)]
mod tests;
