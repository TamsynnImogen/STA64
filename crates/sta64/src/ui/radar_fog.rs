//! The radar's fog and shroud: what the stock minimap draws over its map.
//!
//! Stock (`ui/HUD.md` of Refit, "Fog and shroud") keeps one 128 x 128 sprite per layer
//! and, for every hidden cell of the scan grid, copies a texel of the layer's pattern
//! texture into it: `Gshroud` for ground never in sight, `Gfog` for ground seen before
//! but not in sight now (`map_shroud`/`map_fog` in `gui_global.spr`; the file names
//! come from `minimapShroudSprite` and `minimapFogSprite`). Both are flat in Fleet Ops
//! (shroud 30,36,50, fog 8,8,8) and the Classic mod's fog is a dim teal pattern; the
//! colours here are the mounted mod's own texels, so every mod and game is data. The
//! shroud is opaque; the fog lets the map show through at [`FOG_ALPHA`] (Refit's
//! measured match to stock).
//!
//! The picture is built from [`FogGrid`] (the same grid the 3D view's fog uses), one
//! pixel per [`FOG_PIXELS`]th of the square, so it is blocky like stock's cells.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sta_vfs::Vfs;

use super::radar::{RadarMap, RadarSquare, RadarState};
use crate::render::fog::{FogGrid, FogState};
use crate::viewer::Mounted;

/// Pixels along the fog picture's side (stock's sprites are 128 x 128).
pub const FOG_PIXELS: usize = 128;
/// How much of the map the fog hides (Refit draws it at 60%).
pub const FOG_ALPHA: f32 = 0.6;

/// Stock's own colours when the mounted data has no `Gfog`/`Gshroud`: Fleet Ops'.
pub const DEFAULT_FOG: [u8; 3] = [8, 8, 8];
pub const DEFAULT_SHROUD: [u8; 3] = [30, 36, 50];

/// What a map point is to the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sight {
    /// In sight now: nothing drawn over it.
    Visible,
    /// Seen before, not in sight now: the fog.
    Explored,
    /// Never in sight (or off the grid): the shroud.
    Unexplored,
}

/// The state of a map point (x, z).
pub fn sight_at(grid: &FogGrid, x: f32, z: f32) -> Sight {
    if grid.visible_at(x, z) {
        Sight::Visible
    } else if grid.explored_at(x, z) {
        Sight::Explored
    } else {
        Sight::Unexplored
    }
}

/// The two layers' colours, as the mounted data has them.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct FogColours {
    pub fog: [u8; 3],
    pub shroud: [u8; 3],
}

impl Default for FogColours {
    fn default() -> Self {
        FogColours { fog: DEFAULT_FOG, shroud: DEFAULT_SHROUD }
    }
}

impl FogColours {
    /// RGBA of a state: transparent, the fog at [`FOG_ALPHA`], the opaque shroud.
    pub fn rgba(&self, s: Sight) -> [u8; 4] {
        match s {
            Sight::Visible => [0, 0, 0, 0],
            Sight::Explored => [self.fog[0], self.fog[1], self.fog[2], (FOG_ALPHA * 255.0).round() as u8],
            Sight::Unexplored => [self.shroud[0], self.shroud[1], self.shroud[2], 255],
        }
    }

    /// Read `Gfog` and `Gshroud` from the mounted data (mean texel of each); a missing
    /// texture keeps the Fleet Ops default.
    pub fn read(vfs: &Vfs) -> FogColours {
        let mean = |name: &str, default: [u8; 3]| -> [u8; 3] {
            let Some((path, fmt)) = sta_assets::find_texture(vfs, name) else { return default };
            let Ok(img) = sta_assets::decode_rgba(vfs, &path, fmt) else { return default };
            mean_colour(&img.pixels).unwrap_or(default)
        };
        FogColours { fog: mean("Gfog", DEFAULT_FOG), shroud: mean("Gshroud", DEFAULT_SHROUD) }
    }
}

