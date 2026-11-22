//! The minimap's static layers and sprite icons (#1042, part b).
//!
//! What the stock look draws, bottom to top (read from the executable, static): the backing, the
//! map picture, the field picture, the map-icon sprites, the dots, fog and shroud, the camera
//! outline, and black strips where the square is not map.
//!
//! Both Armada games resolve these keys through the sprite table. A resolved `mapSprite`
//! draws into the map picture and takes precedence over `mapIcon` and ordinary dots.
//! A resolved `mapIcon` draws a centred sprite, with `mapSize` as a minimum size per axis.
//! Neutral icons keep their authored colour. Owned icons use STA64's team mark colours.
//!
//! The map and warp-suppression pictures use a 128-texel grid and additive blending. The
//! warp-suppression circles use the configured class multipliers. The nebula footprint uses
//! model geometry when available, with the existing effect radius as its fallback. Armada I uses
//! its authored `mapSize` in radar texels for area sprites.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, RenderPipelineDescriptor, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::ui_render::UiMaterialPlugin;
use bevy::ui_render::ui_material::{UiMaterial, UiMaterialKey};
use sta_assets::a2fmt::spr::SpriteTable;
use sta_content::{ContentDb, Profile, UnitDef};
use sta_odf::Odf;
use sta_sim::{EntityId, PlayerId};

use super::radar::{Kind, RadarFeed, RadarMap, RadarSquare, RadarState, StockRadar, kind, passes_filters, standing};
use super::radar_sampling::sample_rgb;
use crate::render::sprites::SpriteTables;
use crate::viewer::Mounted;

/// Cells along a side of the minimap's pictures. The original fixes it in the executable (read
/// from the executable, static); no data key carries it.
pub const GRID: usize = 128;
/// The warp-suppression field's colour, 8-bit (read from the executable, static; no data key).
pub const FIELD_COLOUR: [u8; 3] = [80, 0, 48];
/// A dot's edge in texels of [`GRID`] when the definition has no `mapSize`: ships, stations,
/// everything else (read from the executable, static; no data key carries the defaults).
pub const SHIP_TEXELS: f32 = 2.0;
pub const STATION_TEXELS: f32 = 3.0;
pub const OTHER_TEXELS: f32 = 1.0;

/// One object of the static layers, in the map's space (x, z).
#[derive(Debug, Clone, PartialEq)]
pub struct LayerObject {
    pub at: Vec2,
    /// Width of the map sprite in world units.
    pub radius: f32,
    /// Fixed width in radar texels, when authored or required by the profile.
    pub size_texels: Option<f32>,
    /// The `mapSprite` key.
    pub sprite: Option<String>,
    /// The warp-suppression radius, world units.
    pub warp_radius: Option<f32>,
}

/// The keys of a definition the minimap reads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapKeys {
    pub sprite: Option<String>,
    pub icon: Option<String>,
    pub size: Option<f32>,
}

impl MapKeys {
    pub fn of(raw: &Odf) -> MapKeys {
        let key = |k: &str| raw.str(k).map(str::trim).filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("none")).map(str::to_owned);
        MapKeys {
            sprite: key("mapSprite"),
            icon: key("mapIcon"),
            size: raw.f32("mapSize").filter(|s| s.is_finite() && *s > 0.0),
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.sprite.is_none() && self.icon.is_none() && self.size.is_none()
    }
}

/// The shared gameplay and map-view radius readers.
pub use sta_content::{field_radius, nebula_radius};

/// What the minimap reads of a definition, cached per definition so a frame reads no keys.
#[derive(Debug, Clone, PartialEq)]
pub struct DefInfo {
    pub keys: MapKeys,
    /// The bounding radius, world units (zero: not read).
    pub radius: f32,
    pub sprite_texels: Option<f32>,
    icon_filter: Kind,
    /// The warp-suppression radius ([`sta_content::Rules::warp_suppression_radius`]).
    pub warp_radius: Option<f32>,
}

type DefMap = (usize, BTreeMap<sta_sim::ContentKey, Option<Arc<DefInfo>>>);

/// The per-definition cache of [`DefInfo`], reset when the content database changes. It sits in
/// [`RadarState`] behind a lock so the dot pass, the layers and the icons share it.
#[derive(Default)]
pub struct DefCache(std::sync::Mutex<DefMap>);

impl Clone for DefCache {
    fn clone(&self) -> Self {
        DefCache::default()
    }
}

impl std::fmt::Debug for DefCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DefCache")
    }
}

impl DefCache {
    pub fn get(&self, db: &Arc<ContentDb>, def: sta_sim::ContentKey) -> Option<Arc<DefInfo>> {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let id = Arc::as_ptr(db) as usize;
        if g.0 != id {
            *g = (id, BTreeMap::new());
        }
        g.1.entry(def)
            .or_insert_with(|| {
                db.unit(def).map(|d: &UnitDef| {
                    let keys = MapKeys::of(&d.raw);
                    let sprite_texels = keys.size.or_else(|| (db.profile() == Profile::Sta1Classic).then(|| match kind(Some(db), def) {
                        Kind::Ship => SHIP_TEXELS,
                        Kind::Station => STATION_TEXELS,
                        Kind::Body => OTHER_TEXELS,
                    }));
                    let icon_filter = if !sta_content::odf::CRAFT_CLASSES.contains(&d.class.0.as_str()) {
                        Kind::Body
                    } else if d.is_starbase {
                        Kind::Station
                    } else {
                        Kind::Ship
                    };
                    Arc::new(DefInfo { keys, sprite_texels, icon_filter, radius: d.radius.to_f32(), warp_radius: db.rules().warp_suppression_radius(d).map(|r| r.to_f32()) })
                })
            })
            .clone()
    }
}

/// Map objects represented as regions or scenery instead of simulation units.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct RadarStatics {
    layers: Vec<LayerObject>,
    icons: Vec<StaticIcon>,
}

#[derive(Debug, Clone, PartialEq)]
struct StaticIcon {
    at: Vec2,
    keys: MapKeys,
    radius: f32,
    size_texels: Option<f32>,
}

impl RadarStatics {
    pub fn of_map<'a>(map: &sta_bzn::Map, armada1: bool, mut definition: impl FnMut(&str) -> Option<&'a UnitDef>) -> RadarStatics {
        let mut out = RadarStatics::default();
        for o in &map.objects {
            let Some(def) = definition(&o.odf) else { continue };
            let raw = &*def.raw;
            if raw.i64("hidden").unwrap_or(0) != 0 {
                continue;
            }
            let model_radius = def.radius.to_f32();
            let radius = match def.class.0.as_str() {
                "nebula_obj" if model_radius > 0.0 => model_radius,
                "nebula_obj" => nebula_radius(raw),
                "asteroidfield" => 2.0 * field_radius(raw),
                "background_obj" => model_radius,
                _ => continue,
            };
            let at = Vec2::new(o.transform[9], o.transform[11]);
            let keys = MapKeys::of(raw);
            let size_texels = if armada1 || def.class.0 == "background_obj" {
                Some(keys.size.unwrap_or(OTHER_TEXELS))
            } else {
                None
            };
            if keys.sprite.is_some() {
                out.layers.push(LayerObject { at, radius, size_texels, sprite: keys.sprite.clone(), warp_radius: None });
            }
            if keys.icon.is_some() {
                out.icons.push(StaticIcon { at, keys, radius, size_texels });
            }
        }
        out
    }
}

/// A texture's pixels, 8-bit RGBA, top row first.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Texture {
    /// The part `uv` (`[u, v, w, h]`, 0..1) of it, at least one texel.
    pub fn cut(&self, uv: [f32; 4]) -> Texture {
        let x0 = ((uv[0] * self.width as f32).round().max(0.0) as usize).min(self.width.saturating_sub(1));
        let y0 = ((uv[1] * self.height as f32).round().max(0.0) as usize).min(self.height.saturating_sub(1));
        let w = ((uv[2] * self.width as f32).round() as usize).clamp(1, self.width - x0);
        let h = ((uv[3] * self.height as f32).round() as usize).clamp(1, self.height - y0);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in y0..y0 + h {
            rgba.extend_from_slice(&self.rgba[(y * self.width + x0) * 4..(y * self.width + x0 + w) * 4]);
        }
        Texture { width: w, height: h, rgba }
    }
}

/// The two pictures, [`GRID`] x [`GRID`] RGBA8, top row first, alpha opaque (the draw adds the
/// colour and ignores alpha).
#[derive(Debug, Clone, PartialEq)]
pub struct Pictures {
    pub map: Vec<u8>,
    pub field: Vec<u8>,
}

