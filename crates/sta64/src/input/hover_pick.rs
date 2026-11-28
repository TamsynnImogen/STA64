//! The hover pick rule of the Armada II readout (#1164), as pure functions on screen-space data.
//!
//! Grades: the rule is read from the executable, static; nothing here is measured. The values are
//! constants of the executable with no data key, kept as named constants and graded one by one.
//!
//! 1. Candidates are the objects drawn this frame, visible to the viewer, not being removed and not
//!    unpickable; the caller supplies that set (`hover.rs`, the same filter as before).
//! 2. They are compared by squared screen distance from the pointer to the projected centre, nearest
//!    first. Only an object nearer than the best accepted so far is tested, so a nearer object that
//!    fails the acceptance test never blocks a farther one that passes.
//! 3. Acceptance, in three stages: (a) the pointer is within [`MIN_RADIUS_PX`] of the projected centre (the
//!    original works in the HUD's canvas units, so the radius is that many canvas units, converted to window
//!    pixels by the stock HUD's canvas scale, [`CanvasUnit`]);
//!    else (b) it is within the projected bounding-sphere radius times `1 + speed / `[`SPHERE_SPEED_DIVISOR`],
//!    and (c) inside the container rectangle (the screen box of the projected model bounds) scaled about
//!    its centre by `1 + speed / `[`RECT_SPEED_DIVISOR`]. (b) and (c) are both required.
//! 4. Stickiness: when nothing valid is under the pointer, the previously hovered object stays hovered
//!    for up to [`STICKY_SECONDS`] while it moves faster than [`STICKY_MIN_SPEED`]. Moving onto another
//!    valid object switches at once, and hovering anything valid restarts the time.

use bevy::prelude::*;
use sta_sim::EntityId;

/// The pointer is accepted at once within this many canvas units of the projected centre [exe, static; the
/// value is the one the notes quote]. The routine projects into the HUD's canvas, so the unit is a canvas
/// unit, not a window pixel (read from the executable, static).
pub const MIN_RADIUS_PX: f32 = 16.0;

/// Window pixels per canvas unit of the stock HUD (its layout's scale), set while the stock HUD is shown; 1
/// otherwise. Converts [`MIN_RADIUS_PX`] to the pixels the pointer and the projections are in.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CanvasUnit(pub f32);

impl Default for CanvasUnit {
    fn default() -> Self {
        CanvasUnit(1.0)
    }
}
/// The container rectangle grows by `speed / this` [exe, static].
pub const RECT_SPEED_DIVISOR: f32 = 200.0;
/// The bounding-sphere radius grows by `speed / this`. INTERIM: the executable has a constant of its
/// own here that the notes do not give; it is taken equal to [`RECT_SPEED_DIVISOR`] until read.
pub const SPHERE_SPEED_DIVISOR: f32 = RECT_SPEED_DIVISOR;
/// The longest an object stays hovered with nothing valid under the pointer, seconds [exe, static].
pub const STICKY_SECONDS: f32 = 1.0;
/// It stays only while faster than this, world units per second [exe, static].
pub const STICKY_MIN_SPEED: f32 = 50.0;

/// One object as the pick sees it, all in screen pixels.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub id: EntityId,
    /// The projected centre.
    pub centre: Vec2,
    /// The projected bounding-sphere radius at the object's depth.
    pub radius: f32,
    /// The container rectangle: the screen box of the projected model bounds.
    pub rect: Rect,
    /// World units per second.
    pub speed: f32,
}

impl Candidate {
    /// Whether the pointer at `p` is on this object (stage a, or stages b and c together).
    pub fn accepts(&self, p: Vec2, unit_px: f32) -> bool {
        let near = MIN_RADIUS_PX * unit_px;
        if p.distance_squared(self.centre) <= near * near {
            return true;
        }
        let speed = self.speed.max(0.0);
        let sphere = self.radius * (1.0 + speed / SPHERE_SPEED_DIVISOR);
        if p.distance(self.centre) > sphere {
            return false;
        }
        let k = 1.0 + speed / RECT_SPEED_DIVISOR;
        let half = self.rect.half_size() * k;
        let c = self.rect.center();
        (p.x - c.x).abs() <= half.x && (p.y - c.y).abs() <= half.y
    }
}

/// The object under the pointer at `p`: nearest centre first, only an object nearer than the best
/// accepted so far is tested (stage 2 above).
pub fn pick(candidates: &[Candidate], p: Vec2, unit_px: f32) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for c in candidates {
        let d = p.distance_squared(c.centre);
        if best.is_some_and(|(b, _)| d >= b) {
            continue;
        }
        if c.accepts(p, unit_px) {
            best = Some((d, c.id));
        }
    }
    best.map(|(_, id)| id)
}

/// Stickiness (stage 4): the previous hover and the time left to hold it. Kept as a system-local state.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Sticky {
    pub previous: Option<EntityId>,
    pub left: f32,
}

