//! UI-43: the install's own mouse cursors. `sprites/cursor.spr` is read through the mounted
//! VFS (so a mod's own table wins by the normal layering); each cursor is a sprite whose
//! `@origin` is the hotspot and whose `@anim` names a flipbook animation. The cursor is drawn
//! as a UI image at the pointer in place of the OS cursor; with no cursor sprites (no install,
//! no table, no such sprite or texture) the OS cursor stays.
//!
//! What cursor to show is not decided here: [`WantedCursor`] carries a sprite name, set by
//! whichever system decides (today `input/context.rs`, from the same decision a right-click
//! uses). Client-side presentation only.
//!
//! Race variants (UI-45, read from data): Fleet Operations and Roots ship `<cursor>_<race>`
//! sprites whose suffix is a race's `name` key (`_UnitedFederation` for `unitedfederation`,
//! `_KlingonEmpire`, …; the table's lookup ignores case). The local player's race variant is used
//! where the table has it, else the plain sprite; stock tables have none and are unchanged.
//!
//! Rules (read from data; see the package notes outside the repository): the hotspot is in
//! texture pixels; a cell's size on screen is its texture size times the size factors of
//! [`size_factors`] (the Remaster look: one), so a mod that ships a larger texture under the same
//! sprite names gets a larger cursor; a flipbook plays over the animation's length.

use std::collections::HashMap;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use sta_assets::a2fmt::spr::{Animation, Auto, Field, Sprite, SpriteTable};

use super::gui_sprites::{GuiImageSampling, GuiSprites, crop, page};
use super::hud::stock::layout::{HudFit, StockLayout};
use crate::viewer::Mounted;

/// The sprite table the cursors are read from: cursor sprites are resolved against the whole sprite
/// table rooted at `sprites.spr` and its includes (highest mod layer first), as every other sprite
/// is, so the flipbooks a sprite names are found (read from data).
pub(crate) fn load_table(vfs: &sta_vfs::Vfs) -> SpriteTable {
    sta_assets::load_sprite_table(vfs)
}

/// The sprite name of the plain pointer.
pub const DEFAULT: &str = "standard_cursor";

/// The cursor sprite wanted this frame. Reset to [`DEFAULT`] at the start of every frame;
/// a system that knows better sets it before the cursor is placed.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct WantedCursor(pub &'static str);

impl Default for WantedCursor {
    fn default() -> Self {
        Self(DEFAULT)
    }
}

/// One cursor, ready to draw.
#[derive(Clone, Debug)]
pub(crate) struct CursorArt {
    image: Handle<Image>,
    /// Texture-pixel rectangle of each frame (one for a static cursor).
    frames: Vec<Rect>,
    /// The hotspot in texture pixels, from the cell's top-left.
    origin: Vec2,
    /// Seconds one pass over `frames` takes.
    period: f32,
}

#[derive(Resource, Default)]
struct CursorSprites {
    table: Option<SpriteTable>,
    cache: GuiSprites,
    resolved: HashMap<String, Option<CursorArt>>,
    loaded: bool,
}

#[derive(Component)]
struct CursorNode;

/// Is the camera being dragged? The drawn cursor is hidden for as long as it is (rule: grade read
/// from the executable, static, for the original's own camera buttons; INTERIM: STA64 applies it
/// to its own camera buttons, `OrbitButtons`, which are not the original's; with pan bound to the
/// right button, as in the viewer, the cursor is hidden while a right-click order's button is held).
/// While the window is not focused no drag is assumed. The engine does not release mouse buttons on
/// a focus loss, so a button held at that moment can still read as pressed on refocus (when the
/// release was lost); [`DragGuard`] then ignores it until it has been released or freshly pressed.
pub(crate) fn camera_drag(buttons: &ButtonInput<MouseButton>, orbit: &crate::viewer::OrbitButtons, focused: bool, guard: &mut DragGuard) -> bool {
    if !focused {
        guard.stale = [true, true];
        return false;
    }
    let mut drag = false;
    for (i, b) in [Some(orbit.rotate), orbit.pan].into_iter().enumerate() {
        let Some(b) = b else { continue };
        if guard.stale[i] && (buttons.just_pressed(b) || !buttons.pressed(b)) {
            guard.stale[i] = false;
        }
        drag |= buttons.pressed(b) && !guard.stale[i];
    }
    drag
}

