//! The radar's fog and shroud: what the stock minimap draws over its map.
//!
//! Stock (`ui/HUD.md` of Refit, "Fog and shroud") keeps one 128 x 128 sprite per layer
//! and, for every hidden cell of the scan grid, copies a texel of the layer's pattern
//! texture into it: `Gshroud` for ground never in sight, `Gfog` for ground seen before
//! but not in sight now (`map_shroud`/`map_fog` in `gui_global.spr`; the file names
//! come from `minimapShroudSprite` and `minimapFogSprite`). The colours here are the
//! mounted mod's own texels (read from data: the mean of each texture), so every mod and
//! game is data; a texture the mount lacks falls back to a code constant. The
//! shroud is opaque; the fog lets the map show through at [`FOG_ALPHA`] (Refit's
//! measured match to stock).
//!
//! The minimap samples [`FogGrid`]'s distance fields with the same cubic filter as
//! the 3D fog, so the contours follow the sensor radii between the cell centres.

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::ui_render::UiMaterialPlugin;
use bevy::ui_render::ui_material::UiMaterial;
use sta_vfs::Vfs;

use super::radar::{RadarMap, RadarSquare, RadarState, StockRadar};
use crate::render::fog::{FogGrid, FogState, Layer};
use crate::viewer::Mounted;

/// How much of the map the fog hides (Refit draws it at 60%).
pub const FOG_ALPHA: f32 = 0.6;

/// Stock's own colours when the mounted data has no `Gfog`/`Gshroud`: Fleet Ops'.
pub const DEFAULT_FOG: [u8; 3] = [8, 8, 8];
pub const DEFAULT_SHROUD: [u8; 3] = [30, 36, 50];

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
    /// Read `Gfog` and `Gshroud` from the mounted data (mean texel of each); a missing
    /// texture keeps the Fleet Ops default.
    pub fn read(vfs: &Vfs) -> FogColours {
        let mean = |name: &str, default: [u8; 3]| -> [u8; 3] {
            read_pattern(vfs, name).and_then(|img| mean_colour(&img.pixels)).unwrap_or(default)
        };
        FogColours { fog: mean("Gfog", DEFAULT_FOG), shroud: mean("Gshroud", DEFAULT_SHROUD) }
    }
}

/// A layer's pattern texture (`Gfog`, `Gshroud`) from the mounted data, decoded; shared by
/// the radar's colours and the 3D view's textured layers.
pub fn read_pattern(vfs: &Vfs, name: &str) -> Option<sta_assets::Rgba> {
    let (path, fmt) = sta_assets::find_texture(vfs, name)?;
    sta_assets::decode_rgba(vfs, &path, fmt).ok()
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

/// A material over the radar square, using the 3D fog grid's masks and the radar's colours.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct RadarFogMaterial {
    #[uniform(0)]
    pub params: RadarFogParams,
    #[texture(1)]
    #[sampler(2)]
    mask: Handle<Image>,
}

#[derive(ShaderType, Debug, Clone, Copy, PartialEq)]
pub(super) struct RadarFogParams {
    fog: Vec4,
    shroud: Vec4,
    /// Transform the square's top-down UVs to the grid's bottom-up UVs.
    pub mask_uv: Vec4,
}

impl UiMaterial for RadarFogMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/ui/radar_fog.wesl").into_owned())
    }
}

impl RadarFogParams {
    fn new(grid: &FogGrid, map: &RadarMap, colours: &FogColours) -> Self {
        let linear = |[r, g, b]: [u8; 3], alpha| {
            let c = Color::srgb_u8(r, g, b).to_linear();
            Vec4::new(c.red, c.green, c.blue, alpha)
        };
        let extent = Vec2::new(grid.w as f32, grid.h as f32) * grid.cell;
        let offset = (map.to_world(Vec2::ZERO) - Vec2::from_array(grid.min)) / extent;
        let scale = Vec2::new(map.size, -map.size) / extent;
        Self {
            fog: linear(colours.fog, FOG_ALPHA),
            shroud: linear(colours.shroud, 1.0),
            mask_uv: Vec4::new(offset.x, offset.y, scale.x, scale.y),
        }
    }
}

fn mask_data(grid: &FogGrid) -> Vec<u8> {
    grid.mask(Layer::Fog)
        .into_iter()
        .zip(grid.mask(Layer::Shroud))
        .flat_map(|(fog, shroud)| [fog, shroud])
        .collect()
}

fn mask_image(grid: &FogGrid, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width: grid.w as u32, height: grid.h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rg8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    image
}

/// The node that carries the fog and shroud over the radar square.
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

