//! Mod-authored GUI button and static panel pictures. GUI art consults the
//! selected table then the global table; missing art preserves text fallback.
//! The table is read through the mounted VFS (D1, D3); no executable content is
//! loaded (D2).
//! Animated offsets, tiling, non-interface materials, and blend directives are
//! not rendered here; button art uses its static rectangle and alpha channel.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sta_assets::a2fmt::spr::{Skip, Sprite, SpriteTable};

use crate::ui::theme::{UiButton, WidgetState};
use crate::viewer::Mounted;

type PageKey = (String, Option<[u8; 3]>);
type PageImage = (Handle<Image>, UVec2);
type ArtKey = (Vec<String>, usize, GuiImageSource, bool);
type ResolvedArt = Option<(Handle<Image>, Rect)>;

#[derive(Resource, Default)]
pub struct GuiSkin {
    pub table: Option<String>,
}

#[derive(Resource, Default)]
pub struct GuiSprites {
    selected_path: Option<String>,
    selected: Option<SpriteTable>,
    global: Option<SpriteTable>,
    pages: HashMap<PageKey, Option<PageImage>>,
    resolved: HashMap<ArtKey, ResolvedArt>,
    loaded: bool,
}

/// Ordered authored sprite name candidates for one button.
#[derive(Component)]
pub struct GuiButton {
    pub names: Vec<String>,
    pub keep_label: bool,
}

/// Optional tint palette for a GUI button's artwork.
#[derive(Component)]
pub struct GuiButtonColors {
    pub normal: Color,
    pub hovered: Color,
    pub pressed: Color,
    pub disabled: Color,
    pub active: Color,
}

impl Default for GuiButtonColors {
    fn default() -> Self {
        Self {
            normal: Color::WHITE,
            hovered: Color::srgb(0.85, 0.85, 1.0),
            pressed: Color::srgb(0.7, 0.7, 0.7),
            disabled: Color::srgb(0.5, 0.5, 0.5),
            active: Color::WHITE,
        }
    }
}

/// Marks an armed command so its normal artwork can use the active tint.
#[derive(Component)]
pub struct GuiActive(pub bool);

/// Mark a direct child label of a [`GuiButton`] for picture/text fallback.
#[derive(Component)]
pub struct GuiButtonLabel;

/// Which sprite table supplies a static GUI picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(dead_code)] // Explicit shared-table opt-in for wireframe consumers.
pub enum GuiImageSource {
    #[default]
    Gui,
    SharedSprites,
}

/// A static SPR picture. `names` are ordered prefix candidates; `frame` selects
/// the native `<prefix>.<frame>` entry when one exists.
#[derive(Component)]
pub struct GuiImage {
    pub names: Vec<String>,
    pub frame: usize,
    pub color: Color,
    pub source: GuiImageSource,
}

#[allow(dead_code)] // Builder methods are also exercised by isolated art fixtures.
impl GuiImage {
    pub fn new(name: impl Into<String>) -> Self {
        Self { names: vec![name.into()], frame: 0, color: Color::WHITE, source: GuiImageSource::Gui }
    }

