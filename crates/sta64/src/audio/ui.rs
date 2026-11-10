//! UI sounds (AUD-05): click, hover, error and chat sounds on the UI bus.
//!
//! Names follow the engine's config. `RTS_CFG.h` gives `cfgSOUND_BUTTON_SAMPLE`
//! (the HUD's click), `cfgSOUND_SHELL_BUTTON_HOVER` / `_CLICK` (the menus) and the
//! chat pair; [`UiSounds::from_cfg`] reads them and falls back to the Fleet
//! Operations values. The HUD's rollover is not in the config header (stock reads it
//! from the GUI config's button section), so it defaults to the Federation file.
//! Refit's engine layer uses `cornron.wav` / `cornrclk.wav` for a shell button
//! that has no sound of its own; those are the `SHELL_*_FALLBACK` names.
//!
//! The LCARS theme (UI-03) names no sounds itself: [`UiSoundPlugin`] listens to its
//! messages (`ButtonClicked`, `ToggleChanged`, `ListSelected`) and to pointer
//! entry on its widgets, and turns them into [`UiSound`] requests. Anything else
//! (a dialog refusing input, chat) writes a [`UiSound`] message directly.
//! Each request becomes a [`PlaySound`] on [`Bus::Ui`]; the mixer does the rest.
//! Sliders make no sound: Refit plays one click per change, not per pixel.

use bevy::picking::events::PointerOver;
use bevy::prelude::*;
use sta_assets::cfg_h::CfgH;
use sta_vfs::Vfs;

use crate::sim_link::{Bus, PlaySound};
use crate::ui::theme::{ButtonClicked, ListItem, ListSelected, Toggle, ToggleChanged, UiButton, WidgetState};

/// A UI event that makes a sound.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiSound {
    Click,
    Hover,
    /// A refused action (a disabled order, a full queue, a bad input).
    Error,
    ChatSend,
    ChatReceive,
}

impl UiSound {
    pub const ALL: [UiSound; 5] = [UiSound::Click, UiSound::Hover, UiSound::Error, UiSound::ChatSend, UiSound::ChatReceive];
}

/// Which family of controls is on screen: in-game HUD or the front-end menus.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiContext {
    #[default]
    Hud,
    Shell,
}

/// Fleet Operations' defaults, as `RTS_CFG.h` ships them.
pub const HUD_CLICK: &str = "federation_click.wav";
pub const HUD_HOVER: &str = "federation_rollover.wav";
pub const SHELL_HOVER: &str = "Beep1.ogg";
pub const SHELL_CLICK: &str = "Beep2.ogg";
pub const ERROR: &str = "klaxon.wav";
pub const CHAT_SEND: &str = "Chatsend.ogg";
pub const CHAT_RECEIVE: &str = "Chatreceive.ogg";
/// The engine's shell button sounds when a button has none of its own.
pub const SHELL_HOVER_FALLBACK: &str = "cornron.wav";
pub const SHELL_CLICK_FALLBACK: &str = "cornrclk.wav";

/// Sound file names for each UI event.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct UiSounds {
    pub hud_click: String,
    pub hud_hover: String,
    pub shell_click: String,
    pub shell_hover: String,
    pub error: String,
    pub chat_send: String,
    pub chat_receive: String,
}

impl Default for UiSounds {
    fn default() -> Self {
        Self {
            hud_click: HUD_CLICK.into(),
            hud_hover: HUD_HOVER.into(),
            shell_click: SHELL_CLICK.into(),
            shell_hover: SHELL_HOVER.into(),
            error: ERROR.into(),
            chat_send: CHAT_SEND.into(),
            chat_receive: CHAT_RECEIVE.into(),
        }
    }
}

impl UiSounds {
    /// The names `RTS_CFG.h` gives; what it leaves out keeps the default.
    pub fn from_cfg(cfg: &CfgH) -> Self {
        let d = Self::default();
        let get = |key: &str, default: String| cfg.raw(key).map(str::trim).filter(|s| !s.is_empty()).map_or(default, str::to_owned);
        Self {
            hud_click: get("cfgSOUND_BUTTON_SAMPLE", d.hud_click),
            shell_hover: get("cfgSOUND_SHELL_BUTTON_HOVER", d.shell_hover),
            shell_click: get("cfgSOUND_SHELL_BUTTON_CLICK", d.shell_click),
            chat_send: get("cfgSOUND_INGAME_CHAT_SEND", d.chat_send),
            chat_receive: get("cfgSOUND_INGAME_CHAT_RECEIVE", d.chat_receive),
            ..d
        }
    }

    /// The file for an event in a context.
    pub fn name(&self, sound: UiSound, ctx: UiContext) -> &str {
        match (sound, ctx) {
            (UiSound::Click, UiContext::Hud) => &self.hud_click,
            (UiSound::Click, UiContext::Shell) => &self.shell_click,
            (UiSound::Hover, UiContext::Hud) => &self.hud_hover,
            (UiSound::Hover, UiContext::Shell) => &self.shell_hover,
            (UiSound::Error, _) => &self.error,
            (UiSound::ChatSend, _) => &self.chat_send,
            (UiSound::ChatReceive, _) => &self.chat_receive,
        }
    }

