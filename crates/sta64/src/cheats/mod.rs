//! Single-player cheat entry (UI-47). Gameplay changes and cineractive events use the command
//! log; diagnostics stay on the client. See docs/CHEATS.md for the supported rules and gaps.

pub mod input;

use bevy::prelude::*;
use sta_content::Profile;
use sta_sim::command::{CommandKind, Edit, Order, Subsystem, TeamOp};
use sta_sim::{EntityId, Fixed, WorldView};

use crate::app::GameSet;
use crate::game::mission_host::MissionHandle;
use crate::game::net_match::NetGame;
use crate::sim_link::{ContentRes, Selection, SimLink};
use crate::ui::ingame_menu::NetworkedMatch;
use crate::ui::notices::{Notice, Severity};
use crate::ui::theme::{Theme, kit_text};
use input::CheatSubmitted;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Code {
    Money,
    Win,
    Lose,
    TacticalAi,
    Explode,
    Eliminate,
    Move,
    DisableShields,
    LevelUp,
    Version,
    Crash,
    Cinematic,
    Fps,
    Dump,
}

/// These are whole, case-sensitive strings, as in the original chat dispatchers. Fleet Ops debug
/// commands follow the engine lineage, never the selected faction or a mod's display name.
fn code(profile: Profile, text: &str) -> Option<Code> {
    use Code::*;
    match text {
        "showmethemoney" => Some(Money),
        "kobayashimaru" => Some(Win),
        "kobayashimaru_lost" if profile != Profile::Sta1Classic => Some(Lose),
        "canofwhoopass" if profile == Profile::Sta1Classic => Some(TacticalAi),
        _ if matches!(profile, Profile::Fo3 | Profile::Fo4 | Profile::Native) => match text {
            "expl" => Some(Explode),
            "elim" => Some(Eliminate),
            "m" => Some(Move),
            "dis" => Some(DisableShields),
            "levelup" => Some(LevelUp),
            "version" => Some(Version),
            "crash" => Some(Crash),
            "cine" => Some(Cinematic),
            "fps" => Some(Fps),
            "dump" => Some(Dump),
            _ => None,
        },
        _ => None,
    }
}

/// Grants are expressed in engine stock slots, so renamed resources keep their proper slot.
/// A1/A2: 2,000 dilithium. FO: 10,000 dilithium, 10,000 latinum (usually Tritanium),
/// 200 biomatter (usually Supply). This is the stock FO handler, not A2FOCheats' optional override.
fn grants(profile: Profile) -> [(u8, i32); 3] {
    if matches!(profile, Profile::Sta1Classic | Profile::Sta2Classic) {
        [(0, 2000), (1, 0), (3, 0)]
    } else {
        [(0, 10_000), (1, 10_000), (3, 200)]
    }
}

/// Resolve selected live craft/stations once, deterministically, including an inspected enemy.
fn selected<'a>(
    view: &'a WorldView,
    ids: &[EntityId],
    content: &sta_content::ContentDb,
) -> Vec<&'a sta_sim::UnitView> {
    let mut seen = std::collections::BTreeSet::new();
    ids.iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .filter_map(|id| view.unit(id))
        .filter(|u| u.hull[0] > Fixed::ZERO && !u.flags.contains(sta_sim::view::UnitFlags::DYING))
        .filter(|u| {
            content.unit(u.def).is_some_and(|d| {
                matches!(
                    sta_sim::world::EntityKind::of_def(d),
                    sta_sim::world::EntityKind::Craft | sta_sim::world::EntityKind::Station
                )
            })
        })
        .collect()
}

/// Build supported simulation requests from the real view. They are logged under the reserved
/// BENCH sender, which already exists for local sandbox edits; no player obtains edit privileges.
fn requests(
    cheat: Code,
    link: &SimLink,
    ids: &[EntityId],
    content: &sta_content::ContentDb,
) -> Vec<CommandKind> {
    let view = link.real_curr();
    let units = selected(view, ids, content);
    match cheat {
        Code::Money => grants(content.profile())
            .into_iter()
            .filter(|(_, n)| *n != 0)
            .map(|(slot, n)| {
                CommandKind::Edit(Edit::TeamPool(
                    link.player(),
                    TeamOp::ResourceAdd(slot),
                    Fixed::from_int(n),
                ))
            })
            .collect(),
        Code::Explode => units
            .iter()
            .map(|u| CommandKind::Edit(Edit::Kill(u.id)))
            .collect(),
        Code::Eliminate => {
            // The guide's selected player: all of that seat's objects, not its allied seats.
            let Some(owner) = units.first().map(|u| u.owner).filter(|p| p.is_player()) else {
                return Vec::new();
            };
            view.units
                .iter()
                .filter(|u| u.owner == owner)
                .map(|u| CommandKind::Edit(Edit::Kill(u.id)))
                .collect()
        }
        Code::Move => units
            .iter()
            .filter(|u| {
                content
                    .unit(u.def)
                    .is_some_and(|d| d.movement.max_speed > Fixed::ZERO)
                    && !u.flags.contains(sta_sim::view::UnitFlags::ENGINES_OFF)
            })
            .map(|u| {
                let mut to = u.pos;
                to.x = to.x + u.heading.0 * Fixed::from_int(200);
                to.z = to.z + u.heading.1 * Fixed::from_int(200);
                CommandKind::Order(Order::Move {
                    units: vec![u.id],
                    to,
                    queue: false,
                })
            })
            .collect(),
        Code::DisableShields => units
            .iter()
            .filter(|u| u.shields.iter().any(|s| s[1] > Fixed::ZERO))
            .map(|u| CommandKind::Edit(Edit::Subsystem(u.id, Subsystem::Shields, false)))
            .collect(),
        _ => Vec::new(),
    }
}

