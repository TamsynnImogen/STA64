//! The shot director of the Armada II cinematic window (#1167): which event the window follows, when it switches and
//! how the camera stands. Pure rules, no Bevy systems; `action_camera_a2.rs` feeds it and reads it.
//!
//! Grade of every rule below: read from the executable, static (a read of the engine's shot director and camera
//! code); keys read from data where a key is named. **Nothing here is measured**: the dwell times, the cuts and the
//! framing in practice are for a later bench session. The Armada I window keeps its own rules (`action_camera.rs`);
//! these are not assumed to carry over to it.
//!
//! - **Shots.** An event adds a shot with a base priority, a duration and a size. A shot's live priority is its
//!   base priority times the share of its duration still left, so it decays to nothing.
//! - **Scenes.** A new shot joins the nearest scene within [`SCENE_RADIUS`] of the scene's place (a squared
//!   comparison), else it starts a scene, whose place is that shot's position (read from the executable, static).
//! - **Choosing.** Scenes are ranked by their best shot's live priority and the top one is taken. The current scene
//!   stays when it is the top, or when [`SWITCH_FACTOR`] times the top's live priority is at most its own (read from the
//!   executable, static, as the test is written). As read, with live priorities that are never negative, that test
//!   keeps the current scene only while it is on top (or while both are zero): it protects nothing beyond that.
//!   The shot shown is taken out of its scene, and a scene left with no shot is dropped (static). INTERIM: the original
//!   opens a switch with a wide master shot of the scene; that shot is not built.
//! - **Dwell.** The next choice is due after the shown shot's time left, at least [`MIN_DWELL`] and at most its duration.
//! - **Fallback.** With no scene, a random craft the viewer's team can see is shown for [`FALLBACK_DURATION`] at
//!   close-up size, priority zero.
//! - **Framing.** The camera stands at the first live entity's place plus the radius times a direction. The radius
//!   is twice the entity's director radius (its bounding radius times the class key `directorRadiusScale`) times
//!   the shot size's scale (at least one), at least [`MIN_RADIUS`]; with no entity the size times [`MIN_RADIUS`],
//!   clamped. The direction comes from the horizontal line of action between the first two entities; the angle class
//!   puts the camera above when the share of entities of the first entity's team is high, below when low.
//!   INTERIM: how the line of action is mirrored, the threshold under which two entities are too close for a line, the
//!   composition of the height with the direction, and which team the share is taken against are not read; and
//!   every shot is tracked (the movement kind of each call site is not read).
//! - **Weapon-fire shots** ([`fire_shot`]) take their durations from a decode of the engine's constant symbol names
//!   (INTERIM: decoded from symbol names; an earlier note quotes other numbers). The priority of a missile or beam
//!   shot and the special-weapon flag are not read for STA64's weapons: INTERIM, the aimed-ordnance priority is
//!   used and the special priority is never.

use bevy::math::Vec3;
use sta_sim::EntityId;

/// The seven shot sizes, close to far. Not every size is used by a call site STA64 has yet (the wormhole and dock
/// shots of the original are not built).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ShotSize {
    /// Extreme close-up.
    Ecu,
    /// Medium close-up.
    Mcu,
    /// Close-up.
    Cu,
    /// Medium shot.
    Ms,
    /// Wide shot.
    Ws,
    /// Long shot.
    Ls,
    /// Extreme long shot.
    Els,
}

impl ShotSize {
    /// The multiplier of the framing radius (read from the executable, static).
    pub const fn scale(self) -> f32 {
        match self {
            ShotSize::Ecu => 1.0,
            ShotSize::Mcu => 1.25,
            ShotSize::Cu => 1.5,
            ShotSize::Ms => 2.0,
            ShotSize::Ws => 4.0,
            ShotSize::Ls => 6.0,
            ShotSize::Els => 10.0,
        }
    }
}