    pub fn with_frame(mut self, frame: usize) -> Self {
        self.frame = frame;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_source(mut self, source: GuiImageSource) -> Self {
        self.source = source;
        self
    }
}

#[derive(Component)]
pub(crate) struct GuiImageFallbackColor(Color);

fn table_path(name: &str) -> Option<String> {
    let path = sta_vfs::normalize(name);
    let path = path.strip_prefix("sprites/").unwrap_or(&path);
    (!path.is_empty() && path.ends_with(".spr") && !path.contains("..")).then(|| format!("sprites/{path}"))
}

fn read_table(vfs: &sta_vfs::Vfs, path: &str) -> SpriteTable {
    SpriteTable::load(path, |name| {
        let path = table_path(name)?;
        vfs.read(&path).and_then(Result::ok)
    })
}

fn crop(sprite: &Sprite, size: UVec2) -> Option<Rect> {
    if size.x == 0 || size.y == 0 {
        return None;
    }
    let [x, y, w, h] = sprite.rect;
    let [rw, rh] = [
        sprite.reference[0].unwrap_or(size.x as f32),
        sprite.reference[1].unwrap_or(size.y as f32),
    ];
    if ![x, y, w, h, rw, rh].into_iter().all(f32::is_finite)
        || x < 0.0
        || y < 0.0
        || w <= 0.0
        || h <= 0.0
        || rw <= 0.0
        || rh <= 0.0
        || x + w > rw
        || y + h > rh
    {
        return None;
    }
    let sx = size.x as f32 / rw;
    let sy = size.y as f32 / rh;
    Some(Rect::new(x * sx, y * sy, (x + w) * sx, (y + h) * sy))
}

fn page(
    sprite: &Sprite,
    vfs: &sta_vfs::Vfs,
    cache: &mut GuiSprites,
    images: &mut Assets<Image>,
    formats: CompressedImageFormats,
) -> Option<(Handle<Image>, UVec2)> {
    let key_colour = match sprite.skip {
        Skip::Colour(c) => Some(c),
        Skip::Unset | Skip::Off => None,
    };
    let key = (sprite.texture.to_ascii_lowercase(), key_colour);
    if let Some(found) = cache.pages.get(&key) {
        return found.clone();
    }
    let result = (|| {
        let (path, format) = sta_assets::find_texture(vfs, &sprite.texture)?;
        let mut image = if let Ok(mut rgba) = sta_assets::decode_rgba(vfs, &path, format) {
            if let Some(colour) = key_colour {
                for px in rgba.pixels.chunks_exact_mut(4) {
                    if px[..3] == colour {
                        px[3] = 0;
                    }
                }
            }
            Image::new(
                Extent3d {
                    width: rgba.width,
                    height: rgba.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                rgba.pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            )
        } else {
            // Bevy handles PNG/KTX2. Colour-keyed pages need CPU pixels, so
            // unsupported keyed formats deliberately fall back to the label.
            if key_colour.is_some() {
                return None;
            }
            let ext = match format {
                sta_assets::TextureFormat::Png => "png",
                sta_assets::TextureFormat::Ktx2 => "ktx2",
                _ => return None,
            };
            let bytes = vfs.read(&path)?.ok()?;
            Image::from_buffer(
                &bytes,
                ImageType::Extension(ext),
                formats,
                true,
                ImageSampler::nearest(),
                RenderAssetUsages::RENDER_WORLD,
            )
            .ok()?
        };
        image.sampler = ImageSampler::nearest();
        let size = UVec2::new(
            image.texture_descriptor.size.width,
            image.texture_descriptor.size.height,
        );
        if size.x == 0 || size.y == 0 {
            return None;
        }
        Some((images.add(image), size))
    })();
    cache.pages.insert(key, result.clone());
    result
}

fn ordered_sprites(names: &[String], selected: Option<&SpriteTable>, global: Option<&SpriteTable>) -> Vec<Sprite> {
    [selected, global]
        .into_iter()
        .flatten()
        .flat_map(|table| names.iter().filter_map(|name| table.sprite(name)))
        .cloned()
        .collect()
}

fn frame_sprites(
    names: &[String],
    frame: usize,
    selected: Option<&SpriteTable>,
    global: Option<&SpriteTable>,
) -> Vec<Sprite> {
    [selected, global]
        .into_iter()
        .flatten()
        .flat_map(|table| {
            names.iter().filter_map(move |name| {
                let suffix = format!(".{frame}");
                let entry = if name.ends_with(&suffix) { name.clone() } else { format!("{name}{suffix}") };
                table.sprite(&entry).or_else(|| (frame == 0).then(|| table.sprite(name)).flatten())
            })
        })
        .cloned()
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn art(
    names: &[String],
    frame: usize,
    source: GuiImageSource,
    numbered: bool,
    shared: Option<&SpriteTable>,
    vfs: &sta_vfs::Vfs,
    cache: &mut GuiSprites,
    images: &mut Assets<Image>,
    formats: CompressedImageFormats,
) -> Option<(Handle<Image>, Rect)> {
    let key = (names.to_vec(), frame, source, numbered);
    if let Some(found) = cache.resolved.get(&key) {
        return found.clone();
    }
    // Try every candidate in the chosen table before using the global table.
    let candidates = match source {
        GuiImageSource::Gui if !numbered => ordered_sprites(names, cache.selected.as_ref(), cache.global.as_ref()),
        GuiImageSource::Gui => frame_sprites(names, frame, cache.selected.as_ref(), cache.global.as_ref()),
        GuiImageSource::SharedSprites => frame_sprites(names, frame, shared, None),
    };
    let mut found = None;
    for sprite in candidates {
        let Some((handle, size)) = page(&sprite, vfs, cache, images, formats) else {
            continue;
        };
        if let Some(rect) = crop(&sprite, size) {
            found = Some((handle, rect));
            break;
        }
    }
    cache.resolved.insert(key, found.clone());
    found
}

/// Paint after widget state/background systems. Deferred image insertion is
/// flushed by Bevy at the end of the chained HUD systems.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn paint(
    mut commands: Commands,
    mounted: Option<Res<Mounted>>,
    skin: Option<Res<GuiSkin>>,
    sprites: Option<ResMut<GuiSprites>>,
    images: Option<ResMut<Assets<Image>>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    shared: Option<Res<crate::render::sprites::SpriteTables>>,
    mut buttons: Query<
        (
            Entity,
            &GuiButton,
            &WidgetState,
            Option<&GuiButtonColors>,
            Option<&GuiActive>,
            Option<&mut ImageNode>,
            Option<&Children>,
            Option<&mut BackgroundColor>,
        ),
        (With<UiButton>, Without<GuiImage>),
    >,
    mut panels: Query<
        (Entity, &GuiImage, Option<&mut ImageNode>, Option<&mut BackgroundColor>, Option<&GuiImageFallbackColor>),
        Without<GuiButton>,
    >,
    mut labels: Query<&mut Visibility, With<GuiButtonLabel>>,
) {
    let (Some(mut sprites), Some(mut images)) = (sprites, images) else {
        for (entity, _, _, _, _, image, children, _) in &mut buttons {
            if image.is_some() {
                commands.entity(entity).remove::<ImageNode>();
            }
            if let Some(children) = children {
                for child in children.iter() {
                    if let Ok(mut visibility) = labels.get_mut(child) {
                        *visibility = Visibility::Inherited;
                    }
                }
            }
        }
        for (entity, _, image, background, fallback) in &mut panels {
            if image.is_some() {
                commands.entity(entity).remove::<ImageNode>();
            }
            if let (Some(mut background), Some(fallback)) = (background, fallback) {
                background.0 = fallback.0;
            }
        }
        return;
    };
    let selected_path = skin.as_ref().and_then(|s| s.table.as_deref()).and_then(table_path);
    let reload = !sprites.loaded
        || mounted.as_ref().is_some_and(|m| m.is_changed())
        || skin.as_ref().is_some_and(|s| s.is_changed())
        || shared.as_ref().is_some_and(|s| s.is_changed())
        || sprites.selected_path != selected_path;
    if reload {
        sprites.pages.clear();
        sprites.resolved.clear();
        sprites.selected_path = selected_path.clone();
        sprites.selected = mounted
            .as_ref()
            .and_then(|m| selected_path.as_deref().map(|p| read_table(&m.0, p)));
        sprites.global = mounted.as_ref().map(|m| read_table(&m.0, "sprites/gui_global.spr"));
        sprites.loaded = true;
    }
    let has_sources = mounted.is_some() && skin.is_some();
    let formats = device
        .as_ref()
        .map(|d| CompressedImageFormats::from_features(d.features()))
        .unwrap_or_else(CompressedImageFormats::empty);
    if !has_sources {
        sprites.pages.clear();
        sprites.resolved.clear();
        sprites.selected = None;
        sprites.global = None;
        sprites.loaded = false;
    }
    for (entity, button, state, colors, active, image, children, background) in &mut buttons {
        let found = if has_sources {
            art(&button.names, 0, GuiImageSource::Gui, false, None,
                &mounted.as_ref().unwrap().0, &mut sprites, &mut images, formats)
        } else {
            None
        };
        if let Some((handle, rect)) = &found {
            let default_colors = GuiButtonColors::default();
            let colors = colors.unwrap_or(&default_colors);
            let node = ImageNode {
                image: handle.clone(),
                rect: Some(*rect),
                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                color: match state {
                    WidgetState::Normal if active.is_some_and(|a| a.0) => colors.active,
                    WidgetState::Normal => colors.normal,
                    WidgetState::Hovered => colors.hovered,
                    WidgetState::Pressed => colors.pressed,
                    WidgetState::Disabled => colors.disabled,
                },
                ..default()
            };
            if let Some(mut image) = image {
                if image.image != node.image || image.rect != node.rect || image.color != node.color || image.image_mode != node.image_mode {
                    *image = node;
                }
            } else {
                commands.entity(entity).insert(node);
            }
            if let Some(mut background) = background {
                background.0 = Color::NONE;
            }
        } else if image.is_some() {
            commands.entity(entity).remove::<ImageNode>();
        }
        if let Some(children) = children {
            for child in children.iter() {
                if let Ok(mut visibility) = labels.get_mut(child) {
                    *visibility = if found.is_some() && !button.keep_label {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                }
            }
        }
    }
    for (entity, panel, image, background, fallback) in &mut panels {
        let source = match panel.source {
            GuiImageSource::Gui if has_sources => Some((GuiImageSource::Gui, None)),
            GuiImageSource::SharedSprites if mounted.is_some() =>
                shared.as_ref().map(|table| (GuiImageSource::SharedSprites, Some(table.0.as_ref()))),
            _ => None,
        };
        let found = source.and_then(|(source, table)| art(
            &panel.names, panel.frame, source, true, table,
            &mounted.as_ref().unwrap().0, &mut sprites, &mut images, formats,
        ));
        if let Some((handle, rect)) = found {
            let node = ImageNode { image: handle, rect: Some(rect), color: panel.color, image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() };
            if let Some(mut image) = image {
                if image.image != node.image || image.rect != node.rect || image.color != node.color || image.image_mode != node.image_mode {
                    *image = node;
                }
            } else {
                commands.entity(entity).insert(node);
            }
            if let Some(mut background) = background {
                if fallback.is_none() {
                    commands.entity(entity).insert(GuiImageFallbackColor(background.0));
                }
                background.0 = Color::NONE;
            }
        } else {
            if image.is_some() {
                commands.entity(entity).remove::<ImageNode>();
            }
            if let (Some(mut background), Some(fallback)) = (background, fallback) {
                background.0 = fallback.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn crop_scales_each_axis_and_rejects_invalid_rectangles() {
        let mut sprite = SpriteTable::parse(b"@referenceWidth=100\n@referenceHeight=50\nbutton tex 10 5 20 10\n")
            .sprite("button")
            .unwrap()
            .clone();
        assert_eq!(
            crop(&sprite, UVec2::new(200, 150)),
            Some(Rect::new(20.0, 15.0, 60.0, 45.0))
        );
        sprite.rect[2] = 100.0;
        assert_eq!(crop(&sprite, UVec2::new(200, 150)), None);
    }

    #[test]
    fn chosen_table_precedes_global_across_all_candidate_names() {
        let selected = SpriteTable::parse(b"second chosen 0 0 8 8\n");
        let global = SpriteTable::parse(b"first global 0 0 8 8\nsecond global 0 0 8 8\n");
        let names = vec!["first".into(), "second".into()];
        assert_eq!(
            ordered_sprites(&names, Some(&selected), Some(&global))[0].texture,
            "chosen"
        );
        assert_eq!(ordered_sprites(&names, None, Some(&global))[0].texture, "global");
        assert!(ordered_sprites(&names, None, None).is_empty());
    }

    #[test]
    fn numbered_frames_follow_table_priority_and_are_distinct() {
        let selected = SpriteTable::parse(b"second.0 chosen 0 0 2 2\nsecond.1 chosen 2 0 2 2\n");
        let global = SpriteTable::parse(b"first.0 global 0 0 2 2\nfirst.1 global 0 0 2 2\n");
        let names = vec!["first".into(), "second".into()];
        let first = frame_sprites(&names, 0, Some(&selected), Some(&global));
        let second = frame_sprites(&names, 1, Some(&selected), Some(&global));
        assert_eq!(first[0].texture, "chosen");
        assert_eq!(second[0].texture, "chosen");
        assert_eq!(first[0].rect, [0.0, 0.0, 2.0, 2.0]);
        assert_eq!(second[0].rect, [2.0, 0.0, 2.0, 2.0]);
        assert_eq!(ordered_sprites(&names, Some(&selected), Some(&global)).len(), 0);
    }

    #[test]
    fn shared_sprite_names_are_explicit() {
        let selected = SpriteTable::parse(b"wire gui 0 0 2 2\n");
        let shared = SpriteTable::parse(b"wire.0 shared 1 0 2 2\n");
        let names = vec!["wire".into()];
        assert_eq!(ordered_sprites(&names, Some(&selected), None)[0].texture, "gui");
        assert_eq!(frame_sprites(&names, 0, Some(&shared), None)[0].texture, "shared");
    }

    #[test]
    fn table_loader_follows_includes() {
        let table = SpriteTable::load("sprites/gui_test.spr", |path| match path {
            "sprites/gui_test.spr" => Some(b"@include gui_parts.spr\n".to_vec()),
            "gui_parts.spr" => Some(b"included page 0 0 8 8\n".to_vec()),
            _ => None,
        });
        assert!(table.sprite("included").is_some());
    }

    #[test]
    fn mounted_art_and_text_fallback_follow_skin_and_mount() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sta64-gui-sprites-{}-{}", std::process::id(), nonce));
        fs::create_dir(&root).unwrap();
        let data = root.join("Data");
        fs::create_dir_all(data.join("sprites")).unwrap();
        fs::create_dir_all(data.join("textures/rgb")).unwrap();
        fs::write(data.join("sprites/gui_choice.spr"), b"@include parts.spr\n").unwrap();
        fs::write(data.join("sprites/parts.spr"), b"second chosen 0 0 2 2\npanel.0 chosen 0 0 2 2\npanel.1 chosen 2 0 2 2\n").unwrap();
        fs::write(data.join("sprites/gui_global.spr"), b"first global 0 0 2 2\n").unwrap();
        let tga = |red: u8| {
            let mut bytes = vec![0u8; 18];
            bytes[2] = 2; // uncompressed true colour
            bytes[12] = 4;
            bytes[14] = 2;
            bytes[16] = 24;
            for _ in 0..8 {
                bytes.extend_from_slice(&[0, 0, red]);
            }
            bytes
        };
        fs::write(data.join("textures/rgb/chosen.tga"), tga(200)).unwrap();
        fs::write(data.join("textures/rgb/global.tga"), tga(80)).unwrap();
        let vfs = sta_vfs::Vfs::mount_install(&root, None).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<GuiSprites>()
            .insert_resource(GuiSkin {
                table: Some("gui_choice.spr".into()),
            })
            .insert_resource(Mounted(vfs))
            .add_systems(Update, paint);
        let label = app.world_mut().spawn((GuiButtonLabel, Visibility::Inherited)).id();
        let button = app
            .world_mut()
            .spawn((
                UiButton,
                WidgetState::Normal,
                GuiButton {
                    names: vec!["first".into(), "second".into()],
                    keep_label: false,
                },
                Node::default(),
                BackgroundColor(Color::WHITE),
            ))
            .add_child(label)
            .id();
        let panel = app.world_mut().spawn((
            GuiImage::new("panel").with_frame(1).with_color(Color::srgb(0.2, 0.4, 0.6)),
            Node::default(),
            BackgroundColor(Color::srgb(0.1, 0.2, 0.3)),
            Visibility::Hidden,
        )).id();
        app.insert_resource(crate::render::sprites::SpriteTables(std::sync::Arc::new(
            SpriteTable::parse(b"wire.0 chosen 0 0 2 2\n"),
        )));
        let wire = app.world_mut().spawn((
            GuiImage::new("wire").with_source(GuiImageSource::SharedSprites),
            Node::default(),
        )).id();
        app.update();
        assert_eq!(app.world().get::<ImageNode>(wire).unwrap().rect, Some(Rect::new(0.0, 0.0, 2.0, 2.0)));
        assert_eq!(app.world().get::<ImageNode>(panel).unwrap().rect, Some(Rect::new(2.0, 0.0, 4.0, 2.0)));
        assert_eq!(app.world().get::<ImageNode>(panel).unwrap().color, Color::srgb(0.2, 0.4, 0.6));
        assert_eq!(app.world().get::<Visibility>(panel), Some(&Visibility::Hidden));
        let image = app.world().get::<ImageNode>(button).unwrap();
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&image.image)
                .unwrap()
                .data
                .as_ref()
                .unwrap()[0],
            200
        );
        assert_eq!(app.world().get::<Visibility>(label), Some(&Visibility::Hidden));
        app.world_mut().resource_mut::<GuiSkin>().table = None;
        app.update();
        let image = app.world().get::<ImageNode>(button).unwrap();
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&image.image)
                .unwrap()
                .data
                .as_ref()
                .unwrap()[0],
            80
        );
        *app.world_mut().get_mut::<WidgetState>(button).unwrap() = WidgetState::Disabled;
        app.update();
        assert_eq!(
            app.world().get::<ImageNode>(button).unwrap().color,
            Color::srgb(0.5, 0.5, 0.5)
        );
        let palette = GuiButtonColors {
            normal: Color::srgb(0.1, 0.1, 0.1),
            hovered: Color::srgb(0.2, 0.2, 0.2),
            pressed: Color::srgb(0.3, 0.3, 0.3),
            disabled: Color::srgb(0.4, 0.4, 0.4),
            active: Color::srgb(0.6, 0.6, 0.6),
        };
        app.world_mut().entity_mut(button).insert((palette, GuiActive(true)));
        app.update();
        assert_eq!(app.world().get::<ImageNode>(button).unwrap().color, Color::srgb(0.4, 0.4, 0.4));
        *app.world_mut().get_mut::<WidgetState>(button).unwrap() = WidgetState::Pressed;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(button).unwrap().color, Color::srgb(0.3, 0.3, 0.3));
        *app.world_mut().get_mut::<WidgetState>(button).unwrap() = WidgetState::Hovered;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(button).unwrap().color, Color::srgb(0.2, 0.2, 0.2));
        *app.world_mut().get_mut::<WidgetState>(button).unwrap() = WidgetState::Normal;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(button).unwrap().color, Color::srgb(0.6, 0.6, 0.6));
        app.world_mut().get_mut::<GuiActive>(button).unwrap().0 = false;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(button).unwrap().color, Color::srgb(0.1, 0.1, 0.1));
        app.world_mut().get_mut::<GuiButton>(button).unwrap().names = vec!["absent".into()];
        app.world_mut().get_mut::<GuiImage>(panel).unwrap().names = vec!["absent".into()];
        app.update();
        assert!(app.world().get::<ImageNode>(button).is_none());
        assert!(app.world().get::<ImageNode>(panel).is_none());
        assert_eq!(app.world().get::<BackgroundColor>(panel).unwrap().0, Color::srgb(0.1, 0.2, 0.3));
        assert_eq!(app.world().get::<Visibility>(label), Some(&Visibility::Inherited));
        app.world_mut().get_mut::<GuiButton>(button).unwrap().names = vec!["first".into()];
        app.world_mut().get_mut::<GuiImage>(panel).unwrap().names = vec!["panel".into()];
        app.update();
        assert!(app.world().get::<ImageNode>(button).is_some());
        app.world_mut().remove_resource::<crate::render::sprites::SpriteTables>();
        app.update();
        assert!(app.world().get::<ImageNode>(wire).is_none());
        app.world_mut().remove_resource::<Mounted>();
        app.update();
        assert!(app.world().get::<ImageNode>(button).is_none());
        assert!(app.world().get::<ImageNode>(panel).is_none());
        assert_eq!(app.world().get::<Visibility>(label), Some(&Visibility::Inherited));
        fs::remove_dir_all(root).unwrap();
    }

    /// Explicit local-asset audit. No proprietary fixture or GPU is required in CI.
    #[test]
    fn installed_button_atlases() {
        let Some(root) = std::env::var_os("STA64_TEST_INSTALL") else {
            return;
        };
        let active = std::env::var("STA64_TEST_MOD").ok();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active.as_deref()).unwrap();
        let tables =
            std::env::var("STA64_TEST_GUI_TABLES").unwrap_or_else(|_| "gui_federation.spr,gui_borg.spr".into());
        let extra = std::env::var("STA64_TEST_GUI_BUTTONS").unwrap_or_default();
        let mut names = vec![
            "b_build",
            "b_move",
            "b_attack",
            "b_stop",
            "b_rally",
            "b_cancel",
            "b_back",
            "b_harvest",
            "b_form",
        ];
        names.extend(extra.split(',').filter(|n| !n.is_empty()));
        for table in tables.split(',') {
            let mut cache = GuiSprites {
                selected: Some(read_table(&vfs, &table_path(table).unwrap())),
                global: Some(read_table(&vfs, "sprites/gui_global.spr")),
                ..default()
            };
            let mut images = Assets::<Image>::default();
            for name in &names {
                let button = GuiButton {
                    names: vec![(*name).into()],
                    keep_label: false,
                };
                let (handle, rect) = art(&button.names, 0, GuiImageSource::Gui, false, None,
                    &vfs, &mut cache, &mut images, CompressedImageFormats::empty())
                    .unwrap_or_else(|| panic!("{table}: {name} did not resolve to valid artwork"));
                let image = images.get(&handle).unwrap();
                assert!(image.data.as_ref().is_some_and(|d| !d.is_empty()));
                println!(
                    "{table} {name}: image={}x{} crop={rect:?}",
                    image.width(),
                    image.height()
                );
            }
            println!(
                "{table}: {} buttons resolved; {} lazy pages",
                names.len(),
                cache.pages.len()
            );
        }
    }
}