/// Build the map picture and the field picture of `objects` in `map`'s square. `texture` gives the
/// pixels of a `mapSprite`'s sprite (its texture cut to its rectangle), or `None` for a sprite it
/// does not know (nothing is added for that object).
pub fn build(objects: &[LayerObject], map: &RadarMap, texture: &mut dyn FnMut(&str) -> Option<Arc<Texture>>) -> Pictures {
    let n = GRID as i64;
    let mut sum = vec![[0u32; 3]; GRID * GRID];
    let mut field = vec![false; GRID * GRID];
    for o in objects {
        // Texel coordinates of the centre: x right, y down (the square's v runs against z).
        let c = map.to_square(o.at) * GRID as f32;
        if let Some(tex) = o.sprite.as_deref().and_then(&mut *texture) {
            let side = sprite_side(o.radius, o.size_texels, map);
            if side >= 1 && tex.width > 0 && tex.height > 0 {
                let x0 = (c.x - side as f32 / 2.0).round() as i64;
                let y0 = (c.y - side as f32 / 2.0).round() as i64;
                let x1 = x0.saturating_add(side).min(n);
                let y1 = y0.saturating_add(side).min(n);
                for y in y0.max(0)..y1 {
                    for x in x0.max(0)..x1 {
                        let uv = (Vec2::new(x as f32 - x0 as f32, y as f32 - y0 as f32) + Vec2::splat(0.5)) / side as f32;
                        let rgb = sample_rgb(&tex.rgba, tex.width, tex.height, uv);
                        let s = &mut sum[y as usize * GRID + x as usize];
                        for k in 0..3 {
                            s[k] += u32::from(rgb[k]);
                        }
                    }
                }
            }
        }
        if let Some(r) = o.warp_radius.map(|w| w / map.size * GRID as f32).filter(|r| *r > 0.0) {
            let (lo, hi) = ((c - Vec2::splat(r)).floor().max(Vec2::ZERO), (c + Vec2::splat(r)).ceil().min(Vec2::splat(GRID as f32)));
            for y in lo.y as usize..hi.y as usize {
                for x in lo.x as usize..hi.x as usize {
                    let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c;
                    if d.length_squared() <= r * r {
                        field[y * GRID + x] = true;
                    }
                }
            }
        }
    }
    let mut pictures = Pictures { map: vec![0; GRID * GRID * 4], field: vec![0; GRID * GRID * 4] };
    for i in 0..GRID * GRID {
        for k in 0..3 {
            pictures.map[i * 4 + k] = sum[i][k].min(255) as u8;
            pictures.field[i * 4 + k] = if field[i] { FIELD_COLOUR[k] } else { 0 };
        }
        pictures.map[i * 4 + 3] = 255;
        pictures.field[i * 4 + 3] = 255;
    }
    pictures
}

fn sprite_side(radius: f32, size_texels: Option<f32>, map: &RadarMap) -> i64 {
    size_texels.unwrap_or(radius / map.size * GRID as f32) as i64
}

// ---- drawing: an additive UI material ---------------------------------------

const UNTINTED: Vec4 = Vec4::ZERO;

/// A picture drawn additively (`dst + src`, alpha ignored) with the sampler of its image. `rect`
/// is the part of the image drawn, `[u, v, w, h]`.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct AdditiveImage {
    #[uniform(0)]
    pub rect: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub image: Handle<Image>,
    /// Linear team RGB and a 0/1 switch for preserving the source RGB or recolouring its intensity.
    #[uniform(3)]
    pub tint: Vec4,
}

impl UiMaterial for AdditiveImage {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/ui/radar_additive.wesl").into_owned())
    }

    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        let add = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
        let keep = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState { color: add, alpha: keep });
            }
        }
    }
}

// ---- sprites -----------------------------------------------------------------

/// Decoded textures by name, and the images made of them. Tests fill it; the game fills it from
/// the mounted install on demand.
#[derive(Resource, Default)]
pub struct RadarTextures {
    pixels: BTreeMap<String, Option<Arc<Texture>>>,
    handles: BTreeMap<String, Handle<Image>>,
}

impl RadarTextures {
    #[cfg(test)]
    pub fn insert(&mut self, name: &str, texture: Texture) {
        self.pixels.insert(name.to_ascii_lowercase(), Some(Arc::new(texture)));
    }

    /// The pixels of texture `name`, decoded once from `vfs` (a miss is remembered).
    pub fn pixels(&mut self, vfs: Option<&sta_vfs::Vfs>, name: &str) -> Option<Arc<Texture>> {
        let key = name.to_ascii_lowercase();
        if let Some(hit) = self.pixels.get(&key) {
            return hit.clone();
        }
        let vfs = vfs?;
        let decoded = sta_assets::find_texture(vfs, &key).and_then(|(path, fmt)| sta_assets::decode_rgba(vfs, &path, fmt).ok()).and_then(|rgba| {
            let (width, height) = (rgba.width as usize, rgba.height as usize);
            (width > 0 && height > 0 && rgba.pixels.len() >= width * height * 4).then(|| Arc::new(Texture { width, height, rgba: rgba.pixels }))
        });
        self.pixels.insert(key, decoded.clone());
        decoded
    }

    fn sprite_pixels(&mut self, table: &SpriteTable, vfs: Option<&sta_vfs::Vfs>, name: &str) -> Option<Arc<Texture>> {
        let (texture, _, _) = sprite_rect(table, name)?;
        self.pixels(vfs, &texture)
    }

    fn icon_handle(&mut self, vfs: Option<&sta_vfs::Vfs>, images: &mut Assets<Image>, name: &str, info: &IconInfo) -> Option<Handle<Image>> {
        let key = name.to_ascii_lowercase();
        if let Some(h) = self.handles.get(&key) {
            return Some(h.clone());
        }
        // Isolate the atlas rectangle before filtering so adjacent icons cannot bleed in.
        let tex = self.pixels(vfs, &info.texture)?.cut(info.uv);
        let handle = images.add(radar_image(tex.width, tex.height, tex.rgba));
        self.handles.insert(key, handle.clone());
        Some(handle)
    }

    fn clear(&mut self) {
        self.pixels.clear();
        self.handles.clear();
    }
}

fn radar_image(width: usize, height: usize, rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width: width as u32, height: height as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    image
}

/// A sprite of the table: its texture's name and its rectangle in 0..1.
fn sprite_rect(table: &SpriteTable, sprite: &str) -> Option<(String, [f32; 4], [f32; 2])> {
    let s = table.sprite(sprite)?;
    let uv = sta_assets::spr_anim::sprite_uv(table, s, 0.0);
    (uv.iter().all(|v| v.is_finite()) && uv[2] > 0.0 && uv[3] > 0.0)
        .then(|| (s.texture.clone(), uv, [s.rect[2], s.rect[3]]))
}

// ---- systems ------------------------------------------------------------------

/// One of the two layer pictures of the square.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPicture {
    Map,
    Field,
}

/// `(id, definition, position bits)` of the units the layers draw.
pub type LayerSignature = Vec<(EntityId, sta_sim::ContentKey, [i64; 2])>;

/// What the layers were last built from, and how often they were built.
#[derive(Resource, Default)]
pub struct LayerState {
    signature: LayerSignature,
    map: Option<RadarMap>,
    sprites: bool,
    textures: usize,
    pub builds: u32,
}

/// The units of the sim that carry a layer key (a `mapSprite`, or a warp-suppression radius) and
/// a bounding radius: `(id, definition, position bits)` in view order. Comparing two of these says
/// whether an object was created, destroyed or moved, without cloning a name.
pub fn layer_signature(view: &sta_sim::WorldView, db: &Arc<ContentDb>, defs: &DefCache) -> LayerSignature {
    view.units
        .iter()
        .filter(|u| !u.flags.contains(sta_sim::view::UnitFlags::DYING))
        .filter(|u| defs.get(db, u.def).is_some_and(|d| (d.keys.sprite.is_some() && (d.radius > 0.0 || d.sprite_texels.is_some())) || d.warp_radius.is_some()))
        .map(|u| (u.id, u.def, [u.pos.x.to_bits(), u.pos.z.to_bits()]))
        .collect()
}

/// The objects of the layers: the units of the sim that carry a layer key, and the map's statics.
pub fn layer_objects(view: &sta_sim::WorldView, db: Option<&Arc<ContentDb>>, statics: &RadarStatics, defs: &DefCache) -> Vec<LayerObject> {
    let mut out = Vec::new();
    if let Some(db) = db {
        for u in &view.units {
            if u.flags.contains(sta_sim::view::UnitFlags::DYING) {
                continue;
            }
            let Some(d) = defs.get(db, u.def).filter(|d| (d.keys.sprite.is_some() && (d.radius > 0.0 || d.sprite_texels.is_some())) || d.warp_radius.is_some()) else { continue };
            out.push(LayerObject { at: Vec2::new(u.pos.x.to_f32(), u.pos.z.to_f32()), radius: d.radius, size_texels: d.sprite_texels, sprite: d.keys.sprite.clone(), warp_radius: d.warp_radius });
        }
    }
    out.extend(statics.layers.iter().cloned());
    out
}

