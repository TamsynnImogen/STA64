//! App modes, states, system sets and the command line (docs/WORKPLAN.md
//! S10). Owned by R0 and the INT packages; other packages never edit it.
//!
//! - Each package ships one plugin, registers its systems only in its
//!   [`GameSet`], gates them on [`AppState`], and reads seams, not other
//!   packages' internals.
//! - The sets run in a fixed order each frame: bench, input, sim link,
//!   interpolate, then render, UI and audio.

use std::path::PathBuf;

use bevy::prelude::*;

use crate::clock::ClockMode;

/// What the executable was launched to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// The model viewer (`--view`, and the default).
    Viewer,
    /// A map to fly around in (`--map`).
    Map,
    /// A match from a game setup.
    Game,
    /// The front end (menus).
    FrontEnd,
    /// A bench scene (`--scene`); `headless`: no window; `sim_only`: no
    /// renderer at all.
    Bench { headless: bool, sim_only: bool },
}

impl Mode {
    /// Whether render, UI and audio plugins belong in this mode.
    pub fn has_window(self) -> bool {
        !matches!(self, Mode::Bench { headless: true, .. } | Mode::Bench { sim_only: true, .. })
    }
}

/// Where the app is. Packages gate their systems on it.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    Boot,
    FrontEnd,
    Loading,
    InGame,
    Viewer,
}

/// Every package's systems go in one of these.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameSet {
    /// Bench command channel: commands in, answers out.
    Bench,
    /// Input to [`CommandQueue`](crate::sim_link::CommandQueue) and selection.
    Input,
    /// Steps the sim (only the sim link).
    SimLink,
    /// Places things between the last two ticks.
    Interpolate,
    Render,
    Ui,
    Audio,
}

/// Order the sets in `Update`.
pub fn configure_sets(app: &mut App) {
    app.configure_sets(
        Update,
        (GameSet::Bench, GameSet::Input, GameSet::SimLink, GameSet::Interpolate, GameSet::Render, GameSet::Ui, GameSet::Audio).chain(),
    );
}

/// The bench's command channel (`--control`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Stdio,
    Socket,
}

/// Who runs the opponent's seat in a skirmish (`--opponent`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Opponent {
    /// `sta_ai`'s skirmish controller (AI-02).
    #[default]
    Skirmish,
    /// Nobody: the seat has its units and never sends a command (AI-01).
    Idle,
}

/// The command line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaunchConfig {
    pub install_root: Option<PathBuf>,
    pub active_mod: Option<String>,
    pub view: Option<String>,
    /// `--view` was given (with or without a model): the model viewer instead of the front end.
    pub viewer: bool,
    /// `Some("")`: `--map` without a name (the map browser).
    pub map: Option<String>,
    pub mission_report: bool,
    /// `--nointro` (the executables' own argument): skip the intro movies (MED-05).
    pub nointro: bool,
    /// `--skirmish [<map>]`: play a match on the map (the map view with a seat,
    /// starting forces, an opponent, the HUD and audio); `map` holds its name.
    pub skirmish: bool,
    /// `--opponent skirmish|idle`.
    pub opponent: Opponent,
    /// `--race <name>`: the local player's race (default: the first playable).
    pub race: Option<String>,
    /// `--opponent-race <name>`.
    pub opponent_race: Option<String>,
    /// `--seat <n>`: play seat `n` in the map view (the link speaks for it, its fog is drawn).
    pub seat: Option<u8>,
    pub help: bool,
    /// Bench scene file (`--scene <ini>`).
    pub scene: Option<PathBuf>,
    pub headless: bool,
    pub sim_only: bool,
    /// `--clock wall|bench`; `None`: the mode's default.
    pub clock: Option<ClockMode>,
    /// Run this many ticks, then exit.
    pub ticks: Option<u64>,
    pub control: Option<Control>,
    /// `--ui-gallery`: show the widget gallery instead of the viewer.
    pub ui_gallery: bool,
    /// Explicitly use the LCARS interface instead of the game/mod stock profile.
    pub lcars_ui: bool,
    /// `--frames 0,100`: save a frame at each of these ticks (needs `--headless`).
    pub frames: Vec<u64>,
    /// `--frame-dir <dir>`: where frames go (default `frames`).
    pub frame_dir: Option<PathBuf>,
    /// `--frame-size WxH`.
    pub frame_size: Option<(u32, u32)>,
    /// `--postfx bloom,smaa`: opt-in post effects (render/post.rs).
    pub postfx: Vec<String>,
    /// Arguments not understood, kept for the warning.
    pub unknown: Vec<String>,
}

