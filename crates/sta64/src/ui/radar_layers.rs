//! The minimap's static layers and sprite icons (#1042, part b).
//!
//! What the stock look draws, bottom to top (read from the executable, static): the backing, the
//! map picture, the field picture, the map-icon sprites, the dots, fog and shroud, the camera
//! outline, and black strips where the square is not map.
//!
//! - **Map picture** (one picture at the texel grid [`GRID`]): each object whose definition has the
//!   `mapSprite` key adds that sprite's texture, stretched into a square centred on the object, as
//!   wide as its bounding radius in texels (`radius / side * GRID`, truncated; the radius, not the
//!   diameter, is the width). Read from the executable, static; the nebula and belt footprints
//!   are INTERIM (below).
//! - **Field picture**: each object with a warp-suppression radius
//!   ([`sta_content::Rules::warp_suppression_radius`]: bounding radius times the class's
//!   `RTS_CFG.h` multiplier, ADR 0004 D9/D10: render only, not digested) fills a circle in
//!   [`FIELD_COLOUR`]; texels are written, not accumulated, so overlapping fields are a union.
//!   Planets: read from the executable, static. Black holes: INTERIM, inferred from the key's
//!   name, their rule was not read in the executable.
//! - Both pictures are drawn additively with nearest sampling ([`AdditiveImage`]) and rebuilt only
//!   when the set of objects, the radar's square or the sprite source changes (an object is created
//!   or destroyed), never per frame.
//! - **Icons** (stock look): an object whose definition has the `mapIcon` key is drawn as that
//!   sprite, additively, and gets no dot; its width is the sprite's width in texels of [`GRID`], at
//!   least the definition's `mapSize` key. Read from the executable, static.
//!
//! Which look gets what (decided for #1042): STA64's own (Remaster) HUD gains ONLY the two pictures,
//! drawn under its existing dots; it keeps its dots for every object (also those with a `mapIcon`
//! key), its draw order of dots against fog, its selection ring, and has no strips. Icons replacing
//! dots, the original's draw order with fog over the dots, black strips and square dots without a
//! ring are the STOCK look only.
//!
//! Constants without a data key (the texel grid, the field colour, the default dot sizes) are code
//! constants, each graded where it is declared.
//!
//! INTERIM: the nebula and asteroid-belt footprint is STA64's existing drawn radius
//! ([`nebula_radius`], [`field_radius`]), markedly smaller than the original's footprint for a
//! nebula (measured once), because the class radius was not read exactly. The appearance with fog
//! on was not measured. Fleet Operations and Roots may draw their minimap objects differently (not
//! measured); here they get the same layers as the other games. Armada I has the same data keys;
//! its executable was not read.

use std::collections::BTreeMap;
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
use sta_content::{ContentDb, UnitDef};
use sta_odf::Odf;
use sta_sim::EntityId;

use super::radar::{RadarFeed, RadarMap, RadarSquare, RadarState, StockRadar};
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
    /// The bounding radius, world units.
    pub radius: f32,
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
        let key = |k: &str| raw.str(k).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
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

/// A nebula's and an asteroid field's radius: `sta_content`'s one reading of the keys (the
/// sim's region conversion and the map view use it too). INTERIM as the minimap's footprint: the
/// original's footprint for a nebula is markedly larger than this (measured once), because its
/// class radius was not read exactly.
pub use sta_content::{field_radius, nebula_radius};

/// What the minimap reads of a definition, cached per definition so a frame reads no keys.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DefInfo {
    pub keys: MapKeys,
    /// The bounding radius, world units (zero: not read).
    pub radius: f32,
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
                    Arc::new(DefInfo { keys: MapKeys::of(&d.raw), radius: d.radius.to_f32(), warp_radius: db.rules().warp_suppression_radius(d).map(|r| r.to_f32()) })
                })
            })
            .clone()
    }
}

/// The objects of the map that are not units of the sim (nebulae, belts), set once when the map
/// is built. Units of the sim (planets, black holes) come from the view.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct RadarStatics(pub Vec<LayerObject>);