/// Keep the two pictures current: rebuilt when an object is created or destroyed, the square or
/// the sprites change, and not otherwise. The check compares the objects' identities and
/// positions, not names.
#[allow(clippy::too_many_arguments)]
pub fn update_layers(
    mut commands: Commands,
    feed: Res<RadarFeed>,
    state: Res<RadarState>,
    statics: Option<Res<RadarStatics>>,
    tables: Option<Res<SpriteTables>>,
    mounted: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
    materials: Option<ResMut<Assets<AdditiveImage>>>,
    mut textures: ResMut<RadarTextures>,
    mut built: ResMut<LayerState>,
    square: Query<(Entity, Option<&Children>), With<RadarSquare>>,
    pictures: Query<(&LayerPicture, &MaterialNode<AdditiveImage>)>,
) {
    let (Some(mut images), Some(mut materials), Some(map), Ok((sq, kids))) = (images, materials, state.map, square.single()) else { return };
    let sig = feed.content.as_ref().map(|db| layer_signature(&feed.view, db, &state.defs)).unwrap_or_default();
    let have = kids.is_some_and(|k| k.iter().any(|e| pictures.get(e).is_ok()));
    let sprites = tables.is_some();
    let statics_changed = statics.as_ref().is_some_and(|s| s.is_changed());
    if have && !statics_changed && built.signature == sig && built.map == Some(map) && built.sprites == sprites && !tables.as_ref().is_some_and(|t| t.is_changed()) && !mounted.as_ref().is_some_and(|m| m.is_changed()) && built.textures == textures.pixels.len() {
        return;
    }
    let empty = RadarStatics::default();
    let objects = layer_objects(&feed.view, feed.content.as_ref(), statics.as_deref().unwrap_or(&empty), &state.defs);
    let vfs = mounted.as_ref().map(|m| &m.0);
    let mut lookup = |name: &str| -> Option<Arc<Texture>> {
        let (texture, uv, _) = sprite_rect(&tables.as_ref()?.0, name)?;
        Some(Arc::new(textures.pixels(vfs, &texture)?.cut(uv)))
    };
    let made = build(&objects, &map, &mut lookup);
    built.signature = sig;
    built.map = Some(map);
    built.sprites = sprites;
    built.textures = textures.pixels.len();
    built.builds += 1;
    let map_image = images.add(radar_image(GRID, GRID, made.map));
    let field_image = images.add(radar_image(GRID, GRID, made.field));
    for (which, image, z) in [(LayerPicture::Map, map_image, -4), (LayerPicture::Field, field_image, -3)] {
        let material = materials.add(AdditiveImage { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), image, tint: UNTINTED });
        let existing = kids.and_then(|k| k.iter().find(|e| pictures.get(*e).is_ok_and(|(w, _)| *w == which)));
        match existing {
            Some(e) => {
                commands.entity(e).insert(MaterialNode(material));
            }
            None => {
                commands.spawn((
                    which,
                    MaterialNode(material),
                    Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                    ZIndex(z),
                    bevy::picking::Pickable::IGNORE,
                    ChildOf(sq),
                ));
            }
        }
    }
}

/// A map-icon sprite of the square.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RadarIcon {
    Unit(EntityId),
    Static(usize),
}

/// A sprite's atlas rectangle and its width and height on the radar's texel grid.
#[derive(Debug, Clone, PartialEq)]
struct IconInfo {
    texture: String,
    uv: [f32; 4],
    texels: Vec2,
}

fn icon_info(keys: &MapKeys, table: &SpriteTable) -> Option<IconInfo> {
    let (texture, uv, size) = sprite_rect(table, keys.icon.as_deref()?)?;
    let texels = Vec2::from(size).max(Vec2::splat(keys.size.unwrap_or(0.0)));
    Some(IconInfo { texture, uv, texels })
}

fn uses_map_picture(keys: &MapKeys, radius: f32, size_texels: Option<f32>, map: &RadarMap, table: &SpriteTable, vfs: Option<&sta_vfs::Vfs>, textures: &mut RadarTextures) -> bool {
    sprite_side(radius, size_texels, map) >= 1
        && keys.sprite.as_deref().is_some_and(|name| textures.sprite_pixels(table, vfs, name).is_some())
}

fn clear_textures(
    mounted: Option<Res<Mounted>>,
    tables: Option<Res<SpriteTables>>,
    mut textures: ResMut<RadarTextures>,
) {
    if mounted.is_some_and(|m| m.is_changed()) {
        textures.clear();
    } else if tables.is_some_and(|t| t.is_changed()) {
        textures.handles.clear();
    }
}

/// Resolve each visible unit's artwork before the dot pass. A missing sprite or texture leaves
/// the fallback dot available. Region/scenery icons are neutral and follow the map's fog.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_icons(
    mut commands: Commands,
    feed: Res<RadarFeed>,
    mut state: ResMut<RadarState>,
    teams: Option<Res<crate::viewer::TeamColours>>,
    map_teams: Option<Res<crate::viewer::MapTeamColours>>,
    tables: Option<Res<SpriteTables>>,
    mounted: Option<Res<Mounted>>,
    statics: Option<Res<RadarStatics>>,
    images: Option<ResMut<Assets<Image>>>,
    materials: Option<ResMut<Assets<AdditiveImage>>>,
    mut textures: ResMut<RadarTextures>,
    square: Query<Entity, With<RadarSquare>>,
    mut icons: Query<(Entity, &RadarIcon, &mut Node, &MaterialNode<AdditiveImage>, &ChildOf)>,
) {
    let mut art = BTreeSet::new();
    let mut want = BTreeMap::new();
    if let (Some(mut images), Some(mut materials), Some(map), Some(tables), Ok(square)) = (images, materials, state.map, tables, square.single()) {
        let table = &tables.0;
        let vfs = mounted.as_ref().map(|m| &m.0);
        let mut add_icon = |id, at: Vec2, keys: &MapKeys, owner: Option<PlayerId>, textures: &mut RadarTextures| -> bool {
            let Some(name) = keys.icon.as_deref() else { return false };
            let Some(info) = icon_info(keys, table) else { return false };
            let Some(image) = textures.icon_handle(vfs, &mut images, name, &info) else { return false };
            let tint = owner.filter(|o| o.is_player())
                .and_then(|o| crate::viewer::owner_mark_colour(teams.as_deref(), map_teams.as_deref(), o))
                .map(|colour| {
                    let c = colour.to_linear();
                    Vec4::new(c.red, c.green, c.blue, 1.0)
                })
                .unwrap_or(UNTINTED);
            let side = info.texels / GRID as f32 * 100.0;
            let at = map.to_square(at) * 100.0 - side / 2.0;
            let node = Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(at.x),
                top: Val::Percent(at.y),
                width: Val::Percent(side.x),
                height: Val::Percent(side.y),
                ..default()
            };
            want.insert(id, (node, image, tint));
            true
        };

        if let Some(db) = feed.content.as_ref() {
            for u in feed.view.visible_to(feed.player) {
                if u.flags.contains(sta_sim::view::UnitFlags::DYING) {
                    continue;
                }
                let Some(def) = state.defs.get(db, u.def) else { continue };
                let at = Vec2::new(u.pos.x.to_f32(), u.pos.z.to_f32());
                if uses_map_picture(&def.keys, def.radius, def.sprite_texels, &map, table, vfs, &mut textures)
                    || (def.keys.icon.is_some()
                        && passes_filters(&state, def.icon_filter, standing(&feed.view, feed.player, u.owner))
                        && add_icon(RadarIcon::Unit(u.id), at, &def.keys, Some(u.owner), &mut textures))
                {
                    art.insert(u.id);
                }
            }
        }
        if let Some(statics) = statics {
            for (i, icon) in statics.icons.iter().enumerate() {
                if !uses_map_picture(&icon.keys, icon.radius, icon.size_texels, &map, table, vfs, &mut textures) {
                    add_icon(RadarIcon::Static(i), icon.at, &icon.keys, None, &mut textures);
                }
            }
        }
        for (entity, id, mut node, material, parent) in &mut icons {
            if parent.parent() != square {
                commands.entity(entity).despawn();
                continue;
            }
            let Some((next, image, tint)) = want.remove(id) else {
                commands.entity(entity).despawn();
                continue;
            };
            if *node != next {
                *node = next;
            }
            if let Some(m) = materials.get(&material.0)
                && (m.tint != tint || m.image != image)
                && let Some(mut m) = materials.get_mut(&material.0)
            {
                m.image = image;
                m.tint = tint;
            }
        }
        for (id, (node, image, tint)) in want {
            let material = materials.add(AdditiveImage { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), image, tint });
            commands.spawn((id, MaterialNode(material), node, ZIndex(-2), bevy::picking::Pickable::IGNORE, ChildOf(square)));
        }
    } else {
        for (entity, _, _, _, _) in &icons {
            commands.entity(entity).despawn();
        }
    }
    if state.art != art {
        state.art = art;
    }
}

