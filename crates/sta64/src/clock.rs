//! The animation clock (docs/WORKPLAN.md S9, docs/TESTBENCH.md "Determinism
//! and the animation clock").
//!
//! Anything that moves on screen reads [`AnimClock`], never Bevy's `Time`.
//! Cosmetic random sequences take their seed from [`AnimClock::seed`], per
//! object from [`AnimClock::entity_key`] (the sim's id, never a Bevy
//! `Entity`). Effects integrated per frame advance in fixed
//! [`AnimClock::SUBSTEP`]s. Bevy's `Time` is for what is not seen: logs,
//! file polling, profiling, and this clock's own wall input.
//!
//! The clock also decides how many sim ticks a frame runs; the sim link
//! asks it with [`AnimClock::begin_frame`] and reports each tick it ran.

use bevy::prelude::*;
use sta_sim::{EntityId, TICK_HZ, Tick};

/// Where frame time comes from.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ClockMode {
    /// Players: real frame time; cosmetic loops may follow [`AnimClock::wall`]
    /// while the sim is paused.
    #[default]
    Wall,
    /// The bench: every `App::update` is one frame of `step` seconds (the
    /// scene's `Step=`), so every frame's alpha is the same on every run, and
    /// `wall` is sim-driven too.
    Bench { step: f64 },
}

/// Anim time: sim time at the last tick plus `alpha` ticks. Stands while
/// the sim is paused.
#[derive(Resource, Debug, Clone)]
pub struct AnimClock {
    mode: ClockMode,
    /// Ticks the sim has run.
    tick: Tick,
    /// Frame time not yet turned into ticks, in seconds.
    debt: f64,
    alpha: f32,
    paused: bool,
    seed: u64,
    wall: f64,
    /// Seconds the last frame added to `wall`.
    frame_dt: f64,
}

impl Default for AnimClock {
    fn default() -> Self {
        AnimClock::new(ClockMode::Wall, 0)
    }
}

impl AnimClock {
    /// Fixed step for anything integrated per frame (particles, trails).
    pub const SUBSTEP: f64 = 1.0 / 120.0;
    /// At most this many ticks per frame; a longer stall is dropped rather
    /// than caught up in a burst.
    pub const MAX_TICKS_PER_FRAME: u32 = 8;

    pub fn new(mode: ClockMode, seed: u64) -> AnimClock {
        AnimClock { mode, tick: 0, debt: 0.0, alpha: 0.0, paused: false, seed, wall: 0.0, frame_dt: 0.0 }
    }

    pub fn mode(&self) -> ClockMode {
        self.mode
    }

    /// Anim time in seconds: sim time at the last tick plus alpha of a tick.
    pub fn now(&self) -> f64 {
        (self.tick as f64 + self.alpha as f64) / TICK_HZ as f64
    }

    /// Interpolation between the last two ticks, 0..1.
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Seconds of frames shown, running while paused: only for loops stock
    /// keeps running while paused (running lights, glows). In bench mode it
    /// counts frame steps, so it repeats too.
    pub fn wall(&self) -> f64 {
        self.wall
    }

    /// Seconds the last frame added to [`wall`](Self::wall); the step for
    /// per-frame cosmetic motion that follows wall time (spin, light swing).
    pub fn frame_dt(&self) -> f32 {
        self.frame_dt as f32
    }

    /// Seed for cosmetic random sequences; fixed in bench mode.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Cosmetic seed for one sim entity: the same for that id on every run.
    pub fn entity_key(&self, id: EntityId) -> u64 {
        splitmix64(self.seed ^ splitmix64(id.0 as u64))
    }

    /// Ticks the sim has run.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Start a frame `real_dt` seconds after the last one (ignored in bench
    /// mode) and say how many ticks are due. Run them, calling
    /// [`ticked`](Self::ticked) after each, then [`end_frame`](Self::end_frame).
    pub fn begin_frame(&mut self, real_dt: f64) -> u32 {
        let dt = match self.mode {
            ClockMode::Wall => real_dt.max(0.0),
            ClockMode::Bench { step } => step,
        };
        self.wall += dt;
        self.frame_dt = dt;
        if self.paused {
            return 0;
        }
        self.debt += dt;
        ((self.debt * TICK_HZ as f64) as u32).min(Self::MAX_TICKS_PER_FRAME)
    }