/// The mean RGB of an RGBA8 buffer, ignoring fully transparent texels.
pub fn mean_colour(rgba: &[u8]) -> Option<[u8; 3]> {
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for p in rgba.chunks_exact(4).filter(|p| p[3] > 0) {
        for (s, c) in sum.iter_mut().zip(p) {
            *s += u64::from(*c);
        }
        n += 1;
    }
    (n > 0).then(|| [0, 1, 2].map(|i| (sum[i] / n) as u8))
}

/// The picture of the player's sight over the radar's map box: `side` x `side` RGBA
/// pixels, row 0 the top (north, the largest z).
pub fn fog_pixels(grid: &FogGrid, map: &RadarMap, colours: &FogColours, side: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(side * side * 4);
    for row in 0..side {
        for col in 0..side {
            let uv = Vec2::new((col as f32 + 0.5) / side as f32, (row as f32 + 0.5) / side as f32);
            let p = map.to_world(uv);
            out.extend_from_slice(&colours.rgba(sight_at(grid, p.x, p.y)));
        }
    }
    out
}

/// The node that carries the picture, under the blips.
#[derive(Component)]
pub struct RadarFog;

/// (Re)read the colours when the mounted install changes.
fn load_colours(vfs: Option<Res<Mounted>>, mut colours: ResMut<FogColours>) {
    if let Some(vfs) = vfs
        && vfs.is_changed()
    {
        *colours = FogColours::read(&vfs.0);
    }
}