/// A black strip over the part of the square that is not map (one of four: left, right, top,
/// bottom).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadarStrip(pub usize);

/// The strips' rectangles in the square, as fractions `(left, top, width, height)`: the parts
/// outside the map's rectangle `bounds` (x, z). Empty rectangles where the map fills the square.
pub fn strips(map: &RadarMap, bounds: Option<(Vec2, Vec2)>) -> [[f32; 4]; 4] {
    let Some((lo, hi)) = bounds else { return [[0.0; 4]; 4] };
    let (a, b) = (map.to_square(Vec2::new(lo.x, hi.y)), map.to_square(Vec2::new(hi.x, lo.y)));
    let (l, r, t, bt) = (a.x.clamp(0.0, 1.0), b.x.clamp(0.0, 1.0), a.y.clamp(0.0, 1.0), b.y.clamp(0.0, 1.0));
    [[0.0, 0.0, l, 1.0], [r, 0.0, 1.0 - r, 1.0], [l, 0.0, r - l, t], [l, bt, r - l, 1.0 - bt]]
}

/// Paint the strips black, over everything else of the square (the last of the stock order).
/// The stock look only: the other look has no strips.
pub fn update_strips(mut commands: Commands, state: Res<RadarState>, square: Query<(Entity, Has<StockRadar>), With<RadarSquare>>, mut nodes: Query<(Entity, &RadarStrip, &mut Node)>) {
    let (Some(map), Ok((sq, true))) = (state.map, square.single()) else {
        for (e, _, _) in &nodes {
            commands.entity(e).despawn();
        }
        return;
    };
    let rects = strips(&map, state.bounds);
    let pc = |f: f32| Val::Percent(f * 100.0);
    let node = |r: [f32; 4]| Node { position_type: PositionType::Absolute, left: pc(r[0]), top: pc(r[1]), width: pc(r[2]), height: pc(r[3]), ..default() };
    let mut seen = [false; 4];
    for (_, s, mut n) in &mut nodes {
        let want = node(rects[s.0]);
        if *n != want {
            *n = want;
        }
        seen[s.0] = true;
    }
    for (i, r) in rects.iter().enumerate().filter(|(i, _)| !seen[*i]) {
        commands.spawn((RadarStrip(i), node(*r), BackgroundColor(Color::BLACK), ZIndex(10), bevy::picking::Pickable::IGNORE, ChildOf(sq)));
    }
}

pub struct RadarLayersPlugin;

