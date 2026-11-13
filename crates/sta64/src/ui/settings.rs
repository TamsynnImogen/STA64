//! The settings file (UI-29): the player's UI and input choices, kept between runs.
//!
//! One plain-text file of `name = value` lines in the platform config folder
//! ([`platform::config_dir`]): `<config>/settings.cfg`. It holds
//!
//! - the key bindings, as `action = Key, Key` lines in [`KeyBindings::to_text`]'s own
//!   format (`select_type = KeyT`);
//! - `max_selection`, Refit's `MaxSelection` with its range ([`MaxSelection`], 17..=120);
//! - `ui_profile = lcars | stock` ([`UiPreference`]);
//! - the options screen's volumes (`master`, `music`, `effects`, `voice`, `0..=100`),
//!   `scroll_speed`, `fullscreen`, `vsync`, `show_fps` ([`Settings`]);
//! - `player_name` and `relay`, the multiplayer name and relay address last used (UI-11);
//! - `intro = true | false` ([`IntroSettings`]);
//! - `version = 1`.
//!
//! **Loading** happens once, when the plugin is built: the file's values replace the
//! resources' defaults. A missing file, a file from an older version (keys it lacks
//! keep their defaults) and a file with lines this version does not know never fail:
//! what cannot be read is skipped with a `warn!`. Bytes that are not UTF-8 are ignored
//! (the lines that are text still read). Command-line flags win over the file
//! (`--lcars-ui`, `--nointro`) and are not written back unless the player changes that
//! setting too. With no file, `RefitPlayer.ini` of the install (`[QOL.QOL]`
//! `MaxSelection`, the choice the Refit screen wrote) seeds the selection cap; the
//! originals' own `ARMADA.PRF` is an unlabelled number list and is not read.
//!
//! **Saving** follows change: when any watched resource differs from what is on disk,
//! the file is written after [`SAVE_DELAY`] seconds without further change (a slider
//! drag is one write) and when the app exits. The write is atomic (a temporary file in
//! the same folder, synced, renamed over the target), and a failure warns and is
//! retried at the next change; it never panics.
//!
//! Nothing here touches `ui/hud/`. Tests give the plugin a temporary path.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use bevy::app::AppExit;
use bevy::prelude::*;

use crate::input::keys::{Action, KeyBindings};
use crate::input::select::MaxSelection;
use crate::media::IntroSettings;
use crate::platform;
use crate::ui::profile::UiPreference;

/// The file's name inside the config folder.
pub const FILE_NAME: &str = "settings.cfg";
/// Format version written to the file. A newer file is read as far as it is understood.
pub const VERSION: u32 = 1;
/// Seconds without a change before the file is written.
pub const SAVE_DELAY: f32 = 0.5;

/// What the file holds, in the types the game reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub bindings: KeyBindings,
    pub max_selection: MaxSelection,
    pub lcars: bool,
    pub settings: crate::frontend::screens::Settings,
    pub intro: bool,
}

impl Default for Stored {
    fn default() -> Self {
        Stored {
            bindings: KeyBindings::default(),
            max_selection: MaxSelection::default(),
            lcars: false,
            settings: crate::frontend::screens::Settings::default(),
            intro: true,
        }
    }
}