    /// Every name this table can ask for, including the shell fallbacks, without duplicates.
    pub fn all_names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for ctx in [UiContext::Hud, UiContext::Shell] {
            for s in UiSound::ALL {
                out.push(self.name(s, ctx));
            }
        }
        out.extend([SHELL_HOVER_FALLBACK, SHELL_CLICK_FALLBACK]);
        let mut seen = std::collections::HashSet::new();
        out.retain(|n| seen.insert(n.to_ascii_lowercase()));
        out
    }

    /// The names `exists` does not accept.
    pub fn unresolved(&self, exists: impl Fn(&str) -> bool) -> Vec<String> {
        self.all_names().into_iter().filter(|n| !exists(n)).map(str::to_owned).collect()
    }
}

/// Whether the VFS has the sound file `name` (with its extension) under `sounds/effects/`.
pub fn sound_exists(vfs: &Vfs, name: &str) -> bool {
    vfs.resolve(&format!("sounds/effects/{name}")).is_some()
}

/// Adds the UI sound table, the theme listeners and the [`UiSound`] to [`PlaySound`] mapping.
pub struct UiSoundPlugin;

impl Plugin for UiSoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<UiSound>()
            .add_message::<PlaySound>()
            .add_message::<ButtonClicked>()
            .add_message::<ToggleChanged>()
            .add_message::<ListSelected>()
            .init_resource::<UiSounds>()
            .init_resource::<UiContext>()
            .add_observer(hover_sound)
            .add_systems(Update, sync_context.before(theme_clicks).in_set(crate::app::GameSet::Audio))
            .add_systems(Update, (theme_clicks, play_ui_sounds).chain().before(super::play_sounds).in_set(crate::app::GameSet::Audio));
    }
}

type Clickable = Or<(With<UiButton>, With<Toggle>, With<ListItem>)>;

/// The front end's buttons use the shell sounds, the match's HUD the HUD ones.
/// Without an app state (bare test apps) the context stays as set.
fn sync_context(state: Option<Res<State<crate::app::AppState>>>, mut ctx: ResMut<UiContext>) {
    use crate::app::AppState;
    let Some(state) = state else { return };
    let want = match state.get() {
        AppState::InGame | AppState::Viewer => UiContext::Hud,
        AppState::Boot | AppState::FrontEnd | AppState::Loading => UiContext::Shell,
    };
    if *ctx != want {
        *ctx = want;
    }
}

/// The pointer comes onto an enabled button, toggle or list row.
fn hover_sound(
    ev: On<PointerOver>,
    widgets: Query<&WidgetState, Clickable>,
    mut out: MessageWriter<UiSound>,
) {
    if widgets.get(ev.entity).is_ok_and(|s| *s != WidgetState::Disabled) {
        out.write(UiSound::Hover);
    }
}

fn theme_clicks(
    mut buttons: MessageReader<ButtonClicked>,
    mut toggles: MessageReader<ToggleChanged>,
    mut lists: MessageReader<ListSelected>,
    mut out: MessageWriter<UiSound>,
) {
    let n = buttons.read().count() + toggles.read().count() + lists.read().count();
    // One click sound per frame however many widgets answered.
    if n > 0 {
        out.write(UiSound::Click);
    }
}