pub const USAGE: &str = "\
Usage: sta64 [options]

  --install-root <path>    Armada II / Fleet Operations install (default: next to the executable)
  --mod <name>             mod to mount on top of the install
  --view [<model>]         model viewer (without any mode flag, the front end opens)
  --map [<map>]            fly around a map; without a name, the map browser
  --seat <n>               with --map: play seat n (orders for its units only, its fog and shroud drawn)
  --skirmish [<map>]       play a match: you (seat 0, or --seat n) against an opponent on a multiplayer
                           map, each with its race's starting forces; HUD, fog, music, sfx and voices;
                           a notice and a log line when one side is eliminated. Without a name, the map
                           browser (Left/Right change the map)
  --opponent skirmish|idle with --skirmish: the AI (default) or an opponent that never sends a command
  --race <name>            with --skirmish: your race, e.g. federation (default: the first playable)
  --opponent-race <name>   with --skirmish: the opponent's race (default: the next playable one)
  --mission-report         print which mission script every map would run, then exit
  --nointro                skip the logo and title movies at start

Test bench (docs/TESTBENCH.md):
  --scene <ini>            load a bench scene
  --headless               no window
  --sim-only               no renderer at all
  --clock wall|bench       animation clock (bench: fixed frame step)
  --ticks <n>              run n ticks, then exit
  --control stdio|socket   bench command channel
  --frames <t,t,..>        save a PNG frame at each of these ticks (headless, needs a GPU adapter)
  --frame-dir <dir>        where frames go (default: frames)
  --frame-size <WxH>       frame size (default: 640x360)
  --postfx <a,b,..>        opt-in post effects: bloom smaa fxaa vignette chromatic exposure aces reinhard cinematic (F8 cycles)

  --lcars-ui               use the LCARS interface instead of the game/mod stock profile
  --ui-gallery             show every UI widget (LCARS kit)

  --help                   this text
";

/// Bench step per frame when `--clock bench` gives none: 60 frames a second.
const BENCH_STEP: f64 = 1.0 / 60.0;

impl LaunchConfig {
    /// The process's own arguments, with the install root defaulting to the
    /// executable's directory, like the original game.
    pub fn from_env() -> LaunchConfig {
        let mut cfg = LaunchConfig::parse(std::env::args().skip(1));
        if cfg.install_root.is_none() {
            cfg.install_root = crate::platform::default_install_root();
        }
        cfg
    }