/// A shot joins a scene within this distance of it (read from the executable, static).
pub const SCENE_RADIUS: f32 = 600.0;
/// The camera is never nearer than this to what it looks at (read from the executable, static).
pub const MIN_RADIUS: f32 = 50.0;
/// The radius of a shot with no entity is its size times [`MIN_RADIUS`], at most this (read from the executable, static).
pub const MAX_RADIUS: f32 = 600.0;
/// The current scene is kept when this times the top's live priority is at most its own (read from the executable,
/// static; as read it keeps the current scene only while it is on top).
pub const SWITCH_FACTOR: f32 = 2.0;
/// The shortest time a chosen shot is held, seconds (read from the executable, static).
pub const MIN_DWELL: f32 = 1.0;
/// The fallback shot of a random visible craft: duration, seconds (read from the executable, static).
pub const FALLBACK_DURATION: f32 = 2.5;
/// Share of entities of the first entity's team from which the camera stands above (read from the executable, static).
pub const ABOVE_SHARE: f32 = 0.6;
/// Share under which the camera stands below (read from the executable, static).
pub const BELOW_SHARE: f32 = 0.4;
/// How far above or below the camera stands, in radii (read from the executable, static).
pub const HEIGHT_PER_RADIUS: f32 = 0.5;
/// Two entities nearer than this give no line of action (INTERIM: the threshold is not read; this only avoids a
/// degenerate direction).
pub const LINE_MIN: f32 = 1.0;

/// One shot.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub priority: f32,
    pub duration: f32,
    pub size: ShotSize,
    /// Seconds of the app clock it was added.
    pub born: f32,
    /// What it shows; the first live one is tracked.
    pub entities: Vec<EntityId>,
    /// Where it happened, for the scene it joins.
    pub at: Vec3,
}

impl Shot {
    /// Seconds left at `now`.
    pub fn remaining(&self, now: f32) -> f32 {
        (self.born + self.duration - now).max(0.0)
    }

    /// Base priority times the share of the duration left.
    pub fn live_priority(&self, now: f32) -> f32 {
        if self.duration <= 0.0 { 0.0 } else { self.priority * self.remaining(now) / self.duration }
    }
}

#[derive(Debug, Clone)]
struct Scene {
    id: u64,
    at: Vec3,
    shots: Vec<Shot>,
}

impl Scene {
    fn best(&self, now: f32) -> Option<&Shot> {
        // Highest live priority; the earlier shot wins a tie.
        self.shots.iter().filter(|s| s.remaining(now) > 0.0).fold(None, |best: Option<&Shot>, s| match best {
            Some(b) if b.live_priority(now) >= s.live_priority(now) => Some(b),
            _ => Some(s),
        })
    }

    fn live(&self, now: f32) -> f32 {
        self.best(now).map_or(0.0, |s| s.live_priority(now))
    }
}

/// The scenes, the scene shown and when the next choice is due.
#[derive(Debug, Clone, Default)]
pub struct Director {
    scenes: Vec<Scene>,
    next_id: u64,
    current: Option<u64>,
    due: f32,
}

impl Director {
    /// Add a shot to the nearest scene within the radius (a squared comparison), or start one at the shot's position.
    pub fn add(&mut self, shot: Shot) {
        let limit = SCENE_RADIUS * SCENE_RADIUS;
        let nearest = self
            .scenes
            .iter_mut()
            .map(|s| (s.at.distance_squared(shot.at), s))
            .filter(|(d, _)| *d <= limit)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, s)| s);
        if let Some(scene) = nearest {
            scene.shots.push(shot);
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.scenes.push(Scene { id, at: shot.at, shots: vec![shot] });
    }

    /// Whether the next choice is due.
    pub fn due(&self, now: f32) -> bool {
        now >= self.due
    }

    /// Forget everything (a match ended).
    pub fn clear(&mut self) {
        *self = Director::default();
    }

    /// Choose the shot to show at `now` and set when the next choice is due. `fallback` is a random craft the viewer's
    /// team can see, used when no scene has a live shot. `None` when there is nothing to show.
    pub fn choose(&mut self, now: f32, fallback: Option<EntityId>) -> Option<Shot> {
        for scene in &mut self.scenes {
            scene.shots.retain(|s| s.remaining(now) > 0.0);
        }
        self.scenes.retain(|s| !s.shots.is_empty());
        // Ranked by the best live priority; the earlier scene wins a tie.
        let top = self.scenes.iter().fold(None, |best: Option<&Scene>, s| match best {
            Some(b) if b.live(now) >= s.live(now) => Some(b),
            _ => Some(s),
        });
        let Some(top) = top else {
            self.current = None;
            let id = fallback?;
            let shot = Shot { priority: 0.0, duration: FALLBACK_DURATION, size: ShotSize::Cu, born: now, entities: vec![id], at: Vec3::ZERO };
            self.due = now + dwell(&shot, now);
            return Some(shot);
        };
        let current = self.current.and_then(|id| self.scenes.iter().find(|s| s.id == id));
        // The stay test as read: the current scene is the top, or twice the top's live priority is at most its own.
        let chosen = match current {
            Some(cur) if cur.id == top.id || SWITCH_FACTOR * top.live(now) <= cur.live(now) => cur,
            _ => top,
        };
        let id = chosen.id;
        let best: *const Shot = chosen.best(now)?;
        let at = chosen.shots.iter().position(|s| std::ptr::eq(s, best))?;
        // The shot shown leaves its scene; a scene left with no shot is dropped (read from the executable, static).
        let scene = self.scenes.iter_mut().find(|s| s.id == id)?;
        let shot = scene.shots.remove(at);
        self.scenes.retain(|s| !s.shots.is_empty());
        self.current = Some(id);
        self.due = now + dwell(&shot, now);
        Some(shot)
    }
}

