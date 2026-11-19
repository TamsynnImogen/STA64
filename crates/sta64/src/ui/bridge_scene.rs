//! Armada I's bridge scene: where a mission's `PlayBridgeMovie` clip plays.
//!
//! **Rule.** The original has two movie calls on one video object: `PlayBridgeMovie` draws into the
//! bridge display's screen rectangle, `PlayCinematicMovie` into the HUD's cinematic panel (read
//! from the executable, static). Every stock Armada I mission script uses only the first (read from
//! the missions' import tables), and the bridge display lives in the cineractive view, which has no
//! HUD. STA64 therefore plays an Armada I mission's in-panel movie here ([`BridgeScreen`]) and
//! leaves the HUD's panel to Armada II.
//!
//! **Layout** (read from the executable, static, unless said): the scene is a 640x480 picture, the
//! race's `<p>bridge_panel` sprite (`<p>bridgeBackground`, `<p>bridgeBackgroundSize` and
//! `<p>bridgeBackground_<n>` of the gui cfg; `<p>` is `f_` `k_` `r_` `b_` by the bridge type of
//! `SetBridgeSceneRace`, `d_` before one is set), and the clip is stretched to a fixed 400x200
//! rectangle of it, per race ([`movie_rect`]; no cfg key places it). The art has a magenta window
//! where the screen is and is drawn over the clip, so the clip shows through it (draw order read
//! from the executable; that the engine keys magenta is **not measured**, the sprite header says
//! `@skip=off`). The 640x480 group keeps its aspect and is centred as a whole on other window
//! aspects (the stock look rule, ADR 0008), black around it (what the original draws in the bars is
//! **not measured**). The animated `<p>bridgeLayer<n>` lights are not drawn yet (TODO.md).
//!
//! **Fallback.** No frame art (a mod without the sprites or the cfg keys, no install): the clip is
//! drawn on black at its own aspect over the whole window, and the reason is logged once.
//!
//! The frame is shown only while [`BridgeState::shown`] (`ShowBridgeScene`); without it the clip
//! plays in its rectangle on black. The overlay covers the HUD and takes the pointer, as the
//! cineractive view has no HUD panel to receive it.

use std::collections::HashMap;

use bevy::image::CompressedImageFormats;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;
use sta_assets::a2fmt::spr::Skip;

use super::gui_sprites::{self, GuiImageSampling, GuiSprites};
use super::hud::HudConfig;
use super::hud::layout::R;
use super::hud::stock::layout::{StockLayout, fit_canvas};
use super::presentation::BridgeState;
use crate::app::GameSet;
use crate::media::{BridgeScreen, Movies};
use crate::sim_link::ContentRes;
use crate::viewer::Mounted;

/// The scene's own space: the stock gui cfg's bridge rectangles are in it.
pub const CANVAS: Vec2 = Vec2::new(640.0, 480.0);

/// The pixel colour of the art's screen window (read from the stock tiles; the key is only
/// applied to this art).
const WINDOW_KEY: [u8; 3] = [255, 0, 255];

/// The cfg key prefix of a bridge type: 1 Federation, 2 Klingon, 3 Romulan, 4 Borg (the stock scripts'
/// numbers), the default scene for any other.
pub fn prefix(race: Option<i32>) -> &'static str {
    match race {
        Some(1) => "f_",
        Some(2) => "k_",
        Some(3) => "r_",
        Some(4) => "b_",
        _ => "d_",
    }
}

/// Where the clip goes in the 640x480 scene (`x y w h`), per bridge type; fixed in the executable
/// (read from the executable, static), not in any data file. TODO: a cfg key for mods.
pub fn movie_rect(race: Option<i32>) -> [f32; 4] {
    match race {
        Some(1) => [220.0, 10.0, 400.0, 200.0],
        Some(2) => [154.0, 128.0, 400.0, 200.0],
        Some(3) => [84.0, 148.0, 400.0, 200.0],
        Some(4) => [184.0, 126.0, 400.0, 200.0],
        _ => [62.0, 110.0, 400.0, 200.0],
    }
}

