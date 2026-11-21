//! The Stock look's weapon shots (#1041): a beam is a textured ribbon with its grow, full
//! and retract timeline, a pulse a textured streak, a torpedo-like shot an animated
//! camera-facing sprite. The Remaster look (`RenderProfile::Refit`) draws the recipe quads
//! of `weapons.rs`, unchanged.
//!
//! The rules and their grades (read from data: a key's presence; read from the executable,
//! static: what a key does; measured: on the original in a bench session):
//!
//! - **Which classes.** By ordnance class label: `phaser` is a ribbon, `pphaser` a streak,
//!   `photontorpedo`, `polerontorpedo`, `shieldremovingtorpedo`, `artillery`, `plasmacannon` and
//!   `chainpulsar` sprites (read from the executable, static). Every other class, and a shot
//!   whose ordnance lacks the keys below or whose sprite is not an additive sprite of the
//!   table, keeps today's drawing in both looks. The switch is [`Effect::Shots`], registered as
//!   stock here and checked per class in `weapons::spawn_fx`; `Effect::Weapons` is not.
//! - **Sprite.** `Sprite` names a sprite of the whole sprite table (the includes followed); its
//!   rectangle and flipbook are the table's. `spriteDuration` above 0 replaces the length of the
//!   sprite's first animation channel (read from the executable, static). Which ordnance wins
//!   when several name one sprite is INTERIM: the last in content order (the original's class
//!   load order is not determined). All shots of one sprite show one flipbook frame, on the game
//!   clock, which stands while the game is paused (measured).
//! - **Material.** Unlit, texture times vertex colour, added to the frame exactly (the texture's
//!   alpha darkens nothing; measured), depth test on, depth write off (the test measured; the
//!   write off read from the executable, static), no fog, no lighting. White unless
//!   `NORMAL_WEAPON_TEAM_COLOR` is set, then the owner's team colour for phasers, pulses and
//!   photon, poleron and shield-removing torpedoes (read from the executable, static). The
//!   engine's own brightness factor is taken as 1 (INTERIM, not determined). Stock post FX
//!   draws no bloom unless a preset asks for one (`PostFx::default`); the shots add no emphasis.
//! - **Geometry.** A ribbon's side vector is normalize((a - eye) x (b - eye)) and `Radius` is its
//!   FULL width (measured); the texture is stretched once from a to b, a tiled sprite repeats
//!   along the beam once per rectangle width (read from data and the executable, static). A
//!   pulse is the same ribbon from its head back along its path, min(`Length`, distance flown)
//!   long, its texture starting at the head (measured). A sprite is a screen-aligned square of
//!   half size `Radius`, never rotated (measured).
//! - **Beam timeline** (measured for the grow, full and retract times, no fade). Over
//!   `lifeSpan`: the end grows out of the muzzle over the first `travelTimeFraction`, the beam is
//!   full, the start retracts into the end over the last fraction. No brightness fade. The start
//!   is the firing hardpoint's node of the posed model, followed live; without the node the
//!   unit's centre (INTERIM).
//! - **Beam end**, per tick (measured): a planet's bounding sphere towards the muzzle; a craft with
//!   shields up whose ordnance does not `ignoreShield` the shield ellipsoid, found with the shield
//!   hit's own ellipsoid ([`super::impacts::shield_axes`]; INTERIM, until one shared helper
//!   exists); a craft otherwise a hull hardpoint of the target, chosen at fire from its
//!   `hullTargetHardpoints` by the shot's hash (INTERIM: the original chooses per shot, the rule
//!   is not determined), the centre without one.
//! - **Early ends** (read from the executable, static): the owner gone ends a beam at once (a shot in
//!   flight flies on: the sim still delivers its hit); a
//!   target lost while `BEAM_MISS_LIFETIME` (counted from the shot) still runs moves the end to
//!   the weapon's range along the last direction until it has run; lost after that, the beam
//!   ends at once; a target further than the square root of two times the range ends it at
//!   once. A planet blocking the path is not tested (no such information in the view).
//! - **Sim timing.** The sim applies a beam's damage on the fire tick (issue #1068); the original
//!   applies it at the end of `lifeSpan`. The beam is drawn for the data's `lifeSpan` from the
//!   fire event, whatever the sim's timing; a target the shot killed on the fire tick is held at
//!   its last place for the beam's life (INTERIM).
//! - **Flares and lights** are a later package: under the Stock look a shot with a stock drawing
//!   gets no muzzle flash and no impact flare (INTERIM), and a weapon whose class has no muzzle
//!   flare gets none either ([`has_muzzle_flare`]).

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset};
use bevy::ecs::system::SystemParam;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Face, PrimitiveTopology, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use sta_assets::a2fmt::spr::{Animation, Sprite, SpriteTable};
use sta_assets::spr_anim::{self, SpriteBlend};
use sta_content::WeaponDef;
use sta_sim::command::Target;
use sta_sim::{ContentKey, EntityId, SimEvent, TICK_HZ, Tick};

