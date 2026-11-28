//! The campaign screens' backdrops from the mounted content's own shell bitmaps (#931).
//!
//! Under a stock profile ([`UiProfile::is_stock`](crate::ui::profile::UiProfile::is_stock)) the
//! single-player screen (the campaign select) and the mission list draw the mounted content's shell
//! picture behind their controls; under the LCARS profile they keep the widget theme. The briefing is
//! STA64's own screen (the original has no pre-mission briefing screen) and keeps the widget theme.
//! Nothing of the art is in the repository: the pictures are read from the mounted layers (install
//! or mod) at run time, a mod's own files winning by the layering, and a screen whose picture the
//! content does not have keeps the widget theme.
//!
//! Rules and grades:
//! - Which pictures the engine opens: the two paths below are constants of the code, read from the
//!   executable, static (its string table). Which screen draws which is not traced (the shell is
//!   built in the executable; no data file names a screen's picture): INTERIM.
//! - The picture fills the stock box the screen is laid out in ([`StockBoxNode`]), so the art
//!   and the controls move together at every window shape (arranged, STA64's own).
//! - INTERIM, a first step: only the backdrop. The controls keep their arranged regions and the widget
//!   kit's look over it. The original's lists and buttons have their own art and places on these
//!   screens (not drawn here yet), and the single-player screen has more pictures than the one drawn.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::frontend::screens::stock::StockBoxNode;
use crate::frontend::screens::Screen;
use crate::ui::profile::UiProfile;
use crate::viewer::Mounted;

/// The single-player screen's picture, without extension: a path the engine opens (read from the executable, static,
/// string table; which screen draws it is not traced).
pub(crate) const SINGLE_PLAYER: &str = "bitmaps/single/singleplay";
/// The mission list's picture, without extension: a path the engine opens (read from the executable, static,
/// string table; which screen draws it is not traced).
pub(crate) const MISSION_LIST: &str = "bitmaps/missionselect/missionselectbg";
/// Extensions tried, in order.
pub(crate) const EXTENSIONS: [&str; 3] = ["png", "bmp", "tga"];

/// The shell pictures of the mounted content, by screen. `None`: the content has none.
#[derive(Resource, Default)]
pub struct ShellArt {
    single_player: Option<Handle<Image>>,
    mission_list: Option<Handle<Image>>,
    /// The pictures were read from the current mount; a new mount reads again.
    read: bool,
}

impl ShellArt {
    /// The picture behind `screen`, for the screens that have one.
    pub fn backdrop(&self, screen: Screen) -> Option<&Handle<Image>> {
        match screen {
            Screen::SinglePlayer => self.single_player.as_ref(),
            Screen::Missions => self.mission_list.as_ref(),
            _ => None,
        }
    }
}

/// The root of a backdrop: a child of the screen's stock box, under its controls.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellBackdrop(pub Screen);

/// Read a picture of the mount by its stem: PNG or TGA through the image decoder, BMP through the
/// installs' own BMP reader.
fn load(vfs: &sta_vfs::Vfs, stem: &str) -> Option<Image> {
    EXTENSIONS.iter().find_map(|ext| {
        let path = format!("{stem}.{ext}");
        let rgba = if *ext == "bmp" {
            sta_assets::bmp::load(vfs, &path).ok()?
        } else {
            let data = vfs.read(&path)?.ok()?;
            let img = image::load_from_memory(&data).ok()?.to_rgba8();
            sta_assets::Rgba { width: img.width(), height: img.height(), pixels: img.into_raw(), has_alpha: true }
        };
        (rgba.width > 0 && rgba.height > 0).then(|| {
            Image::new(
                Extent3d { width: rgba.width, height: rgba.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                rgba.pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            )
        })
    })
}

/// Read the pictures when the mount is new or changed.
pub(crate) fn load_art(mounted: Option<Res<Mounted>>, images: Option<ResMut<Assets<Image>>>, mut art: ResMut<ShellArt>) {
    let (Some(mounted), Some(mut images)) = (mounted, images) else { return };
    if art.read && !mounted.is_changed() {
        return;
    }
    art.single_player = load(&mounted.0, SINGLE_PLAYER).map(|i| images.add(i));
    art.mission_list = load(&mounted.0, MISSION_LIST).map(|i| images.add(i));
    art.read = true;
}

/// Keep exactly one backdrop in the stock box of a campaign screen drawn under a stock profile that
/// has the picture; none anywhere else. The screens rebuild their boxes on any change, so this runs
/// every frame and puts the backdrop back first among the box's children.
#[allow(clippy::type_complexity)]
pub(crate) fn sync_backdrop(
    mut commands: Commands,
    screen: Res<State<Screen>>,
    profile: Option<Res<UiProfile>>,
    art: Res<ShellArt>,
    boxes: Query<(Entity, Option<&Children>), With<StockBoxNode>>,
    backdrops: Query<(Entity, &ShellBackdrop, &ImageNode)>,
) {
    let screen = *screen.get();
    let want = profile.as_deref().is_some_and(|p| p.is_stock()).then(|| art.backdrop(screen)).flatten();
    let mut have = false;
    for (e, mark, node) in &backdrops {
        if want.is_some_and(|w| mark.0 == screen && node.image == *w) && !have {
            have = true;
        } else {
            commands.entity(e).despawn();
        }
    }
    let (Some(image), false) = (want, have) else { return };
    let Ok((stock_box, children)) = boxes.single() else { return };
    // A backdrop left in the box by an earlier frame counts as present.
    if children.is_some_and(|c| c.iter().any(|k| backdrops.contains(k))) {
        return;
    }
    let backdrop = commands
        .spawn((
            ShellBackdrop(screen),
            Name::new("shell backdrop"),
            ImageNode { image: image.clone(), image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
            Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            Pickable::IGNORE,
        ))
        .id();
    commands.entity(stock_box).insert_children(0, &[backdrop]);
}