/// Which drag buttons were held across a focus loss and have not been seen released since.
#[derive(Default, Debug)]
pub(crate) struct DragGuard {
    stale: [bool; 2],
}

/// The frames of a cursor sprite in texture pixels, its hotspot, and the period. `anim` is the
/// flipbook the sprite names, when the table has it: an `offset` animation generated `@auto=row`
/// (or column or square) moves the rectangle by its offsets, in 0..1 texture units. Anything else
/// leaves the cursor still. `None` when the rectangle does not lie in the texture.
pub(crate) fn frames_of(sprite: &Sprite, anim: Option<&Animation>, size: UVec2) -> Option<(Vec<Rect>, Vec2, f32)> {
    let base = crop(sprite, size)?;
    // The hotspot is written in the same units as the rectangle (texels of the reference size):
    // scale it the way `crop` scales the rectangle, so a larger texture keeps the hotspot on the same spot.
    let k = Vec2::new(size.x as f32 / sprite.reference[0].unwrap_or(size.x as f32), size.y as f32 / sprite.reference[1].unwrap_or(size.y as f32));
    let origin = sprite.origin.map_or(Vec2::ZERO, |o| Vec2::new(o[0], o[1]) * k);
    let mut frames = vec![base];
    let mut period = 0.0;
    if let Some(a) = anim
        && a.field == Field::Offset
        && a.auto.is_some_and(|m| matches!(m, Auto::Row | Auto::Column | Auto::Square))
        && a.length > 0.0
    {
        let moved: Vec<Rect> = a
            .auto_offsets()
            .into_iter()
            .map(|o| {
                let d = Vec2::new(o[0] * size.x as f32, o[1] * size.y as f32);
                Rect { min: base.min + d, max: base.max + d }
            })
            .filter(|r| r.min.x >= -0.5 && r.min.y >= -0.5 && r.max.x <= size.x as f32 + 0.5 && r.max.y <= size.y as f32 + 0.5)
            .collect();
        if moved.len() > 1 {
            frames = moved;
            period = a.length;
        }
    }
    Some((frames, origin, period))
}

/// The part of a frame that is drawn: the rectangle inset by half a texel on every side, so that
/// linear sampling at the rim never reads a texel of the neighbouring frame. The drawn node is
/// inset by the same amount (see [`inset_node`]), so the texel-to-pixel scale and the hotspot stay
/// where they were. A pure artefact fix, not a rule of the original (the report grades its
/// filtering as bilinear, one cursor, one scale, and says nothing about its edges). A frame of a
/// texel or less is left whole.
pub(crate) fn inset_rect(r: Rect) -> Rect {
    if r.width() <= 1.0 || r.height() <= 1.0 {
        return r;
    }
    Rect { min: r.min + Vec2::splat(0.5), max: r.max - Vec2::splat(0.5) }
}

/// The node of the inset frame: its top-left and size, from the full frame's (`pos`, `size` in UI
/// units, the cell's texel scaled by the cursor size factor `k`).
pub(crate) fn inset_node(frame: Rect, pos: Vec2, k: Vec2) -> (Vec2, Vec2) {
    let inner = inset_rect(frame);
    (pos + (inner.min - frame.min) * k, inner.size() * k)
}

/// The fixed reference size the stock engine divides the window by to size its cursor, per axis
/// (the rule: cell texels times window width over `.x` and window height over `.y`). Grade: read
/// from the executable, static; measured at two window sizes for Armada II. It is not the HUD's
/// canvas.
const STOCK_CURSOR_REFERENCE: Vec2 = Vec2::new(800.0, 600.0);

