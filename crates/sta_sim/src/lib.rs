//! Deterministic lockstep simulation core.
//!
//! Rules for everything in this crate:
//! - No floats in simulation state. Positions, velocities and resources use
//!   [`Fixed`] so x86-64, ARM64 and Apple Silicon produce identical results.
//! - No hash-map iteration order, wall-clock time, or thread scheduling may
//!   influence state. Entity iteration is by ascending id.
//! - All randomness comes from the seeded [`Rng`].
//! - Each tick ends with a [`Sim::digest`] that peers compare to detect desyncs.

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

#[derive(Debug, Clone)]
pub struct Unit {
    pub id: EntityId,
    pub player: u8,
    pub pos: Vec3,
    pub vel: Vec3,
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

impl Sim {
    pub fn new(seed: u64) -> Self {
        Sim {
            tick: 0,
            rng: Rng::new(seed),
            units: Vec::new(),
            next_id: 1,
        }
    }

    pub fn spawn(&mut self, player: u8, pos: Vec3) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        self.units.push(Unit {
            id,
            player,
            pos,
            vel: Vec3::default(),
        });
        id
    }

    fn unit_mut(&mut self, id: EntityId) -> Option<&mut Unit> {
        self.units
            .binary_search_by_key(&id, |u| u.id)
            .ok()
            .map(|i| &mut self.units[i])
    }

    /// Advance one tick, applying this tick's commands in the given order.
    pub fn step(&mut self, commands: &[Command]) {
        // Placeholder kinematics: fly straight at a fixed speed. Real movement
        // (ODF accel/turn rates, formations, pathing) replaces this.
        let speed = Fixed::from_int(2);
        for c in commands {
            match *c {
                Command::Move { unit, to } => {
                    if let Some(u) = self.unit_mut(unit) {
                        let d = Vec3 {
                            x: to.x - u.pos.x,
                            y: to.y - u.pos.y,
                            z: to.z - u.pos.z,
                        };
                        let len = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
                        u.vel = if len == Fixed::ZERO {
                            Vec3::default()
                        } else {
                            Vec3 {
                                x: d.x * speed / len,
                                y: d.y * speed / len,
                                z: d.z * speed / len,
                            }
                        };
                    }
                }
                Command::Stop { unit } => {
                    if let Some(u) = self.unit_mut(unit) {
                        u.vel = Vec3::default();
                    }
                }
            }
        }
        for u in &mut self.units {
            u.pos.x = u.pos.x + u.vel.x;
            u.pos.y = u.pos.y + u.vel.y;
            u.pos.z = u.pos.z + u.vel.z;
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
            for f in [u.pos.x, u.pos.y, u.pos.z, u.vel.x, u.vel.y, u.vel.z] {
                eat(f.to_bits() as u64);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> u64 {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, Vec3::default());
        let b = sim.spawn(1, Vec3 { x: Fixed::from_int(100), ..Default::default() });
        for t in 0..100 {
            let cmds = match t {
                0 => vec![Command::Move { unit: a, to: Vec3 { x: Fixed::from_int(50), y: Fixed::from_int(30), z: Fixed::ZERO } }],
                10 => vec![Command::Move { unit: b, to: Vec3::default() }],
                40 => vec![Command::Stop { unit: a }],
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
}