const NUMBERS: [&str; 5] = ["master", "music", "effects", "voice", "scroll_speed"];
const FLAGS: [&str; 3] = ["fullscreen", "vsync", "show_fps"];

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Stored {
    /// Read a file's text over the defaults. Returns what it could understand and one
    /// message per line that it could not; never fails.
    pub fn parse(text: &str) -> (Stored, Vec<String>) {
        let mut s = Stored::default();
        let mut warnings = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let at = n + 1;
            let Some((name, value)) = line.split_once('=') else {
                warnings.push(format!("line {at}: not 'name = value', skipped"));
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if let Some(a) = Action::from_name(name) {
                if let Err(e) = s.bindings.bind_from_text(a, value) {
                    warnings.push(format!("line {at}: {e}"));
                }
                continue;
            }
            let ok = match name {
                "version" => {
                    match value.parse::<u32>() {
                        Ok(v) if v > VERSION => warnings.push(format!("written by a newer version ({v}); reading what is understood")),
                        Ok(_) => {}
                        Err(_) => warnings.push(format!("line {at}: version '{value}' is not a number")),
                    }
                    true
                }
                "max_selection" => match value.parse::<i64>() {
                    Ok(v) => {
                        s.max_selection = MaxSelection::new(v.clamp(0, 100_000) as usize);
                        if s.max_selection.get() as i64 != v {
                            warnings.push(format!("line {at}: max_selection {v} is outside 17..=120, using {}", s.max_selection.get()));
                        }
                        true
                    }
                    Err(_) => false,
                },
                "ui_profile" => match value.to_ascii_lowercase().as_str() {
                    "lcars" => {
                        s.lcars = true;
                        true
                    }
                    "stock" => {
                        s.lcars = false;
                        true
                    }
                    _ => false,
                },
                "intro" => parse_bool(value).map(|b| s.intro = b).is_some(),
                "player_name" => {
                    s.settings.player_name = value.chars().filter(|c| !c.is_control()).take(24).collect();
                    true
                }
                "relay" => {
                    s.settings.relay = value.chars().filter(|c| !c.is_control()).take(80).collect();
                    true
                }
                n if NUMBERS.contains(&n) => match value.parse::<f32>() {
                    Ok(v) if v.is_finite() => {
                        let key = number_key(n);
                        s.settings.set_number(key, v);
                        if s.settings.number(key) != Some(v) {
                            warnings.push(format!("line {at}: {n} {v} is outside 0..=100, clamped"));
                        }
                        true
                    }
                    _ => false,
                },
                n if FLAGS.contains(&n) => parse_bool(value).map(|b| s.settings.set_flag(flag_key(n), b)).is_some(),
                _ => {
                    warnings.push(format!("line {at}: unknown setting '{name}', ignored"));
                    continue;
                }
            };
            if !ok {
                warnings.push(format!("line {at}: bad value '{value}' for '{name}', default kept"));
            }
        }
        s.settings.lcars_ui = s.lcars;
        (s, warnings)
    }

    /// The file's text.
    pub fn to_text(&self) -> String {
        use crate::frontend::screens::SettingKey as K;
        let mut t = format!("# STA64 settings. Written by the game; edit while it is not running.\nversion = {VERSION}\n");
        let st = &self.settings;
        let _ = writeln!(t, "ui_profile = {}", if self.lcars { "lcars" } else { "stock" });
        let _ = writeln!(t, "max_selection = {}", self.max_selection.get());
        let _ = writeln!(t, "intro = {}", self.intro);
        for (name, key) in [("master", K::MasterVolume), ("music", K::MusicVolume), ("effects", K::EffectsVolume), ("voice", K::VoiceVolume), ("scroll_speed", K::ScrollSpeed)] {
            let _ = writeln!(t, "{name} = {}", st.number(key).unwrap_or(0.0));
        }
        for (name, v) in [("fullscreen", st.fullscreen), ("vsync", st.vsync), ("show_fps", st.show_fps)] {
            let _ = writeln!(t, "{name} = {v}");
        }
        if !st.player_name.is_empty() {
            let _ = writeln!(t, "player_name = {}", st.player_name);
        }
        if !st.relay.is_empty() {
            let _ = writeln!(t, "relay = {}", st.relay);
        }
        t += &self.bindings.to_text();
        t
    }
}

fn number_key(name: &str) -> crate::frontend::screens::SettingKey {
    use crate::frontend::screens::SettingKey as K;
    match name {
        "master" => K::MasterVolume,
        "music" => K::MusicVolume,
        "effects" => K::EffectsVolume,
        "voice" => K::VoiceVolume,
        _ => K::ScrollSpeed,
    }
}