use super::weapons::{point, shifted, shot_seed};
use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::render::profile::{Effect, ProfileEffects, RenderProfile};
use crate::render::sprites::SpriteTables;
use crate::render::units::{SelectRadius, to_bevy};
use crate::sim_link::{ContentRes, SimEntities, SimLink, SimMsg};

// ---------------------------------------------------------------------------------------------
// Which classes
// ---------------------------------------------------------------------------------------------

/// How a stock shot is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotPath {
    /// A textured ribbon along a beam, timeline included.
    Ribbon,
    /// A textured streak from a pulse's head back along its path.
    Streak,
    /// A screen-aligned animated sprite at the shot.
    Sprite,
}

/// The ordnance class labels with a stock drawing (read from the executable, static).
pub fn class_path(class: &str) -> Option<ShotPath> {
    match class {
        "phaser" => Some(ShotPath::Ribbon),
        "pphaser" => Some(ShotPath::Streak),
        "photontorpedo" | "polerontorpedo" | "shieldremovingtorpedo" | "artillery" | "plasmacannon" | "chainpulsar" => Some(ShotPath::Sprite),
        _ => None,
    }
}

/// The classes that take the owner's team colour when `NORMAL_WEAPON_TEAM_COLOR` is set (read from
/// the executable, static).
pub fn takes_team_colour(class: &str) -> bool {
    matches!(class, "phaser" | "pphaser" | "photontorpedo" | "polerontorpedo" | "shieldremovingtorpedo")
}

/// Whether the original gives the class a muzzle flare (the phaser class only; read from the
/// executable, static). Under the Stock look no other weapon gets STA64's muzzle flash.
pub fn has_muzzle_flare(class: &str) -> bool {
    class == "phaser"
}

/// What a stock shot needs from its ordnance and the sprite table.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotSpec {
    pub path: ShotPath,
    /// The sprite's name as the ordnance writes it.
    pub sprite: String,
    /// `Radius`: a ribbon's full width, a sprite's half size.
    pub radius: f32,
    /// `Length`: a pulse's streak length (0 for the others).
    pub length: f32,
    /// `lifeSpan` in seconds (beams).
    pub life: f32,
    /// `travelTimeFraction` (beams).
    pub travel: f32,
}

/// The stock drawing of `def`, if its class has one and its data carries what the drawing reads:
/// `Sprite` (an additive sprite of the table), `Radius`, and `Length` (pulses) or `lifeSpan` (beams).
pub fn shot_spec(def: &WeaponDef, table: &SpriteTable) -> Option<ShotSpec> {
    shot_spec_or_reason(def, table).ok()
}

/// [`shot_spec`], or why the class's shot keeps today's drawing.
pub fn shot_spec_or_reason(def: &WeaponDef, table: &SpriteTable) -> Result<ShotSpec, &'static str> {
    let path = class_path(&def.class.0).ok_or("its class has no stock drawing")?;
    let look = &def.look;
    let name = look.sprite.as_deref().ok_or("no Sprite key")?;
    let sprite = table.sprite(name).ok_or("its Sprite is in no sprite table")?;
    if spr_anim::sprite_blend(&sprite.material) != SpriteBlend::Add {
        return Err("its sprite is not additive");
    }
    let radius = look.radius.filter(|r| *r > 0.0).ok_or("no Radius key")?;
    let (length, life, travel) = match path {
        ShotPath::Ribbon => (
            0.0,
            look.life_span.filter(|l| *l > 0.0).ok_or("no lifeSpan key")?,
            look.travel_time_fraction.unwrap_or(sta_content::ShotLook::DEFAULT_TRAVEL_TIME_FRACTION).clamp(0.0, 1.0),
        ),
        ShotPath::Streak => (look.length.filter(|l| *l > 0.0).ok_or("no Length key")?, 0.0, 0.0),
        ShotPath::Sprite => (0.0, 0.0, 0.0),
    };
    Ok(ShotSpec { path, sprite: name.to_owned(), radius, length, life, travel })
}

/// [`shot_spec`] under a look: the Remaster look (`Refit`) never takes the stock path.
pub fn stock_spec(look: RenderProfile, def: &WeaponDef, table: &SpriteTable) -> Option<ShotSpec> {
    (look == RenderProfile::Stock).then(|| shot_spec(def, table)).flatten()
}

// ---------------------------------------------------------------------------------------------
// Sprites and flipbooks
// ---------------------------------------------------------------------------------------------