impl Plugin for RadarLayersPlugin {
    fn build(&self, app: &mut App) {
        // Frame runs and tests without the asset server have neither the shader nor the material.
        if app.world().contains_resource::<bevy::asset::io::embedded::EmbeddedAssetRegistry>() {
            embedded_asset!(app, "radar_additive.wesl");
        }
        if app.get_sub_app(bevy::render::RenderApp).is_some() {
            app.add_plugins(UiMaterialPlugin::<AdditiveImage>::default());
        }
        app.init_resource::<RadarTextures>().init_resource::<LayerState>().init_resource::<RadarStatics>().add_systems(
            Update,
            (clear_textures, update_icons).chain().after(super::radar::fit_map).before(super::radar::refresh).in_set(crate::app::GameSet::Ui),
        ).add_systems(
            Update,
            (update_layers, update_strips).after(super::radar::refresh).in_set(crate::app::GameSet::Ui),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::{Rules, UnitDef};
    use sta_core::{Fixed, TeamId};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{PlayerId, Sim, Vec3 as SimVec3, WorldSource};

    fn odf(text: &str) -> sta_odf::Odf {
        sta_odf::resolve("lt", &mut |_| Some(text.to_owned())).unwrap()
    }

    fn flat(width: usize, height: usize, rgb: [u8; 3]) -> Arc<Texture> {
        Arc::new(Texture { width, height, rgba: (0..width * height).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect() })
    }

    fn square(size: f32) -> RadarMap {
        RadarMap { min: Vec2::ZERO, size }
    }

    fn pixel(p: &[u8], x: usize, y: usize) -> [u8; 3] {
        let i = (y * GRID + x) * 4;
        [p[i], p[i + 1], p[i + 2]]
    }

    fn nebula(at: Vec2, radius: f32) -> LayerObject {
        LayerObject { at, radius, size_texels: None, sprite: Some("sprite_a".into()), warp_radius: None }
    }

    fn planet(at: Vec2, radius: f32, multiplier: f32) -> LayerObject {
        LayerObject { at, radius, size_texels: None, sprite: None, warp_radius: Some(radius * multiplier) }
    }

    fn textures(name: &str) -> Option<Arc<Texture>> {
        (name == "sprite_a").then(|| flat(8, 8, [100, 50, 0]))
    }

    /// The sprite's texture is stretched into a square centred on the object, as wide in texels
    /// as its bounding radius is in texels of the grid, y down with z up.
    #[test]
    fn a_map_sprite_is_a_square_of_its_radius_centred_on_the_object() {
        // A 6400 square: 50 world units a texel. Radius 800 = 16 texels, centred at the middle.
        let p = build(&[nebula(Vec2::new(3200.0, 3200.0), 800.0)], &square(6400.0), &mut textures);
        let inside = |x, y| pixel(&p.map, x, y) == [100, 50, 0];
        assert!(inside(56, 56) && inside(71, 71) && inside(64, 64));
        assert!(!inside(55, 64) && !inside(72, 64) && !inside(64, 55) && !inside(64, 72));
        assert_eq!(p.map.chunks(4).filter(|c| c[..3] == [100, 50, 0]).count(), 16 * 16);
        // z up the screen: an object north of the middle lands higher in the picture.
        let north = build(&[nebula(Vec2::new(3200.0, 5000.0), 800.0)], &square(6400.0), &mut textures);
        assert_eq!(pixel(&north.map, 64, 28), [100, 50, 0]);
        assert_eq!(pixel(&north.map, 64, 100), [0, 0, 0]);
        // An unknown sprite adds nothing; an object part off the map is cut.
        let none = build(&[LayerObject { sprite: Some("nope".into()), ..nebula(Vec2::new(10.0, 10.0), 800.0) }], &square(6400.0), &mut textures);
        assert!(none.map.chunks(4).all(|c| c[..3] == [0, 0, 0]));
        let edge = build(&[nebula(Vec2::new(0.0, 6400.0), 800.0)], &square(6400.0), &mut textures);
        assert_eq!(pixel(&edge.map, 0, 0), [100, 50, 0]);
        assert_eq!(edge.map.chunks(4).filter(|c| c[..3] == [100, 50, 0]).count(), 8 * 8);
    }

    #[test]
    fn sprites_are_added_and_clamped() {
        let two = [nebula(Vec2::new(3200.0, 3200.0), 800.0), nebula(Vec2::new(3200.0, 3200.0), 800.0), nebula(Vec2::new(3200.0, 3200.0), 800.0)];
        let p = build(&two, &square(6400.0), &mut textures);
        assert_eq!(pixel(&p.map, 64, 64), [255, 150, 0], "100 x 3 clamps at 255, 50 x 3 adds");
    }

    /// A planet field is a filled circle of the bounding radius times the multiplier, in texels of
    /// the grid; texels are written, so two overlapping fields are a union.
    #[test]
    fn a_field_is_a_circle_of_radius_times_multiplier_and_overlaps_are_a_union() {
        // 6400 wide: 50 units a texel. Radius 250 x 4 = 1000 = 20 texels, centred on texel 64.
        let p = build(&[planet(Vec2::new(3200.0, 3200.0), 250.0, 4.0)], &square(6400.0), &mut textures);
        let set = |x, y| pixel(&p.field, x, y) == FIELD_COLOUR;
        assert!(set(64, 64) && set(64 + 19, 64) && set(64 - 20, 64) && set(64, 64 + 19));
        assert!(!set(64 + 20, 64) && !set(64 - 21, 64) && !set(64 + 15, 64 + 15));
        let area = p.field.chunks(4).filter(|c| c[..3] == FIELD_COLOUR).count() as f32;
        assert!((area - std::f32::consts::PI * 20.0 * 20.0).abs() < 40.0, "{area}");
        assert!(p.map.chunks(4).all(|c| c[..3] == [0, 0, 0]), "a field has no sprite");
        // The multiplier scales the radius: x 2 halves it.
        let half = build(&[planet(Vec2::new(3200.0, 3200.0), 250.0, 2.0)], &square(6400.0), &mut textures);
        assert!(pixel(&half.field, 64 + 9, 64) == FIELD_COLOUR && pixel(&half.field, 64 + 11, 64) == [0, 0, 0]);
        // Overlapping fields: every texel is the colour, never a sum.
        let two = build(&[planet(Vec2::new(3200.0, 3200.0), 250.0, 4.0), planet(Vec2::new(3400.0, 3200.0), 250.0, 4.0)], &square(6400.0), &mut textures);
        assert!(two.field.chunks(4).all(|c| c[..3] == FIELD_COLOUR || c[..3] == [0, 0, 0]));
        assert_eq!(pixel(&two.field, 66, 64), FIELD_COLOUR);
        assert!(two.field.chunks(4).filter(|c| c[..3] == FIELD_COLOUR).count() > p.field.chunks(4).filter(|c| c[..3] == FIELD_COLOUR).count());
    }

    /// A map that is not square sits in the middle of the square, as the radar's own map box.
    #[test]
    fn a_map_that_is_not_square_keeps_its_aspect_in_the_grid() {
        let map = RadarMap::of_bounds(Vec2::ZERO, Vec2::new(6400.0, 3200.0));
        // The map's centre is texel 64 on both axes; a nebula at its east edge's middle is at x = 127.
        let p = build(&[nebula(Vec2::new(3200.0, 1600.0), 800.0), nebula(Vec2::new(6400.0, 1600.0), 800.0)], &map, &mut textures);
        assert_eq!(pixel(&p.map, 64, 64), [100, 50, 0]);
        assert_eq!(pixel(&p.map, 127, 64), [100, 50, 0]);
        // The strips: the map is the middle half of the square's height.
        let s = strips(&map, Some((Vec2::ZERO, Vec2::new(6400.0, 3200.0))));
        assert_eq!(s[0][2] + s[1][2], 0.0);
        assert!((s[2][3] - 0.25).abs() < 1e-5 && (s[3][1] - 0.75).abs() < 1e-5 && (s[3][3] - 0.25).abs() < 1e-5);
        // A square map has none.
        assert_eq!(strips(&square(6400.0), Some((Vec2::ZERO, Vec2::splat(6400.0)))), [[0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0], [0.0, 1.0, 1.0, 0.0]]);
    }

    #[test]
    fn the_keys_come_from_the_definition() {
        let k = MapKeys::of(&odf("classLabel = \"planet\"\nmapIcon = \"icon_a\"\nmapSprite = \"\"\nmapSize = 4\n"));
        assert_eq!((k.icon.as_deref(), k.sprite.as_deref(), k.size), (Some("icon_a"), None, Some(4.0)));
        assert!(MapKeys::of(&odf("classLabel = \"craft\"\n")).is_empty());
        assert!(MapKeys::of(&odf("mapSprite = \"NONE\"\nmapIcon = \" none \"\n")).is_empty());
    }

    /// The units that carry the keys, a planet with a field and a nebula-like marker with a
    /// sprite, in a sim, and the sprite table that names the sprite.
    struct Scene {
        app: App,
        sim: Sim,
        db: Arc<ContentDb>,
        planet: sta_sim::ContentKey,
        ship: sta_sim::ContentKey,
    }

    fn scene() -> Scene {
        let fake = sta_content::fake::db();
        let mut builder = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for unit in fake.units() {
            builder = builder.unit(unit.clone());
        }
        for weapon in fake.weapons() {
            builder = builder.weapon(weapon.clone());
        }
        let mut planet = UnitDef::new("lt_planet", "planet");
        planet.raw = Arc::new(odf("classLabel = \"planet\"\nmapSprite = \"sprite_a\"\nmapIcon = \"icon_a\"\n"));
        planet.radius = Fixed::from_int(250);
        let db = builder.unit(planet).build().unwrap().with_rules(Rules::from_header("int cfgPLANET_DEWARP_RADIUS_MULTIPLIER = 4;\n").0);
        let db = Arc::new(db);
        let seat = |id, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
        let setup = GameSetup {
            seed: 1,
            map: Arc::new(sta_content::fake::two_bases()),
            content: db.clone(),
            players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
            options: GameOptions::default(),
        };
        let sim = Sim::from_setup(&setup);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Assets::<Image>::default())
            .insert_resource(Assets::<AdditiveImage>::default())
            .init_resource::<RadarFeed>()
            .init_resource::<RadarState>()
            .init_resource::<RadarTextures>()
            .init_resource::<LayerState>()
            .init_resource::<RadarStatics>()
            .insert_resource(SpriteTables(Arc::new(SpriteTable::parse(b"sprite_table\n@reference=8\nsprite_a\ttex_a\t0\t0\t8\t8\nicon_a\ttex_i\t0\t0\t4\t4\n"))))
            .add_systems(Update, update_layers);
        app.world_mut().resource_mut::<RadarTextures>().insert("tex_a", Texture { width: 8, height: 8, rgba: flat(8, 8, [100, 50, 0]).rgba.clone() });
        app.world_mut().spawn(RadarSquare);
        let planet = db.key("lt_planet").unwrap();
        let ship = db.key("fx_cruiser").unwrap();
        Scene { app, sim, db, planet, ship }
    }

    impl Scene {
        fn spawn(&mut self, def: sta_sim::ContentKey, owner: u8, x: f32, z: f32) -> sta_sim::EntityId {
            self.sim.spawn_def(def, PlayerId(owner), SimVec3::from_f32([x, 0.0, z]), (Fixed::ZERO, Fixed::ONE)).unwrap()
        }

        fn show(&mut self) {
            let mut feed = self.app.world_mut().resource_mut::<RadarFeed>();
            // Everything seen (a view without visibility data).
            let mut view = (*WorldSource::view(&self.sim)).clone();
            view.visibility.clear();
            feed.view = Arc::new(view);
            feed.content = Some(self.db.clone());
            feed.player = PlayerId(0);
            self.app.update();
        }
    }

    /// The pictures are built from the objects once, and again only when an object is created or
    /// destroyed (or the square changes), never per frame; the field radius is read through the
    /// rules.
    #[test]
    fn the_layers_are_rebuilt_only_when_an_object_appears_or_disappears() {
        let mut s = scene();
        s.app.world_mut().resource_mut::<RadarState>().map = Some(square(6400.0));
        s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 3200.0, 3200.0);
        s.show();
        assert_eq!(s.app.world().resource::<LayerState>().builds, 1);
        for _ in 0..3 {
            s.show();
        }
        assert_eq!(s.app.world().resource::<LayerState>().builds, 1, "a frame with nothing created or destroyed builds nothing");
        // Two nodes, each over the whole square, under the dots.
        let nodes: Vec<(LayerPicture, Node, i32)> = s.app.world_mut().query::<(&LayerPicture, &Node, &ZIndex)>().iter(s.app.world()).map(|(l, n, z)| (*l, n.clone(), z.0)).collect();
        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().all(|(_, n, z)| n.width == Val::Percent(100.0) && n.height == Val::Percent(100.0) && *z < 0));
        // The picture holds a sprite square and a field circle of 1000 world units (20 texels).
        let handles: Vec<Handle<AdditiveImage>> = s.app.world_mut().query::<&MaterialNode<AdditiveImage>>().iter(s.app.world()).map(|m| m.0.clone()).collect();
        let (mats, imgs) = (s.app.world().resource::<Assets<AdditiveImage>>(), s.app.world().resource::<Assets<Image>>());
        let whole = |h: &Handle<AdditiveImage>| imgs.get(&mats.get(h).unwrap().image).unwrap().data.clone().unwrap();
        let pictures: Vec<Vec<u8>> = handles.iter().map(whole).collect();
        assert!(pictures.iter().any(|p| pixel(p, 64 + 19, 64) == FIELD_COLOUR && pixel(p, 64 + 20, 64) == [0, 0, 0]));
        assert!(pictures.iter().any(|p| pixel(p, 64 + 2, 64 - 2) == [100, 50, 0]), "the sprite is 250 units = 5 texels wide");
        // A ship moving about is no object of the layers.
        s.spawn(s.ship, 0, 100.0, 100.0);
        s.show();
        assert_eq!(s.app.world().resource::<LayerState>().builds, 1);
        // A second planet is a created object; the square changing rebuilds too.
        s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 1000.0, 1000.0);
        s.show();
        assert_eq!(s.app.world().resource::<LayerState>().builds, 2);
        s.app.world_mut().resource_mut::<RadarState>().map = Some(square(6500.0));
        s.show();
        assert_eq!(s.app.world().resource::<LayerState>().builds, 3);
        assert_eq!(s.app.world_mut().query::<&LayerPicture>().iter(s.app.world()).count(), 2, "the nodes are kept, only their picture changes");
    }

    /// The map's statics (nebulae, belts) are objects of the layers beside the units'.
    #[test]
    fn the_maps_statics_join_the_units() {
        let s = scene();
        let view = WorldSource::view(&s.sim);
        let statics = RadarStatics { layers: vec![nebula(Vec2::new(10.0, 20.0), 300.0)], ..default() };
        let objects = layer_objects(&view, Some(&s.db), &statics, &DefCache::default());
        assert_eq!(objects, statics.layers);
    }

    /// The map path that had the nebula bug: map objects of the nebula and belt classes with a
    /// `mapSprite` key are taken from the map (they are regions of the sim, not sim units, though
    /// the content database defines them), at the shared radius, once each; hidden objects, other
    /// classes and objects without the key are not.
    #[test]
    fn the_map_feeds_its_nebulae_and_belts_once_even_when_the_content_defines_them() {
        let def = |name: &str, class: &str, extra: &str| {
            let mut d = UnitDef::new(name, class);
            d.raw = Arc::new(odf(&format!("classLabel = \"{class}\"\n{extra}")));
            d
        };
        let db = ContentDb::builder()
            .unit(def("lt_nebula", "nebula_obj", "mapSprite = \"sprite_a\"\neffectRadius = 450\nmapSize = 5\n"))
            .unit(def("lt_default_nebula", "nebula_obj", "mapSprite = \"sprite_a\"\n"))
            .unit(def("lt_belt", "asteroidfield", "mapSprite = \"sprite_a\"\nfieldRadius = 125\n"))
            .unit(def("lt_hidden", "nebula_obj", "mapSprite = \"sprite_a\"\nhidden = 1\n"))
            .unit(def("lt_bare_nebula", "nebula_obj", "effectRadius = 450\n"))
            .unit(def("lt_planet_like", "planet", "mapSprite = \"sprite_a\"\n"))
            .build()
            .unwrap();
        let at = |odf: &str, x: f32, z: f32| {
            let mut transform = [0.0; 12];
            (transform[9], transform[11]) = (x, z);
            sta_bzn::MapObject { odf: odf.into(), transform, ..Default::default() }
        };
        let map = sta_bzn::Map {
            objects: vec![at("lt_nebula", 100.0, 200.0), at("lt_default_nebula", 1.0, 2.0), at("lt_belt", 300.0, 400.0), at("lt_hidden", 5.0, 5.0), at("lt_bare_nebula", 6.0, 6.0), at("lt_planet_like", 7.0, 7.0), at("lt_unknown", 8.0, 8.0)],
            ..Default::default()
        };
        let statics = RadarStatics::of_map(&map, false, |name| db.key(name).and_then(|key| db.unit(key)));
        let sprite = Some("sprite_a".to_owned());
        let obj = |x, z, radius| LayerObject { at: Vec2::new(x, z), radius, size_texels: None, sprite: sprite.clone(), warp_radius: None };
        assert_eq!(statics.layers, [obj(100.0, 200.0, 450.0), obj(1.0, 2.0, 300.0), obj(300.0, 400.0, 250.0)]);
        let a1 = RadarStatics::of_map(&map, true, |name| db.key(name).and_then(|key| db.unit(key)));
        assert_eq!(a1.layers.iter().map(|o| o.size_texels).collect::<Vec<_>>(), [Some(5.0), Some(1.0), Some(1.0)]);

    }

    /// A planet's keys: its sprite is in the picture, its field is its radius times the rule's
    /// multiplier, and it has an icon so no dot.
    #[test]
    fn a_planet_carries_a_sprite_a_field_and_an_icon() {
        let mut s = scene();
        s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 3200.0, 3200.0);
        let view = WorldSource::view(&s.sim);
        let defs = DefCache::default();
        let o = layer_objects(&view, Some(&s.db), &RadarStatics::default(), &defs);
        assert_eq!(o, [LayerObject { at: Vec2::new(3200.0, 3200.0), radius: 250.0, size_texels: None, sprite: Some("sprite_a".into()), warp_radius: Some(1000.0) }]);
        let table = SpriteTable::parse(b"sprite_table\n@reference=8\nicon_a\ttex_i\t0\t0\t4\t4\n");
        let icon = |db: &Arc<ContentDb>, def| icon_info(&defs.get(db, def).unwrap().keys, &table).map(|i| (i.texture, i.texels));
        assert_eq!(icon(&s.db, s.planet), Some(("tex_i".into(), Vec2::splat(4.0))));
        assert_eq!(icon(&s.db, s.ship), None);
        // `mapSize` makes the icon at least that wide.
        let mut big = UnitDef::new("lt_big", "planet");
        big.raw = Arc::new(odf("mapIcon = \"icon_a\"\nmapSize = 9\n"));
        let db = Arc::new(ContentDb::builder().unit(big).build().unwrap());
        assert_eq!(icon(&db, db.key("lt_big").unwrap()), Some(("tex_i".into(), Vec2::splat(9.0))));
    }

    /// The icons: one node per seen unit with a `mapIcon`, its size and place shares of the
    /// square, so they are the same at any minimap size.
    #[test]
    fn icons_are_nodes_of_the_squares_own_shares() {
        let mut s = scene();
        s.app.add_systems(Update, update_icons);
        {
            let mut state = s.app.world_mut().resource_mut::<RadarState>();
            state.map = Some(square(6400.0));
            state.stations = super::super::radar::Filter::Enemies;
        }
        s.app.world_mut().resource_mut::<RadarTextures>().pixels.remove("tex_a");
        s.app.world_mut().resource_mut::<RadarTextures>().insert("tex_i", Texture { width: 4, height: 4, rgba: flat(4, 4, [200, 100, 50]).rgba.clone() });
        s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 1600.0, 4800.0);
        s.show();
        s.show();
        let icons: Vec<Node> = s.app.world_mut().query::<(&RadarIcon, &Node)>().iter(s.app.world()).map(|(_, n)| n.clone()).collect();
        assert_eq!(icons.len(), 1);
        let n = &icons[0];
        // 4 texels of 128 = 3.125% of the square, centred a quarter in and a quarter down.
        assert_eq!((n.left, n.top, n.width, n.height), (Val::Percent(23.4375), Val::Percent(23.4375), Val::Percent(3.125), Val::Percent(3.125)));
    }

    #[test]
    fn static_icons_crop_the_atlas_and_preserve_aspect() {
        let mut s = scene();
        s.app.add_systems(Update, update_icons);
        s.app.world_mut().resource_mut::<RadarState>().map = Some(square(6400.0));
        s.app.insert_resource(SpriteTables(Arc::new(SpriteTable::parse(
            b"sprite_table\n@reference=4\nicon_a\ttex_i\t1\t1\t2\t1\n",
        ))));
        let mut pixels = vec![255; 4 * 4 * 4];
        pixels[20..28].copy_from_slice(&[0, 100, 0, 255, 0, 200, 0, 255]);
        s.app.world_mut().resource_mut::<RadarTextures>()
            .insert("tex_i", Texture { width: 4, height: 4, rgba: pixels });
        let mut region = UnitDef::new("lt_region", "nebula_obj");
        region.raw = Arc::new(odf("classLabel = \"nebula_obj\"\nmapIcon = \"icon_a\"\n"));
        let db = ContentDb::builder().unit(region).build().unwrap();
        let mut object = sta_bzn::MapObject { odf: "lt_region".into(), ..default() };
        object.transform[9] = 3200.0;
        object.transform[11] = 3200.0;
        s.app.insert_resource(RadarStatics::of_map(&sta_bzn::Map { objects: vec![object], ..default() }, false, |name| db.key(name).and_then(|key| db.unit(key))));
        s.show();
        let w = s.app.world_mut();
        let mut icons = w.query::<(&RadarIcon, &Node, &MaterialNode<AdditiveImage>)>();
        let (icon, node, material) = icons.single(w).unwrap();
        assert_eq!(*icon, RadarIcon::Static(0));
        assert_eq!(node.width, Val::Percent(2.0 / GRID as f32 * 100.0));
        assert_eq!(node.height, Val::Percent(1.0 / GRID as f32 * 100.0));
        let mat = w.resource::<Assets<AdditiveImage>>().get(&material.0).unwrap();
        let image = w.resource::<Assets<Image>>().get(&mat.image).unwrap();
        assert_eq!(image.data.as_deref(), Some([0, 100, 0, 255, 0, 200, 0, 255].as_slice()));
        assert!(matches!(
            &image.sampler,
            ImageSampler::Descriptor(desc)
                if desc.mag_filter == bevy::image::ImageFilterMode::Linear
                    && desc.min_filter == bevy::image::ImageFilterMode::Linear
        ));
    }

    #[test]
    fn authored_icons_follow_owner_mark_colours_and_keep_neutral_untinted() {
        use crate::viewer::{MapTeamColours, TeamColours};

        let mut s = scene();
        s.app.add_systems(Update, update_icons);
        s.app.world_mut().resource_mut::<RadarState>().map = Some(square(6400.0));
        s.app.world_mut().resource_mut::<RadarTextures>().pixels.remove("tex_a");
        s.app.world_mut().resource_mut::<RadarTextures>().insert("tex_i", Texture { width: 4, height: 4, rgba: flat(4, 4, [200, 100, 50]).rgba.clone() });
        let owned = s.spawn(s.planet, 0, 1600.0, 4800.0);
        let neutral = s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 4800.0, 1600.0);
        let tint_of = |app: &mut App, id| {
            let material = app.world_mut().query::<(&RadarIcon, &MaterialNode<AdditiveImage>)>()
                .iter(app.world())
                .find(|(icon, _)| **icon == RadarIcon::Unit(id))
                .map(|(_, material)| material.0.clone())
                .unwrap();
            app.world().resource::<Assets<AdditiveImage>>().get(&material).unwrap().tint
        };
        s.show();
        // The sim initially spawns planets as neutral. Supply the owned view
        // the radar receives after colonisation.
        let mut view = (*s.app.world().resource::<RadarFeed>().view).clone();
        view.units.iter_mut().find(|u| u.id == owned).unwrap().owner = PlayerId(0);
        s.app.world_mut().resource_mut::<RadarFeed>().view = Arc::new(view);
        s.app.update();
        assert_eq!(tint_of(&mut s.app, owned), UNTINTED);
        assert_eq!(tint_of(&mut s.app, neutral), UNTINTED);

        s.app.insert_resource(TeamColours(vec![("empty".into(), Color::WHITE), ("red".into(), Color::srgb(1.0, 0.0, 0.0)), ("blue".into(), Color::srgb(0.0, 0.0, 1.0))]));
        s.app.update();
        assert_eq!(tint_of(&mut s.app, owned), Vec4::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(tint_of(&mut s.app, neutral), UNTINTED);
        s.app.insert_resource(MapTeamColours(vec![[0, 0], [0, 3], [0, 2]]));
        s.app.update();
        assert_eq!(tint_of(&mut s.app, owned), Vec4::new(0.0, 0.0, 1.0, 1.0));
        // The icon node survives a capture; its material must follow the new owner.
        let mut captured = (*s.app.world().resource::<RadarFeed>().view).clone();
        captured.units.iter_mut().find(|u| u.id == owned).unwrap().owner = PlayerId(1);
        s.app.world_mut().resource_mut::<RadarFeed>().view = Arc::new(captured);
        s.app.update();
        assert_eq!(tint_of(&mut s.app, owned), Vec4::new(1.0, 0.0, 0.0, 1.0));
        s.app.world_mut().remove_resource::<TeamColours>();
        s.app.update();
        assert_eq!(tint_of(&mut s.app, owned), UNTINTED);
    }

    #[test]
    fn usable_artwork_replaces_the_dot_in_layer_icon_order() {
        use super::super::radar::{Kind, blips};
        let mut s = scene();
        s.app.add_systems(Update, update_icons);
        let map = square(6400.0);
        s.app.world_mut().resource_mut::<RadarState>().map = Some(map);
        let planet = s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 3200.0, 3200.0);
        let ship = s.spawn(s.ship, 0, 100.0, 100.0);
        let dot = |app: &App, id| {
            let feed = app.world().resource::<RadarFeed>();
            let state = app.world().resource::<RadarState>();
            blips(feed, state, &map).into_iter().find(|b| b.id == id).map(|b| (b.kind, b.texels))
        };
        let icons = |app: &mut App| app.world_mut().query::<&RadarIcon>().iter(app.world()).count();
        s.show();
        assert_eq!(dot(&s.app, ship), Some((Kind::Ship, SHIP_TEXELS)));
        assert_eq!(dot(&s.app, planet), None);
        assert_eq!(icons(&mut s.app), 0, "mapSprite takes precedence");

        let mut textures = s.app.world_mut().resource_mut::<RadarTextures>();
        textures.pixels.remove("tex_a");
        textures.insert("tex_i", (*flat(4, 4, [200, 100, 50])).clone());
        s.show();
        assert_eq!(dot(&s.app, planet), None);
        assert_eq!(icons(&mut s.app), 1, "missing mapSprite falls back to mapIcon");

        s.app.world_mut().resource_mut::<RadarTextures>().clear();
        s.show();
        assert_eq!(dot(&s.app, planet), Some((Kind::Body, OTHER_TEXELS)));
        assert_eq!(icons(&mut s.app), 0, "missing mapIcon falls back to the dot");
    }

    /// Both looks draw authored icons. The stock look still uses its own fog and strips.
    #[test]
    fn both_hud_looks_draw_authored_minimap_artwork() {
        use super::super::radar::{RadarBlip, RadarPlugin, blips};
        use crate::render::fog::{FogGrid, FogState};
        use crate::ui::hud::HudConfig;
        use crate::ui::profile::UiProfile;
        for stock in [false, true] {
            let mut s = scene();
            let planet = s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 3200.0, 1600.0);
            s.spawn(s.ship, 0, 100.0, 100.0);
            let mut app = App::new();
            app.add_plugins(MinimalPlugins).insert_resource(Assets::<Image>::default()).insert_resource(Assets::<AdditiveImage>::default());
            crate::app::configure_sets(&mut app);
            if stock {
                let cfg = "screenWidth = 1600\nscreenHeight = 1200\nminimapPanelArea = 0 800 400 400\nminimapDisplayArea = 10 20 320 280\n";
                let (parsed, warnings) = sta_content::GuiCfg::parse("radar.cfg", cfg);
                assert!(warnings.is_empty(), "{warnings:?}");
                app.insert_resource(UiProfile::Armada2).insert_resource(HudConfig { gui: Some(parsed.layout((1600, 1200))), aspect: 16.0 / 9.0, ..default() });
            }
            app.add_plugins(RadarPlugin)
                .insert_resource(SpriteTables(Arc::new(SpriteTable::parse(b"sprite_table\n@reference=8\nsprite_a\ttex_a\t0\t0\t8\t8\nicon_a\ttex_i\t0\t0\t4\t4\n"))))
                .insert_resource(FogState { grid: FogGrid::new([0.0, 0.0], [6400.0, 3200.0]), updates: 1 });
            let map = RadarMap::of_bounds(Vec2::ZERO, Vec2::new(6400.0, 3200.0));
            {
                let mut st = app.world_mut().resource_mut::<RadarState>();
                st.map = Some(map);
                st.map_fixed = true;
                st.bounds = Some((Vec2::ZERO, Vec2::new(6400.0, 3200.0)));
            }
            app.world_mut().resource_mut::<RadarTextures>().insert("tex_i", Texture { width: 4, height: 4, rgba: flat(4, 4, [200, 100, 50]).rgba.clone() });
            let mut view = (*WorldSource::view(&s.sim)).clone();
            view.visibility.clear();
            {
                let mut feed = app.world_mut().resource_mut::<RadarFeed>();
                feed.view = Arc::new(view);
                feed.content = Some(s.db.clone());
                feed.player = PlayerId(0);
            }
            app.update();
            app.update();
            let w = app.world_mut();
            let count = |w: &mut World, f: &mut dyn FnMut(&mut World) -> usize| f(w);
            let dots = count(w, &mut |w| w.query::<&RadarBlip>().iter(w).count());
            let planet_dot = w.query::<&RadarBlip>().iter(w).any(|b| b.0 == planet);
            let icons = count(w, &mut |w| w.query::<&RadarIcon>().iter(w).count());
            let strips = count(w, &mut |w| w.query::<&RadarStrip>().iter(w).count());
            let pictures = count(w, &mut |w| w.query::<&LayerPicture>().iter(w).count());
            let expected = {
                let feed = w.resource::<RadarFeed>().clone();
                blips(&feed, w.resource::<RadarState>(), &map).len()
            };
            assert_eq!(dots, expected);
            assert_eq!(pictures, 2, "both looks gain the two pictures");
            let square = w.query_filtered::<Entity, With<RadarSquare>>().single(w).unwrap();
            let fog = w.get::<Children>(square).unwrap().iter().find(|e| w.get::<crate::ui::radar_fog::RadarFog>(*e).is_some()).unwrap();
            let fog_z = w.get::<ZIndex>(fog).map(|z| z.0);
            if stock {
                assert!(!planet_dot && icons == 1 && strips == 4 && fog_z == Some(1), "{planet_dot} {icons} {strips} {fog_z:?}");
            } else {
                assert!(!planet_dot && icons == 1 && strips == 0 && fog_z == Some(0), "{planet_dot} {icons} {strips} {fog_z:?}");
                assert_eq!(w.get::<Children>(square).unwrap()[0], fog);
            }
        }
    }

    /// A GPU frame of the additive material (`STA64_GPU_TESTS=1`; skips without a hardware
    /// adapter): the picture is added to what is under it (not blended), sampled linearly, its
    /// alpha is ignored, and `rect` picks the part drawn.
    #[test]
    fn the_additive_material_filters_linearly_and_ignores_alpha_on_the_gpu() {
        use bevy::camera::RenderTarget;
        use bevy::ui::UiTargetCamera;
        use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
        use std::sync::Mutex;
        if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
            eprintln!("SKIP: set STA64_GPU_TESTS=1");
            return;
        }
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() })
                .set(bevy::render::RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<bevy::gilrs::GilrsPlugin>()
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::log::LogPlugin>(),
        );
        crate::app::configure_sets(&mut app);
        app.init_resource::<RadarFeed>().init_resource::<RadarState>().add_plugins(RadarLayersPlugin);
        let mut target = Image::new_target_texture(64, 96, TextureFormat::Rgba8UnormSrgb, None);
        target.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
        let camera = app
            .world_mut()
            .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
            .id();
        // 2 x 2 texels: a, b over c, d; alpha almost 0 everywhere.
        let texels: [[u8; 4]; 4] = [[200, 0, 0, 10], [0, 200, 0, 10], [0, 0, 200, 10], [100, 100, 0, 10]];
        let picture = radar_image(2, 2, texels.concat());
        let picture = app.world_mut().resource_mut::<Assets<Image>>().add(picture);
        let (whole, right, tinted) = {
            let mut m = app.world_mut().resource_mut::<Assets<AdditiveImage>>();
            (
                m.add(AdditiveImage { rect: Vec4::new(0.0, 0.0, 1.0, 1.0), image: picture.clone(), tint: UNTINTED }),
                m.add(AdditiveImage { rect: Vec4::new(0.5, 0.0, 0.5, 1.0), image: picture.clone(), tint: UNTINTED }),
                m.add(AdditiveImage { rect: Vec4::new(0.5, 0.5, 0.5, 0.5), image: picture, tint: Vec4::new(0.0, 1.0, 1.0, 1.0) }),
            )
        };
        let fill = Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Px(64.0), ..default() };
        // Left half of the frame: the whole picture over black. Right half: over a grey backing,
        // and the right half of the picture drawn across all of it.
        app.world_mut().spawn((
            Node { position_type: PositionType::Absolute, width: Val::Px(64.0), height: Val::Px(96.0), ..default() },
            UiTargetCamera(camera),
            children![
                (Node { left: Val::Px(0.0), width: Val::Px(32.0), ..fill.clone() }, MaterialNode(whole)),
                (Node { left: Val::Px(32.0), width: Val::Px(32.0), ..fill.clone() }, BackgroundColor(Color::srgb_u8(60, 60, 60))),
                (Node { left: Val::Px(32.0), width: Val::Px(32.0), ..fill.clone() }, MaterialNode(right)),
                (Node { top: Val::Px(64.0), width: Val::Px(32.0), height: Val::Px(32.0), ..fill }, MaterialNode(tinted)),
            ],
        ));
        app.finish();
        app.cleanup();
        for _ in 0..16 {
            app.update();
        }
        // As the repo's other GPU tests: the main world first, then the render sub-app.
        let adapter = app
            .world()
            .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
            .cloned()
            .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()));
        let Some(adapter) = adapter else {
            panic!("STA64_GPU_TESTS=1 is set but there is no render adapter");
        };
        eprintln!("GPU adapter: {} ({:?})", adapter.name, adapter.device_type);
        if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
            eprintln!("SKIP: hardware adapter unavailable ({})", adapter.name);
            return;
        }
        let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
        let out = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| out.lock().unwrap().push(shot.image.clone()));
        for _ in 0..60 {
            app.update();
            if !frames.lock().unwrap().is_empty() {
                break;
            }
        }
        let shot = frames.lock().unwrap().first().cloned().expect("a GPU capture");
        let rgba = shot.try_into_dynamic().unwrap().to_rgba8();
        let at = |x: u32, y: u32| {
            let p = rgba.get_pixel(x, y);
            [p[0], p[1], p[2]]
        };
        let near = |got: [u8; 3], want: [u8; 3]| got.iter().zip(want).all(|(g, w)| (i16::from(*g) - i16::from(w)).abs() <= 3);
        // Away from the transition each texel keeps its colour even with nearly zero alpha.
        assert!(near(at(6, 8), [200, 0, 0]) && near(at(26, 8), [0, 200, 0]) && near(at(6, 56), [0, 0, 200]), "{:?} {:?} {:?}", at(6, 8), at(26, 8), at(6, 56));
        // Magnification blends neighbouring colours across the boundary.
        for x in [15, 16] {
            let [r, g, b] = at(x, 8);
            assert!(r > 100 && g > 100 && b < 5, "linear edge: {:?}", at(x, 8));
        }
        // Right half over the grey: the sum in linear light, encoded again; `rect` drew the right
        // texel column (b over d) across the 32 columns.
        let lin = |v: u8| LinearRgba::from(Srgba::rgb_u8(v, v, v)).red;
        let sum = |back: u8, texel: u8| {
            let l = lin(back) + lin(texel);
            let s = Srgba::from(LinearRgba::rgb(l, l, l)).red;
            (s * 255.0).round() as u8
        };
        let want = [sum(60, 0), sum(60, 200), sum(60, 0)];
        assert!(near(at(48, 8), want), "{:?} vs {want:?}", at(48, 8));
        let want = [sum(60, 100), sum(60, 100), sum(60, 0)];
        assert!(near(at(48, 56), want), "{:?} vs {want:?}", at(48, 56));
        assert!(near(at(16, 80), [0, 100, 100]), "cyan tint on a yellow texel: {:?}", at(16, 80));
    }

    /// With `STA64_TEST_INSTALL` (a mounted install): every `mapSprite` and `mapIcon` key of its
    /// definitions names a sprite of the install's sprite table whose texture decodes, and at
    /// least one icon, one nebula with a sprite and one belt with a sprite (the map's static
    /// layers) must have been seen. Skips without an install.
    #[test]
    fn install_map_sprite_and_icon_keys_resolve_through_the_sprite_table() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let table = sta_assets::load_sprite_table(&vfs);
        let check = |odf: &str, keys: &MapKeys| {
            for name in keys.sprite.iter().chain(keys.icon.iter()) {
                let (texture, uv, _) = sprite_rect(&table, name).unwrap_or_else(|| panic!("{odf}: sprite {name} is not in the table"));
                assert!(uv[2] > 0.0 && uv[3] > 0.0, "{name}: empty rectangle");
                let found = sta_assets::find_texture(&vfs, &texture).unwrap_or_else(|| panic!("{name}: texture {texture} not found"));
                sta_assets::decode_rgba(&vfs, &found.0, found.1).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        };
        let (mut nebulae, mut belts, mut sprites, mut icons) = (0, 0, 0, 0);
        for def in db.units() {
            let keys = MapKeys::of(&def.raw);
            check(&def.odf, &keys);
            sprites += usize::from(keys.sprite.is_some());
            icons += usize::from(keys.icon.is_some());
            match def.raw.class_label().map(str::to_ascii_lowercase).as_deref() {
                Some("nebula_obj") if keys.sprite.is_some() => nebulae += 1,
                Some("asteroidfield") if keys.sprite.is_some() => belts += 1,
                _ => {}
            }
        }
        assert!(icons > 0, "no definition with a map icon was seen");
        assert!(nebulae > 0 && belts > 0, "a nebula and a belt with a map sprite must be seen (the map's static layers)");
        assert!(sprites >= nebulae + belts);
        eprintln!("{nebulae} nebula, {belts} belt, {icons} icon definitions resolved");
    }
}
