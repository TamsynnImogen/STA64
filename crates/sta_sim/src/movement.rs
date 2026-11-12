//! Orders that move units (SIM-02): queued waypoints, formations and stop,
//! on top of [`kinematics`](crate::kinematics) (SIM-15), which flies a unit
//! to one `target` under the profile's movement model.
//!
//! - A unit's `target` is its current leg. Further legs wait in its
//!   [`Course`]; when a leg is reached the next one becomes the target.
//!   Under the native model a unit stops at each waypoint, then sets off
//!   again; under the smooth model (Armada II, FO) it moves on to the next
//!   leg before reaching a waypoint and flies through.
//! - `Move { queue: false }` replaces the course; `queue: true` appends (a
//!   unit with nothing to do starts at once). `Stop` clears both.
//! - `Formation` sets a unit's formation kind; the next `Move` of a group
//!   sends each unit to its slot around the destination, using the kind of
//!   the lowest-id unit of the order. Slots are taken in ascending id and
//!   laid out about the destination, facing along the way from the group's
//!   centroid. Formation kinds: 0 none (all aim for the point), 1 line abreast,
//!   2 column, 3 wedge. Anything else is rejected.
//! - Courses exist only for units that have waypoints or a formation, are
//!   dropped when the unit is gone, and are in the digest.

use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use crate::command::Order;
use crate::world::{Fnv, Table};
use crate::Sim;

/// Formation kinds of `Order::Formation`.
pub const FORM_NONE: u8 = 0;
pub const FORM_LINE: u8 = 1;
pub const FORM_COLUMN: u8 = 2;
pub const FORM_WEDGE: u8 = 3;

/// Gap between neighbouring slots.
fn spacing() -> Fixed {
    Fixed::from_int(60)
}

/// What a unit does after its current leg.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Course {
    /// Legs after the current target, in order.
    pub waypoints: Vec<Vec3>,
    pub formation: u8,
}

impl Course {
    fn is_default(&self) -> bool {
        self.waypoints.is_empty() && self.formation == FORM_NONE
    }
}

/// Every unit's course, ascending id.
#[derive(Debug, Clone, Default)]
pub struct Movement {
    courses: Table<Course>,
}

impl Movement {
    /// Detach owner-bound courses and formation on an ownership request.
    pub(crate) fn clear(&mut self, id: EntityId) {
        self.courses.remove(id);
    }

    /// Courses ascending by id, for snapshots.
    pub(crate) fn rows(&self) -> Vec<(EntityId, Course)> {
        self.courses.iter().map(|(id, c)| (id, c.clone())).collect()
    }

    pub(crate) fn from_rows(rows: Vec<(EntityId, Course)>) -> Option<Movement> {
        Some(Movement { courses: Table::from_rows(rows)? })
    }

    pub fn course(&self, id: EntityId) -> Option<&Course> {
        self.courses.get(id)
    }

    fn edit(&mut self, id: EntityId, f: impl FnOnce(&mut Course)) {
        let mut c = self.courses.remove(id).unwrap_or_default();
        f(&mut c);
        if !c.is_default() {
            self.courses.insert(id, c);
        }
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        // Nothing for a sim with no courses, so it hashes as before SIM-02.
        if self.courses.is_empty() {
            return;
        }
        h.u64(self.courses.len() as u64);
        for (id, c) in self.courses.iter() {
            h.u64(id.0 as u64);
            h.u64(c.formation as u64);
            h.u64(c.waypoints.len() as u64);
            for w in &c.waypoints {
                for f in [w.x, w.y, w.z] {
                    h.fixed(f);
                }
            }
        }
    }
}

