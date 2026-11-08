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
//!
//! The seams (docs/WORKPLAN.md) live in [`command`] (S2), [`view`] and
//! [`source`] (S3), [`event`] (S4) and [`setup`] (S7); [`fake`] has the
//! stand-ins other lanes build against. [`Sim`] is a [`WorldSource`] over
//! the entity model in [`world`], filled from the map by [`spawn`], with
//! today's movement.

pub mod command;
pub mod event;
pub mod fake;
pub mod movement;
pub mod setup;
pub mod source;
pub mod spawn;
pub mod view;
pub mod world;

/// The fixed-point type and its module, now in `sta_core`; re-exported so
/// `sta_sim::Fixed` and `sta_sim::fixed::Fixed` keep working.
pub use sta_core::fixed;
pub use sta_core::{ContentKey, EntityId, Fixed, PlayerId, SEAM_REV, TICK_HZ, TeamId, Tick, Vec3};
/// Movement limits now live with the rest of a unit's definition.
pub use sta_content::MoveStats;

pub use event::SimEvent;
pub use source::WorldSource;
pub use view::{TickOutput, UnitView, WorldView};

use std::sync::Arc;

use sta_content::{ContentDb, Region, StartPos};
use view::{PlayerView, Relation};
use world::{Entities, Entity, EntityKind, Fnv, Table};

/// The entity, under the name movement and `sta64::units` know it by.
pub type Unit = Entity;

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

/// The simulation state.
#[derive(Debug, Clone)]
pub struct Sim {
    pub tick: u64,
    pub rng: Rng,
    /// Every entity, ascending id (the name is the legacy one).
    pub units: Entities,
    /// Map labels of the entities that have one (missions find them by it).
    pub labels: Table<String>,
    /// Seats from the game setup, ascending id; in the digest.
    players: Vec<(PlayerId, TeamId)>,
    /// What spawns resolve against; `None` for a sim made with [`Sim::new`].
    content: Option<Arc<ContentDb>>,
    /// The map's start positions and regions, kept for the systems that use
    /// them. Fixed by the setup, so not in the digest.
    starts: Vec<StartPos>,
    regions: Vec<Region>,
    /// Events since the last step (spawns between ticks), emitted with it.
    events: Vec<SimEvent>,
    /// Queued waypoints and formations (SIM-02).
    movement: movement::Movement,
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
        // Remaining turn angle from the sine and the side: asin(c) ~ c + c^3/6
        // ahead, pi minus that behind.
        let c = cross.abs();
        let ahead = c + c * c * c / Fixed::from_int(6);
        let angle = if dot >= Fixed::ZERO { ahead } else { Fixed::from_f64(std::f64::consts::PI) - ahead };
        // The fastest turn that can still stop on the heading with the turn
        // acceleration available: omega = sqrt(2 alpha angle). Asking for more
        // overshoots and weaves; this settles without oscillating.
        // Near the heading, also no more than closes the angle this tick, so the
        // discrete steps do not overshoot and chatter across the course.
        let want_omega = sign * (Fixed::from_int(2) * st.turn_alpha * angle).sqrt().min(omega_max).min(angle / dt);
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
            units: Entities::new(),
            labels: Table::new(),
            players: Vec::new(),
            content: None,
            starts: Vec::new(),
            regions: Vec::new(),
            events: Vec::new(),
            movement: movement::Movement::default(),
        }
    }

    /// Add a craft with no definition facing `heading` (x, z; normalised
    /// here). Content-made entities come from [`Sim::spawn_def`].
    pub fn spawn(&mut self, player: u8, pos: Vec3, heading: (Fixed, Fixed), stats: MoveStats) -> EntityId {
        let id = self.units.spawn(Entity::new(EntityKind::Craft, ContentKey::NONE, PlayerId(player), pos, heading, stats));
        self.events.push(SimEvent::Spawned { id });
        id
    }

    pub fn unit(&self, id: EntityId) -> Option<&Unit> {
        self.units.get(id)
    }

    fn unit_mut(&mut self, id: EntityId) -> Option<&mut Unit> {
        self.units.get_mut(id)
    }

    fn set_target(&mut self, unit: EntityId, to: Option<Vec3>) {
        if let Some(u) = self.unit_mut(unit) {
            u.target = to;
        }
    }

    /// Apply one seam command, or say it was rejected. Today moves, stops
    /// and the spawn and kill edits exist; everything else is rejected until
    /// its package lands.
    fn apply(&mut self, c: &command::Command) -> bool {
        use command::{CommandKind, Order};
        let order = match &c.kind {
            CommandKind::Order(o) => o,
            CommandKind::Edit(e) => return self.apply_edit(c.player, e),
        };
        match order {
            Order::Move { .. } | Order::Stop { .. } | Order::Formation { .. } => self.apply_movement(c.player, order),
            _ => false,
        }
    }

    /// Move every unit one tick and close the tick with its view.
    fn advance(&mut self) -> TickOutput {
        for u in self.units.iter_mut().filter(|u| u.mobile()) {
            u.step();
        }
        self.advance_courses();
        self.tick += 1;
        TickOutput { view: Arc::new(self.build_view()), events: std::mem::take(&mut self.events) }
    }

    fn build_view(&self) -> WorldView {
        let units = self.units.iter().map(Entity::view).collect();
        let players = self
            .players
            .iter()
            .map(|&(id, team)| PlayerView {
                id,
                team,
                resources: Vec::new(),
                officers: [Fixed::ZERO; 2],
                eliminated: false,
                relations: self
                    .players
                    .iter()
                    .filter(|o| o.0 != id)
                    .map(|&(o, t)| (o, if t == team { Relation::Ally } else { Relation::Enemy }))
                    .collect(),
            })
            .collect();
        WorldView { tick: self.tick, digest: self.digest(), units, players }
    }

    /// FNV-1a over all simulation state, for desync detection. The content
    /// hash goes in first, so tick 0's digest already tells peers with
    /// different content apart (S5), and every later one carries it.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick);
        h.u64(self.rng.0);
        h.u64(self.content.as_ref().map_or(0, |c| c.hash()));
        h.u64(self.players.len() as u64);
        for &(p, t) in &self.players {
            h.u64(p.0 as u64);
            h.u64(t.0 as u64);
        }
        self.units.hash_into(&mut h);
        self.movement.hash_into(&mut h);
        h.u64(self.labels.len() as u64);
        for (id, label) in self.labels.iter() {
            h.u64(id.0 as u64);
            h.str(label);
        }
        h.0
    }
}