/// A shot sprite resolved against the table: the texture, the rectangle and the flipbook channels,
/// the first one's length replaced by the ordnance's `spriteDuration` where it has one.
#[derive(Debug, Clone)]
pub struct ShotSprite {
    pub texture: String,
    rect: [f32; 4],
    anims: Vec<Animation>,
    /// The sprite repeats along a beam, once per rectangle width.
    pub tile_u: bool,
    /// The rectangle's width in the table's texel space (the length of one repeat).
    pub rect_width: f32,
}

impl ShotSprite {
    /// `name` in `table`; `duration` is `spriteDuration` (0 or `None`: no override).
    pub fn resolve(table: &SpriteTable, name: &str, duration: Option<f32>) -> Option<ShotSprite> {
        let sprite: &Sprite = table.sprite(name)?;
        let rect = sprite.uv_rect().unwrap_or_else(|| {
            let rw = sprite.reference[0].unwrap_or(256.0).max(1.0);
            let rh = sprite.reference[1].unwrap_or(rw).max(1.0);
            [sprite.rect[0] / rw, sprite.rect[1] / rh, sprite.rect[2] / rw, sprite.rect[3] / rh]
        });
        let mut anims: Vec<Animation> = sprite.anims.iter().filter_map(|id| table.animation(id).cloned()).collect();
        if let (Some(d), Some(first)) = (duration.filter(|d| *d > 0.0), anims.first_mut()) {
            first.length = d;
        }
        Some(ShotSprite { texture: sprite.texture.clone(), rect, anims, tile_u: sprite.tile_u, rect_width: sprite.rect[2].max(1.0) })
    }

    /// The texture rectangle `[u, v, w, h]` at game time `t` seconds, flipbook offsets added.
    pub fn rect_at(&self, t: f32) -> [f32; 4] {
        let mut off = [0.0f32; 2];
        for a in &self.anims {
            let o = spr_anim::offset_of(spr_anim::value(a, t, spr_anim::LOOP));
            off[0] += o[0];
            off[1] += o[1];
        }
        [self.rect[0] + off[0], self.rect[1] + off[1], self.rect[2], self.rect[3]]
    }
}

/// `spriteDuration` for `sprite` over all ordnance naming it: the last in content order wins
/// (INTERIM: the original's load order is not determined). 0 counts as no override.
pub fn duration_override<'a>(weapons: impl Iterator<Item = &'a WeaponDef>, sprite: &str) -> Option<f32> {
    weapons
        .filter(|w| w.look.sprite.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(sprite)))
        .filter_map(|w| w.look.sprite_duration.filter(|d| *d > 0.0))
        .last()
}

/// Every shot sprite of the content, resolved, by lower-case name.
#[derive(Resource, Default)]
pub struct ShotSprites {
    map: BTreeMap<String, ShotSprite>,
    stamp: (usize, usize),
}

impl ShotSprites {
    pub fn build(table: &SpriteTable, db: &sta_content::ContentDb) -> ShotSprites {
        let mut map = BTreeMap::new();
        for w in db.weapons() {
            let Some(name) = w.look.sprite.as_deref() else { continue };
            let key = name.to_ascii_lowercase();
            if map.contains_key(&key) {
                continue;
            }
            if let Some(s) = ShotSprite::resolve(table, name, duration_override(db.weapons(), name)) {
                map.insert(key, s);
            }
        }
        ShotSprites { map, stamp: (0, 0) }
    }

    pub fn get(&self, name: &str) -> Option<&ShotSprite> {
        self.map.get(&name.to_ascii_lowercase())
    }

    /// [`Self::get`] for a name already in lower case (no allocation).
    fn get_lower(&self, name: &str) -> Option<&ShotSprite> {
        self.map.get(name)
    }
}

// ---------------------------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------------------------

/// The corners of a ribbon from `a` to `b`: a + s, b + s, b - s, a - s, with s the half width along
/// normalize((a - eye) x (b - eye)). Texture u runs from a to b, v across (0 on the + s side).
pub fn ribbon_corners(eye: Vec3, a: Vec3, b: Vec3, width: f32) -> [Vec3; 4] {
    let side = (a - eye).cross(b - eye).try_normalize().unwrap_or_else(|| (b - a).try_normalize().unwrap_or(Vec3::X).any_orthonormal_vector());
    let s = side * (width * 0.5);
    [a + s, b + s, b - s, a - s]
}

/// A screen-aligned square about `c`, half size `half`: the corners top left, top right, bottom right,
/// bottom left of the view, never rotated.
pub fn billboard_corners(c: Vec3, right: Vec3, up: Vec3, half: f32) -> [Vec3; 4] {
    let (r, u) = (right * half, up * half);
    [c - r + u, c + r + u, c + r - u, c - r - u]
}

