//! Presentation profile selection (UI-17). Gameplay rules remain owned by
//! `sta_content::Profile`; this resource selects only UI presentation.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_content::Profile as GameProfile;
use sta_vfs::Vfs;

use crate::app::GameSet;
use crate::viewer::Mounted;

use super::theme::{self, Theme};

#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiProfile {
    #[default]
    Lcars,
    Armada2,
    FleetOps,
}

impl UiProfile {
    pub fn is_stock(self) -> bool {
        self != Self::Lcars
    }

    /// A neutral fallback for stock presentation. The mounted CFG/SPR files,
    /// rather than these tokens, supply the original menu and HUD artwork.
    pub fn theme(self) -> Theme {
        if !self.is_stock() {
            return Theme::default();
        }
        let mut t = Theme::default();
        let (accent, accent2) = match self {
            Self::Armada2 => (Color::srgb_u8(103, 150, 180), Color::srgb_u8(74, 102, 120)),
            Self::FleetOps => (Color::srgb_u8(159, 175, 186), Color::srgb_u8(78, 95, 106)),
            Self::Lcars => unreachable!(),
        };
        t.panel = Color::srgb_u8(10, 17, 23);
        t.panel_hi = Color::srgb_u8(39, 53, 62);
        t.text = Color::srgb_u8(222, 232, 237);
        t.text_dim = Color::srgb_u8(151, 170, 179);
        t.text_off = Color::srgb_u8(95, 112, 122);
        t.text_on = Color::srgb_u8(8, 15, 19);
        t.title = accent;
        t.accent = accent;
        t.accent2 = accent2;
        t.accent3 = Color::srgb_u8(87, 120, 139);
        t.button = Color::srgb_u8(45, 63, 74);
        t.button_hot = Color::srgb_u8(68, 91, 104);
        t.button_down = Color::srgb_u8(102, 125, 137);
        t.button_on = accent;
        t.button_off = Color::srgb_u8(29, 40, 48);
        t.track = Color::srgb_u8(28, 39, 47);
        t.fill = accent;
        t.focus = Color::srgb_u8(192, 215, 225);
        t.row_hot = Color::srgba_u8(87, 120, 139, 52);
        t.row_sel = Color::srgba_u8(103, 150, 180, 88);
        t.row_alt = Color::srgba_u8(255, 255, 255, 10);
        t.caps_titles = false;
        t.caps_buttons = false;
        t.gap = 4.0;
        t.pad = 8.0;
        t.row_h = 28.0;
        t.button_h = 32.0;
        t.list_row_h = 24.0;
        t.corners = 2.0;
        t.elbow_inner = 0.0;
        t.side_bar = 4.0;
        t.top_bar = 4.0;
        t.bottom_bar = 4.0;
        t.elbow_arm = 0.0;
        t.bar_gap = 2.0;
        t.cap_w = 0.0;
        t.track_h = 4.0;
        t
    }
}

#[derive(Resource, Default, Clone, Copy)]
pub struct UiPreference {
    pub lcars: bool,
}

/// Names supplied by integrations select an existing renderer family.
#[derive(Resource, Debug, Clone)]
pub struct UiProfiles {
    aliases: BTreeMap<String, UiProfile>,
}

impl Default for UiProfiles {
    fn default() -> Self {
        let mut profiles = Self {
            aliases: BTreeMap::new(),
        };
        for (name, profile) in [
            ("lcars", UiProfile::Lcars),
            ("a2", UiProfile::Armada2),
            ("armada2", UiProfile::Armada2),
            ("fo", UiProfile::FleetOps),
            ("fleetops", UiProfile::FleetOps),
        ] {
            profiles.register(name, profile);
        }
        profiles
    }
}

impl UiProfiles {
    pub fn register(&mut self, name: impl Into<String>, profile: UiProfile) {
        self.aliases.insert(name.into().trim().to_ascii_lowercase(), profile);
    }

    pub fn resolve(&self, name: &str, game: GameProfile) -> Option<UiProfile> {
        let name = name.trim().to_ascii_lowercase();
        if name == "stock" {
            return Some(UiProfile::from_game(game));
        }
        self.aliases.get(&name).copied()
    }
}