impl WorldSource for Sim {
    /// Applies `commands` sorted by `(player, seq)` (sorted again here, so a
    /// caller's order cannot matter), then advances. Rejected commands come
    /// out as [`SimEvent::Rejected`], in batch order.
    fn step(&mut self, commands: &[command::Command]) -> TickOutput {
        let mut batch: Vec<&command::Command> = commands.iter().collect();
        batch.sort_by_key(|c| c.sort_key());
        for c in batch {
            if !self.apply(c) {
                self.events.push(SimEvent::Rejected { player: c.player, seq: c.seq });
            }
        }
        self.advance()
    }

    fn view(&self) -> Arc<WorldView> {
        Arc::new(self.build_view())
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

    /// A single order from `player` to one of its units.
    fn go(player: u8, id: EntityId, to: Vec3) -> command::Command {
        order(player, 0, command::Order::Move { units: vec![id], to, queue: false })
    }

    fn halt(player: u8, id: EntityId) -> command::Command {
        order(player, 0, command::Order::Stop { units: vec![id] })
    }

    fn run() -> u64 {
        run_sim().digest()
    }

    fn run_sim() -> Sim {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        let b = sim.spawn(1, v(1000, 0, 0), (-Fixed::ONE, Fixed::ZERO), stats());
        for t in 0..600 {
            let cmds = match t {
                0 => vec![go(0, a, v(500, 30, -300))],
                10 => vec![go(1, b, v(0, 0, 900))],
                200 => vec![halt(0, a)],
                _ => vec![],
            };
            sim.step(&cmds);
            sim.rng.next_u64();
        }
        sim
    }

    /// The digest before the entity model: the fields movement had, hashed
    /// the old way.
    fn movement_digest(sim: &Sim) -> u64 {
        let mut h = Fnv::new();
        h.u64(sim.tick);
        h.u64(sim.rng.0);
        for u in &sim.units {
            h.u64(u.id.0 as u64);
            h.u64(u.owner.0 as u64);
            for f in [u.pos.x, u.pos.y, u.pos.z, u.heading.0, u.heading.1, u.speed, u.omega] {
                h.fixed(f);
            }
            match u.target {
                Some(t) => {
                    h.u64(1);
                    for f in [t.x, t.y, t.z] {
                        h.fixed(f);
                    }
                }
                None => h.u64(0),
            }
        }
        h.0
    }

    #[test]
    fn replay_is_deterministic() {
        assert_eq!(run(), run());
    }

    /// The exact digest pins the arithmetic: any platform or compiler change
    /// that alters a single bit of the simulation fails here (CI runs it on
    /// x86-64, ARM64 Linux, Windows and Apple Silicon).
    ///
    /// The entity model (SIM-01) added kind, definition, stats, hull,
    /// shields, flags, players and the next id to the digest, so its value
    /// moved from `0xa4c0_7121_6986_d7dc`; the motion did not, which the old
    /// formula over the same run still shows.
    #[test]
    fn digest_is_pinned() {
        assert_eq!(run(), 0x2e54_3b95_7f83_da45);
        assert_eq!(movement_digest(&run_sim()), 0xa4c0_7121_6986_d7dc);
    }

    #[test]
    fn unit_turns_around_and_arrives() {
        let mut sim = Sim::new(1);
        // Facing +Z, target behind at -Z and off to the side.
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        sim.step(&[go(0, a, v(300, 50, -800))]);
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

    /// Once a unit has turned onto its course it must not weave across it
    /// (the earlier proportional controller crossed it 806 times).
    #[test]
    fn no_weaving_on_course() {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ONE, Fixed::ZERO), stats());
        sim.step(&[go(0, a, v(0, 0, 4000))]);
        let mut crossings = 0;
        let mut worst = 0.0f32;
        let mut last: Option<bool> = None;
        for _ in 0..30 * 40 {
            sim.step(&[]);
            let u = sim.unit(a).unwrap();
            let Some(t) = u.target else { break };
            let (dx, dz) = (t.x - u.pos.x, t.z - u.pos.z);
            let d = (dx * dx + dz * dz).sqrt();
            // Course error as the sine of the angle; ignore below a milliradian.
            let side = (u.heading.0 * dz - u.heading.1 * dx) / d;
            if side.abs() < Fixed::from_f64(1e-3) {
                continue;
            }
            let s = side > Fixed::ZERO;
            if last.is_some_and(|l| l != s) {
                crossings += 1;
                worst = worst.max(side.abs().to_f32());
            }
            last = Some(s);
        }
        // Settling after the turn may cross the (moving) course line once or
        // twice by a fraction of a degree; weaving means many or large swings.
        assert!(crossings <= 2, "weaved across the course {crossings} times");
        assert!(worst < 0.01, "overshoot of {worst} rad");
        assert!(sim.unit(a).unwrap().target.is_none(), "did not arrive");
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // platform trig as the reference only
    fn small_rotation_matches_trig() {
        let h = rotate((Fixed::ONE, Fixed::ZERO), Fixed::from_f64(0.05));
        assert!((h.0.to_f32() - 0.05f32.cos()).abs() < 1e-6);
        assert!((h.1.to_f32() - 0.05f32.sin()).abs() < 1e-6);
    }

    fn order(player: u8, seq: u32, o: command::Order) -> command::Command {
        command::Command { player: PlayerId(player), seq, kind: command::CommandKind::Order(o) }
    }

    /// The pinned run again, through the seam: same digest as setting targets directly.
    #[test]
    fn world_source_matches_direct_targets() {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        let b = sim.spawn(1, v(1000, 0, 0), (-Fixed::ONE, Fixed::ZERO), stats());
        let first = WorldSource::step(&mut sim, &[]);
        assert_eq!(first.events, [SimEvent::Spawned { id: a }, SimEvent::Spawned { id: b }]);
        for t in 1..600 {
            let cmds = match t {
                1 => vec![order(0, 0, command::Order::Move { units: vec![a], to: v(500, 30, -300), queue: false })],
                10 => vec![order(1, 0, command::Order::Move { units: vec![b], to: v(0, 0, 900), queue: false })],
                200 => vec![order(0, 1, command::Order::Stop { units: vec![a] })],
                _ => vec![],
            };
            let out = WorldSource::step(&mut sim, &cmds);
            assert_eq!(out.view.tick, sim.tick);
            assert_eq!(out.view.digest, sim.digest());
            sim.rng.next_u64();
        }
        // The same run with targets set directly, no commands.
        let mut old = Sim::new(1);
        old.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        old.spawn(1, v(1000, 0, 0), (-Fixed::ONE, Fixed::ZERO), stats());
        old.advance();
        for t in 1..600 {
            match t {
                1 => old.set_target(a, Some(v(500, 30, -300))),
                10 => old.set_target(b, Some(v(0, 0, 900))),
                200 => old.set_target(a, None),
                _ => {}
            }
            old.advance();
            old.rng.next_u64();
        }
        assert_eq!(sim.digest(), old.digest());
        assert_eq!(sim.view().units.len(), 2);
    }

    #[test]
    fn foreign_and_unknown_commands_are_rejected() {
        let mut sim = Sim::new(1);
        let a = sim.spawn(0, v(0, 0, 0), (Fixed::ZERO, Fixed::ONE), stats());
        sim.step(&[]);
        let cmds = [
            // Player 1 may not order player 0's unit.
            order(1, 0, command::Order::Move { units: vec![a], to: v(100, 0, 0), queue: false }),
            // Not implemented yet.
            order(0, 0, command::Order::Cloak { units: vec![a], on: true }),
            // The bench may order anything.
            order(255, 0, command::Order::Move { units: vec![a], to: v(100, 0, 0), queue: false }),
        ];
        let out = WorldSource::step(&mut sim, &cmds);
        let rejected = [SimEvent::Rejected { player: PlayerId(0), seq: 0 }, SimEvent::Rejected { player: PlayerId(1), seq: 0 }];
        assert_eq!(out.events, rejected);
        assert_eq!(sim.unit(a).unwrap().target, Some(v(100, 0, 0)));
    }
}