    /// A frame with no sim to step (the model viewer, a map before its sim is
    /// linked): only `wall` and `frame_dt` advance.
    pub fn advance_idle(&mut self, real_dt: f64) {
        let dt = match self.mode {
            ClockMode::Wall => real_dt.max(0.0),
            ClockMode::Bench { step } => step,
        };
        self.wall += dt;
        self.frame_dt = dt;
    }

    /// A frame the bench holds still (a frame settling, a paused session): nothing
    /// integrated per frame moves and `wall` stays, so a held picture is the one
    /// the last tick left, however many frames it is held for.
    pub fn hold(&mut self) {
        self.frame_dt = 0.0;
    }

    /// One tick ran.
    pub fn ticked(&mut self) {
        self.tick += 1;
        self.debt = (self.debt - 1.0 / TICK_HZ as f64).max(0.0);
    }

    /// Close the frame: drop debt beyond one tick (a stall or the per-frame
    /// cap; anim time does not race to catch up) and set alpha.
    pub fn end_frame(&mut self) {
        let tick_len = 1.0 / TICK_HZ as f64;
        if self.debt > tick_len {
            self.debt = tick_len;
        }
        self.alpha = (self.debt * TICK_HZ as f64).clamp(0.0, 1.0) as f32;
    }
}

/// Advances [`AnimClock::wall`] each frame when no sim link steps it. Add it
/// `run_if(not(resource_exists::<SimLink>))`, before the animation systems.
pub fn advance_idle_clock(time: Res<Time<Real>>, mut clock: ResMut<AnimClock>) {
    clock.advance_idle(time.delta_secs_f64());
}

/// SplitMix64 finaliser: a cheap, well-mixed, portable hash of a `u64`.
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run frames of `dt` and return the ticks each frame ran.
    fn frames(clock: &mut AnimClock, dt: f64, n: usize) -> Vec<u32> {
        (0..n)
            .map(|_| {
                let due = clock.begin_frame(dt);
                for _ in 0..due {
                    clock.ticked();
                }
                clock.end_frame();
                due
            })
            .collect()
    }

    #[test]
    fn wall_mode_runs_thirty_ticks_a_second() {
        let mut c = AnimClock::new(ClockMode::Wall, 1);
        let ran: u32 = frames(&mut c, 1.0 / 60.0, 120).iter().sum();
        assert!((59..=60).contains(&ran), "{ran}");
        assert!((c.now() - 2.0).abs() < 1.0 / 30.0, "{}", c.now());
        assert!((0.0..=1.0).contains(&c.alpha()));
        assert!((c.wall() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn pause_holds_anim_time_but_not_wall() {
        let mut c = AnimClock::new(ClockMode::Wall, 1);
        frames(&mut c, 0.05, 10);
        let (now, wall) = (c.now(), c.wall());
        c.set_paused(true);
        assert_eq!(frames(&mut c, 0.05, 10), vec![0; 10]);
        assert_eq!(c.now(), now);
        assert!(c.wall() > wall);
    }

    #[test]
    fn long_frames_do_not_burst() {
        let mut c = AnimClock::new(ClockMode::Wall, 1);
        assert_eq!(frames(&mut c, 5.0, 1), vec![AnimClock::MAX_TICKS_PER_FRAME]);
        assert!(c.alpha() <= 1.0);
        // The rest of the five seconds is dropped, not caught up.
        let after: u32 = frames(&mut c, 0.0, 3).iter().sum();
        assert!(after <= 1, "{after}");
    }

    #[test]
    fn bench_mode_ignores_real_time() {
        let run = |dts: &[f64]| {
            let mut c = AnimClock::new(ClockMode::Bench { step: 1.0 / 60.0 }, 7);
            let ticks: Vec<u32> = dts.iter().flat_map(|&dt| frames(&mut c, dt, 1)).collect();
            (ticks, c.alpha(), c.now(), c.wall())
        };
        assert_eq!(run(&[0.001, 0.5, 0.016, 0.1]), run(&[0.3, 0.0, 0.02, 0.0]));
    }

    #[test]
    fn entity_keys_are_stable_and_distinct() {
        let c = AnimClock::new(ClockMode::Wall, 42);
        assert_eq!(c.entity_key(EntityId(3)), AnimClock::new(ClockMode::Wall, 42).entity_key(EntityId(3)));
        assert_ne!(c.entity_key(EntityId(3)), c.entity_key(EntityId(4)));
        assert_ne!(c.entity_key(EntityId(3)), AnimClock::new(ClockMode::Wall, 43).entity_key(EntityId(3)));
    }
}