/// The scene on a window: the 640x480 group (centred, aspect kept) and the clip's rectangle in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeLayout {
    pub group: R,
    pub movie: R,
}

pub fn layout(window: Vec2, race: Option<i32>) -> BridgeLayout {
    let (scale, group) = fit_canvas(CANVAS, window);
    let [x, y, w, h] = movie_rect(race);
    BridgeLayout { group, movie: R::new(group.x + x * scale, group.y + y * scale, w * scale, h * scale) }
}

/// The largest rectangle of the picture's aspect that fits the window, centred.
fn fit_window(window: Vec2, picture: (u32, u32)) -> R {
    let (pw, ph) = (picture.0 as f32, picture.1 as f32);
    if pw <= 0.0 || ph <= 0.0 || window.x <= 0.0 || window.y <= 0.0 {
        return R::new(0.0, 0.0, window.x, window.y);
    }
    let scale = (window.x / pw).min(window.y / ph);
    let (w, h) = (pw * scale, ph * scale);
    R::new((window.x - w) / 2.0, (window.y - h) / 2.0, w, h)
}

type Art = Vec<(usize, Handle<Image>, Rect)>;

/// The frame art of each bridge type, loaded once; `None` when it cannot be had (logged once per type).
#[derive(Resource, Default)]
pub struct BridgeArt {
    by_prefix: HashMap<&'static str, Option<Art>>,
    cache: GuiSprites,
}

impl BridgeArt {
    fn get(&mut self, prefix: &'static str, gui: Option<&sta_content::GuiLayout>, vfs: Option<&sta_vfs::Vfs>, images: &mut Assets<Image>) -> Option<Art> {
        if !self.by_prefix.contains_key(prefix) {
            let loaded = vfs.ok_or_else(|| "no install is mounted".to_owned()).and_then(|v| load_art(prefix, gui, v, images, &mut self.cache));
            if let Err(why) = &loaded {
                warn!("bridge scene: {why}; the movie plays full screen on black");
            }
            self.by_prefix.insert(prefix, loaded.ok());
        }
        self.by_prefix[prefix].clone()
    }
}

/// The overlay on screen and what it was built from.
#[derive(Resource, Default)]
pub struct BridgeOverlay {
    entity: Option<Entity>,
    built: Option<Built>,
}

/// How the clip is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// In its rectangle of the scene, the frame art over it.
    Framed,
    /// `ShowBridgeScene` is off: in its rectangle, black around.
    Bare,
    /// No frame art: the whole window.
    Fullscreen,
}