/// Slot offsets `(forward, lateral)` for `n` units of formation `kind`,
/// centred on the destination. `lateral` is to the unit's right (+X when
/// facing +Z).
fn slots(kind: u8, n: usize) -> Vec<(Fixed, Fixed)> {
    let gap = spacing();
    let half = Fixed::from_int(n as i32 - 1) / Fixed::from_int(2);
    (0..n)
        .map(|i| {
            let fi = Fixed::from_int(i as i32);
            match kind {
                FORM_LINE => (Fixed::ZERO, (fi - half) * gap),
                FORM_COLUMN => ((half - fi) * gap, Fixed::ZERO),
                FORM_WEDGE => {
                    // Apex first, then alternating sides, one row back per pair.
                    let row = Fixed::from_int((i as i32 + 1) / 2);
                    let side = if i % 2 == 1 { -Fixed::ONE } else { Fixed::ONE };
                    (-row * gap, side * row * gap)
                }
                _ => (Fixed::ZERO, Fixed::ZERO),
            }
        })
        .collect()
}

impl Sim {
    /// Run after the whole command batch and after late target writers. A later
    /// ForceOntoMap edit in the same batch therefore governs every resolved leg.
    pub(crate) fn constrain_targets(&mut self) {
        if !self.confinement.is_active() { return; }
        for unit in self.units.iter_mut() {
            let Some(volume) = self.confinement.active_volume(unit) else { continue };
            if let Some(target) = unit.target.as_mut() { *target = volume.check(*target, true).position; }
            if let Some(course) = self.movement.courses.get_mut(unit.id) {
                for target in &mut course.waypoints { *target = volume.check(*target, true).position; }
            }
        }
    }

