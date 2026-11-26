//! Legacy transwarp pairs, Galaxy conversion and holographic impersonation.

use serde::{Deserialize, Serialize};
use sta_content::{A2Effect, A2SpecialDef, ContentDb, FederationWeaponDef, WeaponDef};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TICK_HZ, Tick, Vec3};

use super::weapon_at;
use crate::Sim;
use crate::command::{Command, CommandKind, Order, Target};
use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, EntityKind, NEUTRAL};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Federation {
    gates: Vec<Gate>,
    projections: Vec<Projection>,
    fusions: Vec<Fusion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Gate {
    source: EntityId,
    slot: u8,
    owner: PlayerId,
    ends: [EntityId; 2],
    until: Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Projection {
    source: EntityId,
    slot: u8,
    owner: PlayerId,
    model: ContentKey,
    perceived: PlayerId,
    prior: PlayerId,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Fusion {
    ships: [EntityId; 2],
    source: EntityId,
    slot: u8,
    owner: PlayerId,
    combined: ContentKey,
    goals: [Vec3; 2],
    closing: bool,
    until: Tick,
}

fn alive(u: &Entity) -> bool {
    u.hull[0] > Fixed::ZERO && !u.is_unfinished() && !u.flags.contains(UnitFlags::DYING)
}

fn can_sustain(u: &Entity, owner: PlayerId) -> bool {
    alive(u) && u.owner == owner && !u.flags.contains(UnitFlags::WEAPONS_OFF)
}

fn craft_key(db: &ContentDb, name: &str) -> Option<ContentKey> {
    let key = db.key(name)?;
    db.unit(key).filter(|d| {
        EntityKind::of_def(d) == EntityKind::Craft && d.a1_officer_upgrade_gain.is_none()
    })?;
    Some(key)
}

fn offset(at: Vec3, heading: (Fixed, Fixed), distance: Fixed) -> Vec3 {
    Vec3 {
        x: at.x + heading.0 * distance,
        y: at.y,
        z: at.z + heading.1 * distance,
    }
}

fn centre(a: Vec3, b: Vec3) -> Vec3 {
    let half = Fixed::from_bits(Fixed::ONE.to_bits() / 2);
    Vec3 {
        x: a.x + (b.x - a.x) * half,
        y: a.y + (b.y - a.y) * half,
        z: a.z + (b.z - a.z) * half,
    }
}

impl Federation {
    pub(super) fn is_empty(&self) -> bool {
        self.gates.is_empty() && self.projections.is_empty() && self.fusions.is_empty()
    }

    pub(super) fn valid(&self, tick: Tick, units: &Entities, db: Option<&ContentDb>) -> bool {
        let effect = |id, slot| {
            let db = db?;
            Some(weapon_at(db, db.unit(units.get(id)?.def)?, slot)?.2.effect)
        };
        self.gates.iter().all(|g| {
            g.until >= tick
                && g.ends[0] != g.ends[1]
                && !g.ends.contains(&g.source)
                && effect(g.source, g.slot) == Some(A2Effect::TranswarpGate)
                && g.ends.iter().all(|&id| {
                    units
                        .get(id)
                        .and_then(|u| db?.unit(u.def))
                        .is_some_and(|d| d.class.0 == "wormhole_obj")
                })
        }) && self.projections.iter().all(|p| {
            effect(p.source, p.slot) == Some(A2Effect::HoloGenerator)
                && crate::perception::representable(p.perceived)
                && crate::perception::representable(p.prior)
                && db
                    .and_then(|d| d.unit(p.model))
                    .is_some_and(|d| EntityKind::of_def(d) == EntityKind::Craft)
        }) && self.fusions.iter().all(|f| {
            f.until >= tick
                && f.ships[0] != f.ships[1]
                && f.ships.contains(&f.source)
                && f.ships
                    .iter()
                    .all(|&id| units.get(id).is_some_and(|u| u.kind == EntityKind::Craft))
                && matches!(
                    effect(f.source, f.slot),
                    Some(A2Effect::GalaxyIntegrate { .. })
                )
                && db
                    .and_then(|d| d.unit(f.combined))
                    .is_some_and(|d| EntityKind::of_def(d) == EntityKind::Craft)
        })
    }

    pub(super) fn active(&self, tick: Tick) -> impl Iterator<Item = (EntityId, u8)> + '_ {
        self.gates
            .iter()
            .filter(move |g| g.until > tick)
            .map(|g| (g.source, g.slot))
            .chain(self.projections.iter().map(|p| (p.source, p.slot)))
            .chain(self.fusions.iter().map(|f| (f.source, f.slot)))
    }

    pub(crate) fn gate_ends(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.gates.iter().flat_map(|g| g.ends)
    }

    pub(crate) fn gate_pairs(&self, tick: Tick) -> impl Iterator<Item = (EntityId, EntityId)> + '_ {
        self.gates
            .iter()
            .filter(move |g| g.until > tick)
            .flat_map(|g| [(g.ends[0], g.ends[1]), (g.ends[1], g.ends[0])])
    }

    pub(crate) fn projections(&self) -> Vec<(EntityId, ContentKey, PlayerId)> {
        let mut rows: Vec<_> = self
            .projections
            .iter()
            .map(|p| (p.source, p.model, p.perceived))
            .collect();
        rows.sort_unstable_by_key(|r| r.0);
        rows
    }
}

impl Sim {
    pub(crate) fn remove_federation_entity(&mut self, id: EntityId) {
        let state = &mut self.specials.a2.federation;
        state.projections.retain(|p| p.source != id);
        let removed_gates: Vec<_> = state
            .gates
            .extract_if(.., |g| g.source == id || g.ends.contains(&id))
            .collect();
        let removed_fusions: Vec<_> = state
            .fusions
            .extract_if(.., |f| f.ships.contains(&id))
            .collect();
        for gate in removed_gates {
            for end in gate.ends {
                if end != id {
                    self.retire(end);
                }
            }
        }
        for fusion in removed_fusions {
            self.clear_fusion_goals(fusion);
        }
    }

    pub(super) fn cancel_federation(
        &mut self,
        id: EntityId,
        slot: u8,
        target: Target,
    ) -> Option<bool> {
        let state = &mut self.specials.a2.federation;
        if let Some(i) = state
            .gates
            .iter()
            .position(|g| (g.source, g.slot) == (id, slot))
        {
            if target != Target::None {
                return Some(false);
            }
            let gate = state.gates.remove(i);
            for end in gate.ends {
                self.retire(end);
            }
            return Some(true);
        }
        if let Some(i) = state
            .projections
            .iter()
            .position(|p| (p.source, p.slot) == (id, slot))
        {
            if target != Target::None {
                return Some(false);
            }
            let projection = state.projections.remove(i);
            self.clear_projection(projection);
            return Some(true);
        }
        if let Some(i) = state
            .fusions
            .iter()
            .position(|f| (f.source, f.slot) == (id, slot))
        {
            if target != Target::None {
                return Some(false);
            }
            let fusion = state.fusions.remove(i);
            self.clear_fusion_goals(fusion);
            return Some(true);
        }
        None
    }

    pub(super) fn activate_federation(
        &mut self,
        db: &ContentDb,
        source: &Entity,
        weapon: &WeaponDef,
        rule: A2SpecialDef,
        slot: u8,
        target: Target,
    ) -> Option<bool> {
        let accepted =
            match rule.effect {
                A2Effect::TranswarpGate => {
                    let (Some(FederationWeaponDef::Gate { portal }), Target::Point(at)) =
                        (&weapon.federation, target)
                    else {
                        return Some(false);
                    };
                    let Some(gate) = db.key(portal).and_then(|k| db.unit(k)).filter(|d| {
                        d.class.0 == "wormhole_obj" && d.a1_officer_upgrade_gain.is_none()
                    }) else {
                        return Some(false);
                    };
                    let (hx, hz) = source.heading;
                    let entry = db
                        .unit(source.def)
                        .and_then(|d| d.transwarp_gate.as_ref())
                        .map_or_else(
                            || {
                                offset(
                                    source.pos,
                                    source.heading,
                                    self.body_radius(source.def) + Fixed::from_int(45),
                                )
                            },
                            |point| Vec3 {
                                x: source.pos.x + hz * point.offset.0 + hx * point.offset.1,
                                y: source.pos.y + point.height,
                                z: source.pos.z - hx * point.offset.0 + hz * point.offset.1,
                            },
                        );
                    let entry = self.bound_federation_point(entry);
                    if super::a1::in_range(entry, at, Fixed::from_int(10)) {
                        return Some(false);
                    }
                    let exit = Vec3 {
                        y: gate.stats.distance_below_grid.map_or(at.y, |depth| -depth),
                        ..at
                    };
                    let first = self
                        .spawn_unqueued_def(gate.key, NEUTRAL, entry, source.heading)
                        .expect("validated gate");
                    let second = self
                        .spawn_unqueued_def(gate.key, NEUTRAL, exit, source.heading)
                        .expect("validated gate");
                    self.specials.a2.federation.gates.push(Gate {
                        source: source.id,
                        owner: source.owner,
                        slot,
                        ends: [first, second],
                        until: self.tick.saturating_add(rule.duration.max(1)),
                    });
                    true
                }
                A2Effect::HoloGenerator => {
                    let Target::Entity(id) = target else {
                        return Some(false);
                    };
                    if self
                        .specials
                        .a2
                        .federation
                        .projections
                        .iter()
                        .any(|p| p.source == source.id)
                    {
                        return Some(false);
                    }
                    let Some(copied) = self.units.get(id).cloned() else {
                        return Some(false);
                    };
                    let prior = self.perceived_owner(source.id).unwrap_or(source.owner);
                    if !self.set_perceived_owner(source.id, copied.owner) {
                        return Some(false);
                    }
                    self.specials.a2.federation.projections.push(Projection {
                        source: source.id,
                        slot,
                        owner: source.owner,
                        model: copied.def,
                        perceived: copied.owner,
                        prior,
                    });
                    true
                }
                A2Effect::GalaxySeparate => self.separate_galaxy(db, source),
                A2Effect::GalaxyIntegrate { initial, .. } => {
                    self.begin_galaxy_fusion(db, source, weapon, slot, initial)
                }
                _ => return None,
            };
        Some(accepted)
    }

    fn bound_federation_point(&self, at: Vec3) -> Vec3 {
        self.bounds.map_or(at, |(min, max)| Vec3 {
            x: at.x.clamp(min.x, max.x),
            y: at.y,
            z: at.z.clamp(min.z, max.z),
        })
    }

    fn separate_galaxy(&mut self, db: &ContentDb, source: &Entity) -> bool {
        let (Some(saucer), Some(bridge)) = (craft_key(db, "fsaucer"), craft_key(db, "fbbridge"))
        else {
            return false;
        };
        if source.kind != EntityKind::Craft {
            return false;
        }
        let side = (source.heading.1, -source.heading.0);
        let distance = (self.body_radius(saucer) + self.body_radius(bridge)) / Fixed::from_int(2)
            + Fixed::from_int(5);
        let first = self.replace_galaxy_part(
            db,
            saucer,
            source,
            None,
            self.bound_federation_point(offset(source.pos, side, distance)),
        );
        self.replace_galaxy_part(
            db,
            bridge,
            source,
            None,
            self.bound_federation_point(offset(source.pos, side, -distance)),
        );
        self.weapons.retarget_replacement_shots(source.id, first);
        self.retire(source.id);
        self.events.push(SimEvent::Replaced {
            old: source.id,
            new: first,
            select: true,
        });
        true
    }

    fn replace_galaxy_part(
        &mut self,
        db: &ContentDb,
        key: ContentKey,
        source: &Entity,
        second: Option<&Entity>,
        at: Vec3,
    ) -> EntityId {
        let average = |first: Fixed, other: Option<Fixed>| {
            other.map_or(first, |value| first + (value - first) / Fixed::from_int(2))
        };
        let ratio = |now: Fixed, max: Fixed| {
            if max > Fixed::ZERO {
                (now / max).clamp(Fixed::ZERO, Fixed::ONE)
            } else {
                Fixed::ONE
            }
        };
        let id = self
            .spawn_unqueued_def(key, source.owner, at, source.heading)
            .expect("validated Galaxy class");
        self.autonomy.copy_to(source.id, id);
        self.edit_alert(id, self.alert_of(source.id));
        let u = self.units.get_mut(id).expect("spawned Galaxy class");
        let hull = |u: &Entity| ratio(u.hull[0], u.hull[1]);
        u.hull[0] = u.hull[1] * average(hull(source), second.map(hull));
        let faces = if u.shields.directional { 4 } else { 1 };
        for face in 0..faces {
            let shields = |u: &Entity| ratio(u.shields.view()[face][0], u.shields.max);
            u.shields.now[face] = u.shields.max * average(shields(source), second.map(shields));
        }
        if let Some(crew) = &mut u.crew {
            let share = |u: &Entity| u.crew.map_or(Fixed::ONE, |c| ratio(c[0], c[1]));
            crew[0] = crew[1] * average(share(source), second.map(share));
        }
        if let Some(pool) = db.unit(key).and_then(crate::specials::energy_def) {
            let share = |u: &Entity| {
                let max = db.unit(u.def).map_or(Fixed::ZERO, |d| d.special_energy_max);
                ratio(self.specials.energy_of(u.id).unwrap_or(max), max)
            };
            let energy = pool.max * average(share(source), second.map(share));
            self.specials.set_energy(&self.units, db, id, energy);
        }
        id
    }

    fn begin_galaxy_fusion(
        &mut self,
        db: &ContentDb,
        source: &Entity,
        weapon: &WeaponDef,
        slot: u8,
        initial: Fixed,
    ) -> bool {
        let Some(FederationWeaponDef::Integrate { combined, parts }) = &weapon.federation else {
            return false;
        };
        let (Some(combined), Some(part_a), Some(part_b)) = (
            craft_key(db, combined),
            craft_key(db, &parts[0]),
            craft_key(db, &parts[1]),
        ) else {
            return false;
        };
        let other_def = if source.def == part_a {
            part_b
        } else if source.def == part_b {
            part_a
        } else {
            return false;
        };
        let free = |id| {
            !self
                .specials
                .a2
                .federation
                .fusions
                .iter()
                .any(|f| f.ships.contains(&id))
        };
        let can_move = |ship: &Entity| {
            ship.orderable()
                && ship.mobile()
                && !self.production.is_welding(ship.id)
                && !self.specials.engines_held(ship.id, self.tick)
        };
        if part_a == part_b || !free(source.id) || !can_move(source) {
            return false;
        }
        let Some(other) = self
            .units
            .iter()
            .filter(|u| {
                u.def == other_def
                    && u.owner == source.owner
                    && alive(u)
                    && free(u.id)
                    && can_move(u)
            })
            .min_by_key(|u| (crate::combat::weapons::flat(source.pos, u.pos).1, u.id))
            .cloned()
        else {
            return false;
        };
        let [first, second] = if source.def == part_a {
            [source.clone(), other]
        } else {
            [other, source.clone()]
        };
        let mid = centre(first.pos, second.pos);
        let (axis, _) = crate::combat::weapons::flat(second.pos, first.pos);
        let axis = if axis == (Fixed::ZERO, Fixed::ZERO) {
            first.heading
        } else {
            axis
        };
        let half = initial / Fixed::from_int(2);
        let goals = [
            self.bound_federation_point(offset(mid, axis, half)),
            self.bound_federation_point(offset(mid, axis, -half)),
        ];
        for (ship, to) in [&first, &second].into_iter().zip(goals) {
            self.apply(&Command {
                player: source.owner,
                seq: 0,
                kind: CommandKind::Order(Order::Move {
                    units: vec![ship.id],
                    to,
                    queue: false,
                }),
            });
        }
        self.specials.a2.federation.fusions.push(Fusion {
            ships: [first.id, second.id],
            source: source.id,
            slot,
            owner: source.owner,
            combined,
            goals,
            closing: false,
            until: self.tick.saturating_add(120 * u64::from(TICK_HZ)),
        });
        true
    }

    pub(super) fn step_federation(&mut self, db: &ContentDb) {
        for gate in std::mem::take(&mut self.specials.a2.federation.gates) {
            if gate.until > self.tick
                && self
                    .units
                    .get(gate.source)
                    .is_some_and(|u| can_sustain(u, gate.owner))
                && gate.ends.iter().all(|&id| self.units.contains(id))
            {
                self.specials.a2.federation.gates.push(gate);
            } else {
                for end in gate.ends {
                    self.retire(end);
                }
            }
        }
        for projection in std::mem::take(&mut self.specials.a2.federation.projections) {
            if self
                .units
                .get(projection.source)
                .is_some_and(|u| can_sustain(u, projection.owner))
                && !self.specials.a2_held(
                    projection.source,
                    crate::command::Subsystem::Weapons,
                    self.tick,
                )
                && self.perceived_owner(projection.source) == Some(projection.perceived)
            {
                self.specials.a2.federation.projections.push(projection);
            } else {
                self.clear_projection(projection);
            }
        }
        for mut fusion in std::mem::take(&mut self.specials.a2.federation.fusions) {
            if self.step_galaxy_fusion(db, &mut fusion) {
                self.specials.a2.federation.fusions.push(fusion);
            } else {
                self.clear_fusion_goals(fusion);
            }
        }
    }

    fn clear_projection(&mut self, projection: Projection) {
        if self.perceived_owner(projection.source) == Some(projection.perceived) {
            self.set_perceived_owner(projection.source, projection.prior);
        }
    }

    fn clear_fusion_goals(&mut self, fusion: Fusion) {
        for (id, goal) in fusion.ships.into_iter().zip(fusion.goals) {
            if self.units.get(id).is_some_and(|u| u.target == Some(goal)) {
                self.set_target(id, None);
                self.movement.clear(id);
            }
        }
    }

    fn step_galaxy_fusion(&mut self, db: &ContentDb, fusion: &mut Fusion) -> bool {
        let [Some(first), Some(second)] = fusion.ships.map(|id| self.units.get(id).cloned()) else {
            return false;
        };
        let Some((_, _, rule)) = self
            .units
            .get(fusion.source)
            .and_then(|u| weapon_at(db, db.unit(u.def)?, fusion.slot))
        else {
            return false;
        };
        let A2Effect::GalaxyIntegrate { final_distance, .. } = rule.effect else {
            return false;
        };
        if self.tick >= fusion.until
            || [&first, &second]
                .into_iter()
                .zip(fusion.goals)
                .any(|(ship, goal)| {
                    !alive(ship)
                        || ship.owner != fusion.owner
                        || ship.flags.contains(UnitFlags::ENGINES_OFF)
                        || ship.target.is_some_and(|to| to != goal)
                })
        {
            return false;
        }
        if !fusion.closing {
            let arrived = [&first, &second]
                .into_iter()
                .zip(fusion.goals)
                .all(|(ship, goal)| {
                    super::a1::in_range(
                        ship.pos,
                        goal,
                        ship.stats.too_close_to_turn.max(Fixed::from_int(20)),
                    )
                });
            if arrived {
                let mid = centre(first.pos, second.pos);
                fusion.goals = [mid; 2];
                fusion.closing = true;
                for id in fusion.ships {
                    self.set_target(id, Some(mid));
                    self.movement.clear_waypoints(id);
                }
            }
            return true;
        }
        // Model collision radii can keep the halves physically apart even
        // when both have reached the point at which the hulls join.
        let contact = self.body_radius(first.def) + self.body_radius(second.def) + final_distance;
        if !super::a1::in_range(first.pos, second.pos, contact) {
            // The ordinary move controller stops at tooCloseToTurn (50 units
            // for the stock halves). Complete the short docking leg after it
            // stops, retaining the model clearance at the fusion point.
            let (direction, distance) = crate::combat::weapons::flat(first.pos, second.pos);
            let step = (distance - contact).max(Fixed::ZERO) / Fixed::from_int(2);
            for (ship, direction) in [(&first, direction), (&second, (-direction.0, -direction.1))]
            {
                if ship.target.is_none()
                    && let Some(unit) = self.units.get_mut(ship.id)
                {
                    let limit = ship.stats.max_speed / Fixed::from_int(TICK_HZ as i32);
                    let advance = (step + Fixed::ONE).min(limit);
                    unit.pos = offset(ship.pos, direction, advance);
                }
            }
            return true;
        }
        let id = self.replace_galaxy_part(
            db,
            fusion.combined,
            &first,
            Some(&second),
            centre(first.pos, second.pos),
        );
        for old in fusion.ships {
            self.weapons.retarget_replacement_shots(old, id);
            self.retire(old);
            self.events.push(SimEvent::Replaced {
                old,
                new: id,
                select: true,
            });
        }
        false
    }
}