impl UiProfile {
    pub fn from_game(game: GameProfile) -> Self {
        match game {
            GameProfile::Sta2Classic => Self::Armada2,
            GameProfile::Fo4 => Self::FleetOps,
            GameProfile::Native | GameProfile::Sta1Classic => Self::Lcars,
        }
    }
}

/// Runs before widget painting and before clients that rebuild UI on a switch.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileSync;

pub struct UiProfilePlugin;

impl Plugin for UiProfilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Theme>()
            .init_resource::<UiProfile>()
            .init_resource::<UiPreference>()
            .init_resource::<UiProfiles>()
            .add_systems(
                Update,
                sync_profile
                    .in_set(GameSet::Ui)
                    .in_set(ProfileSync)
                    .before(theme::WidgetPaint),
            );
    }
}

/// First `[Mod] UiProfile` among visible mounted `info.ini` sources wins.
/// `resolve_all` stops at a whiteout. Mount layer names are unique in the
/// normal active-mod/parent/Data chain; advance the index to retain order if
/// an integration supplies repeated names.
fn mounted_choice(vfs: &Vfs) -> Option<String> {
    let mut next_layer = 0;
    for (name, source) in vfs.resolve_all("info.ini") {
        let Some(layer) = vfs
            .layers()
            .iter()
            .enumerate()
            .skip(next_layer)
            .find(|(_, layer)| layer.name() == name)
            .map(|(i, _)| i)
        else {
            warn!("UI profile: source layer {name:?} is missing");
            continue;
        };
        next_layer = layer + 1;
        match vfs.read_from(layer, source) {
            Ok(bytes) => {
                if let Some(value) = ini_ui_profile(&String::from_utf8_lossy(&bytes)) {
                    return Some(value);
                }
            }
            Err(error) => warn!("UI profile: cannot read {name}/info.ini: {error}"),
        }
    }
    None
}

fn ini_ui_profile(text: &str) -> Option<String> {
    sta_vfs::mods::Ini::parse(text)
        .get("mod", "UiProfile")
        .map(|value| value.trim().to_owned())
}

pub fn resolve_profile(vfs: Option<&Vfs>, preference: UiPreference, profiles: &UiProfiles) -> UiProfile {
    if preference.lcars {
        return UiProfile::Lcars;
    }
    let Some(vfs) = vfs else {
        return UiProfile::Lcars;
    };
    let game = GameProfile::detect(vfs);
    if let Some(name) = mounted_choice(vfs) {
        if let Some(profile) = profiles.resolve(&name, game) {
            return profile;
        }
        warn!("UI profile: unknown [Mod] UiProfile={name:?}; using game preset");
    }
    UiProfile::from_game(game)
}