/// How long a chosen shot is held: its time left, at least [`MIN_DWELL`], at most its duration.
pub fn dwell(shot: &Shot, now: f32) -> f32 {
    shot.remaining(now).max(MIN_DWELL).min(shot.duration)
}

/// What a camera looks from: above, level or below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Angle {
    Above,
    Level,
    Below,
}

/// The angle class from the share of the shot's entities on the first entity's team: high is above, low is below.
pub fn angle_class(same_team_share: f32) -> Angle {
    if same_team_share >= ABOVE_SHARE {
        Angle::Above
    } else if same_team_share < BELOW_SHARE {
        Angle::Below
    } else {
        Angle::Level
    }
}

/// The framing radius: with an entity (`director_radius` its bounding radius times its class scale) twice that times the
/// size's scale (at least one), at least [`MIN_RADIUS`]; without, the size's scale times [`MIN_RADIUS`], clamped.
pub fn framing_radius(director_radius: Option<f32>, size: ShotSize) -> f32 {
    match director_radius {
        Some(r) => (2.0 * r * size.scale().max(1.0)).max(MIN_RADIUS),
        None => (size.scale() * MIN_RADIUS).clamp(MIN_RADIUS, MAX_RADIUS),
    }
}

/// The horizontal unit direction from `a` to `b`, or none when they are too close for a line of action.
pub fn line_of_action(a: Vec3, b: Vec3) -> Option<Vec3> {
    let d = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
    (d.length() >= LINE_MIN).then(|| d.normalize())
}

/// The camera's offset from what it looks at: the radius along the horizontal direction, and half a radius up or down
/// by the angle class.
pub fn camera_offset(radius: f32, direction: Vec3, angle: Angle) -> Vec3 {
    let height = match angle {
        Angle::Above => HEIGHT_PER_RADIUS,
        Angle::Level => 0.0,
        Angle::Below => -HEIGHT_PER_RADIUS,
    };
    direction * radius + Vec3::Y * (height * radius)
}

// ---------------------------------------------------------------------------------------------
// Shots the events make
// ---------------------------------------------------------------------------------------------

/// What kind of ordnance a fired weapon makes, for its shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordnance {
    /// A bullet-class shot (pulses, artillery).
    Bullet,
    /// A missile-class shot (torpedoes and their kin).
    Missile,
    /// A beam or any other aimed ordnance.
    Beam,
    /// Ordnance with no target entity.
    Free,
}

/// Aimed ordnance: base priority of a weapon-fire shot (read from the executable, static).
pub const FIRE_PRIORITY: f32 = 8.0;
/// Free ordnance: base priority (read from the executable, static).
pub const FREE_PRIORITY: f32 = 1.0;
/// Bullets: the flight time is clamped to this, then [`BULLET_EXTRA`] is added (decoded from symbol names, INTERIM).
pub const BULLET_FLIGHT_MAX: f32 = 4.5;
pub const BULLET_EXTRA: f32 = 0.5;
/// Missiles: the same (decoded from symbol names, INTERIM).
pub const MISSILE_FLIGHT_MAX: f32 = 3.0;
pub const MISSILE_EXTRA: f32 = 2.0;
/// Beams, other aimed ordnance and free ordnance: duration (decoded from symbol names, INTERIM).
pub const OTHER_DURATION: f32 = 3.0;