/// A pulse's streak: its end at the head and its other end back along the path, min(length, distance
/// flown from the muzzle) long. Returns `(head, tail)`.
pub fn streak_ends(muzzle: Vec3, dir: Vec3, head: Vec3, length: f32) -> (Vec3, Vec3) {
    let flown = (head - muzzle).dot(dir).max(0.0);
    (head, head - dir * length.min(flown))
}

/// The beam's two ends at `age` of `life` ticks, from the full `start` and `end`: the end grows out of
/// the start over the first `travel` of the life, the start retracts into the end over the last.
pub fn beam_segment(start: Vec3, end: Vec3, age: f32, life: f32, travel: f32) -> (Vec3, Vec3) {
    let f = if life > 0.0 { ((life - age) / life).clamp(0.0, 1.0) } else { 0.0 };
    let (mut a, mut b) = (start, end);
    if travel > 0.0 {
        if f > 1.0 - travel {
            b = a + (end - a) * ((1.0 - f) / travel);
        }
        if f < travel {
            a = b + (a - b) * (f / travel);
        }
    }
    (a, b)
}

// ---------------------------------------------------------------------------------------------
// Where a beam ends
// ---------------------------------------------------------------------------------------------

/// The target as a beam sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub centre: Vec3,
    /// Unit heading in render space, the shield ellipsoid's long axis.
    pub heading: Vec3,
    /// The unit's selection radius, the shield ellipsoid's unit.
    pub radius: f32,
    pub shields_up: bool,
    pub planet: bool,
    /// The chosen hull hardpoint, where the model has it.
    pub hardpoint: Option<Vec3>,
}

impl Body {
    /// A bare point (a shot at a place): the beam ends there.
    pub fn point(at: Vec3) -> Body {
        Body { centre: at, heading: Vec3::NEG_Z, radius: 0.0, shields_up: false, planet: false, hardpoint: None }
    }
}

/// Where a beam from `start` ends on `body` (measured): a planet's bounding sphere towards the muzzle;
/// a craft with shields up and ordnance that does not ignore shields, the shield ellipsoid (the shield
/// hit's, INTERIM); else the chosen hardpoint, the centre without one.
pub fn beam_end(start: Vec3, body: &Body, ignore_shield: bool) -> Vec3 {
    if body.planet {
        return body.centre + (start - body.centre).try_normalize().unwrap_or(Vec3::Y) * body.radius;
    }
    if body.shields_up && !ignore_shield && body.radius > 0.0 {
        let (rot, scale) = super::impacts::shield_axes(body.heading, body.radius);
        // On the line to the centre the hit is the ellipsoid's surface point in the direction of the start.
        let p = (rot.inverse() * (start - body.centre)) / scale;
        let n = p.length();
        // A muzzle inside the ellipsoid has nothing to cross: the beam is as short as it can be.
        return if n <= 1.0 { start } else { body.centre + rot * (p / n * scale) };
    }
    body.hardpoint.unwrap_or(body.centre)
}

// ---------------------------------------------------------------------------------------------
// The shot
// ---------------------------------------------------------------------------------------------

/// What a stock shot draws this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Geom {
    #[default]
    None,
    /// A ribbon or streak from `a` (the texture's start) to `b`.
    Segment { a: Vec3, b: Vec3 },
    /// A sprite centred on `c`.
    Billboard { c: Vec3 },
}

/// One shot drawn the Stock way. Tests key on `(shooter, weapon, fired)`, never on the Bevy `Entity`.
#[derive(Component, Debug, Clone)]
pub struct StockShot {
    pub path: ShotPath,
    pub shooter: EntityId,
    pub weapon: ContentKey,
    pub target: Target,
    pub fired: Tick,
    /// Ticks it is drawn.
    pub life: f32,
    pub sprite: String,
    /// `Radius`.
    pub radius: f32,
    pub length: f32,
    /// Linear RGB the texture is multiplied with.
    pub colour: Vec3,
    travel: f32,
    range: f32,
    ignore_shield: bool,
    /// `BEAM_MISS_LIFETIME` in ticks, when the header sets it.
    miss: Option<f32>,
    /// The firing hardpoint's node, and the target hardpoint chosen at fire, as entities of the posed
    /// models (found once, at fire; none where the model lacks the node).
    node: Option<Entity>,
    target_node: Option<Entity>,
    /// Where it left, and the speed per tick (projectiles).
    muzzle: Vec3,
    per_tick: f32,
    /// The target's last place (projectiles) and body (beams), and the beam's last end.
    to: Vec3,
    last_body: Option<Body>,
    last_end: Vec3,
    /// The target was already gone from the view when the shot was fired (the shot killed it).
    pinned: bool,
    pub geom: Geom,
}