/// Keep the radar's mask in sync with the player's fog grid.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn paint_fog(
    mut commands: Commands,
    fog: Option<Res<FogState>>,
    state: Res<RadarState>,
    colours: Res<FogColours>,
    images: Option<ResMut<Assets<Image>>>,
    mut materials: ResMut<Assets<RadarFogMaterial>>,
    square: Query<(Entity, Option<&Children>, Has<StockRadar>), With<RadarSquare>>,
    nodes: Query<&MaterialNode<RadarFogMaterial>, With<RadarFog>>,
    mut last: Local<Option<(RadarMap, FogColours)>>,
) {
    let (Some(fog), Some(map), Some(mut images), Ok((sq, kids, stock))) = (fog, state.map, images, square.single()) else { return };
    let key = (map, *colours);
    let existing = kids.and_then(|children| children.iter().find_map(|e| nodes.get(e).ok()));
    if existing.is_some() && !fog.is_changed() && *last == Some(key) {
        return;
    }
    *last = Some(key);
    let grid = &fog.grid;
    let params = RadarFogParams::new(grid, &map, &colours);
    let data = mask_data(grid);
    if let Some(node) = existing
        && let Some(material) = materials.get(&node.0)
        && let Some(mask) = images.get(&material.mask)
    {
        let size = mask.texture_descriptor.size;
        let mask_handle = material.mask.clone();
        let size_changed = size.width as usize != grid.w || size.height as usize != grid.h;
        let params_changed = material.params != params;
        if size_changed {
            let mask = images.add(mask_image(grid, data));
            *materials.get_mut(&node.0).unwrap() = RadarFogMaterial { params, mask };
        } else {
            if params_changed {
                materials.get_mut(&node.0).unwrap().params = params;
            }
            if mask.data.as_deref() != Some(data.as_slice()) {
                images.get_mut(&mask_handle).unwrap().data = Some(data);
            }
        }
        return;
    }

    let mask = images.add(mask_image(grid, data));
    let material = materials.add(RadarFogMaterial { params, mask });
    let node = commands
        .spawn((
            RadarFog,
            MaterialNode(material),
            Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            bevy::picking::Pickable::IGNORE,
        ))
        .id();
    // Stock draws fog over its dots. The other look keeps it as the first child under the dots.
    if stock {
        commands.entity(node).insert(ZIndex(1));
    }
    commands.entity(sq).insert_children(0, &[node]);
}

pub struct RadarFogPlugin;