fn flag_key(name: &str) -> crate::frontend::screens::SettingKey {
    use crate::frontend::screens::SettingKey as K;
    match name {
        "fullscreen" => K::Fullscreen,
        "vsync" => K::Vsync,
        _ => K::ShowFps,
    }
}

/// Reads the file at `path`: what it holds plus the warnings to show, and whether a
/// file was there at all. A missing file is not a warning; an unreadable one is.
pub fn load(path: &Path) -> (Stored, Vec<String>, bool) {
    match std::fs::read(path) {
        Ok(bytes) => {
            // Not-UTF-8 bytes (a Latin-1 comment, say) become U+FFFD: the lines that are
            // text still read, the others warn. A leading byte order mark is dropped.
            let text = String::from_utf8_lossy(&bytes);
            let (s, mut w) = Stored::parse(text.strip_prefix('\u{feff}').unwrap_or(&text));
            if matches!(text, std::borrow::Cow::Owned(_)) {
                w.insert(0, format!("{} is not valid UTF-8; unreadable bytes ignored", path.display()));
            }
            (s, w, true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Stored::default(), Vec::new(), false),
        Err(e) => (Stored::default(), vec![format!("cannot read {}: {e}; using defaults", path.display())], true),
    }
}

/// Writes `text` to `path` without ever leaving a half-written file: a temporary file
/// beside it, synced, then renamed over it (an existing file is replaced on every
/// platform).
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `[QOL.QOL] MaxSelection` of the install's `RefitPlayer.ini` (the Refit screen's
/// choice; the key name is Refit's, read from data), when it is a number.
pub fn seed_max_selection(install_root: &Path) -> Option<MaxSelection> {
    let text = std::fs::read_to_string(install_root.join("RefitPlayer.ini")).ok()?;
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_section = sec.trim().eq_ignore_ascii_case("QOL.QOL");
        } else if in_section
            && let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("MaxSelection")
        {
            return v.trim().parse::<usize>().ok().map(MaxSelection::new);
        }
    }
    None
}

/// A setting given on the command line: the file's value stays what is written
/// until the player changes that setting.
#[derive(Debug, Clone, Copy, Default)]
struct Pinned {
    forced: bool,
    file: bool,
    start: bool,
    touched: bool,
}

impl Pinned {
    fn new(forced: bool, file: bool, start: bool) -> Self {
        Pinned { forced, file, start, touched: false }
    }

    /// What to write for the live value `now`.
    fn write(&mut self, now: bool) -> bool {
        if self.forced {
            self.touched |= now != self.start;
            if !self.touched {
                return self.file;
            }
        }
        now
    }
}

/// The plugin's state: where the file is, what is on disk and what waits to be written.
#[derive(Resource)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    /// The text on disk (or that was read, or that failed to write).
    on_disk: String,
    /// The last values written or read, for resources that are absent.
    stored: Stored,
    lcars: Pinned,
    intro: Pinned,
    /// Seconds the current change has waited since it last changed.
    waiting: Option<f32>,
    /// The text of the last change seen while waiting, to restart the wait on a newer one.
    pending: String,
    /// The text a write failed on.
    failed: Option<String>,
    /// Seconds to wait; tests set it to 0.
    pub delay: f32,
}

/// Loads the file at startup and saves on change. Add it after the plugins whose
/// resources it carries.
pub struct SettingsPlugin {
    /// The settings file; `None` neither reads nor writes.
    pub path: Option<PathBuf>,
    /// The install that may seed a first run (see the module docs).
    pub install_root: Option<PathBuf>,
    /// `--lcars-ui` was given.
    pub lcars_forced: bool,
    /// `--nointro` was given.
    pub intro_forced: bool,
}