fn sync_profile(
    mounted: Option<Res<Mounted>>,
    preference: Res<UiPreference>,
    profiles: Res<UiProfiles>,
    mut effective: ResMut<UiProfile>,
    mut theme: ResMut<Theme>,
    mut initialized: Local<bool>,
    mut had_mount: Local<bool>,
) {
    let mounted_now = mounted.is_some();
    let changed = !*initialized
        || *had_mount != mounted_now
        || mounted.as_ref().is_some_and(|mounted| mounted.is_changed())
        || preference.is_changed()
        || profiles.is_changed();
    *initialized = true;
    *had_mount = mounted_now;
    if !changed {
        return;
    }
    let selected = resolve_profile(mounted.as_ref().map(|mounted| &mounted.0), *preference, &profiles);
    if *effective != selected {
        *effective = selected;
        *theme = selected.theme();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;

    fn fixture() -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sta64-ui-profile-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("Data/Mods/Parent")).unwrap();
        std::fs::create_dir_all(root.join("Data/Mods/Child")).unwrap();
        std::fs::write(
            root.join("Data/Mods/Parent/info.ini"),
            "[Mod]\nName=Parent\nUiProfile=fo\n",
        )
        .unwrap();
        std::fs::write(
            root.join("Data/Mods/Child/info.ini"),
            "[Mod]\nName=Child\nParentMod=Parent\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn aliases_and_game_defaults() {
        let mut profiles = UiProfiles::default();
        assert_eq!(profiles.resolve(" A2 ", GameProfile::Native), Some(UiProfile::Armada2));
        assert_eq!(
            profiles.resolve("FLEETOPS", GameProfile::Native),
            Some(UiProfile::FleetOps)
        );
        assert_eq!(profiles.resolve("stock", GameProfile::Fo4), Some(UiProfile::FleetOps));
        profiles.register("custom", UiProfile::Armada2);
        assert_eq!(
            profiles.resolve("CUSTOM", GameProfile::Native),
            Some(UiProfile::Armada2)
        );
        assert_eq!(profiles.resolve("unknown", GameProfile::Fo4), None);
        assert_eq!(UiProfile::from_game(GameProfile::Sta1Classic), UiProfile::Lcars);
    }

    #[test]
    fn ini_only_uses_mod_section_and_first_key() {
        assert_eq!(
            ini_ui_profile("[Other]\nUiProfile=fo\n[mod]\nUiProfile = A2\nUiProfile=lcars\n"),
            Some("A2".into())
        );
        assert_eq!(ini_ui_profile("[Other]\nUiProfile=fo\n"), None);
    }

    #[test]
    fn explicit_lcars_without_mount() {
        let profiles = UiProfiles::default();
        assert_eq!(
            resolve_profile(None, UiPreference::default(), &profiles),
            UiProfile::Lcars
        );
        assert_eq!(
            resolve_profile(None, UiPreference { lcars: true }, &profiles),
            UiProfile::Lcars
        );
    }

    #[test]
    fn child_before_parent_and_runtime_switches() {
        let root = fixture();
        let mount = || Vfs::mount_install(&root, Some("Child")).unwrap();
        let profiles = UiProfiles::default();
        assert_eq!(
            resolve_profile(Some(&mount()), UiPreference::default(), &profiles),
            UiProfile::FleetOps
        );
        assert_eq!(
            resolve_profile(Some(&mount()), UiPreference { lcars: true }, &profiles),
            UiProfile::Lcars
        );

        std::fs::write(
            root.join("Data/Mods/Child/info.ini"),
            "[Mod]\nName=Child\nParentMod=Parent\nUiProfile=a2\n",
        )
        .unwrap();
        assert_eq!(
            resolve_profile(Some(&mount()), UiPreference::default(), &profiles),
            UiProfile::Armada2
        );

        std::fs::write(
            root.join("Data/Mods/Child/info.ini"),
            "[Mod]\nName=Child\nParentMod=Parent\nUiProfile=custom\n",
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        configure_sets(&mut app);
        app.add_plugins(UiProfilePlugin);
        app.world_mut().insert_resource(Mounted(mount()));
        app.update();
        assert_eq!(*app.world().resource::<UiProfile>(), UiProfile::Armada2); // unknown falls back to A2

        app.world_mut()
            .resource_mut::<UiProfiles>()
            .register("custom", UiProfile::FleetOps);
        app.update();
        assert_eq!(*app.world().resource::<UiProfile>(), UiProfile::FleetOps);
        assert_eq!(
            app.world().resource::<Theme>().accent,
            UiProfile::FleetOps.theme().accent
        );

        app.world_mut().resource_mut::<UiPreference>().lcars = true;
        app.update();
        assert_eq!(*app.world().resource::<UiProfile>(), UiProfile::Lcars);
        assert_eq!(app.world().resource::<Theme>().accent, Theme::default().accent);

        app.world_mut().resource_mut::<UiPreference>().lcars = false;
        std::fs::write(
            root.join("Data/Mods/Child/info.ini"),
            "[Mod]\nName=Child\nParentMod=Parent\nUiProfile=lcars\n",
        )
        .unwrap();
        app.world_mut().insert_resource(Mounted(mount()));
        app.update();
        assert_eq!(*app.world().resource::<UiProfile>(), UiProfile::Lcars);
        std::fs::remove_dir_all(root).unwrap();
    }
}