/// The nodes and bodies of the render side, for the systems of this module.
#[derive(SystemParam)]
pub struct StockShots<'w, 's> {
    profile: Option<Res<'w, RenderProfile>>,
    effects: Option<Res<'w, ProfileEffects>>,
    tables: Option<Res<'w, SpriteTables>>,
    content: Option<Res<'w, ContentRes>>,
    ents: Option<Res<'w, SimEntities>>,
    teams: Option<Res<'w, crate::viewer::TeamColours>>,
    map_teams: Option<Res<'w, crate::viewer::MapTeamColours>>,
    children: Query<'w, 's, &'static Children>,
    named: Query<'w, 's, (&'static Name, &'static GlobalTransform)>,
    transforms: Query<'w, 's, &'static GlobalTransform>,
    radii: Query<'w, 's, &'static SelectRadius>,
}

impl StockShots<'_, '_> {
    /// The look the shots are drawn in: the Stock one only where the profile says so and this effect
    /// has a stock version registered. Without the profile resources (windowless tests) Refit's.
    pub fn look(&self) -> RenderProfile {
        match (self.profile.as_deref(), self.effects.as_deref()) {
            (Some(p), Some(e)) => e.implementation(*p, Effect::Shots),
            _ => RenderProfile::Refit,
        }
    }

    /// The entity of node `node` in unit `unit`'s posed model.
    fn find_node(&self, unit: EntityId, node: &str) -> Option<Entity> {
        let root = *self.ents.as_ref()?.0.get(&unit)?;
        self.children.iter_descendants(root).find(|e| self.named.get(*e).is_ok_and(|(name, _)| name.as_str().eq_ignore_ascii_case(node)))
    }

    /// Where a node found by [`Self::find_node`] is now, from the unit's own origin (render space).
    fn node_offset(&self, unit: EntityId, node: Entity) -> Option<Vec3> {
        let root = *self.ents.as_ref()?.0.get(&unit)?;
        let base = self.transforms.get(root).ok()?.translation();
        Some(self.transforms.get(node).ok()?.translation() - base)
    }

    fn body(&self, link: &SimLink, id: EntityId, alpha: f32, offsets: &Option<Res<crate::render::mining::VisualOffsets>>, hardpoint: Option<Entity>) -> Option<Body> {
        let u = link.curr().unit(id)?;
        let centre = point(link, id, alpha)? + shifted(offsets, id);
        let heading = Vec3::new(u.heading.0.to_f32(), 0.0, -u.heading.1.to_f32()).normalize_or(Vec3::NEG_Z);
        let radius = self.ents.as_ref().and_then(|m| m.0.get(&id)).and_then(|e| self.radii.get(*e).ok()).map_or(super::impacts::DEFAULT_RADIUS, |r| r.0);
        let planet = self.content.as_ref().and_then(|c| c.0.unit(u.def)).is_some_and(|d| matches!(d.class.0.as_str(), "planet" | "planet_obj"));
        let hardpoint = hardpoint.and_then(|n| self.node_offset(id, n)).map(|o| centre + o);
        Some(Body { centre, heading, radius, shields_up: u.shields.iter().any(|s| s[0].to_f32() > 0.0), planet, hardpoint })
    }