/// The priority, duration and size of the shot a weapon's fire makes. `gap` is the distance between the two
/// bounding spheres and `speed` the ordnance speed (zero: no flight time).
pub fn fire_shot(kind: Ordnance, gap: f32, speed: f32) -> (f32, f32, ShotSize) {
    let flight = if speed > 0.0 { (gap / speed).max(0.0) } else { 0.0 };
    match kind {
        Ordnance::Bullet => (FIRE_PRIORITY, flight.min(BULLET_FLIGHT_MAX) + BULLET_EXTRA, ShotSize::Cu),
        Ordnance::Missile => (FIRE_PRIORITY, flight.min(MISSILE_FLIGHT_MAX) + MISSILE_EXTRA, ShotSize::Ecu),
        // INTERIM: the notes give a beam or other ordnance one of two sizes (medium or extreme long) by a case that
        // is not stated; the medium one is taken always.
        Ordnance::Beam => (FIRE_PRIORITY, OTHER_DURATION, ShotSize::Ms),
        Ordnance::Free => (FREE_PRIORITY, OTHER_DURATION, ShotSize::Cu),
    }
}

/// A construction starting: priority, duration, size (read from the executable, static).
pub const CONSTRUCTION_START: (f32, f32, ShotSize) = (4.0, 5.0, ShotSize::Cu);
/// A construction finishing.
pub const CONSTRUCTION_FINISH: (f32, f32, ShotSize) = (4.0, 3.0, ShotSize::Cu);
/// A ship finished at a shipyard.
pub const SHIP_FINISHED: (f32, f32, ShotSize) = (0.0, 3.0, ShotSize::Ws);
/// A craft's fireball (INTERIM: the original also adds a shot when the death sequence starts, and takes the larger
/// of two sizes by a case not read; STA64 has no death sequence yet, #1100, so its removal stands in for the fireball).
pub const FIREBALL: (f32, f32, ShotSize) = (14.0, 1.0, ShotSize::Ms);

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(priority: f32, duration: f32, born: f32, at: Vec3, e: u32) -> Shot {
        Shot { priority, duration, size: ShotSize::Cu, born, entities: vec![EntityId(e)], at }
    }

    #[test]
    fn a_shots_priority_decays_with_the_time_it_has_left() {
        let s = shot(8.0, 4.0, 10.0, Vec3::ZERO, 1);
        assert_eq!(s.live_priority(10.0), 8.0);
        assert_eq!(s.live_priority(12.0), 4.0);
        assert_eq!(s.live_priority(14.0), 0.0);
        assert_eq!(s.live_priority(99.0), 0.0, "never negative");
        assert_eq!(shot(8.0, 0.0, 0.0, Vec3::ZERO, 1).live_priority(0.0), 0.0, "no division by a zero duration");
    }

    #[test]
    fn shots_within_the_scene_radius_share_a_scene_and_others_start_one() {
        let mut d = Director::default();
        d.add(shot(1.0, 5.0, 0.0, Vec3::ZERO, 1));
        d.add(shot(1.0, 5.0, 0.0, Vec3::new(SCENE_RADIUS, 0.0, 0.0), 2));
        d.add(shot(1.0, 5.0, 0.0, Vec3::new(SCENE_RADIUS + 1.0, 0.0, 0.0), 3));
        assert_eq!(d.scenes.len(), 2);
        assert_eq!(d.scenes[0].shots.len(), 2, "exactly the radius still joins (a squared comparison)");
    }

    #[test]
    fn the_scene_with_the_best_live_priority_is_shown_and_the_best_shot_in_it() {
        let mut d = Director::default();
        d.add(shot(4.0, 5.0, 0.0, Vec3::ZERO, 1));
        d.add(shot(8.0, 5.0, 0.0, Vec3::ZERO, 2));
        d.add(shot(14.0, 5.0, 0.0, Vec3::new(5000.0, 0.0, 0.0), 3));
        let pick = d.choose(0.0, None).unwrap();
        assert_eq!(pick.entities, vec![EntityId(3)], "the far scene's 14 beats the near scene's 8");
    }

    #[test]
    fn the_current_scene_stays_only_while_it_is_the_top_as_the_stay_test_reads() {
        // Scene one holds two shots; the shown shot leaves it.
        let mut d = Director::default();
        d.add(shot(10.0, 100.0, 0.0, Vec3::ZERO, 1));
        d.add(shot(9.0, 100.0, 0.0, Vec3::ZERO, 2));
        d.add(shot(5.0, 100.0, 0.0, Vec3::new(5000.0, 0.0, 0.0), 3));
        assert_eq!(d.choose(0.0, None).unwrap().entities, vec![EntityId(1)]);
        // Another scene, weaker than the current one's next shot: the current scene is still the top and stays.
        d.add(shot(8.0, 100.0, 0.0, Vec3::new(-5000.0, 0.0, 0.0), 4));
        assert_eq!(d.choose(0.0, None).unwrap().entities, vec![EntityId(2)], "the current scene is the top");
        // A scene that is the top takes over from the current one whatever the ratio: twice the top's priority is
        // never at most the current's (the rule as read; it protects nothing beyond the top).
        let mut d = Director::default();
        d.add(shot(10.0, 100.0, 0.0, Vec3::ZERO, 1));
        d.add(shot(9.0, 100.0, 0.0, Vec3::ZERO, 2));
        assert_eq!(d.choose(0.0, None).unwrap().entities, vec![EntityId(1)]);
        d.add(shot(9.5, 100.0, 0.0, Vec3::new(5000.0, 0.0, 0.0), 3));
        assert_eq!(d.choose(0.0, None).unwrap().entities, vec![EntityId(3)], "9.5 is above the current scene's 9: it takes over");
    }

    #[test]
    fn a_shown_shot_leaves_its_scene_and_an_emptied_scene_is_dropped() {
        let mut d = Director::default();
        d.add(shot(8.0, 100.0, 0.0, Vec3::ZERO, 1));
        assert_eq!(d.choose(0.0, None).unwrap().entities, vec![EntityId(1)]);
        assert!(d.scenes.is_empty(), "the scene had one shot");
        assert!(d.choose(0.0, None).is_none(), "the shot is not shown again");
        assert_eq!(d.choose(0.0, Some(EntityId(9))).unwrap().entities, vec![EntityId(9)], "the fallback instead");
    }

    #[test]
    fn a_shot_joins_the_nearest_scene_in_range_and_a_scenes_place_is_its_first_shots() {
        let mut d = Director::default();
        d.add(shot(1.0, 5.0, 0.0, Vec3::ZERO, 1));
        d.add(shot(1.0, 5.0, 0.0, Vec3::new(700.0, 0.0, 0.0), 2));
        assert_eq!(d.scenes.len(), 2);
        // In range of both scenes (400 and 300 away): the nearer.
        d.add(shot(1.0, 5.0, 0.0, Vec3::new(400.0, 0.0, 0.0), 3));
        assert_eq!((d.scenes[0].shots.len(), d.scenes[1].shots.len()), (1, 2));
        assert_eq!(d.scenes[1].at, Vec3::new(700.0, 0.0, 0.0), "joining does not move the scene");
    }

    #[test]
    fn the_next_choice_is_due_after_the_time_left_between_the_minimum_dwell_and_the_duration() {
        let s = shot(8.0, 5.0, 0.0, Vec3::ZERO, 1);
        assert_eq!(dwell(&s, 0.0), 5.0);
        assert_eq!(dwell(&s, 2.0), 3.0);
        assert_eq!(dwell(&s, 4.5), MIN_DWELL, "a short rest is held for the minimum");
        let short = shot(8.0, 0.5, 0.0, Vec3::ZERO, 1);
        assert_eq!(dwell(&short, 0.0), 0.5, "never more than the duration");
        let mut d = Director::default();
        d.add(s);
        d.choose(2.0, None);
        assert!(!d.due(4.9) && d.due(5.0));
    }

    #[test]
    fn with_no_live_shot_a_random_visible_craft_is_shown_for_the_fallback_duration() {
        let mut d = Director::default();
        let pick = d.choose(3.0, Some(EntityId(7))).unwrap();
        assert_eq!((pick.entities, pick.priority, pick.duration, pick.size), (vec![EntityId(7)], 0.0, FALLBACK_DURATION, ShotSize::Cu));
        assert!(!d.due(3.0 + FALLBACK_DURATION - 0.01) && d.due(3.0 + FALLBACK_DURATION));
        assert!(Director::default().choose(0.0, None).is_none(), "nothing to show");
        // An expired shot is forgotten.
        let mut d = Director::default();
        d.add(shot(8.0, 2.0, 0.0, Vec3::ZERO, 1));
        assert_eq!(d.choose(5.0, Some(EntityId(9))).unwrap().entities, vec![EntityId(9)]);
    }

    #[test]
    fn the_framing_radius_follows_the_size_the_entity_and_the_minimum() {
        assert_eq!(framing_radius(Some(30.0), ShotSize::Cu), 2.0 * 30.0 * 1.5);
        assert_eq!(framing_radius(Some(30.0), ShotSize::Ecu), 60.0);
        assert_eq!(framing_radius(Some(5.0), ShotSize::Ms), MIN_RADIUS, "never nearer than the minimum");
        assert_eq!(framing_radius(Some(30.0), ShotSize::Els), 600.0);
        assert_eq!(framing_radius(None, ShotSize::Cu), 1.5 * MIN_RADIUS, "no entity: the size times the minimum");
        assert_eq!(framing_radius(None, ShotSize::Ecu), MIN_RADIUS);
        assert_eq!(framing_radius(None, ShotSize::Els), ShotSize::Els.scale() * MIN_RADIUS);
        assert!(framing_radius(None, ShotSize::Els) <= MAX_RADIUS, "within the clamp");
        assert!(ShotSize::Ecu.scale() < ShotSize::Mcu.scale() && ShotSize::Ls.scale() < ShotSize::Els.scale());
    }

    #[test]
    fn the_angle_class_and_the_line_of_action_place_the_camera() {
        assert_eq!((angle_class(1.0), angle_class(ABOVE_SHARE), angle_class(0.5), angle_class(BELOW_SHARE), angle_class(0.39)), (Angle::Above, Angle::Above, Angle::Level, Angle::Level, Angle::Below));
        let dir = line_of_action(Vec3::ZERO, Vec3::new(0.0, 7.0, -10.0)).unwrap();
        assert!((dir - Vec3::NEG_Z).length() < 1e-6, "horizontal: the height is dropped");
        assert!(line_of_action(Vec3::ZERO, Vec3::new(0.2, 0.0, 0.2)).is_none(), "too close for a line");
        let up = camera_offset(100.0, Vec3::X, Angle::Above);
        let down = camera_offset(100.0, Vec3::X, Angle::Below);
        assert_eq!((up, down, camera_offset(100.0, Vec3::X, Angle::Level)), (Vec3::new(100.0, 50.0, 0.0), Vec3::new(100.0, -50.0, 0.0), Vec3::new(100.0, 0.0, 0.0)));
    }

    #[test]
    fn a_weapon_fire_shot_follows_its_ordnance_class() {
        // A bullet: flight time clamped, plus its extra; close-up.
        assert_eq!(fire_shot(Ordnance::Bullet, 100.0, 50.0), (FIRE_PRIORITY, 2.0 + BULLET_EXTRA, ShotSize::Cu));
        assert_eq!(fire_shot(Ordnance::Bullet, 1.0e6, 50.0).1, BULLET_FLIGHT_MAX + BULLET_EXTRA);
        // A missile: its own clamp and extra; extreme close-up.
        assert_eq!(fire_shot(Ordnance::Missile, 100.0, 50.0), (FIRE_PRIORITY, 2.0 + MISSILE_EXTRA, ShotSize::Ecu));
        assert_eq!(fire_shot(Ordnance::Missile, 1.0e6, 50.0).1, MISSILE_FLIGHT_MAX + MISSILE_EXTRA);
        // Overlapping spheres: no negative flight; a zero speed: none.
        assert_eq!(fire_shot(Ordnance::Bullet, -20.0, 50.0).1, BULLET_EXTRA);
        assert_eq!(fire_shot(Ordnance::Missile, 100.0, 0.0).1, MISSILE_EXTRA);
        assert_eq!(fire_shot(Ordnance::Beam, 100.0, 0.0), (FIRE_PRIORITY, OTHER_DURATION, ShotSize::Ms));
        assert_eq!(fire_shot(Ordnance::Free, 100.0, 0.0), (FREE_PRIORITY, OTHER_DURATION, ShotSize::Cu));
    }
}