/// The mode for the scene's state and whether its art could be had.
pub fn mode(shown: bool, art: bool) -> Mode {
    match (shown, art) {
        (true, true) => Mode::Framed,
        (false, _) => Mode::Bare,
        (true, false) => Mode::Fullscreen,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Built {
    window: (u32, u32),
    race: Option<i32>,
    mode: Mode,
}

/// The clip's picture when there is no frame: fitted to the window as the picture's size is known.
#[derive(Component)]
struct FallbackPicture;

/// The screen the mission's clip is written to exists while an Armada I game runs.
pub fn provide_screen(
    mut commands: Commands,
    content: Option<Res<ContentRes>>,
    images: Option<ResMut<Assets<Image>>>,
    screen: Option<Res<BridgeScreen>>,
) {
    let a1 = content.is_some_and(|c| c.0.profile() == sta_content::Profile::Sta1Classic);
    match (a1, screen.is_some(), images) {
        (true, false, Some(mut images)) => {
            let blank = Image::new_fill(
                Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                TextureDimension::D2,
                &[0, 0, 0, 255],
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            );
            commands.insert_resource(BridgeScreen { image: images.add(blank) });
        }
        (false, true, _) => commands.remove_resource::<BridgeScreen>(),
        _ => {}
    }
}

/// The art of one bridge type: every `<p>bridgeBackground_<n>` piece of the cfg cropped from its
/// sprite `<name>.<n>` of `gui_global.spr`, with the window colour keyed out. `None` (and a log
/// line) when the cfg has no pieces, the table lacks a sprite or a texture is missing.
fn load_art(
    prefix: &str,
    gui: Option<&sta_content::GuiLayout>,
    vfs: &sta_vfs::Vfs,
    images: &mut Assets<Image>,
    cache: &mut GuiSprites,
) -> Result<Art, String> {
    let gui = gui.ok_or("no gui cfg")?;
    let pieces = StockLayout::new(gui, CANVAS).pieces(&format!("{prefix}bridgeBackground"));
    if pieces.is_empty() {
        return Err(format!("the gui cfg has no `{prefix}bridgeBackground` pieces"));
    }
    let table = gui_sprites::read_table(vfs, "sprites/gui_global.spr");
    let mut out = Vec::new();
    for piece in pieces {
        let name = format!("{}.{}", piece.name, piece.frame);
        let sprite = table.sprite(&name).ok_or_else(|| format!("sprite `{name}` is not in gui_global.spr"))?;
        let mut keyed = sprite.clone();
        keyed.skip = Skip::Colour(WINDOW_KEY);
        let (handle, size) = gui_sprites::page(&keyed, GuiImageSampling::Nearest, vfs, cache, images, CompressedImageFormats::empty())
            .ok_or_else(|| format!("texture of `{name}` is missing"))?;
        let crop = gui_sprites::crop(&keyed, size).ok_or_else(|| format!("sprite `{name}` has no usable rectangle"))?;
        out.push((piece.frame, handle, crop));
    }
    Ok(out)
}

fn absolute(r: R, window: Vec2) -> Node {
    // Fractions of the window, so the scene does not depend on the UI scale.
    Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(r.x / window.x * 100.0),
        top: Val::Percent(r.y / window.y * 100.0),
        width: Val::Percent(r.w / window.x * 100.0),
        height: Val::Percent(r.h / window.y * 100.0),
        ..default()
    }
}

/// Show the scene while a clip plays on the bridge screen, and remove it after.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    mut commands: Commands,
    screen: Option<Res<BridgeScreen>>,
    movies: Option<Res<Movies>>,
    state: Res<BridgeState>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    hud: Option<Res<HudConfig>>,
    mounted: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
    mut art: ResMut<BridgeArt>,
    mut overlay: ResMut<BridgeOverlay>,
) {
    let busy = screen.as_ref().zip(movies.as_ref()).is_some_and(|(s, m)| m.panel_busy_for(&s.image));
    if !busy {
        if let Some(e) = overlay.entity.take() {
            commands.entity(e).despawn();
        }
        overlay.built = None;
        return;
    }
    let (Some(screen), Some(window), Some(mut images)) = (screen, window, images) else { return };
    let size = Vec2::new(window.width(), window.height());
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let race = state.race;
    let key = prefix(race);
    let gui = hud.as_ref().and_then(|h| h.gui.as_ref());
    let pieces = if state.shown { art.get(key, gui, mounted.as_ref().map(|m| &m.0), &mut images) } else { None };
    let mode = mode(state.shown, pieces.is_some());
    let built = Built { window: (size.x as u32, size.y as u32), race, mode };
    let alive = overlay.entity.is_some_and(|e| commands.get_entity(e).is_ok());
    if alive && overlay.built.as_ref() == Some(&built) {
        return;
    }
    if let Some(e) = overlay.entity.take() {
        commands.entity(e).despawn();
    }
    let l = layout(size, race);
    let root = commands
        .spawn((
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            BackgroundColor(Color::BLACK),
            // Below the full-screen movies' overlay, above the HUD and the notices.
            GlobalZIndex(i32::MAX - 8),
            FocusPolicy::Block,
            Pickable { should_block_lower: true, is_hoverable: true },
        ))
        .id();
    match mode {
        Mode::Fullscreen => {
            commands.spawn((FallbackPicture, absolute(R::new(0.0, 0.0, size.x, size.y), size), ImageNode::new(screen.image.clone()), Pickable::IGNORE, ChildOf(root)));
        }
        Mode::Bare | Mode::Framed => {
            commands.spawn((absolute(l.movie, size), ImageNode::new(screen.image.clone()), Pickable::IGNORE, ChildOf(root)));
        }
    }
    if let (Some(pieces), Some(gui)) = (pieces, gui) {
        for p in StockLayout::new(gui, size).pieces(&format!("{key}bridgeBackground")) {
            if let Some((_, image, crop)) = pieces.iter().find(|(f, _, _)| *f == p.frame) {
                commands.spawn((absolute(p.rect, size), ImageNode { image: image.clone(), rect: Some(*crop), ..default() }, Pickable::IGNORE, ChildOf(root)));
            }
        }
    }
    overlay.entity = Some(root);
    overlay.built = Some(built);
}