    /// Spawn the stock drawing of a shot, if its weapon has one under this look. False: the caller
    /// draws what it draws today.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_spawn(
        &self,
        cmds: &mut Commands,
        link: &SimLink,
        clock: &AnimClock,
        offsets: &Option<Res<crate::render::mining::VisualOffsets>>,
        def: &WeaponDef,
        shooter: EntityId,
        weapon: ContentKey,
        hardpoint: u8,
        target: &Target,
    ) -> bool {
        let (Some(tables), Some(content)) = (self.tables.as_deref(), self.content.as_deref()) else { return false };
        let Some(spec) = stock_spec(self.look(), def, &tables.0) else { return false };
        let alpha = clock.alpha();
        // A shot at nothing in particular has no end to reach: today's drawing.
        let (to, pinned) = match target {
            Target::Entity(id) => match point(link, *id, alpha) {
                Some(p) => (p + shifted(offsets, *id), false),
                None => match link.prev().unit(*id) {
                    Some(u) => (to_bevy(u.pos.to_f32()), true),
                    None => return false,
                },
            },
            Target::Point(p) => (to_bevy(p.to_f32()), false),
            _ => return false,
        };
        let Some(shooter_view) = link.curr().unit(shooter) else { return false };
        let Some(from) = point(link, shooter, alpha).map(|p| p + shifted(offsets, shooter)) else { return false };
        let node = content.0.unit(shooter_view.def).and_then(|d| d.hardpoints.get(hardpoint as usize)).and_then(|h| self.find_node(shooter, &h.node));
        let muzzle = from + node.and_then(|n| self.node_offset(shooter, n)).unwrap_or(Vec3::ZERO);
        let seed = shot_seed(shooter, weapon, clock.tick());
        let target_node = match target {
            Target::Entity(id) => link
                .curr()
                .unit(*id)
                .or_else(|| link.prev().unit(*id))
                .and_then(|u| content.0.unit(u.def))
                .map(|d| d.stats.hull_target_hardpoints.iter().filter(|n| !n.is_empty()).collect::<Vec<_>>())
                .filter(|v| !v.is_empty())
                .map(|v| v[(seed * 100.0) as usize % v.len()].clone())
                .and_then(|name| self.find_node(*id, &name)),
            _ => None,
        };
        let colour = if content.0.rules().shots.team_colour() && takes_team_colour(&def.class.0) {
            crate::viewer::owner_hull_colour(self.teams.as_deref(), self.map_teams.as_deref(), shooter_view.owner).map_or(Vec3::ONE, |c| {
                let l = c.to_linear();
                Vec3::new(l.red, l.green, l.blue)
            })
        } else {
            Vec3::ONE
        };
        let per_tick = def.shot_speed.to_f32() / TICK_HZ as f32;
        let flight = if per_tick > 0.0 { (muzzle.distance(to) / per_tick).ceil().max(1.0) } else { 1.0 };
        let life = if spec.path == ShotPath::Ribbon { spec.life * TICK_HZ as f32 } else { flight };
        let miss = content.0.rules().shots.beam_miss_lifetime.map(|m| m.to_f32() * TICK_HZ as f32);
        cmds.spawn(StockShot {
            path: spec.path,
            shooter,
            weapon,
            target: *target,
            fired: clock.tick(),
            life,
            sprite: spec.sprite.to_ascii_lowercase(),
            radius: spec.radius,
            length: spec.length,
            colour,
            travel: spec.travel,
            range: def.range.to_f32(),
            ignore_shield: def.base.ignore_shield.unwrap_or(false),
            miss,
            node,
            target_node,
            muzzle,
            per_tick,
            to,
            last_body: pinned.then(|| Body::point(to)),
            last_end: to,
            pinned,
            geom: Geom::None,
        });
        true
    }
}

