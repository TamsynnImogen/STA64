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
//!   a banner.

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
    let pick = |wanted: &Option<String>, taken: Option<&str>, skip: usize| -> String {
        if let Some(w) = wanted {
            match races.get(w) {
                Some(r) => return r.name.clone(),
                None => warn!("race {w:?} is not one of this install's ({})", available.join(", ")),
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

    let def = races.get(&mine);
    let local = LocalRace {
        name: mine.clone(),
        interface: def.and_then(|r| r.interface.clone()),
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
    Some(cfg.layout((640, 480)))
}

/// Point the HUD and the music at the local race, when the match is (re)built.
pub fn apply_race(
    race: Option<Res<LocalRace>>,
    vfs: Option<Res<crate::viewer::Mounted>>,
    hud: Option<ResMut<crate::ui::hud::HudConfig>>,
    music: Option<ResMut<crate::audio::MusicConfig>>,
) {
    let Some(race) = race.filter(|r| r.is_changed()) else { return };
    if let (Some(mut hud), Some(vfs), Some(cfg)) = (hud, vfs, &race.interface) {
        hud.gui = gui_layout(&vfs.0, cfg);
        info!("hud: {} for {}", cfg, race.name);
    }
    if let Some(mut music) = music {
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
            .add_systems(Update, (apply_race, outcome).in_set(crate::app::GameSet::Ui));
    }
}

/// Keeps `Arc<Races>` handy for the map view.
pub type SharedRaces = Arc<Races>;

#[cfg(test)]
mod tests;
