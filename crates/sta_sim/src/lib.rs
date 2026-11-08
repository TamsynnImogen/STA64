//! Deterministic lockstep simulation core.
//!
//! Rules for everything in this crate:
//! - No floats in simulation state. Positions, velocities and resources use
//!   [`Fixed`] so x86-64, ARM64 and Apple Silicon produce identical results.
//!   Floats only enter when loading data (`Fixed::from_f64` of parsed ODF values,
//!   which are correctly rounded everywhere) and leave only for rendering.
//! - No hash-map iteration order, wall-clock time, or thread scheduling may
//!   influence state. Entity iteration is by ascending id.
//! - All randomness comes from the seeded [`Rng`].
//! - No platform maths functions (`sin`, `atan2`, ...): steering uses dot and
//!   cross products, small-angle polynomials and the exact integer square root.
//! - Each tick ends with a [`Sim::digest`] that peers compare to detect desyncs.
//!
//! Space is the maps' own: Direct3D left-handed, +Y up. Units move in their
//! heading in the XZ plane and climb or sink toward a target height.

pub mod fixed;
pub use fixed::Fixed;

/// Simulation ticks per second. Armada ran its sim at a fixed rate independent
/// of frame rate; we do the same and let the renderer interpolate.
pub const TICK_HZ: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vec3 {
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,
}

impl Vec3 {
    pub fn from_f32(v: [f32; 3]) -> Vec3 {
        Vec3 {
            x: Fixed::from_f64(v[0] as f64),
            y: Fixed::from_f64(v[1] as f64),
            z: Fixed::from_f64(v[2] as f64),
        }
    }
    pub fn to_f32(self) -> [f32; 3] {
        [self.x.to_f32(), self.y.to_f32(), self.z.to_f32()]
    }
}

/// Movement limits, from a craft's physics ODF (`physicsFile`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveStats {
    /// Cruise speed, units per second (`impulseSpeed`).
    pub max_speed: Fixed,
    /// Seconds-to-full-speed reciprocal (`forwardAccel`): speed changes by
    /// `forward_accel * max_speed` per second. Our reading of the key.
    pub forward_accel: Fixed,
    /// Turn rate, radians per second (`turnOmega`).
    pub turn_omega: Fixed,
    /// Turn acceleration, radians per second squared (`turnAlpha`).
    pub turn_alpha: Fixed,
    /// Share of the turn rate available at rest (`turnOmegaFractionAtRest`).
    pub turn_at_rest: Fixed,
}

impl MoveStats {
    pub fn from_f32(max_speed: f32, forward_accel: f32, turn_omega: f32, turn_alpha: f32, turn_at_rest: f32) -> MoveStats {
        let f = |v: f32| Fixed::from_f64(v as f64);
        MoveStats {
            max_speed: f(max_speed),
            forward_accel: f(forward_accel.max(0.01)),
            turn_omega: f(turn_omega),
            turn_alpha: f(turn_alpha.max(0.01)),
            turn_at_rest: f(turn_at_rest.clamp(0.0, 1.0)),
        }
    }
}

impl Default for MoveStats {
    fn default() -> Self {
        MoveStats::from_f32(100.0, 1.0, 1.0, 1.0, 0.2)
    }
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub id: EntityId,
    pub player: u8,
    pub pos: Vec3,
    /// Unit heading in the XZ plane: (x, z), length one.
    pub heading: (Fixed, Fixed),
    pub speed: Fixed,
    /// Current turn rate, radians per second; positive turns from +X toward +Z.
    pub omega: Fixed,
    pub target: Option<Vec3>,
    pub stats: MoveStats,
}

/// Player commands are the only input to the simulation.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Move { unit: EntityId, to: Vec3 },
    Stop { unit: EntityId },
}

/// xorshift64* — small, fast and identical everywhere.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[derive(Debug, Clone)]
pub struct Sim {
    pub tick: u64,
    pub rng: Rng,
    /// Sorted by id; never reordered.
    pub units: Vec<Unit>,
    next_id: u32,
}

fn dt() -> Fixed {
    Fixed::ONE / Fixed::from_int(TICK_HZ as i32)
}