impl SettingsPlugin {
    /// The game's own: at the platform path, in the windowed modes only (a bench run
    /// or a headless test never reads or writes the player's file).
    pub fn from_cfg(cfg: &crate::app::LaunchConfig) -> Self {
        let persists = cfg.mode().has_window() && !matches!(cfg.mode(), crate::app::Mode::Bench { .. });
        SettingsPlugin {
            path: persists.then(platform::config_dir).flatten().map(|d| d.join(FILE_NAME)),
            install_root: cfg.install_root.clone(),
            lcars_forced: cfg.lcars_ui,
            intro_forced: cfg.nointro,
        }
    }
}

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let mut file_values = Stored::default();
        if let Some(path) = &self.path {
            let (mut s, warnings, existed) = load(path);
            for w in &warnings {
                warn!("settings: {w}");
            }
            if !existed && let Some(m) = self.install_root.as_deref().and_then(seed_max_selection) {
                info!("settings: no settings file; MaxSelection {} from the install's RefitPlayer.ini", m.get());
                s.max_selection = m;
            }
            file_values = s;
        }
        // What counts as "already saved": what was read (or the defaults); only a
        // difference from it is written, so a start that changes nothing writes nothing.
        let on_disk = file_values.to_text();
        let mut applied = file_values.clone();
        let world = app.world_mut();
        if let Some(mut r) = world.get_resource_mut::<KeyBindings>() {
            *r = applied.bindings.clone();
        }
        if let Some(mut r) = world.get_resource_mut::<MaxSelection>() {
            *r = applied.max_selection;
        }
        if self.lcars_forced {
            applied.lcars = true;
        }
        if let Some(mut r) = world.get_resource_mut::<UiPreference>() {
            r.lcars = applied.lcars;
        }
        if let Some(mut r) = world.get_resource_mut::<crate::frontend::screens::Settings>() {
            *r = applied.settings.clone();
            r.lcars_ui = applied.lcars;
        }
        if self.intro_forced {
            applied.intro = false;
        }
        if let Some(mut r) = world.get_resource_mut::<IntroSettings>() {
            r.enabled = applied.intro;
        }
        app.insert_resource(SettingsStore {
            path: self.path.clone(),
            on_disk,
            stored: file_values.clone(),
            lcars: Pinned::new(self.lcars_forced, file_values.lcars, applied.lcars),
            intro: Pinned::new(self.intro_forced, file_values.intro, applied.intro),
            waiting: None,
            pending: String::new(),
            failed: None,
            delay: SAVE_DELAY,
        });
        app.add_systems(Last, (save_on_change, save_on_exit).chain());
    }
}

/// The values the resources hold now (absent resources keep what was stored).
fn current(store: &mut SettingsStore, world: &World) -> Stored {
    let mut s = store.stored.clone();
    if let Some(r) = world.get_resource::<KeyBindings>() {
        s.bindings = r.clone();
    }
    if let Some(r) = world.get_resource::<MaxSelection>() {
        s.max_selection = *r;
    }
    let mut lcars = world.get_resource::<UiPreference>().map(|p| p.lcars);
    if let Some(r) = world.get_resource::<crate::frontend::screens::Settings>() {
        s.settings = r.clone();
        lcars = lcars.or(Some(r.lcars_ui));
    }
    if let Some(l) = lcars {
        s.lcars = store.lcars.write(l);
    }
    if let Some(r) = world.get_resource::<IntroSettings>() {
        s.intro = store.intro.write(r.enabled);
    }
    s.settings.lcars_ui = s.lcars;
    s
}

fn flush(store: &mut SettingsStore, now: &Stored) {
    let Some(path) = store.path.clone() else { return };
    let text = now.to_text();
    if text == store.on_disk {
        store.waiting = None;
        return;
    }
    match write_atomic(&path, &text) {
        Ok(()) => {
            store.failed = None;
            store.on_disk = text;
            store.stored = now.clone();
            store.waiting = None;
        }
        Err(e) => {
            warn!("settings: cannot write {}: {e}", path.display());
            // Not retried every frame: the next change tries again, and so does exit.
            store.failed = Some(text);
            store.waiting = None;
        }
    }
}