    /// Apply a move, stop or formation order. `false`: reject (no unit could
    /// take it, or an unknown formation kind).
    pub(crate) fn apply_movement(&mut self, player: PlayerId, order: &Order) -> bool {
        let units = match order {
            Order::Move { units, .. } | Order::Stop { units } | Order::Formation { units, .. } => {
                units
            }
            _ => return false,
        };
        // Players order their own units; scripts and the bench any unit.
        // Units that are gone, not the player's or unable to move are
        // skipped; the order is rejected only when none is left.
        let mut ids: Vec<EntityId> = units
            .iter()
            .copied()
            .filter(|&id| {
                (matches!(order, Order::Stop { .. }) || !self.production.is_welding(id))
                    && self.unit(id)
                    .is_some_and(|u| u.orderable() && (!player.is_player() || u.owner == player))
            })
            .collect();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return false;
        }
        match *order {
            Order::Move { to, queue, .. } => self.move_group(&ids, to, queue),
            Order::Stop { .. } => {
                for &id in &ids {
                    self.set_target(id, None);
                    self.movement.edit(id, |c| c.waypoints.clear());
                }
            }
            Order::Formation { kind, .. } => {
                if kind > FORM_WEDGE {
                    return false;
                }
                for &id in &ids {
                    self.movement.edit(id, |c| c.formation = kind);
                }
            }
            _ => return false,
        }
        true
    }

    /// Stop both residual velocity and old waypoints at the weld boundary.
    /// Also runs before physics so later target writers cannot restart a rig.
    pub(crate) fn hold_welding_builders(&mut self) {
        for id in self.production.welding_builders() {
            if let Some(unit) = self.units.get_mut(id) {
                unit.target = None;
                unit.speed = Fixed::ZERO;
                unit.omega = Fixed::ZERO;
            }
            self.movement.clear(id);
        }
    }

    fn move_group(&mut self, ids: &[EntityId], to: Vec3, queue: bool) {
        let kind = self
            .movement
            .course(ids[0])
            .map_or(FORM_NONE, |c| c.formation);
        let dests = self.destinations(ids, to, kind);
        for (&id, dest) in ids.iter().zip(dests) {
            let idle = self.unit(id).is_some_and(|u| u.target.is_none());
            if queue && !idle {
                self.movement.edit(id, |c| c.waypoints.push(dest));
            } else {
                self.set_target(id, Some(dest));
                self.movement.edit(id, |c| c.waypoints.clear());
            }
        }
    }

    /// Where each of `ids` (ascending, all present) goes for a move to `to`.
    fn destinations(&self, ids: &[EntityId], to: Vec3, kind: u8) -> Vec<Vec3> {
        if kind == FORM_NONE || ids.len() < 2 {
            return vec![to; ids.len()];
        }
        let n = Fixed::from_int(ids.len() as i32);
        let (mut sx, mut sz) = (Fixed::ZERO, Fixed::ZERO);
        for &id in ids {
            let p = self.unit(id).map_or(Vec3::ZERO, |u| u.pos);
            sx = sx + p.x;
            sz = sz + p.z;
        }
        let (dx, dz) = (to.x - sx / n, to.z - sz / n);
        let len = (dx * dx + dz * dz).sqrt();
        let (fx, fz) = if len == Fixed::ZERO {
            (Fixed::ZERO, Fixed::ONE)
        } else {
            (dx / len, dz / len)
        };
        slots(kind, ids.len())
            .into_iter()
            .map(|(fwd, lat)| Vec3 {
                x: to.x + fx * fwd + fz * lat,
                y: to.y,
                z: to.z + fz * fwd - fx * lat,
            })
            .collect()
    }

    /// After the units have stepped: units that reached their leg take the
    /// next one; courses of units that are gone are dropped.
    pub(crate) fn advance_courses(&mut self) {
        let mut gone = Vec::new();
        let mut next = Vec::new();
        for (id, c) in self.movement.courses.iter() {
            match self.units.get(id) {
                None => gone.push(id),
                Some(u) if u.target.is_none() && !c.waypoints.is_empty() => next.push(id),
                _ => {}
            }
        }
        for id in gone {
            self.movement.courses.remove(id);
        }
        for id in next {
            let mut leg = None;
            self.movement
                .edit(id, |c| leg = Some(c.waypoints.remove(0)));
            self.set_target(id, leg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandKind, Edit};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use crate::{SimEvent, WorldSource};
    use sta_content::MapDef;
    use sta_core::TeamId;
    use std::sync::Arc;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::from_int(z),
        }
    }

    fn game() -> (Sim, Arc<sta_content::ContentDb>) {
        let seat = |id, control| PlayerSetup {
            id: PlayerId(id),
            team: TeamId(id),
            race: "fx".into(),
            control,
        };
        let content = Arc::new(sta_content::fake::db());
        let s = GameSetup {
            seed: 3,
            map: Arc::new(MapDef::empty()),
            content: Arc::clone(&content),
            players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
            options: GameOptions::default(),
        };
        (Sim::from_setup(&s), content)
    }

    fn cmd(player: PlayerId, seq: u32, kind: CommandKind) -> Command {
        Command { player, seq, kind }
    }

    fn spawn(seq: u32, db: &sta_content::ContentDb, odf: &str, player: u8, pos: Vec3) -> Command {
        let e = Edit::Spawn {
            odf: db.key(odf).unwrap(),
            player: PlayerId(player),
            pos,
            heading: (Fixed::ZERO, Fixed::ONE),
        };
        cmd(PlayerId::BENCH, seq, CommandKind::Edit(e))
    }

    fn order(player: u8, seq: u32, o: Order) -> Command {
        cmd(PlayerId(player), seq, CommandKind::Order(o))
    }

    fn ids(v: &[u32]) -> Vec<EntityId> {
        v.iter().map(|&i| EntityId(i)).collect()
    }

    fn pos(sim: &Sim, id: u32) -> Vec3 {
        sim.unit(EntityId(id)).unwrap().pos
    }

    /// Steps until `done`, at most `max` ticks; the count it took.
    fn run_until(sim: &mut Sim, max: u32, mut done: impl FnMut(&Sim) -> bool) -> Option<u32> {
        (1..=max).find(|_| {
            WorldSource::step(sim, &[]);
            done(sim)
        })
    }

    /// Within this many ticks (30 per second).
    const GOTO_TICKS: u32 = 30 * 60;

    /// The bench `goto` scenario, in-crate until the runner (TOOL-02) exists:
    /// a cruiser flies through two queued waypoints to the last.
    fn goto() -> (Sim, u32) {
        let (mut sim, db) = game();
        WorldSource::step(&mut sim, &[spawn(0, &db, "fx_cruiser", 0, v(0, 0, 0))]);
        let a = 1;
        let cmds = [
            order(
                0,
                0,
                Order::Move {
                    units: ids(&[a]),
                    to: v(0, 0, 600),
                    queue: false,
                },
            ),
            order(
                0,
                1,
                Order::Move {
                    units: ids(&[a]),
                    to: v(500, 20, 600),
                    queue: true,
                },
            ),
            order(
                0,
                2,
                Order::Move {
                    units: ids(&[a]),
                    to: v(500, 0, -200),
                    queue: true,
                },
            ),
        ];
        WorldSource::step(&mut sim, &cmds);
        assert_eq!(sim.movement.course(EntityId(a)).unwrap().waypoints.len(), 2);
        let t = run_until(&mut sim, GOTO_TICKS, |s| {
            s.unit(EntityId(a)).unwrap().target.is_none()
                && s.movement.course(EntityId(a)).is_none()
        })
        .expect("did not arrive");
        (sim, t)
    }

    #[test]
    fn goto_arrives_through_its_waypoints_within_n_ticks() {
        let (sim, t) = goto();
        assert!(t < GOTO_TICKS);
        let u = sim.unit(EntityId(1)).unwrap();
        assert_eq!((u.pos, u.speed), (v(500, 0, -200), Fixed::ZERO));
    }

    /// Golden: pinned so any change to movement arithmetic or hashing shows.
    #[test]
    fn goto_digest_golden() {
        let (a, ta) = goto();
        let (b, tb) = goto();
        assert_eq!((a.digest(), ta), (b.digest(), tb));
        assert_eq!(crate::unit_status_tests::digest_before_crew(&a), 0xcea6_a6f2_d1c1_391c);
        assert_eq!((a.digest(), ta), (GOTO_DIGEST, GOTO_ARRIVAL));
    }
    const GOTO_DIGEST: u64 = 0xa5b7_f03b_da3c_d432;
    const GOTO_ARRIVAL: u32 = 544;

    #[test]
    fn waypoints_are_visited_in_order() {
        let (mut sim, db) = game();
        WorldSource::step(&mut sim, &[spawn(0, &db, "fx_cruiser", 0, v(0, 0, 0))]);
        let cmds = [
            order(
                0,
                0,
                Order::Move {
                    units: ids(&[1]),
                    to: v(0, 0, 300),
                    queue: true,
                },
            ), // idle: starts at once
            order(
                0,
                1,
                Order::Move {
                    units: ids(&[1]),
                    to: v(300, 0, 300),
                    queue: true,
                },
            ),
        ];
        WorldSource::step(&mut sim, &cmds);
        assert_eq!(sim.unit(EntityId(1)).unwrap().target, Some(v(0, 0, 300)));
        run_until(&mut sim, 3000, |s| {
            s.unit(EntityId(1)).unwrap().target == Some(v(300, 0, 300))
        })
        .expect("second leg never began");
        assert_eq!(pos(&sim, 1), v(0, 0, 300));
    }

    #[test]
    fn stop_clears_target_and_queue() {
        let (mut sim, db) = game();
        WorldSource::step(&mut sim, &[spawn(0, &db, "fx_cruiser", 0, v(0, 0, 0))]);
        let mvq = |seq, to, queue| {
            order(
                0,
                seq,
                Order::Move {
                    units: ids(&[1]),
                    to,
                    queue,
                },
            )
        };
        WorldSource::step(
            &mut sim,
            &[mvq(0, v(0, 0, 900), false), mvq(1, v(900, 0, 900), true)],
        );
        for _ in 0..60 {
            WorldSource::step(&mut sim, &[]);
        }
        WorldSource::step(&mut sim, &[order(0, 2, Order::Stop { units: ids(&[1]) })]);
        assert!(sim.movement.course(EntityId(1)).is_none());
        run_until(&mut sim, 600, |s| {
            s.unit(EntityId(1)).unwrap().speed == Fixed::ZERO
        })
        .expect("never stopped");
        let at = pos(&sim, 1);
        for _ in 0..100 {
            WorldSource::step(&mut sim, &[]);
        }
        assert_eq!(pos(&sim, 1), at);
        assert!(at.z < v(0, 0, 900).z && at.x == Fixed::ZERO);
    }

    #[test]
    fn formation_sends_units_to_distinct_slots_and_arrives() {
        let (mut sim, db) = game();
        let spawns: Vec<_> = (0..3)
            .map(|i| spawn(i, &db, "fx_frigate", 0, v(-100 + 100 * i as i32, 0, 0)))
            .collect();
        WorldSource::step(&mut sim, &spawns);
        let go = [
            order(
                0,
                0,
                Order::Formation {
                    units: ids(&[1, 2, 3]),
                    kind: FORM_LINE,
                },
            ),
            order(
                0,
                1,
                Order::Move {
                    units: ids(&[1, 2, 3]),
                    to: v(0, 0, 1000),
                    queue: false,
                },
            ),
        ];
        WorldSource::step(&mut sim, &go);
        // Group faces +Z: a line along X, 60 apart, centred on the point.
        let t: Vec<_> = (1..=3)
            .map(|i| sim.unit(EntityId(i)).unwrap().target.unwrap())
            .collect();
        assert_eq!(t, [v(-60, 0, 1000), v(0, 0, 1000), v(60, 0, 1000)]);
        run_until(&mut sim, 3000, |s| {
            (1..=3).all(|i| s.unit(EntityId(i)).unwrap().target.is_none())
        })
        .expect("never arrived");
        assert_eq!(
            [pos(&sim, 1), pos(&sim, 2), pos(&sim, 3)],
            [t[0], t[1], t[2]]
        );
    }

    #[test]
    fn formation_layouts() {
        let col = slots(FORM_COLUMN, 3);
        assert_eq!(col[0].0, spacing());
        assert_eq!(col[2].0, -spacing());
        let wedge = slots(FORM_WEDGE, 3);
        assert_eq!(wedge[0], (Fixed::ZERO, Fixed::ZERO));
        assert_eq!(wedge[1].1, -wedge[2].1);
        assert_eq!(wedge[1].0, wedge[2].0);
    }

    #[test]
    fn bad_orders_are_rejected() {
        let (mut sim, db) = game();
        WorldSource::step(&mut sim, &[spawn(0, &db, "fx_cruiser", 0, v(0, 0, 0))]);
        let cmds = [
            order(
                0,
                0,
                Order::Formation {
                    units: ids(&[1]),
                    kind: 9,
                },
            ), // unknown kind
            order(
                1,
                0,
                Order::Formation {
                    units: ids(&[1]),
                    kind: FORM_LINE,
                },
            ), // foreign
            order(
                0,
                1,
                Order::Formation {
                    units: ids(&[1]),
                    kind: FORM_LINE,
                },
            ),
        ];
        let out = WorldSource::step(&mut sim, &cmds);
        assert_eq!(
            out.events,
            [
                SimEvent::Rejected {
                    player: PlayerId(0),
                    seq: 0
                },
                SimEvent::Rejected {
                    player: PlayerId(1),
                    seq: 0
                }
            ]
        );
        assert_eq!(
            sim.movement.course(EntityId(1)).unwrap().formation,
            FORM_LINE
        );
    }

    #[test]
    fn courses_of_dead_units_are_dropped() {
        let (mut sim, db) = game();
        WorldSource::step(&mut sim, &[spawn(0, &db, "fx_cruiser", 0, v(0, 0, 0))]);
        let mvq = |seq, to| {
            order(
                0,
                seq,
                Order::Move {
                    units: ids(&[1]),
                    to,
                    queue: true,
                },
            )
        };
        WorldSource::step(&mut sim, &[mvq(0, v(0, 0, 900)), mvq(1, v(900, 0, 900))]);
        WorldSource::step(
            &mut sim,
            &[cmd(
                PlayerId::BENCH,
                0,
                CommandKind::Edit(Edit::Kill(EntityId(1))),
            )],
        );
        assert!(sim.movement.course(EntityId(1)).is_none());
    }
}
