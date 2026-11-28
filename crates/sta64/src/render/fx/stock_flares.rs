//! The Stock look's weapon flares (#1041, slice C): the muzzle flare of a beam weapon and the impact
//! flare where a beam lands on a hull. The Remaster look draws its own flashes and impact flares
//! (`weapons.rs`), unchanged.
//!
//! The rules and their grades (read from data: a key's presence; read from the executable, static:
//! what a key does; measured: on the original in a bench session):
//!
//! - **Node particles.** A flare is a camera-facing, constant-colour, additive sprite attached to an
//!   object, never fading, with a flipbook on the game clock like the shots (read from the executable,
//!   static; the flare's own flipbook phase is INTERIM: the original's depends on what it drew last).
//!   It is drawn only while its object is visible (read from the executable, static).
//! - **Cap.** At most [`MAX_PER_OBJECT`] live per object (the owner for a muzzle flare, the target for
//!   an impact flare) and [`MAX_TOTAL`] in all (read from the executable, static; the four per object
//!   measured). A flare over the cap is not drawn; which one loses is INTERIM: the newest (the
//!   evidence does not say). Hull-hit sparks and death sparks share the same cap in the original, so
//!   they take their slots from [`NodeParticles`] too, when they come.
//! - **Muzzle flare** (the ordnance class that carries `flareSprite` / `flareRadiusMultiplier`, the
//!   phaser class: `ShotLook::flare`): at the firing hardpoint's node, following it; half size
//!   `Radius` times the multiplier (measured); in the shot's colour; it lasts the part of the beam's
//!   `lifeSpan` after the grow, (1 - `travelTimeFraction`) times `lifeSpan`, at most [`MUZZLE_LIFE_CAP`]
//!   seconds (measured for the span, read from the executable, static for the cap).
//! - **Impact flare**: when a beam first reaches its target (after `travelTimeFraction` of `lifeSpan`),
//!   and only if the hit is a hull hit (read from the executable, static; measured with shields up and
//!   with `ignoreShield`): the target's shields are down, or the ordnance has `ignoreShield`, or the target
//!   is not a craft ([`hull_hit`]). With shields up the shield hit takes its place (a later slice).
//!   A white sprite of a constant name ([`IMPACT_SPRITE`]), half size [`IMPACT_SIZE_FACTOR`] times
//!   `Radius`, [`IMPACT_LIFE`] seconds (measured), at the impact point in the target's frame, so it
//!   follows the target.
//! - **What the view says.** Shields up is the view's unit having any shield facing above zero at that
//!   moment, as the beam's end uses it. The sim applies a beam's damage on the fire tick (issue #1068),
//!   earlier than the original, so a shield a beam has just brought down is already down when it arrives
//!   (INTERIM, until the sim's timing follows). A craft is anything but a planet (INTERIM: the original's
//!   test is not known beyond planets). The target's frame is its position and heading (the view has no
//!   pitch or roll). A beam at a bare point has no object to attach a flare to: none (INTERIM).

use bevy::prelude::*;
use sta_sim::{EntityId, TICK_HZ};

use super::stock_shots::{Body, StockShots};
use super::weapons::{point, shifted};
use crate::clock::AnimClock;
use crate::sim_link::SimLink;

/// Live flares per object (read from the executable, static; measured).
pub const MAX_PER_OBJECT: usize = 4;
/// Live flares in all (read from the executable, static).
pub const MAX_TOTAL: usize = 200;
/// The longest a muzzle flare lasts, seconds (read from the executable, static).
pub const MUZZLE_LIFE_CAP: f32 = 10.0;
/// The sprite-table name STA64 looks up for a beam's impact flare. No data key names it; the original uses
/// this fixed name (read from the executable, static). A mod's sprite table may define it.
pub const IMPACT_SPRITE: &str = "explosion1";
/// The impact flare's half size in `Radius` (read from the executable, static; measured).
pub const IMPACT_SIZE_FACTOR: f32 = 2.0;
/// The impact flare's life, seconds (measured).
pub const IMPACT_LIFE: f32 = 1.0;

/// The sprite-table names of the spark family a weapon hit on a bare hull and a dying hull draw (#1153): one member,
/// picked at random. No data key names them; the original uses these fixed names (read from the executable,
/// static). A mod's sprite table may define them.
pub const SPARK_FAMILY: [&str; 7] = ["systemexpl1", "systemexpl2", "systemexpl3", "systemexpl4", "systemexpl5", "systemexpl6", "systemexpl7"];
/// A spark's half size is this share of the unit's radius, at least [`SPARK_MIN_HALF`] (read from the executable,
/// static). The radius the original uses is not resolved: STA64 takes the unit's selection radius (INTERIM).
pub const SPARK_SIZE_FACTOR: f32 = 0.14;
/// The smallest half size of a spark, world units (read from the executable, static).
pub const SPARK_MIN_HALF: f32 = 4.0;
/// A spark's life, seconds (read from the executable, static).
pub const SPARK_LIFE: f32 = 1.0;
/// Seconds between the sparks a dying hull emits (read from the executable, static).
pub const DEATH_SPARK_INTERVAL: f32 = 0.05;

/// A spark's half size for a unit of `radius`.
pub fn spark_half(radius: f32) -> f32 {
    (radius * SPARK_SIZE_FACTOR).max(SPARK_MIN_HALF)
}

/// The muzzle flare's life in seconds: (1 - `travelTimeFraction`) times `lifeSpan`, at most the cap.
pub fn muzzle_life(life_span: f32, travel: f32) -> f32 {
    ((1.0 - travel.clamp(0.0, 1.0)) * life_span).clamp(0.0, MUZZLE_LIFE_CAP)
}