/// How many UI units one texel of the cursor covers, horizontally and vertically (D-06, #1152).
/// `stock` is the HUD fit with the window and the HUD area's height in the HUD's theme units
/// (`StockLayout::extent_for_aspect`), or `None` for the Remaster look.
///
/// - Remaster look (LCARS, and Armada I's HUD, whose cursor scaling is not measured): the texel
///   covers one unit, times the player's `cursor_scale`.
/// - Stock look, Stretched HUD fit: the original's rule, the window over
///   [`STOCK_CURSOR_REFERENCE`] per axis, so the cursor is wider than tall on a wide window. Grade:
///   read from the executable, static, and measured for Armada II. INTERIM for the Fleet Operations
///   and Roots profiles: their cursor scaling was never measured; they take the rule on inference
///   (same engine family, same strings and tables).
/// - Stock look, Centred HUD fit: the centred HUD area's height over the reference height on both
///   axes. STA64's own choice (D-06): it equals the original at the sheet's own aspect and keeps
///   the cursor's size relative to the panels on any other.
///
/// Under the Stock look `cursor_scale` multiplies the result.
pub(crate) fn size_factors(stock: Option<(HudFit, Vec2, f32)>, cursor_scale: f32) -> Vec2 {
    let base = match stock {
        None => Vec2::ONE,
        Some((HudFit::Stretched, window, _)) => window / STOCK_CURSOR_REFERENCE,
        Some((HudFit::Centred, _, hud_height)) => Vec2::splat(hud_height / STOCK_CURSOR_REFERENCE.y),
    };
    base * cursor_scale
}

/// The frame to show `t` seconds in.
pub(crate) fn frame_at(frames: usize, period: f32, t: f32) -> usize {
    if frames <= 1 || period <= 0.0 || !t.is_finite() {
        return 0;
    }
    (((t / period).rem_euclid(1.0) * frames as f32) as usize).min(frames - 1)
}

/// The UI node's top-left in UI units. `pointer` is in logical window pixels, and a `Val::Px`
/// is multiplied by `UiScale` when the UI is laid out (`k`), so the pointer is divided by it;
/// the hotspot is in the node's own units and scales with it.
pub(crate) fn node_pos(pointer: Vec2, origin: Vec2, k: f32) -> Vec2 {
    pointer / if k > 0.0 { k } else { 1.0 } - origin
}

fn resolve(name: &str, sprites: &mut CursorSprites, vfs: &sta_vfs::Vfs, images: &mut Assets<Image>, formats: CompressedImageFormats) -> Option<CursorArt> {
    if let Some(found) = sprites.resolved.get(name) {
        return found.clone();
    }
    let found = (|| {
        let table = sprites.table.as_ref()?;
        let sprite = table.sprite(name)?.clone();
        let anim = sprite.anims.first().and_then(|id| table.animation(id)).cloned();
        let (handle, size) = page(&sprite, GuiImageSampling::Linear, vfs, &mut sprites.cache, images, formats)?;
        let (frames, origin, period) = frames_of(&sprite, anim.as_ref(), size)?;
        Some(CursorArt { image: handle, frames, origin, period })
    })();
    sprites.resolved.insert(name.to_owned(), found.clone());
    found
}

/// The sprite names to try, in order: the wanted cursor's race variant, the wanted cursor, the
/// cursor the click would otherwise show (the move cursor for the warp cursor), the default's race
/// variant, the default.
pub(crate) fn candidates(wanted: &str, race: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut names = vec![wanted];
    if wanted == "c_warp" {
        names.push("c_move");
    }
    names.push(DEFAULT);
    for name in names {
        if let Some(r) = race {
            out.push(format!("{name}_{r}"));
        }
        out.push(name.to_owned());
    }
    out.dedup();
    out
}

fn reset_wanted(mut wanted: ResMut<WantedCursor>) {
    if wanted.0 != DEFAULT {
        wanted.0 = DEFAULT;
    }
}