fn clamp(v: Fixed, lo: Fixed, hi: Fixed) -> Fixed {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Move `v` toward `goal` by at most `step`.
fn approach(v: Fixed, goal: Fixed, step: Fixed) -> Fixed {
    if v < goal { if goal - v < step { goal } else { v + step } } else if v - goal < step { goal } else { v - step }
}

/// Rotate a unit vector by a small angle `a` (radians; |a| well under 1):
/// sin and cos from their Taylor series, then renormalised.
fn rotate(h: (Fixed, Fixed), a: Fixed) -> (Fixed, Fixed) {
    let a2 = a * a;
    let s = a - a * a2 / Fixed::from_int(6);
    let c = Fixed::ONE - a2 / Fixed::from_int(2) + a2 * a2 / Fixed::from_int(24);
    let x = h.0 * c - h.1 * s;
    let z = h.0 * s + h.1 * c;
    let len = (x * x + z * z).sqrt();
    if len == Fixed::ZERO { h } else { (x / len, z / len) }
}

/// Distance at which a unit counts as arrived.
fn arrive_distance() -> Fixed {
    Fixed::from_int(4)
}

impl Unit {
    fn step(&mut self) {
        let dt = dt();
        let st = self.stats;
        let accel = st.forward_accel * st.max_speed;
        let Some(target) = self.target else {
            self.speed = approach(self.speed, Fixed::ZERO, accel * dt);
            self.omega = approach(self.omega, Fixed::ZERO, st.turn_alpha * dt);
            self.advance(dt);
            return;
        };
        let (dx, dz) = (target.x - self.pos.x, target.z - self.pos.z);
        let dist = (dx * dx + dz * dz).sqrt();
        if dist <= arrive_distance().max(self.speed * dt) {
            self.pos = target;
            self.target = None;
            self.speed = Fixed::ZERO;
            self.omega = Fixed::ZERO;
            return;
        }
        let (tx, tz) = (dx / dist, dz / dist);
        // cross > 0: the target lies toward +omega; dot: how aligned we are.
        let cross = self.heading.0 * tz - self.heading.1 * tx;
        let dot = self.heading.0 * tx + self.heading.1 * tz;

        let frac = if st.max_speed > Fixed::ZERO { clamp(self.speed / st.max_speed, Fixed::ZERO, Fixed::ONE) } else { Fixed::ZERO };
        let omega_max = st.turn_omega * (st.turn_at_rest + (Fixed::ONE - st.turn_at_rest) * frac);
        let sign = if cross < Fixed::ZERO { -Fixed::ONE } else { Fixed::ONE };
        // Ahead: turn in proportion to the remaining sine; behind: turn flat out.
        let want_omega = if dot > Fixed::ZERO {
            let gain = Fixed::from_int(4);
            sign * (cross.abs() * gain).min(omega_max)
        } else {
            sign * omega_max
        };
        self.omega = approach(self.omega, want_omega, st.turn_alpha * dt);

        // Speed: slower when facing away, and able to stop at the target.
        let align = clamp(dot, Fixed::from_f64(0.1), Fixed::ONE);
        let brake = (Fixed::from_int(2) * accel * dist).sqrt();
        let want_speed = (st.max_speed * align).min(brake);
        self.speed = approach(self.speed, want_speed, accel * dt);

        // Height: straight toward the target's, no faster than the cruise speed.
        let dy = target.y - self.pos.y;
        self.pos.y = self.pos.y + clamp(dy, -st.max_speed * dt, st.max_speed * dt);
        self.advance(dt);
    }

    fn advance(&mut self, dt: Fixed) {
        if self.omega != Fixed::ZERO {
            self.heading = rotate(self.heading, self.omega * dt);
        }
        self.pos.x = self.pos.x + self.heading.0 * self.speed * dt;
        self.pos.z = self.pos.z + self.heading.1 * self.speed * dt;
    }
}

impl Sim {
    pub fn new(seed: u64) -> Self {
        Sim {
            tick: 0,
            rng: Rng::new(seed),
            units: Vec::new(),
            next_id: 1,
        }
    }

    /// Add a unit facing `heading` (x, z; normalised here).
    pub fn spawn(&mut self, player: u8, pos: Vec3, heading: (Fixed, Fixed), stats: MoveStats) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        let len = (heading.0 * heading.0 + heading.1 * heading.1).sqrt();
        let heading = if len == Fixed::ZERO { (Fixed::ZERO, Fixed::ONE) } else { (heading.0 / len, heading.1 / len) };
        self.units.push(Unit {
            id,
            player,
            pos,
            heading,
            speed: Fixed::ZERO,
            omega: Fixed::ZERO,
            target: None,
            stats,
        });
        id
    }

    pub fn unit(&self, id: EntityId) -> Option<&Unit> {
        self.units.binary_search_by_key(&id, |u| u.id).ok().map(|i| &self.units[i])
    }

    fn unit_mut(&mut self, id: EntityId) -> Option<&mut Unit> {
        self.units.binary_search_by_key(&id, |u| u.id).ok().map(|i| &mut self.units[i])
    }

    /// Advance one tick, applying this tick's commands in the given order.
    pub fn step(&mut self, commands: &[Command]) {
        for c in commands {
            match *c {
                Command::Move { unit, to } => {
                    if let Some(u) = self.unit_mut(unit) {
                        u.target = Some(to);
                    }
                }
                Command::Stop { unit } => {
                    if let Some(u) = self.unit_mut(unit) {
                        u.target = None;
                    }
                }
            }
        }
        for u in &mut self.units {
            u.step();
        }
        self.tick += 1;
    }

    /// FNV-1a over all simulation state, for desync detection.
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        };
        eat(self.tick);
        eat(self.rng.0);
        for u in &self.units {
            eat(u.id.0 as u64);
            eat(u.player as u64);
            for f in [u.pos.x, u.pos.y, u.pos.z, u.heading.0, u.heading.1, u.speed, u.omega] {
                eat(f.to_bits() as u64);
            }
            match u.target {
                Some(t) => {
                    eat(1);
                    for f in [t.x, t.y, t.z] {
                        eat(f.to_bits() as u64);
                    }
                }
                None => eat(0),
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::from_int(z),
        }
    }

    fn stats() -> MoveStats {
        MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2)
    }

    fn run() -> u64 {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        let b = sim.spawn(1, v(1000, 0, 0), (-Fixed::ONE, Fixed::ZERO), stats());
        for t in 0..600 {
            let cmds = match t {
                0 => vec![Command::Move { unit: a, to: v(500, 30, -300) }],
                10 => vec![Command::Move { unit: b, to: v(0, 0, 900) }],
                200 => vec![Command::Stop { unit: a }],
                _ => vec![],
            };
            sim.step(&cmds);
            sim.rng.next_u64();
        }
        sim.digest()
    }

    #[test]
    fn replay_is_deterministic() {
        assert_eq!(run(), run());
    }

    /// The exact digest pins the arithmetic: any platform or compiler change
    /// that alters a single bit of the simulation fails here (CI runs it on
    /// x86-64, ARM64 Linux, Windows and Apple Silicon).
    #[test]
    fn digest_is_pinned() {
        assert_eq!(run(), 0xb78f_2311_5fb6_844e);
    }

    #[test]
    fn unit_turns_around_and_arrives() {
        let mut sim = Sim::new(1);
        // Facing +Z, target behind at -Z and off to the side.
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        sim.step(&[Command::Move { unit: a, to: v(300, 50, -800) }]);
        let mut arrived = None;
        for t in 0..30 * 60 {
            sim.step(&[]);
            if sim.unit(a).unwrap().target.is_none() {
                arrived = Some(t);
                break;
            }
        }
        let u = sim.unit(a).unwrap();
        assert!(arrived.is_some(), "never arrived: {:?}", u.pos.to_f32());
        assert_eq!(u.pos, v(300, 50, -800));
        assert_eq!(u.speed, Fixed::ZERO);
        // Heading stays unit length.
        let l = (u.heading.0 * u.heading.0 + u.heading.1 * u.heading.1).to_f32();
        assert!((l - 1.0).abs() < 1e-6, "{l}");
    }

    #[test]
    fn small_rotation_matches_trig() {
        let h = rotate((Fixed::ONE, Fixed::ZERO), Fixed::from_f64(0.05));
        assert!((h.0.to_f32() - 0.05f32.cos()).abs() < 1e-6);
        assert!((h.1.to_f32() - 0.05f32.sin()).abs() < 1e-6);
    }
}