#[derive(Resource, Default)]
struct DebugDisplay {
    fps: bool,
}

#[derive(Component)]
struct FpsText;

/// The dump uses the source's real snapshot, never its revealed drawing view. Exclusive creation
/// preserves earlier dumps, even if the player enters the command twice in the same millisecond.
fn dump(link: &mut SimLink) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    let bytes = link.snapshot().ok_or("This game source has no snapshot")?;
    let root = sta_bench::capture::default_capture_dir().join("dumps");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    for n in 0..1000 {
        let path = root.join(format!("state-{stamp}-{n}.s64s"));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        return Ok(path);
    }
    Err("Could not allocate a new snapshot dump filename".into())
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    mut lines: MessageReader<CheatSubmitted>,
    link: Option<ResMut<SimLink>>,
    content: Option<Res<ContentRes>>,
    selection: Option<Res<Selection>>,
    net: Option<Res<NetGame>>,
    network: Option<Res<NetworkedMatch>>,
    scene: Option<Res<crate::frontend::scene::MenuScene>>,
    mission: Option<Res<MissionHandle>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    mut display: ResMut<DebugDisplay>,
    mut notices: MessageWriter<Notice>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut link) = link.filter(|_| scene.is_none()) else {
        lines.read().for_each(drop);
        return;
    };
    for line in lines.read() {
        if line.0.is_empty() {
            continue;
        }
        let result = if net.is_some() || network.is_some() {
            Err("Cheats are available only in single-player games".to_owned())
        } else if let Some(db) = content.as_ref() {
            match code(db.0.profile(), &line.0) {
                Some(Code::Win | Code::Lose) => {
                    use sta_mission::cheat::{CheatGame, CheatSession, ChatCheat};
                    match mission.as_ref() {
                        Some(handle) => {
                            let game = if db.0.profile() == Profile::Sta1Classic { CheatGame::A1 } else { CheatGame::A2 };
                            match handle.with(|rt| sta_mission::cheat::try_chat_cheat(rt, game, CheatSession::SinglePlayer, link.real_curr().tick, &line.0)) {
                                ChatCheat::Forced(_) => Ok("Mission result forced".into()),
                                _ => Err("The mission has already ended".into()),
                            }
                        }
                        None => Err("Mission-ending cheats require a running campaign mission".into()),
                    }
                }
                Some(Code::TacticalAi) => Err("canofwhoopass needs Armada I's tactical-AI modes, which STA64 does not implement yet".into()),
                Some(Code::LevelUp) => Err("levelup needs vessel rank promotion, which STA64 does not implement yet".into()),
                Some(Code::Version) => Ok(format!("STA64 {} ({})", env!("CARGO_PKG_VERSION"), option_env!("STA64_GIT_COMMIT").unwrap_or("local build"))),
                Some(Code::Fps) => { display.fps = !display.fps; Ok(format!("Frame rate display {}", if display.fps { "on" } else { "off" })) }
                Some(Code::Crash) => { exit.write(AppExit::Error(std::num::NonZeroU8::new(1).unwrap())); Ok("Closing STA64 (crash cheat)".into()) }
                Some(Code::Dump) => dump(&mut link).map(|path| format!("Snapshot: {}", path.display())),
                Some(Code::Cinematic) => {
                    let on = !cine.as_ref().is_some_and(|c| c.on);
                    link.submit_sandbox(CommandKind::Edit(Edit::Present(sta_sim::present::Presentation::Cineractive { on, skippable: true })));
                    Ok(format!("Cinematic mode {}", if on { "on" } else { "off" }))
                }
                Some(cheat) => {
                    let commands = requests(cheat, &link, selection.as_ref().map_or(&[], |s| s.0.as_slice()), &db.0);
                    if commands.is_empty() {
                        Err("Select a live eligible ship or station first".into())
                    } else {
                        for command in commands { link.submit_sandbox(command); }
                        Ok(format!("Cheat queued: {}", line.0))
                    }
                }
                None => Err("Unknown cheat for this game (codes are case-sensitive)".into()),
            }
        } else {
            Err("No game content is loaded".into())
        };
        let (severity, text) = match result {
            Ok(text) => (Severity::Info, text),
            Err(text) => (Severity::Warn, text),
        };
        notices.write(Notice::new(severity, text));
    }
}

fn fps_overlay(
    mut commands: Commands,
    display: Res<DebugDisplay>,
    theme: Res<Theme>,
    time: Option<Res<Time<Real>>>,
    mut texts: Query<(Entity, &mut crate::ui::sprite_font::SpriteText), With<FpsText>>,
) {
    if !display.fps {
        for (entity, _) in &mut texts {
            commands.entity(entity).despawn();
        }
        return;
    }
    let fps = time
        .as_ref()
        .map_or(0.0, |t| 1.0 / t.delta_secs().max(0.000_001));
    let text = format!("{fps:.0} FPS");
    if texts.is_empty() {
        commands.spawn((
            FpsText,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                ..default()
            },
            GlobalZIndex(1000),
            kit_text(&theme, text, theme.small_size, theme.text),
        ));
    } else {
        for (_, mut caption) in &mut texts {
            caption.text.clone_from(&text);
        }
    }
}

pub struct CheatsPlugin;

impl Plugin for CheatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(input::CheatInputPlugin)
            .init_resource::<DebugDisplay>()
            .add_message::<Notice>()
            .add_message::<AppExit>()
            .add_systems(Update, dispatch.in_set(GameSet::Input))
            .add_systems(Update, fps_overlay.in_set(GameSet::Ui));
    }
}

#[cfg(test)]
mod tests;