#[allow(clippy::too_many_arguments)]
fn place(
    mut commands: Commands,
    wanted: Res<WantedCursor>,
    mounted: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    ui_scale: Option<Res<UiScale>>,
    (profile, hud): (Option<Res<super::profile::UiProfile>>, Option<Res<super::hud::HudConfig>>),
    race: Option<Res<crate::game::LocalRace>>,
    settings: Option<Res<crate::frontend::screens::Settings>>,
    // INTERIM (not measured): the animation clock is real time, so it also runs while the game is paused.
    mut guard: Local<DragGuard>,
    time: Res<Time<Real>>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    orbit: Option<Res<crate::viewer::OrbitButtons>>,
    mut sprites: ResMut<CursorSprites>,
    mut window: Query<(&Window, Option<&mut CursorOptions>), With<PrimaryWindow>>,
    mut nodes: Query<(Entity, &mut Node, &mut ImageNode, &mut Visibility), With<CursorNode>>,
) {
    let Some(mut images) = images else { return };
    if !sprites.loaded || mounted.as_ref().is_some_and(|m| m.is_changed()) {
        sprites.cache = GuiSprites::default();
        sprites.resolved.clear();
        sprites.table = mounted.as_ref().map(|m| load_table(&m.0));
        sprites.loaded = true;
    }
    let formats = device.as_ref().map(|d| CompressedImageFormats::from_features(d.features())).unwrap_or_else(CompressedImageFormats::empty);
    let pointer = window.single().ok().and_then(|(w, _)| w.cursor_position());
    let focused = window.single().ok().is_none_or(|(w, _)| w.focused);
    let dragging = buttons.as_deref().zip(orbit.as_deref()).is_some_and(|(b, o)| camera_drag(b, o, focused, &mut guard));
    let art = match (&mounted, pointer) {
        (Some(m), Some(_)) if !dragging => {
            let vfs = &m.0;
            let race = race.as_deref().map(|r| r.name.as_str()).filter(|n| !n.is_empty());
            candidates(wanted.0, race).iter().find_map(|name| resolve(name, &mut sprites, vfs, &mut images, formats))
        }
        _ => None,
    };
    if let Ok((_, Some(mut options))) = window.single_mut() {
        // Hidden too while the camera is dragged: no OS cursor either.
        let visible = art.is_none() && !dragging;
        if options.visible != visible {
            options.visible = visible;
        }
    }
    let (Some(art), Some(pointer)) = (art, pointer) else {
        for (_, _, _, mut v) in &mut nodes {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
        }
        return;
    };
    let frame = frame_at(art.frames.len(), art.period, time.elapsed_secs());
    let rect = art.frames[frame];
    // The size rule (D-06) and the player's factor (UI-45) scale the drawn cell and the hotspot alike.
    let player = settings.as_deref().map_or(1.0, |s| s.cursor_scale).clamp(0.5, 3.0);
    let stock = profile.as_deref().filter(|p| p.is_stock() && **p != super::profile::UiProfile::Armada1)
        .zip(hud.as_deref().and_then(|h| h.gui.as_ref().map(|gui| (h, gui))))
        .map(|(_, (h, gui))| {
            let (window, hud_height) = StockLayout::extent_for_aspect(gui, h.aspect, h.fit);
            (h.fit, window, hud_height)
        });
    let size = size_factors(stock, player);
    let pos = node_pos(pointer, art.origin * size, ui_scale.map_or(1.0, |s| s.0));
    // The sampled rectangle and the node are inset together (see `inset_rect`).
    let (pos, node_size) = inset_node(rect, pos, size);
    let rect = inset_rect(rect);
    let image = ImageNode { image: art.image.clone(), rect: Some(rect), image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() };
    if let Some((_, mut node, mut current, mut v)) = nodes.iter_mut().next() {
        node.left = Val::Px(pos.x);
        node.top = Val::Px(pos.y);
        node.width = Val::Px(node_size.x);
        node.height = Val::Px(node_size.y);
        if current.image != image.image || current.rect != image.rect {
            *current = image;
        }
        if *v != Visibility::Inherited {
            *v = Visibility::Inherited;
        }
    } else {
        commands.spawn((
            CursorNode,
            Node { position_type: PositionType::Absolute, left: Val::Px(pos.x), top: Val::Px(pos.y), width: Val::Px(node_size.x), height: Val::Px(node_size.y), ..default() },
            image,
            GlobalZIndex(i32::MAX - 1),
            Pickable::IGNORE,
        ));
    }
}

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WantedCursor>()
            .init_resource::<CursorSprites>()
            .add_systems(PreUpdate, reset_wanted)
            .add_systems(PostUpdate, place.before(bevy::ui::UiSystems::Prepare));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An invented table: a 4-frame flipbook and a still cursor, in a 128 x 64 texture.
    fn table() -> SpriteTable {
        SpriteTable::parse(
            b"sprite_table\n@referenceWidth=128\n@referenceHeight=64\n\
              spin tex_a 0 0 32 32 @anim=flip4 @cursor @origin=(3,5)\n\
              still tex_a 0 32 32 32 @cursor @origin=(16,16)\n\
              @animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n",
        )
    }

    /// The drawn part of a frame leaves half a texel at every rim; the node shrinks and moves with it,
    /// so a texel still covers `k` units and the hotspot stays on the same spot. Invented sheet size.
    /// An install whose table has no warp cursor shows the move cursor, not the pointer.
    #[test]
    fn a_missing_warp_cursor_falls_back_to_the_move_cursor() {
        assert_eq!(candidates("c_warp", None), ["c_warp", "c_move", DEFAULT]);
        assert_eq!(candidates("c_warp", Some("fx")), ["c_warp_fx", "c_warp", "c_move_fx", "c_move", "standard_cursor_fx", "standard_cursor"]);
    }

    #[test]
    fn the_frame_is_inset_by_half_a_texel_and_the_node_follows() {
        let t = table();
        let s = t.sprite("spin").unwrap();
        let (frames, ..) = frames_of(s, s.anims.first().and_then(|i| t.animation(i)), UVec2::new(128, 64)).unwrap();
        // No sampled texel lies outside its own frame, so none of a neighbour's is read.
        for (i, f) in frames.iter().enumerate() {
            let r = inset_rect(*f);
            assert_eq!(r, Rect::new(f.min.x + 0.5, 0.5, f.max.x - 0.5, 31.5), "frame {i}");
            let (pos, size) = inset_node(*f, Vec2::new(10.0, 20.0), Vec2::splat(2.0));
            assert_eq!((pos, size), (Vec2::new(11.0, 21.0), Vec2::new(62.0, 62.0)));
        }
        // The scale is unchanged: units per texel are the node's over the rectangle's.
        let (_, size) = inset_node(frames[0], Vec2::ZERO, Vec2::splat(2.0));
        assert_eq!(size / inset_rect(frames[0]).size(), Vec2::splat(2.0));
        // A frame of a texel or less is not inset.
        let tiny = Rect::new(4.0, 4.0, 5.0, 9.0);
        assert_eq!(inset_rect(tiny), tiny);
    }

    /// D-06 (#1152): the cursor's size under each HUD fit, derived from the rule (the window over
    /// [`STOCK_CURSOR_REFERENCE`] per axis when stretched; the centred HUD area's height over the
    /// reference height when centred) at 4:3, 16:9 and a window narrower than 4:3. The HUD area comes
    /// from a layout on the layout's own default canvas (no sheet is loaded).
    #[test]
    fn the_cursor_size_follows_the_window_over_the_reference_under_each_fit_and_the_player_factor_multiplies() {
        let gui = sta_content::GuiLayout::default();
        let factors = |aspect: f32, fit: HudFit, player: f32| {
            let (window, hud_height) = StockLayout::extent_for_aspect(&gui, aspect, fit);
            size_factors(Some((fit, window, hud_height)), player)
        };
        let near = |a: Vec2, b: Vec2| assert!((a - b).abs().max_element() < 1e-3, "{a:?} vs {b:?}");
        // The HUD's theme units: a window is 1080 high.
        let window = |aspect: f32| Vec2::new(1080.0 * aspect, 1080.0);
        let reference = STOCK_CURSOR_REFERENCE;
        // 4:3 window: both fits are the original's rule, window over reference.
        near(factors(4.0 / 3.0, HudFit::Stretched, 1.0), window(4.0 / 3.0) / reference);
        near(factors(4.0 / 3.0, HudFit::Centred, 1.0), window(4.0 / 3.0) / reference);
        // 16:9 window: Stretched is wider than tall by the window's ratio; Centred keeps the height's factor on both axes.
        let wide = factors(16.0 / 9.0, HudFit::Stretched, 1.0);
        near(wide, window(16.0 / 9.0) / reference);
        assert!(wide.x > wide.y);
        near(factors(16.0 / 9.0, HudFit::Centred, 1.0), Vec2::splat(1080.0 / reference.y));
        // A window narrower than the sheet: the centred area is shorter than the window, so is the cursor.
        let narrow = factors(1.0, HudFit::Centred, 1.0);
        assert!(narrow.x == narrow.y && narrow.y < 1080.0 / reference.y, "{narrow:?}");
        near(factors(1.0, HudFit::Stretched, 1.0), window(1.0) / reference);
        // The player's factor multiplies either.
        near(factors(16.0 / 9.0, HudFit::Stretched, 2.0), 2.0 * window(16.0 / 9.0) / reference);
        near(factors(16.0 / 9.0, HudFit::Centred, 0.5), Vec2::splat(0.5 * 1080.0 / reference.y));
        // The Remaster look draws the texture size times the player's factor, whatever the window.
        assert_eq!(size_factors(None, 1.0), Vec2::ONE);
        assert_eq!(size_factors(None, 1.5), Vec2::splat(1.5));
        // The drawn cell of 32 x 32 texels on the stretched 16:9 window (half a texel inset each side).
        let (_, node) = inset_node(Rect::new(0.0, 0.0, 32.0, 32.0), Vec2::ZERO, wide);
        near(node, 31.0 * wide);
    }

    #[test]
    fn flipbook_frames_step_along_the_row() {
        let t = table();
        let s = t.sprite("spin").unwrap();
        let (frames, origin, period) = frames_of(s, s.anims.first().and_then(|i| t.animation(i)), UVec2::new(128, 64)).unwrap();
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0], Rect::new(0.0, 0.0, 32.0, 32.0));
        assert_eq!(frames[3], Rect::new(96.0, 0.0, 128.0, 32.0));
        assert_eq!(origin, Vec2::new(3.0, 5.0));
        assert_eq!(period, 1.0);
    }

    #[test]
    fn a_bigger_texture_makes_a_bigger_cursor_with_the_same_names() {
        let t = table();
        let s = t.sprite("still").unwrap();
        let (frames, origin, period) = frames_of(s, None, UVec2::new(160, 80)).unwrap();
        assert_eq!(frames, vec![Rect::new(0.0, 40.0, 40.0, 80.0)]);
        // the hotspot (16,16) of the 32 px cell stays at the cell's centre in the 40 px cell
        assert_eq!(origin, Vec2::new(20.0, 20.0));
        assert_eq!(period, 0.0);
    }

    /// The three-file chain the engine reads: a root that includes the table declaring the
    /// flipbooks, then the cursor table that only names them.
    fn chain(files: &'static [(&'static str, &'static str)]) -> SpriteTable {
        SpriteTable::load("sprites.spr", |name| files.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, d)| d.as_bytes().to_vec()))
    }

    const FILES: &[(&str, &str)] = &[
        ("sprites.spr", "sprite_table\n@include tex_anim.spr\n@include cursor.spr\n"),
        ("tex_anim.spr", "sprite_table\n@animation flip4\noffset 4 1.0 step\n@auto=row\n@keyframes\n"),
        ("cursor.spr", "sprite_table\n@referenceWidth=128\n@referenceHeight=64\nspin tex_a 0 0 32 32 @anim=flip4 @cursor @origin=(3,5)\n"),
    ];

    #[test]
    fn the_include_chain_yields_the_frames() {
        let t = chain(FILES);
        let s = t.sprite("spin").unwrap();
        let (frames, _, period) = frames_of(s, s.anims.first().and_then(|i| t.animation(i)), UVec2::new(128, 64)).unwrap();
        assert_eq!(frames.len(), 4);
        assert_eq!(period, 1.0);
    }

    /// Only the cursor table mounted (a mod that ships just it): the flipbook is unknown, so one
    /// still frame, and nothing panics.
    #[test]
    fn the_cursor_table_alone_falls_back_to_one_frame() {
        let t = SpriteTable::load("cursor.spr", |name| FILES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, d)| d.as_bytes().to_vec()));
        let s = t.sprite("spin").unwrap();
        assert!(s.anims.first().and_then(|i| t.animation(i)).is_none());
        let (frames, _, period) = frames_of(s, None, UVec2::new(128, 64)).unwrap();
        assert_eq!((frames.len(), period), (1, 0.0));
        assert_eq!(frame_at(frames.len(), period, 123.4), 0);
    }

    /// Time is an argument, not read from a clock: a frozen time gives a fixed frame.
    #[test]
    fn the_frame_follows_the_given_time_and_loops() {
        let t = chain(FILES);
        let s = t.sprite("spin").unwrap();
        let (frames, _, period) = frames_of(s, s.anims.first().and_then(|i| t.animation(i)), UVec2::new(128, 64)).unwrap();
        let at = |secs: f32| frame_at(frames.len(), period, secs);
        assert_eq!([at(0.0), at(0.3), at(0.6), at(0.9), at(1.0), at(2.3)], [0, 1, 2, 3, 0, 1]);
    }

    #[test]
    fn the_camera_drag_hides_the_cursor() {
        let orbit = crate::viewer::OrbitButtons { rotate: MouseButton::Middle, pan: None };
        let mut g = DragGuard::default();
        let mut b = ButtonInput::<MouseButton>::default();
        assert!(!camera_drag(&b, &orbit, true, &mut g));
        b.press(MouseButton::Right);
        assert!(!camera_drag(&b, &orbit, true, &mut g), "a right click is an order, not a camera drag");
        b.press(MouseButton::Middle);
        assert!(camera_drag(&b, &orbit, true, &mut g));
        b.release(MouseButton::Middle);
        assert!(!camera_drag(&b, &orbit, true, &mut g));
        let pan = crate::viewer::OrbitButtons { rotate: MouseButton::Left, pan: Some(MouseButton::Right) };
        assert!(camera_drag(&b, &pan, true, &mut g));
    }

    /// A drag in progress when the window loses focus: no drag while unfocused, and on refocus a
    /// button still reading as pressed (the release was lost) is ignored until released or freshly pressed.
    #[test]
    fn a_stale_press_does_not_hide_the_cursor_after_refocus() {
        let orbit = crate::viewer::OrbitButtons { rotate: MouseButton::Middle, pan: None };
        let mut g = DragGuard::default();
        let mut b = ButtonInput::<MouseButton>::default();
        b.press(MouseButton::Middle);
        assert!(camera_drag(&b, &orbit, true, &mut g));
        b.clear();
        assert!(!camera_drag(&b, &orbit, false, &mut g), "unfocused");
        // The frame after refocus: still held, never seen released.
        assert!(!camera_drag(&b, &orbit, true, &mut g), "stale held button");
        assert!(!camera_drag(&b, &orbit, true, &mut g), "and it stays ignored while held");
        // Released, then pressed again: a fresh drag.
        b.release(MouseButton::Middle);
        assert!(!camera_drag(&b, &orbit, true, &mut g));
        b.clear();
        b.press(MouseButton::Middle);
        assert!(camera_drag(&b, &orbit, true, &mut g), "a fresh press hides it");
        // Refocus where the input was reset and the button is pressed anew in the same frame.
        b.clear();
        assert!(!camera_drag(&b, &orbit, false, &mut g));
        b.release(MouseButton::Middle);
        b.clear();
        b.press(MouseButton::Middle);
        assert!(camera_drag(&b, &orbit, true, &mut g), "just pressed after refocus counts");
    }

    #[test]
    fn missing_animation_or_bad_rect_is_handled() {
        let t = table();
        let s = t.sprite("spin").unwrap();
        let (frames, _, _) = frames_of(s, None, UVec2::new(128, 64)).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames_of(s, None, UVec2::ZERO).is_none());
    }

    #[test]
    fn frames_loop_over_the_period() {
        assert_eq!(frame_at(4, 1.0, 0.0), 0);
        assert_eq!(frame_at(4, 1.0, 0.26), 1);
        assert_eq!(frame_at(4, 1.0, 0.99), 3);
        assert_eq!(frame_at(4, 1.0, 1.1), 0);
        assert_eq!(frame_at(1, 1.0, 5.0), 0);
        assert_eq!(frame_at(4, 0.0, 5.0), 0);
    }

    #[test]
    fn node_position_divides_the_pointer_by_ui_scale_not_the_hotspot() {
        // Pointer at 400,300 window pixels, UI scale 2: the node sits at 200,150 UI units,
        // less the hotspot (which scales with the node).
        assert_eq!(node_pos(Vec2::new(400.0, 300.0), Vec2::new(16.0, 16.0), 2.0), Vec2::new(184.0, 134.0));
        assert_eq!(node_pos(Vec2::new(10.0, 10.0), Vec2::ZERO, 0.0), Vec2::new(10.0, 10.0));
    }

    #[test]
    fn the_local_races_variant_comes_first_then_the_plain_cursor() {
        assert_eq!(candidates("c_attack", Some("klingonempire")), ["c_attack_klingonempire", "c_attack", &format!("{DEFAULT}_klingonempire"), DEFAULT]);
        assert_eq!(candidates("c_attack", None), ["c_attack", DEFAULT]);
        // The table's own lookup ignores case, so `c_attack_KlingonEmpire` is found by this name.
        let table = SpriteTable::parse(b"sprite_table\n@referenceWidth=64\n@referenceHeight=32\nc_attack_KlingonEmpire tex_a 0 0 32 32 @cursor\nc_attack tex_a 32 0 32 32 @cursor\n");
        let picked = candidates("c_attack", Some("klingonempire")).into_iter().find(|n| table.sprite(n).is_some());
        assert_eq!(picked.as_deref(), Some("c_attack_klingonempire"));
        assert_eq!(table.sprite("c_attack_klingonempire").unwrap().name, "c_attack_KlingonEmpire");
        let plain = candidates("c_attack", Some("borg")).into_iter().find(|n| table.sprite(n).is_some());
        assert_eq!(plain.as_deref(), Some("c_attack"), "no Borg variant: the plain cursor");
    }

    #[test]
    fn wanted_resets_to_the_pointer() {
        let mut app = App::new();
        app.init_resource::<WantedCursor>().add_systems(PreUpdate, reset_wanted);
        app.world_mut().resource_mut::<WantedCursor>().0 = "c_attack";
        app.update();
        assert_eq!(app.world().resource::<WantedCursor>().0, DEFAULT);
    }

    /// With `STA64_TEST_INSTALL` (+ optional `STA64_TEST_MOD`): every cursor the client asks for
    /// by name is a sprite of the install's own table, its texture decodes, and its frames lie
    /// inside the texture with the hotspot inside the cell. Names are the client's, the values
    /// are whatever the install says.
    #[test]
    fn real_install_if_available() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).unwrap();
        let table = load_table(&vfs);
        let mut animated = 0;
        for name in [DEFAULT, "c_move", "c_attack", "c_mining", "c_rally", "c_wormin"] {
            let sprite = table.sprite(name).unwrap_or_else(|| panic!("{name}: no such cursor sprite"));
            let (path, format) = sta_assets::find_texture(&vfs, &sprite.texture).unwrap_or_else(|| panic!("{name}: no texture {}", sprite.texture));
            let rgba = sta_assets::decode_rgba(&vfs, &path, format).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let size = UVec2::new(rgba.width, rgba.height);
            let anim = sprite.anims.first().and_then(|id| table.animation(id));
            let (frames, origin, period) = frames_of(sprite, anim, size).unwrap_or_else(|| panic!("{name}: rectangle outside the texture"));
            let cell = frames[0].size();
            assert!(frames.iter().all(|f| (f.size() - cell).abs().max_element() < 1e-3), "{name}: frames differ in size");
            assert!(origin.x <= cell.x && origin.y <= cell.y, "{name}: hotspot {origin:?} outside the {cell:?} cell");
            // The rule: a sprite that names an offset flipbook the table declares yields one frame per
            // step of it, and a positive period. The expectation is the mounted data's own.
            if let Some(a) = anim.filter(|a| a.field == Field::Offset && a.auto.is_some_and(|m| matches!(m, Auto::Row | Auto::Column | Auto::Square))) {
                animated += 1;
                assert!(frames.len() > 1, "{name}: names the flipbook {} but yields one frame", a.id);
                assert!(frames.len() <= a.auto_offsets().len() && period > 0.0, "{name}: frames {} period {period}", frames.len());
            } else {
                assert_eq!(frames.len(), 1, "{name}: no flipbook, one frame");
            }
        }
        // The rule: a flipbook a cursor sprite names resolves against the whole table.
        let pointer = table.sprite(DEFAULT).unwrap();
        assert!(pointer.anims.first().is_none_or(|id| table.animation(id).is_some()), "the pointer names a flipbook the chain does not declare");
        eprintln!("animated cursors among those asked for: {animated}");
        // UI-45: where the table has race variants, each playable race's `name` finds its own.
        let db_index = sta_content::OdfIndex::build(&vfs).0;
        let (races, _) = sta_content::Races::load(&vfs, &db_index);
        let names: Vec<String> = races.playable().iter().map(|r| r.name.clone()).collect();
        let with_variants = names.iter().filter(|r| table.sprite(&format!("c_attack_{r}")).is_some()).count();
        eprintln!("race cursor variants: {with_variants} of {} playable races ({names:?})", names.len());
        for r in &names {
            let first = candidates("c_attack", Some(r)).into_iter().find(|n| table.sprite(n).is_some());
            assert!(first.is_some(), "{r}: no attack cursor at all");
        }
    }
}