/// Advance every stock shot to this frame: its ends, its timeline, its end of life.
#[allow(clippy::too_many_arguments)]
fn update_shots(
    mut cmds: Commands,
    offsets: Option<Res<crate::render::mining::VisualOffsets>>,
    mut msgs: MessageReader<SimMsg>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    ctx: StockShots,
    mut q: Query<(Entity, &mut StockShot)>,
) {
    let alpha = clock.alpha();
    let now_ticks = clock.now() * TICK_HZ as f64;
    // The earliest matching shot in flight has arrived.
    for ev in msgs.read().map(|m| &m.0) {
        if let SimEvent::Hit { target, by, weapon, .. } = ev {
            let hit = q.iter().filter(|(_, s)| s.path != ShotPath::Ribbon && s.shooter == *by && s.weapon == *weapon && s.target == Target::Entity(*target)).min_by_key(|(_, s)| s.fired).map(|(e, _)| e);
            if let Some(e) = hit {
                cmds.entity(e).try_despawn();
            }
        }
    }
    for (e, mut s) in &mut q {
        let age = (now_ticks - s.fired as f64) as f32;
        if age >= s.life {
            cmds.entity(e).try_despawn();
            continue;
        }
        // The owner gone ends a beam at once; a shot in flight flies on (the sim still delivers its hit).
        let start = if s.path == ShotPath::Ribbon {
            let Some(shooter_at) = point(&link, s.shooter, alpha) else {
                cmds.entity(e).try_despawn();
                continue;
            };
            shooter_at + shifted(&offsets, s.shooter) + s.node.and_then(|n| ctx.node_offset(s.shooter, n)).unwrap_or(Vec3::ZERO)
        } else {
            s.muzzle
        };
        if s.path == ShotPath::Ribbon {
            let live = match s.target {
                Target::Entity(id) => ctx.body(&link, id, alpha, &offsets, s.target_node),
                Target::Point(p) => Some(Body::point(to_bevy(p.to_f32()))),
                _ => None,
            };
            // A target the shot itself killed on the fire tick is held where it was for the beam's life.
            let lost = live.is_none() && !s.pinned;
            if let Some(b) = live {
                s.last_body = Some(b);
            }
            let end = if lost {
                // A target lost while the miss timer runs: the end goes to the range along the last direction;
                // lost after it, or with no timer, the beam ends.
                if !s.miss.is_some_and(|m| age < m) {
                    cmds.entity(e).try_despawn();
                    continue;
                }
                start + (s.last_end - start).try_normalize().unwrap_or(Vec3::Y) * s.range
            } else {
                let body = s.last_body.clone().expect("a live or held body");
                // Further than the square root of two times the range: ends at once.
                if s.range > 0.0 && start.distance_squared(body.centre) > 2.0 * s.range * s.range {
                    cmds.entity(e).try_despawn();
                    continue;
                }
                beam_end(start, &body, s.ignore_shield)
            };
            if !lost {
                s.last_end = end;
            }
            let (a, b) = beam_segment(start, end, age, s.life, s.travel);
            s.geom = Geom::Segment { a, b };
        } else {
            // Straight at the target's live place, as the other shots fly (the sim hits on arrival).
            let to = match s.target {
                Target::Entity(id) => point(&link, id, alpha).map(|p| p + shifted(&offsets, id)),
                Target::Point(p) => Some(to_bevy(p.to_f32())),
                _ => None,
            };
            if let Some(t) = to {
                s.to = t;
            }
            let pos = s.muzzle.lerp(s.to, (age / s.life).clamp(0.0, 1.0));
            let dir = (s.to - s.muzzle).normalize_or(Vec3::Z);
            s.geom = match s.path {
                ShotPath::Sprite => Geom::Billboard { c: pos },
                _ => {
                    let (head, tail) = streak_ends(s.muzzle, dir, pos + dir * s.per_tick, s.length);
                    Geom::Segment { a: head, b: tail }
                }
            };
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct ShotParams {
    /// The texture rectangle `u, v, w, h`.
    pub rect: Vec4,
    /// x: the sprite repeats in u inside its rectangle.
    pub flags: Vec4,
}

/// One shot sprite's page: texture times vertex colour, added to the frame.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct StockShotMaterial {
    #[uniform(0)]
    pub params: ShotParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

/// Exactly additive, depth test as the pipeline has it, depth write off, no culling: the stock blend is
/// source ONE, destination ONE for the colour, so the texture's alpha darkens nothing.
pub fn additive_state(d: &mut RenderPipelineDescriptor) {
    d.primitive.cull_mode = None::<Face>;
    if let Some(ds) = d.depth_stencil.as_mut() {
        ds.depth_write_enabled = Some(false);
    }
    let one = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    if let Some(f) = d.fragment.as_mut() {
        for t in f.targets.iter_mut().flatten() {
            t.blend = Some(BlendState { color: one, alpha: one });
        }
    }
}

impl Material for StockShotMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://sta64/render/fx/stock_shots.wesl").into_owned())
    }
    /// A blended pass (drawn after the opaque ones, never writing depth); the blend itself is set in
    /// [`additive_state`].
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        additive_state(descriptor);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

/// A sprite's mesh, material and entity.
struct Batch {
    mesh: Handle<Mesh>,
    material: Handle<StockShotMaterial>,
    entity: Entity,
}

/// One batch per sprite: all its live shots in one mesh, rebuilt every frame.
#[derive(Resource, Default)]
pub struct ShotBatches {
    map: BTreeMap<String, Batch>,
}

#[derive(Default)]
struct Geo {
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    col: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

impl Geo {
    fn quad(&mut self, corners: [Vec3; 4], u_end: f32, colour: Vec3) {
        let base = self.pos.len() as u32;
        let uvs = [[0.0, 0.0], [u_end, 0.0], [u_end, 1.0], [0.0, 1.0]];
        for (c, uv) in corners.iter().zip(uvs) {
            self.pos.push(c.to_array());
            self.uv.push(uv);
            self.col.push([colour.x, colour.y, colour.z, 1.0]);
        }
        self.idx.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
}

/// Resolve the sprites again when the table or the content is a different one.
fn refresh_sprites(tables: Option<Res<SpriteTables>>, content: Option<Res<ContentRes>>, mut sprites: ResMut<ShotSprites>) {
    let (Some(t), Some(c)) = (tables, content) else { return };
    let stamp = (std::sync::Arc::as_ptr(&t.0) as usize, std::sync::Arc::as_ptr(&c.0) as usize);
    if sprites.stamp != stamp {
        *sprites = ShotSprites::build(&t.0, &c.0);
        sprites.stamp = stamp;
    }
}

/// Lay the frame's shots out in their sprites' meshes.
#[allow(clippy::too_many_arguments)]
fn draw_shots(
    mut cmds: Commands,
    clock: Res<AnimClock>,
    sprites: Res<ShotSprites>,
    camera: Query<&GlobalTransform, crate::map::MainCamera>,
    shots: Query<&StockShot>,
    mut batches: ResMut<ShotBatches>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StockShotMaterial>>,
    mut loader: crate::render::textures::LoaderParams,
    mut missing: Local<BTreeSet<String>>,
) {
    let Ok(cam) = camera.single() else { return };
    let (eye, right, up) = (cam.translation(), cam.right().as_vec3(), cam.up().as_vec3());
    let t = clock.now() as f32;
    let mut geo: BTreeMap<String, Geo> = BTreeMap::new();
    for s in &shots {
        let Some(sprite) = sprites.get_lower(&s.sprite) else { continue };
        let g = geo.entry(s.sprite.clone()).or_default();
        match s.geom {
            Geom::Segment { a, b } => {
                let u_end = if s.path == ShotPath::Ribbon && sprite.tile_u { a.distance(b) / sprite.rect_width } else { 1.0 };
                g.quad(ribbon_corners(eye, a, b, s.radius), u_end, s.colour);
            }
            Geom::Billboard { c } => g.quad(billboard_corners(c, right, up, s.radius), 1.0, s.colour),
            Geom::None => {}
        }
    }
    for k in batches.map.keys().cloned().collect::<Vec<_>>() {
        geo.entry(k).or_default();
    }
    for (key, g) in geo {
        let Some(sprite) = sprites.get(&key) else { continue };
        if !batches.map.contains_key(&key) {
            if g.idx.is_empty() {
                continue;
            }
            let Some(texture) = loader.loader().colour(&sprite.texture, false) else {
                if missing.insert(key.clone()) {
                    warn!("stock shot sprite {key}: its texture is not in the mounted files");
                }
                continue;
            };
            let material = mats.add(StockShotMaterial { params: ShotParams::default(), texture });
            let mesh = meshes.add(Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()));
            let entity = cmds
                .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), Transform::IDENTITY, bevy::camera::visibility::NoFrustumCulling, Visibility::Hidden))
                .id();
            batches.map.insert(key.clone(), Batch { mesh, material, entity });
        }
        let b = &batches.map[&key];
        // The flipbook frame is the sprite's, shared by every shot of it, on the game clock.
        let params = ShotParams { rect: Vec4::from(sprite.rect_at(t)), flags: Vec4::new(f32::from(sprite.tile_u), 0.0, 0.0, 0.0) };
        if mats.get(&b.material).is_some_and(|m| m.params != params)
            && let Some(mut m) = mats.get_mut(&b.material)
        {
            m.params = params;
        }
        if g.idx.is_empty() {
            cmds.entity(b.entity).try_insert(Visibility::Hidden);
            continue;
        }
        // The batch sorts among the blended things at its centroid; its vertices are relative to it.
        let centre = g.pos.iter().fold(Vec3::ZERO, |a, p| a + Vec3::from(*p)) / g.pos.len() as f32;
        let pos: Vec<[f32; 3]> = g.pos.iter().map(|p| (Vec3::from(*p) - centre).to_array()).collect();
        let normals = vec![[0.0, 0.0, 1.0]; pos.len()];
        cmds.entity(b.entity).try_insert((Visibility::Inherited, Transform::from_translation(centre)));
        if let Some(mut m) = meshes.get_mut(&b.mesh) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, g.uv);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, g.col);
            m.insert_indices(Indices::U32(g.idx));
        }
    }
}

