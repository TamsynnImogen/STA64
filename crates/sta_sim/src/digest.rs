//! Section digests (SIM-31, NET_DECISIONS N6): one digest per section of the
//! sim's state, beside [`Sim::digest`], so peers that disagree can say where
//! the states first differ instead of exchanging a state dump.
//!
//! The sections, and what each holds:
//!
//! | section | state |
//! |---|---|
//! | `roster` | tick, content hash, seats and teams, who exists (id, kind, definition, owner), labels, perceived owners, eliminations, pending start-effect rows |
//! | `movement` | position, heading, speed, turn rate, target and movement stats of every entity; queued waypoints and formations; map confinement |
//! | `combat` | hull, shields, status flags, unit crew; weapon mounts, shots and attack orders; point defence; special energy and cloaks |
//! | `economy` | stock, nodes, freighters, personnel; yard queues, rig deploys and rallies; build progress |
//! | `rng` | the sim generator |
//!
//! Every part of the state [`Sim::digest`] covers is in exactly one section,
//! so the full digest differs only if some section does. `Sim::digest` itself,
//! its value and its pinned test are untouched. A system added to the digest
//! picks its section here (the exhaustive `Entity` destructuring below breaks
//! the build when an entity field is added without one).
//!
//! A section digest is FNV-1a over its fields in a fixed order, each section
//! starting from its own name, so equal values in two sections do not collide.
//! Platform-independent like the full digest (fixed point, integers only).

use crate::world::{Entity, Fnv};
use crate::Sim;

/// One digest per section of a [`Sim`]'s state (module docs: the list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SectionDigests {
    pub roster: u64,
    pub movement: u64,
    pub combat: u64,
    pub economy: u64,
    pub rng: u64,
}

impl SectionDigests {
    /// The section names, in the order of [`SectionDigests::values`].
    pub const NAMES: [&'static str; 5] = ["roster", "movement", "combat", "economy", "rng"];

    /// The digests in [`SectionDigests::NAMES`] order.
    pub fn values(&self) -> [u64; 5] {
        [self.roster, self.movement, self.combat, self.economy, self.rng]
    }

    /// `(name, digest)` pairs.
    pub fn named(&self) -> [(&'static str, u64); 5] {
        let v = self.values();
        std::array::from_fn(|i| (Self::NAMES[i], v[i]))
    }

    /// The names of the sections whose digests differ from `other`'s, in order.
    pub fn differing(&self, other: &SectionDigests) -> Vec<&'static str> {
        self.named().into_iter().zip(other.values()).filter(|((_, a), b)| a != b).map(|((n, _), _)| n).collect()
    }
}

/// Fails to compile when `Entity` gains a field: give it a section below.
fn _every_entity_field_has_a_section(e: &Entity) {
    let Entity { id: _, kind: _, def: _, owner: _, pos: _, heading: _, speed: _, omega: _, target: _, stats: _, hull: _, shields: _, crew: _, flags: _, progress: _ } = e;
}

/// Fails to compile when `Sim` gains a field: decide its section (or that it is
/// not in the digest) here and in `Sim::digest`.
fn _every_sim_field_is_decided(s: &Sim) {
    let Sim {
        tick: _, rng: _, units: _, labels: _, players: _, ai_seats: _, content: _, starts: _, regions: _,
        bounds: _, confinement: _, perception: _, events: _, weapons: _, movement: _, economy: _,
        production: _, victory: _, specials: _, start_effects: _, nav: _, team_changes: _,
    } = s;
}

fn start(name: &str) -> Fnv {
    let mut h = Fnv::new();
    h.str(name);
    h
}

impl Sim {
    /// A digest per section of the state; see the [module](crate::digest).
    pub fn section_digests(&self) -> SectionDigests {
        SectionDigests {
            roster: self.roster_digest(),
            movement: self.movement_digest(),
            combat: self.combat_digest(),
            economy: self.economy_digest(),
            rng: {
                let mut h = start("rng");
                h.u64(self.rng.0);
                h.0
            },
        }
    }

    fn roster_digest(&self) -> u64 {
        let mut h = start("roster");
        h.u64(self.tick);
        h.u64(self.content.as_ref().map_or(0, |c| c.hash()));
        h.u64(self.players.len() as u64);
        for &(p, t) in &self.players {
            h.u64(p.0 as u64);
            h.u64(t.0 as u64);
        }
        h.u64(self.units.next_id().0 as u64);
        h.u64(self.units.len() as u64);
        for e in self.units.iter() {
            h.u64(e.id.0 as u64);
            h.u64(e.kind as u64);
            h.u64(e.def.0 as u64);
            h.u64(e.owner.0 as u64);
        }
        h.u64(self.labels.len() as u64);
        for (id, label) in self.labels.iter() {
            h.u64(id.0 as u64);
            h.str(label);
        }
        h.u64(self.perception.len() as u64);
        for (id, owner) in self.perception.iter() {
            h.u64(id.0 as u64);
            h.u64(owner.0 as u64);
        }
        if !self.victory.is_empty() {
            self.victory.hash_into(&mut h);
        }
        // SIM-28: pending expiry and replacement of start helpers decide who
        // will exist, so they belong with the roster.
        if !self.start_effects.is_empty() {
            self.start_effects.hash_into(&mut h);
        }
        h.0
    }