impl RadarStatics {
    /// The map's nebulae and belts that have a `mapSprite` (the map conversion makes them regions,
    /// not sim units, though the content database defines them): a nebula at its
    /// `effectRadius`, a belt at its `fieldRadius` (INTERIM footprints, see the module).
    pub fn of_map(map: &sta_bzn::Map, db: &ContentDb) -> RadarStatics {
        let mut out = Vec::new();
        for o in &map.objects {
            let Some(def) = db.key(&o.odf).and_then(|k| db.unit(k)) else { continue };
            let raw = &*def.raw;
            if raw.i64("hidden").unwrap_or(0) != 0 {
                continue;
            }
            let keys = MapKeys::of(raw);
            let radius = match raw.class_label().map(str::to_ascii_lowercase).as_deref() {
                Some("nebula_obj") => nebula_radius(raw),
                Some("asteroidfield") => field_radius(raw),
                _ => continue,
            };
            if keys.sprite.is_some() {
                out.push(LayerObject { at: Vec2::new(o.transform[9], o.transform[11]), radius, sprite: keys.sprite, warp_radius: None });
            }
        }
        RadarStatics(out)
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
            let side = (o.radius / map.size * GRID as f32) as i64;
            if side >= 1 && tex.width > 0 && tex.height > 0 {
                let (x0, y0) = ((c.x - side as f32 / 2.0).round() as i64, (c.y - side as f32 / 2.0).round() as i64);
                for j in 0..side {
                    let y = y0 + j;
                    if !(0..n).contains(&y) {
                        continue;
                    }
                    for i in 0..side {
                        let x = x0 + i;
                        if !(0..n).contains(&x) {
                            continue;
                        }
                        let (sx, sy) = ((i as usize * tex.width) / side as usize, (j as usize * tex.height) / side as usize);
                        let p = &tex.rgba[(sy * tex.width + sx) * 4..][..3];
                        let s = &mut sum[y as usize * GRID + x as usize];
                        for k in 0..3 {
                            s[k] += u32::from(p[k]);
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

    fn handle(&mut self, vfs: Option<&sta_vfs::Vfs>, images: &mut Assets<Image>, name: &str) -> Option<Handle<Image>> {
        let key = name.to_ascii_lowercase();
        if let Some(h) = self.handles.get(&key) {
            return Some(h.clone());
        }
        let tex = self.pixels(vfs, name)?;
        let mut image = Image::new(
            Extent3d { width: tex.width as u32, height: tex.height as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            tex.rgba.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::nearest();
        let handle = images.add(image);
        self.handles.insert(key, handle.clone());
        Some(handle)
    }

    fn clear(&mut self) {
        self.pixels.clear();
        self.handles.clear();
    }
}

/// A sprite of the table: its texture's name and its rectangle in 0..1.
fn sprite_rect(table: &SpriteTable, sprite: &str) -> Option<(String, [f32; 4], [f32; 2])> {
    let s = table.sprite(sprite)?;
    Some((s.texture.clone(), sta_assets::spr_anim::sprite_uv(table, s, 0.0), [s.rect[2], s.rect[3]]))
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
        .filter(|u| defs.get(db, u.def).is_some_and(|d| d.radius > 0.0 && (d.keys.sprite.is_some() || d.warp_radius.is_some())))
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
            let Some(d) = defs.get(db, u.def).filter(|d| d.radius > 0.0 && (d.keys.sprite.is_some() || d.warp_radius.is_some())) else { continue };
            out.push(LayerObject { at: Vec2::new(u.pos.x.to_f32(), u.pos.z.to_f32()), radius: d.radius, sprite: d.keys.sprite.clone(), warp_radius: d.warp_radius });
        }
    }
    out.extend(statics.0.iter().cloned());
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
    if mounted.as_ref().is_some_and(|m| m.is_changed()) {
        textures.clear();
    }
    let sig = feed.content.as_ref().map(|db| layer_signature(&feed.view, db, &state.defs)).unwrap_or_default();
    let have = kids.is_some_and(|k| k.iter().any(|e| pictures.get(e).is_ok()));
    let sprites = tables.is_some();
    let statics_changed = statics.as_ref().is_some_and(|s| s.is_changed());
    if have && !statics_changed && built.signature == sig && built.map == Some(map) && built.sprites == sprites && built.textures == textures.pixels.len() {
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
    let mut make = |bytes: Vec<u8>| {
        let mut image = Image::new(
            Extent3d { width: GRID as u32, height: GRID as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            bytes,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::nearest();
        images.add(image)
    };
    let (map_image, field_image) = (make(made.map), make(made.field));
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
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadarIcon(pub EntityId);

/// What an icon needs of its sprite: the texture's name, the rectangle drawn and the width in
/// texels (the sprite's width, at least the definition's `mapSize`).
#[derive(Debug, Clone, PartialEq)]
pub struct IconInfo {
    pub texture: String,
    pub uv: [f32; 4],
    pub texels: f32,
}

/// The icon of a definition: its `mapIcon` sprite resolved through the sprite table.
pub fn icon_info(info: &DefInfo, table: &SpriteTable) -> Option<IconInfo> {
    let name = info.keys.icon.as_deref()?;
    let (texture, uv, size) = sprite_rect(table, name)?;
    Some(IconInfo { texture, uv, texels: size[0].max(info.keys.size.unwrap_or(0.0)) })
}

/// The icons resolved per definition, for one content database and sprite table (by address).
type ResolvedIcons = (usize, usize, BTreeMap<sta_sim::ContentKey, Option<IconInfo>>);

/// Draw the `mapIcon` sprites of the units the player sees, additively, under the dots. The stock
/// look only: the other look keeps its dots for these objects.
#[allow(clippy::too_many_arguments)]
pub fn update_icons(
    mut commands: Commands,
    feed: Res<RadarFeed>,
    state: Res<RadarState>,
    teams: Option<Res<crate::viewer::TeamColours>>,
    map_teams: Option<Res<crate::viewer::MapTeamColours>>,
    tables: Option<Res<SpriteTables>>,
    mounted: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
    materials: Option<ResMut<Assets<AdditiveImage>>>,
    mut textures: ResMut<RadarTextures>,
    square: Query<(Entity, Has<StockRadar>), With<RadarSquare>>,
    mut icons: Query<(Entity, &RadarIcon, &mut Node, &MaterialNode<AdditiveImage>)>,
    mut found: Local<BTreeMap<EntityId, Entity>>,
    mut resolved: Local<ResolvedIcons>,
) {
    let (Some(mut images), Some(mut materials), Some(map), Ok((sq, stock))) = (images, materials, state.map, square.single()) else { return };
    let (Some(db), Some(tables), true) = (feed.content.as_ref(), tables, stock) else {
        for (e, _, _, _) in &icons {
            commands.entity(e).despawn();
        }
        found.clear();
        return;
    };
    let ids = (Arc::as_ptr(db) as usize, Arc::as_ptr(&tables.0) as usize);
    if (resolved.0, resolved.1) != ids {
        *resolved = (ids.0, ids.1, BTreeMap::new());
    }
    let vfs = mounted.as_ref().map(|m| &m.0);
    let mut want: BTreeMap<EntityId, (Vec2, sta_sim::ContentKey, sta_sim::PlayerId)> = BTreeMap::new();
    for u in feed.view.visible_to(feed.player) {
        if u.flags.contains(sta_sim::view::UnitFlags::DYING) {
            continue;
        }
        let Some(def) = state.defs.get(db, u.def) else { continue };
        if def.keys.icon.is_none() {
            continue;
        }
        resolved.2.entry(u.def).or_insert_with(|| icon_info(&def, &tables.0));
        if resolved.2.get(&u.def).is_some_and(Option::is_some) {
            want.insert(u.id, (map.to_square(Vec2::new(u.pos.x.to_f32(), u.pos.z.to_f32())), u.def, u.owner));
        }
    }
    found.retain(|id, e| {
        let keep = want.contains_key(id) && icons.get(*e).is_ok();
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for (id, (at, def, owner)) in want {
        let Some(Some(info)) = resolved.2.get(&def) else { continue };
        let tint = if owner.is_player() {
            crate::viewer::owner_mark_colour(teams.as_deref(), map_teams.as_deref(), owner)
                .map(|colour| {
                    let c = colour.to_linear();
                    Vec4::new(c.red, c.green, c.blue, 1.0)
                })
                .unwrap_or(UNTINTED)
        } else {
            UNTINTED
        };
        let side = info.texels / GRID as f32 * 100.0;
        let node = Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(at.x * 100.0),
            top: Val::Percent(at.y * 100.0),
            margin: UiRect::all(Val::Percent(-side / 2.0)),
            width: Val::Percent(side),
            height: Val::Percent(side),
            ..default()
        };
        if let Some(e) = found.get(&id) {
            if let Ok((_, _, mut n, material)) = icons.get_mut(*e) {
                if *n != node {
                    *n = node;
                }
                if materials.get(&material.0).is_some_and(|m| m.tint != tint)
                    && let Some(mut material) = materials.get_mut(&material.0)
                {
                    material.tint = tint;
                }
            }
            continue;
        }
        let Some(image) = textures.handle(vfs, &mut images, &info.texture) else { continue };
        let material = materials.add(AdditiveImage { rect: Vec4::from(info.uv), image, tint });
        let e = commands.spawn((RadarIcon(id), MaterialNode(material), node, ZIndex(-2), bevy::picking::Pickable::IGNORE, ChildOf(sq))).id();
        found.insert(id, e);
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
            (update_layers, update_icons, update_strips).after(super::radar::refresh).in_set(crate::app::GameSet::Ui),
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
        LayerObject { at, radius, sprite: Some("sprite_a".into()), warp_radius: None }
    }

    fn planet(at: Vec2, radius: f32, multiplier: f32) -> LayerObject {
        LayerObject { at, radius, sprite: None, warp_radius: Some(radius * multiplier) }
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
        let statics = RadarStatics(vec![nebula(Vec2::new(10.0, 20.0), 300.0)]);
        let objects = layer_objects(&view, Some(&s.db), &statics, &DefCache::default());
        assert_eq!(objects, statics.0);
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
            .unit(def("lt_nebula", "nebula_obj", "mapSprite = \"sprite_a\"\neffectRadius = 450\n"))
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
        let statics = RadarStatics::of_map(&map, &db);
        let sprite = Some("sprite_a".to_owned());
        let obj = |x, z, radius| LayerObject { at: Vec2::new(x, z), radius, sprite: sprite.clone(), warp_radius: None };
        assert_eq!(statics.0, [obj(100.0, 200.0, 450.0), obj(1.0, 2.0, 300.0), obj(300.0, 400.0, 125.0)]);
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
        assert_eq!(o, [LayerObject { at: Vec2::new(3200.0, 3200.0), radius: 250.0, sprite: Some("sprite_a".into()), warp_radius: Some(1000.0) }]);
        let table = SpriteTable::parse(b"sprite_table\n@reference=8\nicon_a\ttex_i\t0\t0\t4\t4\n");
        let icon = |db: &Arc<ContentDb>, def| icon_info(&defs.get(db, def).unwrap(), &table).map(|i| (i.texture, i.texels));
        assert_eq!(icon(&s.db, s.planet), Some(("tex_i".into(), 4.0)));
        assert_eq!(icon(&s.db, s.ship), None);
        // `mapSize` makes the icon at least that wide.
        let mut big = UnitDef::new("lt_big", "planet");
        big.raw = Arc::new(odf("mapIcon = \"icon_a\"\nmapSize = 9\n"));
        let db = Arc::new(ContentDb::builder().unit(big).build().unwrap());
        assert_eq!(icon(&db, db.key("lt_big").unwrap()), Some(("tex_i".into(), 9.0)));
    }

    /// The icons: one node per seen unit with a `mapIcon`, its size and place shares of the
    /// square, so they are the same at any minimap size.
    #[test]
    fn icons_are_nodes_of_the_squares_own_shares() {
        let mut s = scene();
        s.app.add_systems(Update, update_icons);
        s.app.world_mut().resource_mut::<RadarState>().map = Some(square(6400.0));
        let sq = s.app.world_mut().query_filtered::<Entity, With<RadarSquare>>().single(s.app.world()).unwrap();
        s.app.world_mut().entity_mut(sq).insert(StockRadar);
        s.app.world_mut().resource_mut::<RadarTextures>().insert("tex_i", Texture { width: 4, height: 4, rgba: flat(4, 4, [200, 100, 50]).rgba.clone() });
        s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 1600.0, 4800.0);
        s.show();
        s.show();
        let icons: Vec<Node> = s.app.world_mut().query::<(&RadarIcon, &Node)>().iter(s.app.world()).map(|(_, n)| n.clone()).collect();
        assert_eq!(icons.len(), 1);
        let n = &icons[0];
        // 4 texels of 128 = 3.125% of the square, centred a quarter in and a quarter down.
        assert_eq!((n.left, n.top, n.width, n.height), (Val::Percent(25.0), Val::Percent(25.0), Val::Percent(3.125), Val::Percent(3.125)));
        assert_eq!(n.margin.left, Val::Percent(-1.5625));
    }

    #[test]
    fn authored_icons_follow_owner_mark_colours_and_keep_neutral_untinted() {
        use crate::viewer::{MapTeamColours, TeamColours};

        let mut s = scene();
        s.app.add_systems(Update, update_icons);
        s.app.world_mut().resource_mut::<RadarState>().map = Some(square(6400.0));
        let sq = s.app.world_mut().query_filtered::<Entity, With<RadarSquare>>().single(s.app.world()).unwrap();
        s.app.world_mut().entity_mut(sq).insert(StockRadar);
        s.app.world_mut().resource_mut::<RadarTextures>().insert("tex_i", Texture { width: 4, height: 4, rgba: flat(4, 4, [200, 100, 50]).rgba.clone() });
        let owned = s.spawn(s.planet, 0, 1600.0, 4800.0);
        let neutral = s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 4800.0, 1600.0);
        let tint_of = |app: &mut App, id| {
            let material = app.world_mut().query::<(&RadarIcon, &MaterialNode<AdditiveImage>)>()
                .iter(app.world())
                .find(|(icon, _)| icon.0 == id)
                .map(|(_, material)| material.0.clone())
                .unwrap();
            app.world().resource::<Assets<AdditiveImage>>().get(&material).unwrap().tint
        };
        s.show();
        assert_eq!(tint_of(&mut s.app, owned), UNTINTED);
        assert_eq!(tint_of(&mut s.app, neutral), UNTINTED);

        s.app.insert_resource(TeamColours(vec![("empty".into(), Color::WHITE), ("red".into(), Color::srgb(1.0, 0.0, 0.0)), ("blue".into(), Color::srgb(0.0, 0.0, 1.0))]));
        s.show();
        assert_eq!(tint_of(&mut s.app, owned), Vec4::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(tint_of(&mut s.app, neutral), UNTINTED);
        s.app.insert_resource(MapTeamColours(vec![[0, 0], [0, 3], [0, 2]]));
        s.show();
        assert_eq!(tint_of(&mut s.app, owned), Vec4::new(0.0, 0.0, 1.0, 1.0));
        // The icon node survives a capture; its material must follow the new owner.
        let mut captured = (*s.app.world().resource::<RadarFeed>().view).clone();
        captured.units.iter_mut().find(|u| u.id == owned).unwrap().owner = PlayerId(1);
        s.app.world_mut().resource_mut::<RadarFeed>().view = Arc::new(captured);
        s.app.update();
        assert_eq!(tint_of(&mut s.app, owned), Vec4::new(1.0, 0.0, 0.0, 1.0));
        s.app.world_mut().remove_resource::<TeamColours>();
        s.show();
        assert_eq!(tint_of(&mut s.app, owned), UNTINTED);
    }

    /// An object with a `mapIcon` gets no dot; the others do, with the stock sizes in texels, and a
    /// definition's `mapSize` overrides them.
    #[test]
    fn a_unit_with_a_map_icon_gets_no_dot_and_the_others_their_texels() {
        use super::super::radar::{Kind, blips};
        let mut s = scene();
        let planet = s.spawn(s.planet, sta_sim::world::NEUTRAL.0, 3200.0, 3200.0);
        let ship = s.spawn(s.ship, 0, 100.0, 100.0);
        let mut feed = RadarFeed::default();
        let mut view = (*WorldSource::view(&s.sim)).clone();
        view.visibility.clear();
        feed.view = Arc::new(view);
        feed.content = Some(s.db.clone());
        feed.player = PlayerId(0);
        let list = blips(&feed, &RadarState::default(), &square(6400.0), true);
        let of = |list: &[super::super::radar::Blip], id| list.iter().find(|b| b.id == id).map(|b| (b.kind, b.texels));
        assert_eq!(of(&list, ship), Some((Kind::Ship, SHIP_TEXELS)));
        assert_eq!(of(&list, planet), None, "stock look: the planet has a map icon, so no dot");
        // The other look keeps its dot for it.
        let other = blips(&feed, &RadarState::default(), &square(6400.0), false);
        assert_eq!(of(&other, planet).map(|b| b.0), Some(Kind::Body));
        assert!(list.iter().any(|b| b.kind == Kind::Body && b.texels == OTHER_TEXELS), "a body without one keeps its dot");
        // `mapSize` replaces the default.
        let mut big = UnitDef::new("lt_big", "craft");
        big.raw = Arc::new(odf("mapSize = 6\n"));
        big.movement = s.db.unit(s.ship).unwrap().movement;
        let db = ContentDb::builder().unit(big).build().unwrap();
        let keys = MapKeys::of(&db.unit(db.key("lt_big").unwrap()).unwrap().raw);
        assert_eq!(keys.size, Some(6.0));
    }

    /// STA64's own (Remaster) HUD changes only by gaining the two pictures: its dot set (also for
    /// objects with a `mapIcon` key), its fog under the dots, its ring and its lack of strips are
    /// as on main. The stock look has the icons, strips, fog over the dots.
    #[test]
    fn the_other_look_gains_only_the_two_pictures_and_the_stock_look_the_rest() {
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
            app.world_mut().resource_mut::<RadarTextures>().insert("tex_a", Texture { width: 8, height: 8, rgba: flat(8, 8, [100, 50, 0]).rgba.clone() });
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
                blips(&feed, w.resource::<RadarState>(), &map, stock).len()
            };
            assert_eq!(dots, expected);
            assert_eq!(pictures, 2, "both looks gain the two pictures");
            let square = w.query_filtered::<Entity, With<RadarSquare>>().single(w).unwrap();
            let fog = w.get::<Children>(square).unwrap().iter().find(|e| w.get::<crate::ui::radar_fog::RadarFog>(*e).is_some()).unwrap();
            let fog_z = w.get::<ZIndex>(fog).map(|z| z.0);
            if stock {
                assert!(!planet_dot && icons == 1 && strips == 4 && fog_z == Some(1), "{planet_dot} {icons} {strips} {fog_z:?}");
            } else {
                // The dot set of main: a dot for every object, the planet's included; no icons, no
                // strips, the fog a first child with the default stacking index (under the dots).
                assert!(planet_dot && icons == 0 && strips == 0 && fog_z == Some(0), "{planet_dot} {icons} {strips} {fog_z:?}");
                assert_eq!(w.get::<Children>(square).unwrap()[0], fog);
            }
        }
    }

    /// A GPU frame of the additive material (`STA64_GPU_TESTS=1`; skips without a hardware
    /// adapter): the picture is added to what is under it (not blended), sampled nearest, its
    /// alpha is ignored, and `rect` picks the part drawn.
    #[test]
    fn the_additive_material_adds_samples_nearest_and_ignores_alpha_on_the_gpu() {
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
        let mut picture = Image::new(
            Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            TextureDimension::D2,
            texels.concat(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        picture.sampler = ImageSampler::nearest();
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
        // Left half over black: the texel itself, hard-edged (nearest), whatever its alpha.
        assert!(near(at(8, 8), [200, 0, 0]) && near(at(24, 8), [0, 200, 0]) && near(at(8, 48), [0, 0, 200]), "{:?} {:?} {:?}", at(8, 8), at(24, 8), at(8, 48));
        assert!(near(at(15, 8), [200, 0, 0]) && near(at(16, 8), [0, 200, 0]), "nearest: a hard edge at the texel border: {:?} {:?}", at(15, 8), at(16, 8));
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
