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
//! texture pixels; a cell's size on screen is its texture size, so a mod that ships a larger
//! texture under the same sprite names gets a larger cursor; a flipbook plays over the
//! animation's length.

use std::collections::HashMap;

use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use sta_assets::a2fmt::spr::{Animation, Auto, Field, Sprite, SpriteTable};

use super::gui_sprites::{GuiImageSampling, GuiSprites, crop, page, read_table};
use crate::viewer::Mounted;

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
        let (handle, size) = page(&sprite, GuiImageSampling::Nearest, vfs, &mut sprites.cache, images, formats)?;
        let (frames, origin, period) = frames_of(&sprite, anim.as_ref(), size)?;
        Some(CursorArt { image: handle, frames, origin, period })
    })();
    sprites.resolved.insert(name.to_owned(), found.clone());
    found
}

/// The sprite names to try, in order: the wanted cursor's race variant, the wanted cursor, the
/// default's race variant, the default.
pub(crate) fn candidates(wanted: &str, race: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for name in [wanted, DEFAULT] {
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
    race: Option<Res<crate::game::LocalRace>>,
    settings: Option<Res<crate::frontend::screens::Settings>>,
    time: Res<Time>,
    mut sprites: ResMut<CursorSprites>,
    mut window: Query<(&Window, Option<&mut CursorOptions>), With<PrimaryWindow>>,
    mut nodes: Query<(Entity, &mut Node, &mut ImageNode, &mut Visibility), With<CursorNode>>,
) {
    let Some(mut images) = images else { return };
    if !sprites.loaded || mounted.as_ref().is_some_and(|m| m.is_changed()) {
        sprites.cache = GuiSprites::default();
        sprites.resolved.clear();
        sprites.table = mounted.as_ref().map(|m| read_table(&m.0, "sprites/cursor.spr"));
        sprites.loaded = true;
    }
    let formats = device.as_ref().map(|d| CompressedImageFormats::from_features(d.features())).unwrap_or_else(CompressedImageFormats::empty);
    let pointer = window.single().ok().and_then(|(w, _)| w.cursor_position());
    let art = match (&mounted, pointer) {
        (Some(m), Some(_)) => {
            let vfs = &m.0;
            let race = race.as_deref().map(|r| r.name.as_str()).filter(|n| !n.is_empty());
            candidates(wanted.0, race).iter().find_map(|name| resolve(name, &mut sprites, vfs, &mut images, formats))
        }
        _ => None,
    };
    if let Ok((_, Some(mut options))) = window.single_mut() {
        let visible = art.is_none();
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
    // The player's size factor (UI-45) scales the drawn cell and the hotspot alike.
    let size = settings.as_deref().map_or(1.0, |s| s.cursor_scale).clamp(0.5, 3.0);
    let pos = node_pos(pointer, art.origin * size, ui_scale.map_or(1.0, |s| s.0));
    let image = ImageNode { image: art.image.clone(), rect: Some(rect), image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() };
    if let Some((_, mut node, mut current, mut v)) = nodes.iter_mut().next() {
        node.left = Val::Px(pos.x);
        node.top = Val::Px(pos.y);
        node.width = Val::Px(rect.width() * size);
        node.height = Val::Px(rect.height() * size);
        if current.image != image.image || current.rect != image.rect {
            *current = image;
        }
        if *v != Visibility::Inherited {
            *v = Visibility::Inherited;
        }
    } else {
        commands.spawn((
            CursorNode,
            Node { position_type: PositionType::Absolute, left: Val::Px(pos.x), top: Val::Px(pos.y), width: Val::Px(rect.width() * size), height: Val::Px(rect.height() * size), ..default() },
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
        let table = read_table(&vfs, "sprites/cursor.spr");
        for name in [DEFAULT, "c_attack", "c_mining", "c_rally", "c_wormin"] {
            let sprite = table.sprite(name).unwrap_or_else(|| panic!("{name}: no such cursor sprite"));
            let (path, format) = sta_assets::find_texture(&vfs, &sprite.texture).unwrap_or_else(|| panic!("{name}: no texture {}", sprite.texture));
            let rgba = sta_assets::decode_rgba(&vfs, &path, format).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let size = UVec2::new(rgba.width, rgba.height);
            let anim = sprite.anims.first().and_then(|id| table.animation(id));
            let (frames, origin, _) = frames_of(sprite, anim, size).unwrap_or_else(|| panic!("{name}: rectangle outside the texture"));
            let cell = frames[0].size();
            assert!(frames.iter().all(|f| (f.size() - cell).abs().max_element() < 1e-3), "{name}: frames differ in size");
            assert!(origin.x <= cell.x && origin.y <= cell.y, "{name}: hotspot {origin:?} outside the {cell:?} cell");
        }
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