fn save_on_change(world: &mut World) {
    let Some(mut store) = world.remove_resource::<SettingsStore>() else { return };
    if store.path.is_some() {
        let dt = world.get_resource::<Time<Real>>().map_or(0.0, |t| t.delta_secs());
        let now = current(&mut store, world);
        let text = now.to_text();
        if text == store.on_disk {
            store.waiting = None;
        } else if store.failed.as_deref() != Some(text.as_str()) {
            // The wait runs from the last change: a drag is one write, when it stops.
            if text != store.pending {
                store.pending = text;
                store.waiting = Some(0.0);
            }
            let waited = store.waiting.unwrap_or(0.0) + dt;
            store.waiting = Some(waited);
            if waited >= store.delay {
                flush(&mut store, &now);
            }
        }
    }
    world.insert_resource(store);
}

fn save_on_exit(world: &mut World) {
    let exiting = world.get_resource::<Messages<AppExit>>().is_some_and(|m| !m.is_empty());
    if !exiting {
        return;
    }
    let Some(mut store) = world.remove_resource::<SettingsStore>() else { return };
    let now = current(&mut store, world);
    flush(&mut store, &now);
    world.insert_resource(store);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::screens::{SettingKey, Settings};
    use crate::platform::{ConfigOs, config_dir_in};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!("sta64-settings-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A headless app with the resources the plugin carries, loading `path`.
    fn app(path: &Path, lcars_forced: bool, intro_forced: bool) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<KeyBindings>()
            .init_resource::<MaxSelection>()
            .init_resource::<UiPreference>()
            .init_resource::<Settings>()
            .init_resource::<IntroSettings>()
            .add_plugins(SettingsPlugin { path: Some(path.to_path_buf()), install_root: None, lcars_forced, intro_forced });
        app.world_mut().resource_mut::<SettingsStore>().delay = 0.0;
        app
    }

    #[test]
    fn changes_survive_a_restart() {
        let dir = temp_dir("restart");
        let file = dir.join("config/settings.cfg");
        let mut a = app(&file, false, false);
        a.world_mut().resource_mut::<KeyBindings>().rebind(Action::SelectType, KeyCode::KeyQ);
        *a.world_mut().resource_mut::<MaxSelection>() = MaxSelection::new(75);
        a.world_mut().resource_mut::<UiPreference>().lcars = true;
        a.world_mut().resource_mut::<Settings>().set_number(SettingKey::MusicVolume, 33.0);
        a.update();
        assert!(file.is_file(), "written after a change, folder created");
        drop(a);

        let b = app(&file, false, false);
        assert_eq!(b.world().resource::<KeyBindings>().keys(Action::SelectType), [KeyCode::KeyQ]);
        assert_eq!(b.world().resource::<KeyBindings>().keys(Action::Stop), [KeyCode::KeyS, KeyCode::Space]);
        assert_eq!(b.world().resource::<MaxSelection>().get(), 75);
        assert!(b.world().resource::<UiPreference>().lcars);
        assert!(b.world().resource::<Settings>().lcars_ui);
        assert_eq!(b.world().resource::<Settings>().music, 33.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn untouched_defaults_write_nothing() {
        let dir = temp_dir("quiet");
        let file = dir.join("settings.cfg");
        let mut a = app(&file, false, false);
        a.update();
        a.update();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_older_and_corrupt_files_fall_back() {
        let dir = temp_dir("bad");
        let file = dir.join("settings.cfg");
        // Missing: defaults.
        let a = app(&file, false, false);
        assert_eq!(a.world().resource::<MaxSelection>().get(), 40);
        drop(a);
        // Older: no version, one key; the rest default.
        std::fs::write(&file, "select_type = KeyZ\n").unwrap();
        let a = app(&file, false, false);
        assert_eq!(a.world().resource::<KeyBindings>().keys(Action::SelectType), [KeyCode::KeyZ]);
        assert_eq!(a.world().resource::<Settings>().master, Settings::default().master);
        drop(a);
        // Garbled text: bad lines skipped with warnings, good lines kept.
        let (s, w) = Stored::parse("max_selection = lots\nnonsense\nmusic = 12\nfuture_key = 1\nstop = KeyNope\nversion = 9\n");
        assert_eq!(s.max_selection.get(), 40);
        assert_eq!(s.settings.music, 12.0);
        assert_eq!(s.bindings, KeyBindings::default());
        assert_eq!(w.len(), 5, "{w:?}");
        // Out of range is clamped, with a warning.
        let (s, w) = Stored::parse("max_selection = 9999\nmaster = 400\n");
        assert_eq!((s.max_selection.get(), s.settings.master), (120, 100.0));
        assert_eq!(w.len(), 2, "{w:?}");
        // Not text at all: defaults, the app starts, the file is left alone.
        std::fs::write(&file, [0xff, 0xfe, 0x00, 0x9f]).unwrap();
        let a = app(&file, false, false);
        assert_eq!(*a.world().resource::<KeyBindings>(), KeyBindings::default());
        assert!(file.is_file());
        // A Latin-1 byte in a comment and a byte order mark lose nothing.
        std::fs::write(&file, b"\xef\xbb\xbfmax_selection = 90\n# caf\xe9\nmusic = 11\n").unwrap();
        let (s, w, _) = load(&file);
        assert_eq!((s.max_selection.get(), s.settings.music), (90, 11.0));
        assert_eq!(w.len(), 1, "{w:?}");
        // A directory where the file should be: unreadable, defaults, no panic.
        let blocked = dir.join("blocked.cfg");
        std::fs::create_dir(&blocked).unwrap();
        let (s, w, _) = load(&blocked);
        assert_eq!(s, Stored::default());
        assert_eq!(w.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_write_warns_and_the_app_goes_on() {
        let dir = temp_dir("fail");
        // The "folder" is a file, so the settings folder cannot be made.
        std::fs::write(dir.join("config"), "x").unwrap();
        let file = dir.join("config/settings.cfg");
        let mut a = app(&file, false, false);
        a.world_mut().resource_mut::<MaxSelection>().clone_from(&MaxSelection::new(90));
        a.update();
        a.update();
        assert!(!file.exists());
        // The folder becomes writable: exit tries again and the change is saved.
        std::fs::remove_file(dir.join("config")).unwrap();
        a.world_mut().write_message(AppExit::Success);
        a.update();
        assert_eq!(load(&file).0.max_selection.get(), 90);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn writes_are_atomic_and_leave_no_temporary_files() {
        let dir = temp_dir("atomic");
        let file = dir.join("settings.cfg");
        write_atomic(&file, "a = 1\n").unwrap();
        write_atomic(&file, "a = 2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a = 2\n");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn command_line_flags_win_and_are_not_made_permanent() {
        let dir = temp_dir("flags");
        let file = dir.join("settings.cfg");
        let mut seed = Stored::default();
        seed.intro = true;
        write_atomic(&file, &seed.to_text()).unwrap();
        let mut a = app(&file, true, true);
        assert!(a.world().resource::<UiPreference>().lcars);
        assert!(!a.world().resource::<IntroSettings>().enabled);
        *a.world_mut().resource_mut::<MaxSelection>() = MaxSelection::new(60);
        a.update();
        let (s, w, _) = load(&file);
        assert!(w.is_empty());
        assert!(!s.lcars && s.intro, "the flags were not written");
        assert_eq!(s.max_selection.get(), 60);
        // The player switching it off and on again is a choice and is written.
        a.world_mut().resource_mut::<UiPreference>().lcars = false;
        a.update();
        a.world_mut().resource_mut::<UiPreference>().lcars = true;
        a.update();
        assert!(load(&file).0.lcars);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cap_is_seeded_from_the_installs_refit_choice_when_there_is_no_file() {
        let dir = temp_dir("seed");
        std::fs::write(dir.join("RefitPlayer.ini"), "; c\n[Other]\nMaxSelection=5\n[QOL.QOL]\nMaxSelection=120\nBarKeys=0\n").unwrap();
        assert_eq!(seed_max_selection(&dir).map(|m| m.get()), Some(120));
        assert_eq!(seed_max_selection(&dir.join("none")), None);
        let file = dir.join("cfg/settings.cfg");
        let mut a = App::new();
        a.add_plugins(MinimalPlugins).init_resource::<MaxSelection>().add_plugins(SettingsPlugin {
            path: Some(file.clone()),
            install_root: Some(dir.clone()),
            lcars_forced: false,
            intro_forced: false,
        });
        assert_eq!(a.world().resource::<MaxSelection>().get(), 120);
        // A file wins over the seed.
        write_atomic(&file, "max_selection = 30\n").unwrap();
        let mut b = App::new();
        b.add_plugins(MinimalPlugins).init_resource::<MaxSelection>().add_plugins(SettingsPlugin { path: Some(file), install_root: Some(dir.clone()), lcars_forced: false, intro_forced: false });
        assert_eq!(b.world().resource::<MaxSelection>().get(), 30);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exit_flushes_a_pending_change() {
        let dir = temp_dir("exit");
        let file = dir.join("settings.cfg");
        let mut a = app(&file, false, false);
        a.world_mut().resource_mut::<SettingsStore>().delay = 3600.0;
        *a.world_mut().resource_mut::<MaxSelection>() = MaxSelection::new(100);
        a.update();
        assert!(!file.exists(), "still waiting");
        a.world_mut().write_message(AppExit::Success);
        a.update();
        assert_eq!(load(&file).0.max_selection.get(), 100);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_text_round_trips() {
        let mut s = Stored::default();
        s.bindings.rebind(Action::Stop, KeyCode::KeyX);
        s.max_selection = MaxSelection::new(120);
        s.lcars = true;
        s.settings.fullscreen = true;
        s.settings.master = 12.5;
        s.settings.lcars_ui = true;
        s.intro = false;
        let (back, w) = Stored::parse(&s.to_text());
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back, s);
    }

    #[test]
    fn config_folders_follow_each_platforms_convention() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| PathBuf::from(v));
        let p = |parts: &[&str]| parts.iter().collect::<PathBuf>();
        assert_eq!(config_dir_in(ConfigOs::Linux, env(&[("HOME", "/h")])), Some(p(&["/h", ".config", "sta64"])));
        assert_eq!(config_dir_in(ConfigOs::Linux, env(&[("HOME", "/h"), ("XDG_CONFIG_HOME", "/x")])), Some(p(&["/x", "sta64"])));
        assert_eq!(config_dir_in(ConfigOs::Windows, env(&[("APPDATA", "C:\\Users\\a\\AppData\\Roaming")])), Some(PathBuf::from("C:\\Users\\a\\AppData\\Roaming").join("sta64")));
        assert_eq!(config_dir_in(ConfigOs::Windows, env(&[("USERPROFILE", "C:\\u")])), Some(PathBuf::from("C:\\u").join("AppData/Roaming/sta64")));
        assert_eq!(config_dir_in(ConfigOs::MacOs, env(&[("HOME", "/Users/a")])), Some(p(&["/Users/a", "Library/Application Support", "sta64"])));
        assert_eq!(config_dir_in(ConfigOs::Linux, env(&[("STA64_CONFIG_DIR", "/c"), ("HOME", "/h")])), Some(p(&["/c"])));
        assert_eq!(config_dir_in(ConfigOs::Linux, env(&[("HOME", "")])), None);
    }
}