    /// Parse arguments (without the program name).
    pub fn parse(args: impl IntoIterator<Item = String>) -> LaunchConfig {
        let mut cfg = LaunchConfig::default();
        /// A flag's value: the next argument unless it is another flag.
        fn value(args: &mut std::iter::Peekable<impl Iterator<Item = String>>) -> Option<String> {
            args.next_if(|a| !a.starts_with("--"))
        }
        let mut args = args.into_iter().peekable();
        while let Some(a) = args.next() {
            match a.as_str() {
                "--install-root" => cfg.install_root = value(&mut args).map(PathBuf::from),
                "--mod" => cfg.active_mod = value(&mut args),
                "--view" => {
                    cfg.viewer = true;
                    cfg.view = value(&mut args);
                }
                "--map" => cfg.map = Some(value(&mut args).unwrap_or_default()),
                "--skirmish" => {
                    cfg.skirmish = true;
                    cfg.map = Some(value(&mut args).unwrap_or_default());
                }
                "--opponent" => match value(&mut args).as_deref() {
                    Some("skirmish") => cfg.opponent = Opponent::Skirmish,
                    Some("idle") => cfg.opponent = Opponent::Idle,
                    other => cfg.unknown.push(format!("--opponent {} (skirmish or idle)", other.unwrap_or(""))),
                },
                "--race" => cfg.race = value(&mut args),
                "--opponent-race" => cfg.opponent_race = value(&mut args),
                "--seat" => match value(&mut args).and_then(|v| v.parse().ok()) {
                    Some(n) => cfg.seat = Some(n),
                    None => cfg.unknown.push("--seat (needs a seat number)".into()),
                },
                "--ui-gallery" => cfg.ui_gallery = true,
                "--lcars-ui" => cfg.lcars_ui = true,
                "--mission-report" => cfg.mission_report = true,
                "--nointro" | "-nointro" => cfg.nointro = true,
                "--help" | "-h" => cfg.help = true,
                "--scene" => cfg.scene = value(&mut args).map(PathBuf::from),
                "--headless" => cfg.headless = true,
                "--sim-only" => cfg.sim_only = true,
                "--clock" => match value(&mut args).as_deref() {
                    Some("wall") => cfg.clock = Some(ClockMode::Wall),
                    Some("bench") => cfg.clock = Some(ClockMode::Bench { step: BENCH_STEP }),
                    other => cfg.unknown.push(format!("--clock {}", other.unwrap_or(""))),
                },
                "--ticks" => match value(&mut args).and_then(|v| v.parse().ok()) {
                    Some(n) => cfg.ticks = Some(n),
                    None => cfg.unknown.push("--ticks (needs a number)".into()),
                },
                "--control" => match value(&mut args).as_deref() {
                    Some("stdio") => cfg.control = Some(Control::Stdio),
                    Some("socket") => cfg.control = Some(Control::Socket),
                    other => cfg.unknown.push(format!("--control {}", other.unwrap_or(""))),
                },
                "--frames" => match value(&mut args).map(|v| v.split(',').map(|t| t.trim().parse().ok()).collect::<Option<Vec<u64>>>()) {
                    Some(Some(t)) => cfg.frames = t,
                    _ => cfg.unknown.push("--frames (needs ticks like 0,100)".into()),
                },
                "--postfx" => cfg.postfx.extend(value(&mut args)),
                "--frame-dir" => cfg.frame_dir = value(&mut args).map(PathBuf::from),
                "--frame-size" => match value(&mut args).and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))) {
                    Some(wh) => cfg.frame_size = Some(wh),
                    None => cfg.unknown.push("--frame-size (needs WxH)".into()),
                },
                _ => cfg.unknown.push(a),
            }
        }
        cfg
    }

    pub fn mode(&self) -> Mode {
        if self.scene.is_some() || self.sim_only || self.headless {
            Mode::Bench { headless: self.headless || self.sim_only, sim_only: self.sim_only }
        } else if self.skirmish {
            Mode::Game
        } else if self.map.is_some() {
            Mode::Map
        } else if self.viewer || self.ui_gallery {
            Mode::Viewer
        } else {
            // No flags that pick a mode: the front end (INT-M4).
            Mode::FrontEnd
        }
    }

    /// The animation clock mode: as given, else bench for bench runs.
    pub fn clock_mode(&self) -> ClockMode {
        self.clock.unwrap_or(match self.mode() {
            Mode::Bench { .. } => ClockMode::Bench { step: BENCH_STEP },
            _ => ClockMode::Wall,
        })
    }

    /// Flags that were given but do nothing yet, for `unsupported: <flag>`
    /// lines. Each disappears when its package (TOOL-03) lands.
    pub fn unsupported(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let flags = [
            (self.control == Some(Control::Socket), "--control socket"),
            (self.headless && self.scene.is_none() && !self.sim_only, "--headless without --scene"),
        ];
        for (given, flag) in flags {
            if given {
                out.push(flag);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcars_interface_is_an_explicit_presentation_option() {
        assert!(!LaunchConfig::default().lcars_ui);
        let cfg = LaunchConfig::parse(["--lcars-ui".to_owned()]);
        assert!(cfg.lcars_ui && cfg.unknown.is_empty());
    }

    fn parse(s: &str) -> LaunchConfig {
        LaunchConfig::parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn existing_launches_parse_as_before() {
        let c = parse("--install-root /games/a2 --mod Fleet-Ops --map a2_borg01");
        assert_eq!(c.install_root, Some(PathBuf::from("/games/a2")));
        assert_eq!(c.active_mod.as_deref(), Some("Fleet-Ops"));
        assert_eq!(c.map.as_deref(), Some("a2_borg01"));
        assert_eq!(c.mode(), Mode::Map);
        assert!(c.unsupported().is_empty() && c.unknown.is_empty());

        assert_eq!(parse("--map").map.as_deref(), Some(""));
        assert_eq!(parse("--map x --seat 2").seat, Some(2));
        assert_eq!(parse("--map x").seat, None);
        assert_eq!(parse("--view fdefiant").view.as_deref(), Some("fdefiant"));
        assert_eq!(parse("--view fdefiant").mode(), Mode::Viewer);
        assert!(parse("--mission-report").mission_report);
        assert_eq!(parse("--view").mode(), Mode::Viewer);
        assert_eq!(parse("--ui-gallery").mode(), Mode::Viewer);
        assert_eq!(parse("").mode(), Mode::FrontEnd);
        assert_eq!(parse("--install-root /g --mod FO").mode(), Mode::FrontEnd);
        assert_eq!(parse("").clock_mode(), ClockMode::Wall);
    }

    #[test]
    fn nointro_is_the_executables_own_flag() {
        assert!(!parse("").nointro);
        for a in ["--nointro", "-nointro"] {
            let c = parse(a);
            assert!(c.nointro && c.unknown.is_empty(), "{a}");
            assert_eq!(c.mode(), Mode::FrontEnd);
        }
    }

    #[test]
    fn skirmish_flags_parse() {
        let c = parse("--install-root /g --mod FO --skirmish blueamber --seat 1 --opponent idle --race klingon --opponent-race borg");
        assert!(c.skirmish);
        assert_eq!(c.map.as_deref(), Some("blueamber"));
        assert_eq!((c.seat, c.opponent), (Some(1), Opponent::Idle));
        assert_eq!((c.race.as_deref(), c.opponent_race.as_deref()), (Some("klingon"), Some("borg")));
        assert_eq!(c.mode(), Mode::Game);
        assert!(c.unknown.is_empty());
        let d = parse("--skirmish");
        assert_eq!((d.map.as_deref(), d.opponent, d.seat), (Some(""), Opponent::Skirmish, None));
        assert!(!parse("--map x").skirmish);
        assert_eq!(parse("--map x").mode(), Mode::Map);
        assert_eq!(parse("--skirmish x --opponent robots").unknown.len(), 1);
    }

    #[test]
    fn a_flag_after_map_is_not_its_name() {
        let c = parse("--map --mod FO");
        assert_eq!(c.map.as_deref(), Some(""));
        assert_eq!(c.active_mod.as_deref(), Some("FO"));
    }

    #[test]
    fn bench_flags_parse_and_report_unsupported() {
        let c = parse("--sim-only --scene bench/scenes/synthetic.ini --clock bench --ticks 300 --control stdio");
        assert_eq!(c.scene, Some(PathBuf::from("bench/scenes/synthetic.ini")));
        assert_eq!(c.ticks, Some(300));
        assert_eq!(c.control, Some(Control::Stdio));
        assert_eq!(c.mode(), Mode::Bench { headless: true, sim_only: true });
        assert!(!c.mode().has_window());
        assert_eq!(c.clock_mode(), ClockMode::Bench { step: BENCH_STEP });
        assert!(c.unsupported().is_empty());
        assert_eq!(parse("--control socket").unsupported(), ["--control socket"]);

        let c = parse("--scene s.ini --headless");
        assert_eq!(c.mode(), Mode::Bench { headless: true, sim_only: false });
        assert_eq!(c.clock_mode(), ClockMode::Bench { step: BENCH_STEP });
    }

    #[test]
    fn bad_values_are_reported() {
        let c = parse("--clock sometimes --ticks many --control pigeon --frobnicate");
        assert_eq!(c.unknown, ["--clock sometimes", "--ticks (needs a number)", "--control pigeon", "--frobnicate"]);
        assert!(parse("--help").help);
    }
}