fn play_ui_sounds(
    mut requests: MessageReader<UiSound>,
    sounds: Res<UiSounds>,
    ctx: Res<UiContext>,
    mut out: MessageWriter<PlaySound>,
) {
    // The same event twice in a frame (two rows under a fast pointer) sounds once.
    let mut seen: Vec<UiSound> = Vec::new();
    for &s in requests.read() {
        if !seen.contains(&s) {
            seen.push(s);
            out.write(PlaySound { name: sounds.name(s, *ctx).to_owned(), at: None, bus: Bus::Ui });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::audio::{AudioPlugin, Call, CallLog, Mixer, NullAudio, Resolver, Sound, SoundResolver};
    use crate::ui::theme::{ThemePlugin, click};

    /// What a Fleet Operations 4.0 install's `sounds/effects` holds of the UI names.
    const FIXTURE_FILES: &[&str] = &[
        "federation_click.wav",
        "federation_rollover.wav",
        "Beep1.ogg",
        "Beep2.ogg",
        "klaxon.wav",
        "Chatsend.ogg",
        "Chatreceive.ogg",
        "cornron.wav",
        "cornrclk.wav",
    ];

    const RTS_CFG_FIXTURE: &str = "#define cfgSOUND_BUTTON_SAMPLE \"federation_click.wav\"\n\
        #define cfgSOUND_SHELL_BUTTON_HOVER \"Beep1.ogg\"\n\
        #define cfgSOUND_SHELL_BUTTON_CLICK \"Beep2.ogg\"\n\
        #define cfgSOUND_INGAME_CHAT_SEND \"Chatsend.ogg\"\n\
        #define cfgSOUND_INGAME_CHAT_RECEIVE \"Chatreceive.ogg\"\n";

    fn fixture_vfs(dir: &Path) -> Vfs {
        let fx = dir.join("sounds/effects");
        std::fs::create_dir_all(&fx).unwrap();
        for f in FIXTURE_FILES {
            std::fs::write(fx.join(f), b"").unwrap();
        }
        let mut vfs = Vfs::default();
        vfs.push_dir("Data".into(), dir.to_path_buf()).unwrap();
        vfs
    }

    #[test]
    fn theme_sound_names_resolve_against_the_fixture_list() {
        let dir = std::env::temp_dir().join(format!("sta64-aud05-{}", std::process::id()));
        let vfs = fixture_vfs(&dir);
        let sounds = UiSounds::from_cfg(&CfgH::parse(RTS_CFG_FIXTURE));
        assert_eq!(sounds, UiSounds::default(), "the fixture header is the defaults");
        assert_eq!(sounds.unresolved(|n| sound_exists(&vfs, n)), Vec::<String>::new());
        // Lookup is case-insensitive, as the engine's is.
        assert!(sound_exists(&vfs, "beep1.OGG"));
        assert_eq!(sounds.unresolved(|n| n != "klaxon.wav"), ["klaxon.wav"], "a missing name is reported");
        std::fs::remove_dir_all(dir).ok();
    }

    /// With `STA64_TEST_INSTALL` (+ optional `STA64_TEST_MOD`): the install's own
    /// `RTS_CFG.h` and sound files satisfy every UI name.
    #[test]
    fn real_install_if_available() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
        let sounds = match vfs.read("RTS_CFG.h") {
            Some(Ok(b)) => UiSounds::from_cfg(&CfgH::parse(&String::from_utf8_lossy(&b))),
            _ => UiSounds::default(),
        };
        let missing = sounds.unresolved(|n| sound_exists(&vfs, n));
        eprintln!("ui sounds: {:?}, missing {missing:?}", sounds.all_names());
        assert!(missing.is_empty(), "unresolved UI sounds: {missing:?}");
    }

    /// Accepts any name and decodes it to silence.
    struct Any;
    impl SoundResolver for Any {
        fn resolve(&mut self, name: &str) -> Option<Sound> {
            Some(Sound { name: name.to_owned(), sample_rate: 44_100, frames: Arc::from(vec![[0.0; 2]; 4]) })
        }
    }

    fn app() -> (App, CallLog) {
        let (null, log) = NullAudio::new();
        let mut app = App::new();
        app.insert_non_send(Mixer::new(Box::new(null)));
        app.add_plugins((ThemePlugin, AudioPlugin::default()));
        app.insert_resource(Resolver(Box::new(Any)));
        app.update();
        log.drain();
        (app, log)
    }

    fn plays(log: &CallLog) -> Vec<(String, Bus)> {
        log.drain()
            .into_iter()
            .filter_map(|c| match c {
                Call::Play { name, bus, at: None } => Some((name, bus)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn events_play_on_the_ui_bus() {
        let (mut app, log) = app();
        for (s, name) in [
            (UiSound::Click, HUD_CLICK),
            (UiSound::Hover, HUD_HOVER),
            (UiSound::Error, ERROR),
            (UiSound::ChatSend, CHAT_SEND),
            (UiSound::ChatReceive, CHAT_RECEIVE),
        ] {
            app.world_mut().write_message(s);
            app.update();
            assert_eq!(plays(&log), [(name.to_owned(), Bus::Ui)], "{s:?}");
        }
        *app.world_mut().resource_mut::<UiContext>() = UiContext::Shell;
        app.world_mut().write_message(UiSound::Click);
        app.world_mut().write_message(UiSound::Click);
        app.update();
        assert_eq!(plays(&log), [(SHELL_CLICK.to_owned(), Bus::Ui)], "shell click, once per frame");
    }

    #[test]
    fn theme_clicks_make_the_click_sound() {
        let (mut app, log) = app();
        let button = app.world_mut().spawn((UiButton, WidgetState::Normal)).id();
        let off = app.world_mut().spawn((UiButton, WidgetState::Disabled)).id();
        let toggle = app.world_mut().spawn((Toggle { on: false }, WidgetState::Normal)).id();
        for e in [button, toggle] {
            click(app.world_mut(), e);
            app.update();
            assert_eq!(plays(&log), [(HUD_CLICK.to_owned(), Bus::Ui)]);
        }
        click(app.world_mut(), off);
        app.update();
        assert!(plays(&log).is_empty(), "a disabled button is silent");
    }
}