/// Keep the picture current: rebuilt when the sight, the map's box or the colours change.
#[allow(clippy::too_many_arguments)]
fn paint_fog(
    mut commands: Commands,
    fog: Option<Res<FogState>>,
    state: Res<RadarState>,
    colours: Res<FogColours>,
    images: Option<ResMut<Assets<Image>>>,
    square: Query<(Entity, Option<&Children>), With<RadarSquare>>,
    mut nodes: Query<&mut ImageNode, With<RadarFog>>,
    mut last: Local<Option<(u32, RadarMap, FogColours)>>,
) {
    let (Some(fog), Some(map), Some(mut images), Ok((sq, kids))) = (fog, state.map, images, square.single()) else { return };
    let key = (fog.updates, map, *colours);
    let existing = kids.and_then(|k| k.iter().find(|e| nodes.contains(*e)));
    if existing.is_some() && *last == Some(key) {
        return;
    }
    *last = Some(key);
    let mut img = Image::new(
        Extent3d { width: FOG_PIXELS as u32, height: FOG_PIXELS as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        fog_pixels(&fog.grid, &map, &colours, FOG_PIXELS),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::nearest(); // cells stay square, as stock's
    let handle = images.add(img);
    match existing {
        Some(e) => {
            if let Ok(mut node) = nodes.get_mut(e) {
                node.image = handle;
            }
        }
        None => {
            let e = commands
                .spawn((
                    RadarFog,
                    ImageNode::new(handle),
                    Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                    bevy::picking::Pickable::IGNORE,
                ))
                .id();
            // First child: under every blip and the camera's outline.
            commands.entity(sq).insert_children(0, &[e]);
        }
    }
}

pub struct RadarFogPlugin;

impl Plugin for RadarFogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FogColours>().add_systems(
            Update,
            (load_colours, paint_fog).chain().after(super::radar::refresh).in_set(crate::app::GameSet::Ui),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A 4 x 4 cell grid over (0,0)..(4*CELL, 4*CELL): the lower left quadrant visible,
    /// the lower right explored, the rest never seen.
    fn grid() -> (FogGrid, f32) {
        let c = crate::render::fog::CELL;
        let mut g = FogGrid::new([0.0, 0.0], [4.0 * c, 4.0 * c]);
        for cz in 0..2 {
            for cx in 0..4 {
                g.set_cell_for_test(cx, cz, cx < 2, true);
            }
        }
        (g, c)
    }

    #[test]
    fn sight_follows_the_grid() {
        let (g, c) = grid();
        assert_eq!(sight_at(&g, 0.5 * c, 0.5 * c), Sight::Visible);
        assert_eq!(sight_at(&g, 3.5 * c, 0.5 * c), Sight::Explored);
        assert_eq!(sight_at(&g, 0.5 * c, 3.5 * c), Sight::Unexplored);
        assert_eq!(sight_at(&g, -10.0, 0.0), Sight::Unexplored, "off the grid is shroud");
    }

    #[test]
    fn states_map_to_stock_colours() {
        let fo = FogColours::default();
        assert_eq!(fo.rgba(Sight::Visible)[3], 0);
        assert_eq!(fo.rgba(Sight::Unexplored), [30, 36, 50, 255], "Gshroud.tga of Fleet Ops, opaque");
        assert_eq!(fo.rgba(Sight::Explored), [8, 8, 8, 153], "Gfog.tga of Fleet Ops at 60%");
    }

    /// The picture's rows: north (large z) on top, so the lower-left quadrant of the map
    /// is the picture's lower left.
    #[test]
    fn picture_puts_each_state_in_its_place() {
        let (g, c) = grid();
        let map = RadarMap::of_bounds(Vec2::ZERO, Vec2::splat(4.0 * c));
        let col = FogColours::default();
        let px = fog_pixels(&g, &map, &col, 8);
        let at = |x: usize, y: usize| <[u8; 4]>::try_from(&px[(y * 8 + x) * 4..][..4]).unwrap();
        assert_eq!(at(1, 6), col.rgba(Sight::Visible));
        assert_eq!(at(6, 6), col.rgba(Sight::Explored));
        assert_eq!(at(1, 1), col.rgba(Sight::Unexplored));
        assert_eq!(at(6, 1), col.rgba(Sight::Unexplored));
    }

    #[test]
    fn mean_ignores_transparent_texels() {
        let px = [10, 20, 30, 255, 30, 40, 50, 255, 200, 200, 200, 0];
        assert_eq!(mean_colour(&px), Some([20, 30, 40]));
        assert_eq!(mean_colour(&[1, 2, 3, 0]), None);
    }

    /// The picture reaches the square as its first child and follows the grid.
    #[test]
    fn the_square_gets_a_fog_picture_under_the_blips() {
        let (g, c) = grid();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Assets<Image>>();
        crate::app::configure_sets(&mut app);
        app.add_plugins(super::super::radar::RadarPlugin);
        app.insert_resource(FogState { grid: g, updates: 1 });
        app.update();
        app.world_mut().resource_mut::<RadarState>().map = Some(RadarMap::of_bounds(Vec2::ZERO, Vec2::splat(4.0 * c)));
        app.update();
        let sq = app.world_mut().query_filtered::<Entity, With<RadarSquare>>().single(app.world()).unwrap();
        let first = app.world().get::<Children>(sq).unwrap()[0];
        assert!(app.world().get::<RadarFog>(first).is_some());
        let handle = app.world().get::<ImageNode>(first).unwrap().image.clone();
        let data = app.world().resource::<Assets<Image>>().get(&handle).unwrap().data.clone().unwrap();
        assert_eq!(data[3], 255, "top left is shroud");
        let bottom_left = ((FOG_PIXELS - 2) * FOG_PIXELS + 1) * 4;
        assert_eq!(data[bottom_left + 3], 0, "bottom left is in sight");
    }

    /// Install-gated: the mounted mods' own textures give the colours.
    #[test]
    fn installs_give_their_own_colours() {
        let roots = Path::new("/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots");
        let colours = |m: Option<&str>| Vfs::mount_install(roots, m).ok().map(|v| FogColours::read(&v));
        if let Some(c) = colours(Some("Fleet Ops 4.0")) {
            assert_eq!((c.fog, c.shroud), ([8, 8, 8], [30, 36, 50]));
        }
        if let Some(c) = colours(Some("STA2 Classic Mod")) {
            assert_eq!(c.shroud, [65, 71, 85]);
            assert_ne!(c.fog, [8, 8, 8], "Classic's fog is a pattern of its own");
        }
    }
}