impl Sticky {
    /// One display tick of `dt` seconds. `picked` is what the pick found; `previous_speed` the speed of
    /// the previous hover if it is still a valid object, else `None`.
    pub fn resolve(&mut self, dt: f32, picked: Option<EntityId>, previous_speed: impl Fn(EntityId) -> Option<f32>) -> Option<EntityId> {
        if let Some(id) = picked {
            self.previous = Some(id);
            self.left = STICKY_SECONDS;
            return Some(id);
        }
        if self.left > 0.0
            && let Some(prev) = self.previous
            && previous_speed(prev).is_some_and(|s| s > STICKY_MIN_SPEED)
        {
            self.left -= dt.max(0.0);
            return Some(prev);
        }
        self.previous = None;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: u32, centre: Vec2, radius: f32, half: Vec2, speed: f32) -> Candidate {
        Candidate { id: EntityId(id), centre, radius, rect: Rect::from_center_half_size(centre, half), speed }
    }

    #[test]
    fn near_the_centre_is_accepted_whatever_the_size() {
        let c = cand(1, Vec2::new(100.0, 100.0), 2.0, Vec2::splat(2.0), 0.0);
        assert!(c.accepts(Vec2::new(100.0 + MIN_RADIUS_PX - 0.1, 100.0), 1.0));
        assert!(!c.accepts(Vec2::new(100.0 + MIN_RADIUS_PX + 3.0, 100.0), 1.0), "past the radius and outside the box");
    }

    #[test]
    fn the_fixed_radius_is_in_canvas_units_so_it_follows_the_window_size() {
        let c = cand(1, Vec2::new(100.0, 100.0), 2.0, Vec2::splat(2.0), 0.0);
        let p = Vec2::new(100.0 + 1.5 * MIN_RADIUS_PX, 100.0);
        assert!(!c.accepts(p, 1.0), "a small window (one pixel per unit): outside");
        assert!(c.accepts(p, 2.0), "a window twice as large: the same pointer is still within the radius");
        assert_eq!(pick(&[c], p, 2.0), Some(EntityId(1)));
    }

    #[test]
    fn a_big_object_is_accepted_inside_its_sphere_and_box_not_in_a_corner_of_the_sphere() {
        let c = cand(1, Vec2::ZERO, 60.0, Vec2::new(60.0, 20.0), 0.0);
        assert!(c.accepts(Vec2::new(50.0, 10.0), 1.0));
        assert!(!c.accepts(Vec2::new(30.0, 40.0), 1.0), "inside the sphere, outside the box: both are required");
    }

    #[test]
    fn speed_grows_the_tolerance() {
        let slow = cand(1, Vec2::ZERO, 30.0, Vec2::new(30.0, 30.0), 0.0);
        let fast = cand(1, Vec2::ZERO, 30.0, Vec2::new(30.0, 30.0), RECT_SPEED_DIVISOR);
        let p = Vec2::new(40.0, 0.0);
        assert!(!slow.accepts(p, 1.0) && fast.accepts(p, 1.0), "twice the tolerance at speed = the divisor");
    }

    #[test]
    fn nearest_first_and_a_nearer_object_that_fails_does_not_block_a_farther_one() {
        let near_small = cand(1, Vec2::new(30.0, 0.0), 5.0, Vec2::splat(5.0), 0.0);
        let far_big = cand(2, Vec2::new(60.0, 0.0), 80.0, Vec2::splat(80.0), 0.0);
        let p = Vec2::ZERO;
        assert!(!near_small.accepts(p, 1.0) && far_big.accepts(p, 1.0));
        assert_eq!(pick(&[near_small, far_big], p, 1.0), Some(EntityId(2)));
        // Both accept: the nearer centre wins, in either order.
        let a = cand(3, Vec2::new(5.0, 0.0), 10.0, Vec2::splat(10.0), 0.0);
        let b = cand(4, Vec2::new(9.0, 0.0), 10.0, Vec2::splat(10.0), 0.0);
        assert_eq!(pick(&[a, b], p, 1.0), Some(EntityId(3)));
        assert_eq!(pick(&[b, a], p, 1.0), Some(EntityId(3)));
        assert_eq!(pick(&[], p, 1.0), None);
    }

    #[test]
    fn a_fast_mover_stays_hovered_over_empty_space_for_a_second_and_a_still_ship_does_not() {
        let speeds = |fast: bool| move |_: EntityId| Some(if fast { STICKY_MIN_SPEED + 1.0 } else { 0.0 });
        let mut s = Sticky::default();
        assert_eq!(s.resolve(0.1, Some(EntityId(1)), speeds(true)), Some(EntityId(1)));
        // The pointer leaves it: held while the time lasts.
        let mut held = 0.0;
        while s.resolve(0.1, None, speeds(true)).is_some() {
            held += 0.1;
            assert!(held < STICKY_SECONDS + 0.5);
        }
        assert!((STICKY_SECONDS - 0.1..=STICKY_SECONDS + 0.2).contains(&held), "{held}");
        // A still ship is dropped at once.
        let mut s = Sticky::default();
        s.resolve(0.1, Some(EntityId(1)), speeds(false));
        assert_eq!(s.resolve(0.1, None, speeds(false)), None);
        // Another valid object switches at once, and hovering restarts the time.
        let mut s = Sticky::default();
        s.resolve(0.1, Some(EntityId(1)), speeds(true));
        s.resolve(0.9, None, speeds(true));
        assert_eq!(s.resolve(0.1, Some(EntityId(2)), speeds(true)), Some(EntityId(2)));
        assert_eq!(s.left, STICKY_SECONDS);
        // A previous hover that is no longer valid is not held.
        let mut s = Sticky::default();
        s.resolve(0.1, Some(EntityId(1)), speeds(true));
        assert_eq!(s.resolve(0.1, None, |_| None), None);
    }
}