/// Whether a beam landing on `body` is a hull hit: shields down, ordnance that ignores shields, or a
/// target that is not a craft.
pub fn hull_hit(body: &Body, ignore_shield: bool) -> bool {
    body.planet || ignore_shield || !body.shields_up
}

/// What a shot needs to raise a flare: its sprite, half size and life in ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct FlareSpec {
    /// The sprite's name, lower case.
    pub sprite: String,
    pub half: f32,
    pub life: f32,
}

/// Which flare this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlareKind {
    Muzzle,
    Impact,
    /// The spark of a weapon hit on a bare hull (`stock_hull_hits.rs`).
    HullHit,
    /// A spark of a dying hull (`stock_hull_hits.rs`).
    DeathSpark,
}

/// Where a flare hangs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    /// The owner's hardpoint node (its model's node entity, where the model has it; else the owner's centre).
    Node(Option<Entity>),
    /// A point in the object's frame (its position and heading).
    Frame(Vec3),
}

/// One flare. Tests key on `(kind, object, born)`, never on the Bevy `Entity`.
#[derive(Component, Debug, Clone)]
pub struct Flare {
    /// Which flare; read by the tests and by the spark slices that will share the store.
    #[cfg_attr(not(test), allow(dead_code))]
    pub kind: FlareKind,
    /// The sim unit it hangs on and is visible with.
    pub object: EntityId,
    /// The sprite's name, lower case.
    pub sprite: String,
    /// Half size in world units.
    pub half: f32,
    /// Linear RGB the texture is multiplied with.
    pub colour: Vec3,
    /// Game time it began, in ticks.
    pub born: f64,
    /// Ticks it lives.
    pub life: f32,
    pub anchor: Anchor,
    /// Where it is now, render space.
    pub at: Vec3,
    /// Its object is visible: only then it is drawn.
    pub shown: bool,
}

/// The live flares per object and in all: the one store every node particle takes its slot from.
#[derive(Resource, Default, Debug)]
pub struct NodeParticles {
    slots: Vec<(EntityId, Entity)>,
}

impl NodeParticles {
    /// Live particles on `object`.
    pub fn live(&self, object: EntityId) -> usize {
        self.slots.iter().filter(|s| s.0 == object).count()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn total(&self) -> usize {
        self.slots.len()
    }

    /// Whether one more may be live on `object` (the newest loses over the cap, INTERIM).
    pub fn has_room(&self, object: EntityId) -> bool {
        self.slots.len() < MAX_TOTAL && self.live(object) < MAX_PER_OBJECT
    }

    #[cfg(test)]
    pub fn slots_for_test(&self) -> Vec<Entity> {
        self.slots.iter().map(|s| s.1).collect()
    }

    /// Take a slot for `entity` on `object`; false (nothing taken) over the cap.
    pub fn admit(&mut self, object: EntityId, entity: Entity) -> bool {
        if !self.has_room(object) {
            return false;
        }
        self.slots.push((object, entity));
        true
    }

    /// Give `entity`'s slot back.
    pub fn release(&mut self, entity: Entity) {
        self.slots.retain(|s| s.1 != entity);
    }

    /// Spawn `flare` if its object has a slot left.
    pub fn spawn(&mut self, cmds: &mut Commands, flare: Flare) -> Option<Entity> {
        if !self.has_room(flare.object) {
            return None;
        }
        let object = flare.object;
        let e = cmds.spawn(flare).id();
        self.admit(object, e).then_some(e)
    }
}

/// A unit's frame in render space: its position and its heading as a turn about the vertical.
pub fn frame_rotation(heading: Vec3) -> Quat {
    Quat::from_rotation_y((-heading.x).atan2(-heading.z))
}

/// The impact point's offset in a target's frame, from its world offset to the centre.
pub fn to_local(heading: Vec3, world_offset: Vec3) -> Vec3 {
    frame_rotation(heading).inverse() * world_offset
}

fn unit_frame(link: &SimLink, id: EntityId, alpha: f32, offsets: &Option<Res<crate::render::mining::VisualOffsets>>) -> Option<(Vec3, Vec3)> {
    let u = link.curr().unit(id)?;
    let centre = point(link, id, alpha)? + shifted(offsets, id);
    let heading = Vec3::new(u.heading.0.to_f32(), 0.0, -u.heading.1.to_f32()).normalize_or(Vec3::NEG_Z);
    Some((centre, heading))
}

/// Move every flare with its object, end the old ones, and hide those whose object is hidden.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_flares(
    mut cmds: Commands,
    offsets: Option<Res<crate::render::mining::VisualOffsets>>,
    clock: Res<AnimClock>,
    link: Res<SimLink>,
    ctx: StockShots,
    visibility: Query<&InheritedVisibility>,
    mut store: ResMut<NodeParticles>,
    mut q: Query<(Entity, &mut Flare)>,
) {
    let now = clock.now() * TICK_HZ as f64;
    let alpha = clock.alpha();
    for (e, mut f) in &mut q {
        // Its object gone, or its life run out: it ends, and its slot is free again.
        let place = match f.anchor {
            Anchor::Node(node) => point(&link, f.object, alpha).map(|p| p + shifted(&offsets, f.object) + node.and_then(|n| ctx.node_offset(f.object, n)).unwrap_or(Vec3::ZERO)),
            Anchor::Frame(local) => unit_frame(&link, f.object, alpha, &offsets).map(|(c, h)| c + frame_rotation(h) * local),
        };
        let Some(at) = place.filter(|_| now - f.born < f64::from(f.life)) else {
            store.release(e);
            cmds.entity(e).try_despawn();
            continue;
        };
        f.at = at;
        f.shown = ctx.object_visible(f.object, &visibility);
    }
}