/// Fit the fallback picture to the window at its own aspect, as its size becomes known.
fn fit_fallback(
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    screen: Option<Res<BridgeScreen>>,
    images: Option<Res<Assets<Image>>>,
    mut pictures: Query<&mut Node, With<FallbackPicture>>,
) {
    let (Some(window), Some(screen), Some(images)) = (window, screen, images) else { return };
    let Some(img) = images.get(&screen.image) else { return };
    let size = Vec2::new(window.width(), window.height());
    let next = absolute(fit_window(size, (img.width(), img.height())), size);
    for mut node in &mut pictures {
        if node.left != next.left || node.top != next.top || node.width != next.width || node.height != next.height {
            node.left = next.left;
            node.top = next.top;
            node.width = next.width;
            node.height = next.height;
        }
    }
}

pub struct BridgeScenePlugin;

impl Plugin for BridgeScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BridgeState>()
            .init_resource::<BridgeArt>()
            .init_resource::<BridgeOverlay>()
            .add_systems(Update, (provide_screen, drive, fit_fallback).chain().in_set(GameSet::Ui));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::media::mission::target_for;
    use crate::media::{MovieId, MovieTarget, PanelKind, PanelScreen, PlayMovie};

    fn gui(text: &str) -> sta_content::GuiLayout {
        let (cfg, warnings) = sta_content::GuiCfg::parse("synthetic.cfg", text);
        assert!(warnings.is_empty(), "{warnings:?}");
        cfg.layout((640, 480))
    }

    /// Authored cfg text: two pieces for the Federation prefix (numbers match no stock sheet).
    const CFG: &str = "\
f_bridgeBackground = \"f_bridge_panel\"
f_bridgeBackgroundSize = 2
f_bridgeBackground_0 = 0 0 320 480
f_bridgeBackground_1 = 320 0 320 480
";

    #[test]
    fn the_scene_fills_a_4_by_3_window_and_the_clip_scales_with_it() {
        let l = layout(Vec2::new(1280.0, 960.0), Some(1));
        assert_eq!(l.group, R::new(0.0, 0.0, 1280.0, 960.0));
        let [x, y, w, h] = movie_rect(Some(1));
        assert_eq!(l.movie, R::new(x * 2.0, y * 2.0, w * 2.0, h * 2.0));
        let native = layout(Vec2::new(640.0, 480.0), Some(1));
        assert_eq!(native.movie, R::new(x, y, w, h));
    }

    #[test]
    fn on_a_wide_window_the_group_keeps_its_aspect_and_is_centred_as_a_whole() {
        let l = layout(Vec2::new(1920.0, 1080.0), Some(3));
        assert_eq!(l.group.h, 1080.0);
        assert!((l.group.w / l.group.h - 4.0 / 3.0).abs() < 1e-4, "{:?}", l.group);
        assert_eq!(l.group.x, (1920.0 - l.group.w) / 2.0);
        // The clip keeps its place inside the group and its own aspect, whatever the window.
        let [x, y, w, h] = movie_rect(Some(3));
        let s = l.group.h / 480.0;
        assert_eq!(l.movie, R::new(l.group.x + x * s, y * s, w * s, h * s));
        assert!(l.movie.x >= l.group.x && l.movie.right() <= l.group.right() && l.movie.bottom() <= l.group.bottom());
        assert!((l.movie.w / l.movie.h - w / h).abs() < 1e-4);
        // A narrow window scales by the width and centres vertically.
        let tall = layout(Vec2::new(640.0, 960.0), None);
        assert_eq!((tall.group.w, tall.group.h, tall.group.y), (640.0, 480.0, 240.0));
    }

    #[test]
    fn every_bridge_type_has_a_clip_rectangle_inside_the_scene_and_its_own_prefix() {
        let mut seen = std::collections::BTreeSet::new();
        for race in [None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(9)] {
            let [x, y, w, h] = movie_rect(race);
            assert!(x >= 0.0 && y >= 0.0 && x + w <= CANVAS.x && y + h <= CANVAS.y, "{race:?}");
            seen.insert(prefix(race));
        }
        assert_eq!(seen.len(), 5);
        assert_eq!(prefix(Some(7)), prefix(None), "an unknown type takes the default scene");
    }

    #[test]
    fn an_armada_i_bridge_movie_goes_to_the_bridge_screen_and_other_movies_keep_their_target() {
        let mut images = Assets::<Image>::default();
        let bridge = BridgeScreen { image: images.add(Image::default()) };
        let panel = PanelScreen { image: images.add(Image::default()), kind: PanelKind::Cinematic };
        // Armada I: the HUD's panel is not used for the call.
        assert!(matches!(target_for(true, Some(&bridge), Some(&panel)), MovieTarget::Panel(h) if h.id() == bridge.image.id()));
        assert!(matches!(target_for(true, Some(&bridge), None), MovieTarget::Panel(h) if h.id() == bridge.image.id()));
        // Armada II: the HUD's panel, or full screen when it has none.
        assert!(matches!(target_for(true, None, Some(&panel)), MovieTarget::Panel(h) if h.id() == panel.image.id()));
        assert!(matches!(target_for(true, None, None), MovieTarget::Fullscreen));
        // A full-screen request stays full screen (the bridge screen is for the bridge call only).
        assert!(matches!(target_for(false, Some(&bridge), Some(&panel)), MovieTarget::Fullscreen));
    }

    #[test]
    fn the_mode_follows_the_scene_flag_and_the_art() {
        assert_eq!(mode(true, true), Mode::Framed);
        assert_eq!(mode(true, false), Mode::Fullscreen);
        assert_eq!(mode(false, true), Mode::Bare);
        assert_eq!(mode(false, false), Mode::Bare);
    }

    fn mount(files: &[(&str, Vec<u8>)]) -> sta_vfs::Vfs {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("sta64-bridge-scene-{}-{nonce}", std::process::id()));
        let data = root.join("Data");
        for (name, bytes) in files {
            let path = data.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fs::create_dir_all(&data).unwrap();
        sta_vfs::Vfs::mount_install(&root, None).unwrap()
    }

    /// A 2x1 true-colour TGA: one magenta pixel (the window) and one grey.
    fn tga() -> Vec<u8> {
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12] = 2;
        bytes[14] = 1;
        bytes[16] = 24;
        bytes[17] = 0x20;
        bytes.extend_from_slice(&[255, 0, 255, 90, 90, 90]);
        bytes
    }

    #[test]
    fn the_frame_art_is_the_cfg_pieces_cropped_from_the_table_with_the_window_keyed_out() {
        let spr = b"f_bridge_panel.0 tile 0 0 1 1\nf_bridge_panel.1 tile 1 0 1 1\n".to_vec();
        let vfs = mount(&[("sprites/gui_global.spr", spr), ("textures/rgb/tile.tga", tga())]);
        let mut images = Assets::<Image>::default();
        let mut cache = GuiSprites::default();
        let layout = gui(CFG);
        let art = load_art("f_", Some(&layout), &vfs, &mut images, &mut cache).expect("art");
        assert_eq!(art.iter().map(|a| a.0).collect::<Vec<_>>(), [0, 1]);
        let (_, handle, _) = &art[0];
        let img = images.get(handle).unwrap();
        let data = img.data.as_ref().unwrap();
        assert_eq!(data[3], 0, "the magenta pixel is transparent");
        assert_eq!(data[7], 255, "the other pixel is opaque");
    }

    #[test]
    fn missing_pieces_sprites_or_textures_give_no_art_and_say_why() {
        let layout = gui(CFG);
        let mut images = Assets::<Image>::default();
        let mut cache = GuiSprites::default();
        let no_table = mount(&[]);
        let why = load_art("f_", Some(&layout), &no_table, &mut images, &mut cache).unwrap_err();
        assert!(why.contains("f_bridge_panel.0"), "{why}");
        let no_texture = mount(&[("sprites/gui_global.spr", b"f_bridge_panel.0 tile 0 0 1 1\nf_bridge_panel.1 tile 1 0 1 1\n".to_vec())]);
        let why = load_art("f_", Some(&layout), &no_texture, &mut images, &mut cache).unwrap_err();
        assert!(why.contains("texture"), "{why}");
        // Another bridge type's keys are not in the cfg; no cfg at all.
        assert!(load_art("k_", Some(&layout), &no_table, &mut images, &mut cache).unwrap_err().contains("no `k_bridgeBackground`"));
        assert!(load_art("f_", None, &no_table, &mut images, &mut cache).is_err());
    }

    fn app(shown: bool, race: Option<i32>) -> (App, Handle<Image>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Assets<Image>>().init_resource::<Movies>().init_resource::<BridgeArt>().init_resource::<BridgeOverlay>();
        app.insert_resource(BridgeState { race, shown, ..default() });
        app.world_mut().spawn((Window { resolution: (1920u32, 1080u32).into(), ..default() }, PrimaryWindow));
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(Image::default());
        app.insert_resource(BridgeScreen { image: handle.clone() });
        app.add_systems(Update, (drive, fit_fallback).chain());
        (app, handle)
    }

    fn play(app: &mut App, image: &Handle<Image>) {
        app.world_mut().resource_mut::<Movies>().queue_for_test(PlayMovie { id: MovieId(1), name: "x".into(), target: MovieTarget::Panel(image.clone()), skippable: false });
        app.update();
    }

    fn overlay_nodes(app: &mut App) -> Vec<Node> {
        let entity = app.world().resource::<BridgeOverlay>().entity;
        let Some(root) = entity else { return Vec::new() };
        let mut q = app.world_mut().query::<(&ChildOf, &Node)>();
        q.iter(app.world()).filter(|(c, _)| c.parent() == root).map(|(_, n)| n.clone()).collect()
    }

    #[test]
    fn without_frame_art_the_clip_plays_full_screen_on_black_and_the_overlay_goes_with_the_clip() {
        let (mut app, image) = app(true, Some(1));
        app.update();
        assert!(app.world().resource::<BridgeOverlay>().entity.is_none(), "nothing without a clip");
        play(&mut app, &image);
        // No install, no cfg: the fallback is one picture over the whole window.
        assert_eq!(app.world().resource::<BridgeOverlay>().built.as_ref().map(|b| b.mode), Some(Mode::Fullscreen));
        assert_eq!(overlay_nodes(&mut app).len(), 1);
        // The clip ends: the overlay is gone.
        app.world_mut().resource_mut::<Movies>().stop_for_test();
        app.update();
        assert!(app.world().resource::<BridgeOverlay>().entity.is_none());
    }

    #[test]
    fn with_the_scene_off_the_clip_plays_in_its_rectangle_on_black() {
        let (mut app, image) = app(false, Some(1));
        play(&mut app, &image);
        assert_eq!(app.world().resource::<BridgeOverlay>().built.as_ref().map(|b| b.mode), Some(Mode::Bare));
        let nodes = overlay_nodes(&mut app);
        assert_eq!(nodes.len(), 1);
        let l = layout(Vec2::new(1920.0, 1080.0), Some(1));
        let Val::Percent(left) = nodes[0].left else { panic!("{:?}", nodes[0].left) };
        assert!((left - l.movie.x / 1920.0 * 100.0).abs() < 1e-3);
    }
}