pub struct StockShotsPlugin;

impl Plugin for StockShotsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProfileEffects>();
        app.world_mut().resource_mut::<ProfileEffects>().register_stock(Effect::Shots);
        app.init_resource::<ShotSprites>().init_resource::<ShotBatches>();
        app.add_systems(
            Update,
            update_shots.after(super::weapons::update_fx).in_set(GameSet::Interpolate).distributive_run_if(resource_exists::<SimLink>).distributive_run_if(resource_exists::<ContentRes>),
        );
        app.add_systems(
            Update,
            (refresh_sprites, draw_shots)
                .chain()
                .after(update_shots)
                .in_set(GameSet::Interpolate)
                .distributive_run_if(resource_exists::<Assets<StockShotMaterial>>)
                .distributive_run_if(resource_exists::<Assets<Mesh>>)
                .distributive_run_if(resource_exists::<crate::viewer::Mounted>)
                .distributive_run_if(resource_exists::<crate::render::textures::TextureCache>),
        );
    }
}

/// Adds the stock shot shader and material; the windowless tests leave it out.
pub struct StockShotsMaterialPlugin;

impl Plugin for StockShotsMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "stock_shots.wesl");
        app.add_plugins(MaterialPlugin::<StockShotMaterial>::default());
    }
}

#[cfg(test)]
#[path = "stock_shots_tests.rs"]
mod tests;