impl Plugin for RadarFogPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<bevy::asset::io::embedded::EmbeddedAssetRegistry>() {
            embedded_asset!(app, "radar_fog.wesl");
        }
        if app.get_sub_app(bevy::render::RenderApp).is_some() {
            app.add_plugins(UiMaterialPlugin::<RadarFogMaterial>::default());
        } else {
            app.init_resource::<Assets<RadarFogMaterial>>();
        }
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
    fn the_mask_preserves_visible_explored_and_unexplored_cells() {
        let (g, _) = grid();
        let pixels = mask_data(&g);
        let at = |x: usize, z: usize| &pixels[(z * g.w + x) * 2..][..2];
        assert_eq!(at(0, 0), [0, 0], "visible");
        assert_eq!(at(3, 0), [255, 0], "explored");
        assert_eq!(at(0, 3), [255, 255], "unexplored");
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
        let handle = &app.world().get::<MaterialNode<RadarFogMaterial>>(first).unwrap().0;
        let material = app.world().resource::<Assets<RadarFogMaterial>>().get(handle).unwrap();
        assert_eq!(material.params.mask_uv, Vec4::new(0.0, 1.0, 1.0, -1.0));
        assert_eq!((material.params.fog.w, material.params.shroud.w), (FOG_ALPHA, 1.0));
        let image = app.world().resource::<Assets<Image>>().get(&material.mask).unwrap();
        assert_eq!(image.sampler, ImageSampler::linear());
        let data = image.data.as_ref().unwrap();
        assert_eq!(data[..2], [0, 0], "southwest is in sight");
        let north_west = 3 * 4 * 2;
        assert_eq!(data[north_west..north_west + 2], [255, 255], "northwest is shrouded");
    }

    /// Compare the binary and signed-distance masks in the same live material at about
    /// six display pixels per sensor cell, close to the reported stair steps.
    #[test]
    fn the_drawn_shroud_rounds_the_sensor_grid_on_the_gpu() {
        use bevy::camera::RenderTarget;
        use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
        use bevy::ui::UiTargetCamera;
        use std::sync::{Arc, Mutex};

        if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
            eprintln!("SKIP: set STA64_GPU_TESTS=1");
            return;
        }
        let _gpu = crate::bench::frames::gpu_serial();
        const SIDE: u32 = 320;
        let mut app = App::new();
        app.add_plugins(DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() })
            .set(bevy::render::RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::log::LogPlugin>());
        crate::app::configure_sets(&mut app);
        app.add_plugins(RadarFogPlugin);
        let map = RadarMap::of_bounds(Vec2::ZERO, Vec2::splat(6400.0));
        let mut state = RadarState::default();
        state.map = Some(map);
        let mut grid = FogGrid::new([0.0; 2], [6400.0; 2]);
        let mut binary = grid.clone();
        let centre = Vec2::new(24.3, 26.1);
        let radius = 9.2;
        for z in 0..binary.h {
            for x in 0..binary.w {
                if (Vec2::new(x as f32, z as f32) + Vec2::splat(0.5)).distance(centre) < radius {
                    binary.set_cell_for_test(x, z, true, true);
                }
            }
        }
        let centre_px = map.to_square(centre * grid.cell) * SIDE as f32;
        grid.set_sensor_for_test(centre * grid.cell, radius * grid.cell);
        let colours = FogColours { fog: [0; 3], shroud: [0; 3] };
        app.insert_resource(state)
            .insert_resource(FogState { grid, updates: 1 })
            .insert_resource(colours);
        let mut target = Image::new_target_texture(2 * SIDE, SIDE, TextureFormat::Rgba8UnormSrgb, None);
        target.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let (target, mask) = (images.add(target), images.add(mask_image(&binary, mask_data(&binary))));
        let old = app.world_mut().resource_mut::<Assets<RadarFogMaterial>>().add(RadarFogMaterial {
            params: RadarFogParams::new(&binary, &map, &colours),
            mask,
        });
        let camera = app.world_mut().spawn((
            Camera2d, Camera { clear_color: ClearColorConfig::Custom(Color::WHITE), ..default() },
            RenderTarget::from(target.clone()), Msaa::Off,
        )).id();
        let panel = Node { position_type: PositionType::Absolute, width: Val::Px(SIDE as f32), height: Val::Px(SIDE as f32), ..default() };
        app.world_mut().spawn((panel.clone(), UiTargetCamera(camera), MaterialNode(old)));
        app.world_mut().spawn((Node { left: Val::Px(SIDE as f32), ..panel }, UiTargetCamera(camera), RadarSquare, StockRadar));
        app.finish();
        app.cleanup();
        for _ in 0..16 {
            app.update();
        }
        let adapter = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()
            .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()))
            .expect("STA64_GPU_TESTS=1 needs an adapter");
        if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
            eprintln!("SKIP: hardware adapter unavailable ({})", adapter.name);
            return;
        }
        let frames = Arc::new(Mutex::new(None::<Image>));
        let out = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
            *out.lock().unwrap() = Some(shot.image.clone());
        });
        for _ in 0..60 {
            app.update();
            if frames.lock().unwrap().is_some() {
                break;
            }
        }
        let rgba = frames.lock().unwrap().take().expect("captured the fog material").try_into_dynamic().unwrap().to_rgba8();
        if let Some(dir) = std::env::var_os("STA64_UI_CAPTURE_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            rgba.save(dir.join("radar-shroud-comparison.png")).unwrap();
        }
        let red = |p: Vec2| f32::from(super::super::radar_sampling::sample_rgb(
            rgba.as_raw(), (2 * SIDE) as usize, SIDE as usize, p / Vec2::new((2 * SIDE) as f32, SIDE as f32),
        )[0]);
        let roughness = |offset| {
            let centre = centre_px + Vec2::new(offset, 0.0);
            assert_eq!(red(centre), 255.0, "the known visible centre stays clear");
            assert_eq!(red(centre + Vec2::splat(90.0)), 0.0, "distant unexplored cells stay opaque");
            let radii: Vec<_> = (0..128).map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 128.0;
                let direction = Vec2::from_angle(angle);
                let (mut lo, mut hi) = (40.0, 75.0);
                for _ in 0..11 {
                    let mid = (lo + hi) / 2.0;
                    if red(centre + direction * mid) >= 184.0 { lo = mid } else { hi = mid }
                }
                (lo + hi) / 2.0
            }).collect();
            (0..128).map(|i| (radii[(i + 1) % 128] - 2.0 * radii[i] + radii[(i + 127) % 128]).powi(2)).sum::<f32>() / 128.0
        };
        let old = roughness(0.0);
        let smooth = roughness(SIDE as f32);
        eprintln!("{} shroud edge roughness: binary {old:.3}, distance {smooth:.3}", adapter.name);
        assert!(smooth < 0.85 * old, "the grid outline should be smoother: {smooth:.3} vs {old:.3}");
    }

    /// Install-gated: the mounted mods' own textures give the colours.
    #[test]
    fn installs_give_their_own_colours() {
        let roots = sta_testkit::game_root("Star Trek Armada II Fleet Operations Roots");
        let colours = |m: Option<&str>| Vfs::mount_install(&roots, m).ok().map(|v| FogColours::read(&v));
        if let Some(c) = colours(Some("Fleet Ops 4.0")) {
            assert_eq!((c.fog, c.shroud), ([8, 8, 8], [30, 36, 50]));
        }
        if let Some(c) = colours(Some("STA2 Classic Mod")) {
            assert_eq!(c.shroud, [65, 71, 85]);
            assert_ne!(c.fog, [8, 8, 8], "Classic's fog is a pattern of its own");
        }
    }
}