    fn movement_digest(&self) -> u64 {
        let mut h = start("movement");
        for e in self.units.iter() {
            h.u64(e.id.0 as u64);
            for f in [e.pos.x, e.pos.y, e.pos.z, e.heading.0, e.heading.1, e.speed, e.omega] {
                h.fixed(f);
            }
            match e.target {
                Some(t) => {
                    h.u64(1);
                    for f in [t.x, t.y, t.z] {
                        h.fixed(f);
                    }
                }
                None => h.u64(0),
            }
            let s = &e.stats;
            for f in [s.max_speed, s.forward_accel, s.turn_omega, s.turn_alpha, s.turn_at_rest] {
                h.fixed(f);
            }
            if s.has_smooth_keys() {
                for f in s.smooth_fields() {
                    h.fixed(f);
                }
            }
        }
        self.movement.hash_into(&mut h);
        self.confinement.hash_into(&mut h);
        h.0
    }

    fn combat_digest(&self) -> u64 {
        let mut h = start("combat");
        for e in self.units.iter() {
            h.u64(e.id.0 as u64);
            for f in e.hull.into_iter().chain(e.shields.now).chain([e.shields.max]) {
                h.fixed(f);
            }
            h.u64(e.shields.directional as u64);
            h.u64(e.flags.0 as u64);
            match e.crew {
                Some([now, max]) => {
                    h.u64(1);
                    h.fixed(now);
                    h.fixed(max);
                }
                None => h.u64(0),
            }
        }
        if !self.weapons.is_empty() {
            self.weapons.hash_into(&mut h);
        }
        if !self.specials.is_empty() {
            self.specials.hash_into(&mut h);
        }
        h.0
    }

    fn economy_digest(&self) -> u64 {
        let mut h = start("economy");
        for e in self.units.iter() {
            match e.progress {
                Some(p) => {
                    h.u64(1);
                    h.fixed(p);
                }
                None => h.u64(0),
            }
        }
        if !self.economy.is_empty() {
            self.economy.hash_into(&mut h);
        }
        if !self.production.is_empty() {
            self.production.hash_into(&mut h);
        }
        h.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandKind, Order};
    use crate::{Fixed, MoveStats, PlayerId, Vec3, WorldSource};

    fn v(x: i32, z: i32) -> Vec3 {
        Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
    }

    fn sim() -> Sim {
        let mut sim = Sim::new(1);
        let stats = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        let a = sim.spawn(0, v(0, 0), (Fixed::ZERO, Fixed::ONE), stats);
        sim.spawn(1, v(1000, 0), (-Fixed::ONE, Fixed::ZERO), stats);
        sim.step(&[Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![a], to: v(500, -300), queue: false }) }]);
        for _ in 0..40 {
            sim.step(&[]);
            sim.rng.next_u64();
        }
        sim
    }

    /// The sections that differ after `change`, and whether the full digest did.
    fn effect(change: impl FnOnce(&mut Sim)) -> (Vec<&'static str>, bool) {
        let before = sim();
        let mut after = before.clone();
        change(&mut after);
        (before.section_digests().differing(&after.section_digests()), before.digest() != after.digest())
    }

    #[test]
    fn deterministic_and_pinned() {
        assert_eq!(sim().section_digests(), sim().section_digests());
        // Equal on every platform (fixed point, integer hashing).
        assert_eq!(sim().section_digests().values(), PINNED, "{:#x?}", sim().section_digests().values());
    }

    /// Measured once on this scenario; a change here is a change of a section's
    /// meaning and goes with a note in the PR, like the pinned full digest.
    const PINNED: [u64; 5] = [0x152b_54c2_e0ee_8e52, 0xab80_c411_9972_840f, 0x7dce_5ffd_36e3_d6b0, 0xb514_87e8_d035_289e, 0xcc8f_d429_d225_a655];

    #[test]
    fn a_moved_unit_changes_the_movement_section_and_no_other() {
        let (sections, full) = effect(|s| s.units.iter_mut().next().unwrap().pos.x = Fixed::from_int(77));
        assert_eq!(sections, ["movement"]);
        assert!(full);
    }

    #[test]
    fn each_kind_of_state_changes_its_own_section_only() {
        type Change = Box<dyn FnOnce(&mut Sim)>;
        let cases: Vec<(&str, Change, &str)> = vec![
            ("hull", Box::new(|s| s.units.iter_mut().next().unwrap().hull[0] = Fixed::from_int(5)), "combat"),
            ("shields", Box::new(|s| s.units.iter_mut().next().unwrap().shields.max = Fixed::from_int(5)), "combat"),
            ("flags", Box::new(|s| s.units.iter_mut().next().unwrap().flags = crate::view::UnitFlags::default()), "combat"),
            ("crew", Box::new(|s| s.units.iter_mut().next().unwrap().crew = Some([Fixed::ONE; 2])), "combat"),
            ("owner", Box::new(|s| s.units.iter_mut().next().unwrap().owner = PlayerId(1)), "roster"),
            ("label", Box::new(|s| { let id = s.units.iter().next().unwrap().id; s.labels.insert(id, "x".into()); }), "roster"),
            ("progress", Box::new(|s| s.units.iter_mut().next().unwrap().progress = Some(Fixed::from_int(1))), "economy"),
            ("stock", Box::new(|s| s.economy.set_stock(PlayerId(0), [Fixed::from_int(9); 4])), "economy"),
            ("rng", Box::new(|s| { s.rng.next_u64(); }), "rng"),
            ("tick", Box::new(|s| s.tick += 1), "roster"),
        ];
        for (name, change, section) in cases {
            let (sections, full) = effect(change);
            assert_eq!(sections, [section], "{name}");
            assert!(full, "{name}: the full digest must differ too");
        }
    }

    #[test]
    fn section_names_and_values_line_up() {
        let d = sim().section_digests();
        assert_eq!(d.named().map(|(n, _)| n), SectionDigests::NAMES);
        assert_eq!(d.named().map(|(_, x)| x), d.values());
        assert!(d.differing(&d).is_empty());
    }
}
