//! Armada II special ordnance and timed effects. Casts, impacts and status
//! durations are deterministic sim state, separate from ordinary gun fire.

use serde::{Deserialize, Serialize};
use sta_content::{A2Effect, A2SpecialDef, A2Target, ContentDb, UnitDef, WeaponDef};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TICK_HZ, Tick, Vec3};

use super::{SpecialKind, Specials, a1, specials_indexed};
use crate::Sim;
use crate::combat::weapons::{facing, flat};
use crate::command::{Subsystem, Target};
use crate::event::SimEvent;
use crate::relations::Diplomacy;
use crate::view::{Relation, UnitFlags, WorldView};
use crate::world::{Entities, Entity, EntityKind, Fnv};

#[path = "a2_channel.rs"]
mod channel;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct State {
    cooling: Vec<(EntityId, u8, Tick)>,
    shots: Vec<Shot>,
    effects: Vec<Effect>,
    blasts: Vec<Blast>,
    channels: Vec<channel::Channel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Shot {
    source: EntityId,
    owner: PlayerId,
    target: EntityId,
    weapon: ContentKey,
    at: Vec3,
    expires: Tick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Blast {
    source: EntityId,
    def: ContentKey,
    weapon: ContentKey,
    at: Vec3,
    radius: Fixed,
    reached: Fixed,
    hit: Vec<EntityId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Status {
    Disable(Subsystem),
    Remodulate,
    Drain,
    Destruct(u8),
    Regenerate(u8),
    Chant,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Effect {
    target: EntityId,
    source: EntityId,
    weapon: ContentKey,
    status: Status,
    until: Tick,
}

const SYSTEMS: [Subsystem; 5] = [
    Subsystem::Engines,
    Subsystem::Weapons,
    Subsystem::Shields,
    Subsystem::LifeSupport,
    Subsystem::Sensors,
];

fn weapon_at<'a>(
    db: &'a ContentDb,
    def: &'a UnitDef,
    slot: u8,
) -> Option<(usize, &'a WeaponDef, A2SpecialDef)> {
    let (mount, _, w) = specials_indexed(db, def).nth(usize::from(slot))?;
    Some((mount, w, w.a2_special?))
}

fn allowed(w: &WeaponDef, rule: A2SpecialDef, def: &UnitDef, relation: Relation) -> bool {
    let mask = &w.base.targets;
    let defaults_ally = matches!(
        rule.effect,
        A2Effect::RestoreShields(_)
            | A2Effect::RestoreEnergy(_)
            | A2Effect::Remodulate
            | A2Effect::DeathChant { .. }
            | A2Effect::ShieldProjector(_)
    );
    if (rule.hit_condition == 1 && relation != Relation::Ally
        || rule.hit_condition == 2 && relation != Relation::Enemy)
        && rule.target == A2Target::Immediate
    {
        return false;
    }
    if def.planet_role.is_some() {
        return relation == Relation::Enemy
            && (mask.enemy_planets == Some(true) || mask.enemy_planets_only == Some(true));
    }
    if mask.enemy_planets_only == Some(true) {
        return false;
    }
    match (relation, EntityKind::of_def(def)) {
        (Relation::Enemy, EntityKind::Craft) => mask.enemy_craft.unwrap_or(!defaults_ally),
        (Relation::Enemy, EntityKind::Station) => mask.enemy_buildings.unwrap_or(!defaults_ally),
        (Relation::Ally, EntityKind::Craft) => mask.allied_craft.unwrap_or(defaults_ally),
        (Relation::Ally, EntityKind::Station) => mask.allied_buildings.unwrap_or(defaults_ally),
        _ => false,
    }
}

fn valid_def(w: &WeaponDef, def: &UnitDef) -> bool {
    !def.hidden && (!def.invalid_as_target || w.base.ignore_valid_as_target == Some(true))
}

fn target_ok(
    db: &ContentDb,
    w: &WeaponDef,
    rule: A2SpecialDef,
    caster: &Entity,
    target: &Entity,
    dip: Diplomacy<'_>,
) -> bool {
    target.hull[0] > Fixed::ZERO
        && !target.is_unfinished()
        && db.unit(target.def).is_some_and(|def| {
            valid_def(w, def) && allowed(w, rule, def, dip.view(caster.owner, target.owner))
        })
        && (target.owner == caster.owner || !target.flags.contains(UnitFlags::CLOAKED))
        && a1::in_range(caster.pos, target.pos, w.range)
}

/// Whether the selected ship can arm or activate this special now.
pub fn a2_special_ready(view: &WorldView, db: &ContentDb, unit: EntityId, slot: u8) -> bool {
    let Some(u) = view.unit(unit) else {
        return false;
    };
    let Some(def) = db.unit(u.def) else {
        return false;
    };
    let Some((_, w, _)) = weapon_at(db, def, slot) else {
        return false;
    };
    u.hull[0] > Fixed::ZERO
        && u.progress.is_none_or(|p| p >= Fixed::ONE)
        && (a1::a1_special_active(view, unit, slot)
            || !u.flags.contains(UnitFlags::WEAPONS_OFF)
                && (!u.flags.contains(UnitFlags::CLOAKED) || w.base.cloak_enable == Some(true))
                && view.a1_specials.stasis.binary_search(&unit).is_err()
                && view.a1_specials.copies.binary_search(&unit).is_err()
                && view
                    .a1_specials
                    .cooling
                    .binary_search(&(unit, slot))
                    .is_err()
                && a1::researched_in_view(view, db, unit, w)
                && u.energy.map_or(def.special_energy_max, |e| e[0]) >= w.energy_cost)
}

/// Check the local cursor pick against the same target masks and range as the sim.
pub fn a2_special_target_ok(
    view: &WorldView,
    db: &ContentDb,
    unit: EntityId,
    slot: u8,
    target: Target,
) -> bool {
    let Some(u) = view.unit(unit) else {
        return false;
    };
    let Some((_, w, rule)) = db.unit(u.def).and_then(|def| weapon_at(db, def, slot)) else {
        return false;
    };
    match (rule.target, target) {
        (A2Target::Immediate, Target::None) => true,
        (A2Target::Point, Target::Point(at)) => a1::in_range(u.pos, at, w.range),
        (A2Target::Entity, Target::Entity(id)) => view.unit(id).is_some_and(|t| {
            let relation = if u.owner == t.owner {
                Relation::Ally
            } else {
                view.player(u.owner)
                    .and_then(|p| p.relations.iter().find(|r| r.0 == t.owner))
                    .map_or(Relation::Neutral, |r| r.1)
            };
            t.hull[0] > Fixed::ZERO
                && t.progress.is_none_or(|p| p >= Fixed::ONE)
                && (t.owner == u.owner || !t.flags.contains(UnitFlags::CLOAKED))
                && (view.visibility.is_empty()
                    || view
                        .visibility
                        .binary_search_by_key(&u.owner, |row| row.0)
                        .is_ok_and(|i| view.visibility[i].1.binary_search(&id).is_ok()))
                && db
                    .unit(t.def)
                    .is_some_and(|def| valid_def(w, def) && allowed(w, rule, def, relation))
                && a1::in_range(u.pos, t.pos, w.range)
        }),
        _ => false,
    }
}

impl State {
    pub(super) fn is_empty(&self) -> bool {
        self.cooling.is_empty()
            && self.shots.is_empty()
            && self.effects.is_empty()
            && self.blasts.is_empty()
            && self.channels.is_empty()
    }

    pub(super) fn hash_into(&self, h: &mut Fnv) {
        if self.is_empty() {
            return;
        }
        h.str("a2-specials-v1");
        let bytes = postcard::to_allocvec(self).expect("special state serializes");
        h.u64(bytes.len() as u64);
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            h.u64(u64::from_le_bytes(word));
        }
    }

    pub(crate) fn valid(&self, tick: Tick, units: &Entities, db: Option<&ContentDb>) -> bool {
        let by_slot = |id, slot| {
            let db = db?;
            weapon_at(db, db.unit(units.get(id)?.def)?, slot)
        };
        let by_key = |weapon| db?.weapon(weapon)?.a2_special;
        self.cooling
            .windows(2)
            .all(|p| (p[0].0, p[0].1) < (p[1].0, p[1].1))
            && self
                .cooling
                .iter()
                .all(|&(id, slot, until)| until >= tick && by_slot(id, slot).is_some())
            && self.shots.iter().all(|s| {
                units.contains(s.target)
                    && s.expires >= tick
                    && by_key(s.weapon)
                        .is_some_and(|w| !matches!(w.effect, A2Effect::SelfDestruct(_)))
            })
            && self.channels.iter().all(|c| {
                c.until >= tick
                    && units.contains(c.target)
                    && c.source != c.target
                    && by_slot(c.source, c.slot).is_some_and(|(_, w, data)| {
                        w.key == c.weapon
                            && data.effect.sustained_beam()
                            && c.holding == matches!(data.effect, A2Effect::HoldingBeam { .. })
                    })
            })
            && self.blasts.iter().all(|blast| {
                blast.source != EntityId(0)
                    && !units.contains(blast.source)
                    && blast.reached >= Fixed::ZERO
                    && blast.radius > blast.reached
                    && blast.hit.windows(2).all(|w| w[0] < w[1])
                    && db
                        .and_then(|db| db.unit(blast.def))
                        .and_then(|def| def.self_destruct_blast)
                        .is_some()
                    && by_key(blast.weapon)
                        .is_some_and(|w| matches!(w.effect, A2Effect::SelfDestruct(_)))
            })
            && self.effects.iter().all(|e| {
                let Some(w) = by_key(e.weapon) else {
                    return false;
                };
                units.contains(e.target)
                    && e.until >= tick
                    && match (e.status, w.effect) {
                        (Status::Destruct(slot), A2Effect::SelfDestruct(_))
                        | (Status::Regenerate(slot), A2Effect::Regeneration(_)) => {
                            by_slot(e.target, slot).is_some_and(|(_, w, _)| w.key == e.weapon)
                        }
                        (Status::Disable(Subsystem::Weapons), A2Effect::Myotron)
                        | (Status::Disable(Subsystem::Sensors), A2Effect::SensorJammer)
                        | (Status::Disable(_), A2Effect::Polaron)
                        | (Status::Drain, A2Effect::MicroDrain { .. })
                        | (Status::Remodulate, A2Effect::Remodulate)
                        | (Status::Chant, A2Effect::DeathChant { .. }) => true,
                        _ => false,
                    }
            })
    }

    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.cooling.retain(|r| r.0 != id);
        self.effects.retain(|e| e.target != id);
        self.shots.retain(|s| s.target != id);
        self.channels.retain(|c| c.source != id && c.target != id);
    }

    fn record_effect(&mut self, effect: Effect) {
        if let Some(e) = self.effects.iter_mut().find(|e| {
            e.target == effect.target && e.status == effect.status && e.weapon == effect.weapon
        }) {
            e.until = e.until.max(effect.until);
        } else {
            self.effects.push(effect);
        }
    }

    pub(super) fn merge_view(&self, view: &mut a1::SpecialView, tick: Tick) {
        if self.is_empty() {
            return;
        }
        view.cooling.extend(
            self.cooling
                .iter()
                .filter(|r| r.2 > tick)
                .map(|r| (r.0, r.1)),
        );
        view.cooling.sort_unstable();
        view.active
            .extend(self.effects.iter().filter_map(|e| match e.status {
                Status::Destruct(slot) | Status::Regenerate(slot) if e.until > tick => {
                    Some((e.target, slot))
                }
                _ => None,
            }));
        view.active.extend(
            self.channels
                .iter()
                .filter(|c| c.until > tick)
                .map(|c| (c.source, c.slot)),
        );
        view.active.sort_unstable();
    }
}

impl Specials {
    pub(crate) fn a2_held(&self, id: EntityId, sub: Subsystem, now: Tick) -> bool {
        self.a2
            .effects
            .iter()
            .any(|e| e.target == id && e.status == Status::Disable(sub) && e.until > now)
            || sub == Subsystem::Engines
                && self
                    .a2
                    .channels
                    .iter()
                    .any(|c| c.holding && c.target == id && c.until > now)
    }

    pub(crate) fn a2_invulnerable(&self, id: EntityId, now: Tick) -> bool {
        self.a2
            .effects
            .iter()
            .any(|e| e.target == id && e.status == Status::Remodulate && e.until > now)
    }

    pub(crate) fn a2_disabled_units(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.a2
            .effects
            .iter()
            .filter_map(|e| matches!(e.status, Status::Disable(_)).then_some(e.target))
            .chain(
                self.a2
                    .channels
                    .iter()
                    .filter(|c| c.holding)
                    .map(|c| c.target),
            )
    }

    pub(crate) fn a2_chant(&self, db: &ContentDb, id: EntityId, tick: Tick) -> Option<A2Effect> {
        let e = self
            .a2
            .effects
            .iter()
            .find(|e| e.target == id && e.status == Status::Chant && e.until > tick)?;
        Some(db.weapon(e.weapon)?.a2_special?.effect)
    }

    pub(crate) fn effect_view(
        &self,
        tick: Tick,
        units: &Entities,
        db: Option<&ContentDb>,
    ) -> a1::SpecialView {
        let mut view = self.a1.view(tick, units, db);
        self.a2.merge_view(&mut view, tick);
        view
    }
}

impl Sim {
    pub(crate) fn order_a2_special(
        &mut self,
        db: &ContentDb,
        player: PlayerId,
        id: EntityId,
        slot: u8,
        target: Target,
    ) -> Option<bool> {
        let caster = self.units.get(id)?.clone();
        let def = db.unit(caster.def)?;
        let (mount, w, rule) = weapon_at(db, def, slot)?;
        debug_assert_eq!(SpecialKind::of(w), Some(SpecialKind::A2(rule.target)));
        if (player.is_player() && caster.owner != player)
            || caster.hull[0] <= Fixed::ZERO
            || caster.is_unfinished()
        {
            return Some(false);
        }
        let toggle = match rule.effect {
            A2Effect::SelfDestruct(_) => Some(Status::Destruct(slot)),
            A2Effect::Regeneration(_) => Some(Status::Regenerate(slot)),
            _ => None,
        };
        if let Some(toggle) = toggle
            && target == Target::None
            && let Some(index) = self
                .specials
                .a2
                .effects
                .iter()
                .position(|e| e.target == id && e.status == toggle)
        {
            self.specials.a2.effects.remove(index);
            return Some(true);
        }
        if let Some(index) = self
            .specials
            .a2
            .channels
            .iter()
            .position(|c| c.source == id && c.slot == slot)
        {
            if target == Target::None {
                self.specials.a2.channels.remove(index);
                return Some(true);
            }
            return Some(false);
        }
        let dip = Diplomacy::new(&self.players, &self.relations);
        let cooldown = self
            .specials
            .a2
            .cooling
            .binary_search_by_key(&(id, slot), |r| (r.0, r.1));
        if caster.flags.contains(UnitFlags::WEAPONS_OFF)
            || caster.flags.contains(UnitFlags::CLOAKED) && w.base.cloak_enable != Some(true)
            || self.specials.a2_held(id, Subsystem::Weapons, self.tick)
            || self.specials.stasis(id, self.tick)
            || self.specials.is_gemini(id)
            || cooldown.is_ok_and(|i| self.specials.a2.cooling[i].2 > self.tick)
            || !a1::researched_in_world(db, &self.units, dip, &caster, w)
            || self
                .specials
                .energy_of(id)
                .unwrap_or(def.special_energy_max)
                < w.energy_cost
        {
            return Some(false);
        }
        let at = match (rule.target, target) {
            (A2Target::Immediate, Target::None) => caster.pos,
            (A2Target::Entity, Target::Entity(target)) => {
                let Some(t) = self
                    .units
                    .get(target)
                    .filter(|t| target_ok(db, w, rule, &caster, t, dip))
                else {
                    return Some(false);
                };
                t.pos
            }
            (A2Target::Point, Target::Point(at))
                if a1::in_range(caster.pos, at, w.range)
                    && self.bounds.is_none_or(|(min, max)| {
                        at.x >= min.x && at.x <= max.x && at.z >= min.z && at.z <= max.z
                    }) =>
            {
                at
            }
            _ => return Some(false),
        };
        if matches!(rule.effect, A2Effect::SelfDestruct(_))
            && !matches!(caster.kind, EntityKind::Craft | EntityKind::Station)
        {
            return Some(false);
        }
        if rule.effect.sustained_beam() && !matches!(target, Target::Entity(t) if t != id) {
            return Some(false);
        }
        let targets = match target {
            Target::Entity(t) => vec![t],
            _ if toggle.is_some() => Vec::new(),
            _ => {
                let mut targets: Vec<_> = self
                    .units
                    .iter()
                    .filter(|t| t.id != id && target_ok(db, w, rule, &caster, t, dip))
                    .map(|t| (flat(at, t.pos).1, t.id))
                    .collect();
                targets.sort_unstable();
                targets
                    .into_iter()
                    .take(rule.max_targets as usize)
                    .map(|(_, id)| id)
                    .collect()
            }
        };
        if targets.is_empty() && toggle.is_none() {
            return Some(false);
        }

        self.specials.ensure_pool(id, def);
        if !matches!(rule.effect, A2Effect::Regeneration(_))
            && let Some(energy) = self.specials.energy_mut(id)
        {
            *energy = *energy - w.energy_cost;
        }
        if w.refire > 0 {
            let row = (id, slot, self.tick.saturating_add(w.refire));
            match cooldown {
                Ok(i) => self.specials.a2.cooling[i] = row,
                Err(i) => self.specials.a2.cooling.insert(i, row),
            }
        }
        if let Some(status) = toggle {
            self.specials.a2.record_effect(Effect {
                target: id,
                source: id,
                weapon: w.key,
                status,
                until: if matches!(status, Status::Regenerate(_)) {
                    Tick::MAX
                } else {
                    self.tick.saturating_add(rule.duration.max(1))
                },
            });
            self.events.push(SimEvent::Fired {
                shooter: id,
                weapon: w.key,
                hardpoint: mount as u8,
                target: Target::None,
            });
        }
        for target in targets {
            self.events.push(SimEvent::Fired {
                shooter: id,
                weapon: w.key,
                hardpoint: mount as u8,
                target: Target::Entity(target),
            });
            let expires = self.tick.saturating_add(rule.flight.max(1));
            if rule.effect.sustained_beam() {
                let delay = if let A2Effect::HoldingBeam { delay, .. } = rule.effect {
                    delay
                } else {
                    0
                };
                self.specials.a2.channels.push(channel::Channel {
                    source: id,
                    owner: caster.owner,
                    target,
                    weapon: w.key,
                    slot,
                    holding: matches!(rule.effect, A2Effect::HoldingBeam { .. }),
                    until: expires,
                    next_pulse: self.tick.saturating_add(delay),
                });
            } else {
                self.specials.a2.shots.push(Shot {
                    source: id,
                    owner: caster.owner,
                    target,
                    weapon: w.key,
                    at: caster.pos,
                    expires,
                });
            }
        }
        Some(true)
    }

    pub(crate) fn step_a2_specials(&mut self, db: &ContentDb) -> Vec<PlayerId> {
        let mut lost = Vec::new();
        if self.specials.a2.is_empty() {
            return lost;
        }
        self.specials.a2.cooling.retain(|r| r.2 > self.tick);
        let effects = self.specials.a2.effects.clone();
        self.specials.a2.effects.retain(|e| e.until > self.tick);
        for effect in effects {
            if effect.until <= self.tick {
                if matches!(effect.status, Status::Destruct(_)) {
                    lost.extend(self.a2_self_destruct(db, effect));
                }
                continue;
            }
            match effect.status {
                Status::Drain => lost.extend(self.a2_drain(db, effect)),
                Status::Regenerate(_) if !self.a2_regenerate(db, effect) => {
                    self.specials.a2.effects.retain(|e| *e != effect);
                }
                _ => {}
            }
        }
        self.step_a2_blasts(db);
        for mut channel in std::mem::take(&mut self.specials.a2.channels) {
            if self.step_a2_channel(db, &mut channel) {
                self.specials.a2.channels.push(channel);
            }
        }
        for mut shot in std::mem::take(&mut self.specials.a2.shots) {
            if shot.expires <= self.tick {
                continue;
            }
            let Some(w) = db.weapon(shot.weapon) else {
                continue;
            };
            let Some(rule) = w.a2_special else { continue };
            let Some(t) = self
                .units
                .get(shot.target)
                .filter(|u| u.hull[0] > Fixed::ZERO)
            else {
                continue;
            };
            if rule.projectile {
                let (dir, distance) = flat(shot.at, t.pos);
                let speed = w.shot_speed / Fixed::from_int(TICK_HZ as i32);
                if speed > Fixed::ZERO && distance > speed {
                    shot.at.x = shot.at.x + dir.0 * speed;
                    shot.at.z = shot.at.z + dir.1 * speed;
                    self.specials.a2.shots.push(shot);
                    continue;
                }
            }
            let relation = Diplomacy::new(&self.players, &self.relations).view(shot.owner, t.owner);
            if db
                .unit(t.def)
                .is_some_and(|def| allowed(w, rule, def, relation))
                && !self.specials.stasis(t.id, self.tick)
            {
                self.a2_impact(db, w, rule, shot);
            }
        }
        lost
    }

    fn a2_impact(&mut self, db: &ContentDb, w: &WeaponDef, rule: A2SpecialDef, shot: Shot) {
        let id = shot.target;
        let status = match rule.effect {
            A2Effect::Myotron => Some(Status::Disable(Subsystem::Weapons)),
            A2Effect::DeathChant { .. } => Some(Status::Chant),
            A2Effect::Plasma => None,
            A2Effect::Commando(size) => {
                let party = crate::combat::boarding::Party {
                    arrive: self.tick,
                    seq: 0,
                    target: id,
                    size,
                    strength: crate::combat::boarding::race_strength(db, "klingon"),
                    race: "klingon".into(),
                    team: shot.owner,
                    source: shot.source,
                };
                if size > Fixed::ZERO {
                    self.resolve_party(db, party);
                }
                None
            }
            A2Effect::SensorJammer => Some(Status::Disable(Subsystem::Sensors)),
            A2Effect::Polaron => self.a2_polaron(db, id).map(Status::Disable),
            A2Effect::Remodulate => Some(Status::Remodulate),
            A2Effect::MicroDrain { .. } => Some(Status::Drain),
            A2Effect::RestoreShields(amount) => {
                if let Some(u) = self.units.get_mut(id) {
                    let count = if u.shields.directional { 4 } else { 1 };
                    for shield in &mut u.shields.now[..count] {
                        *shield = add_capped(*shield, amount, u.shields.max);
                    }
                }
                None
            }
            A2Effect::RestoreEnergy(amount) => {
                if let Some(def) = self.units.get(id).and_then(|u| db.unit(u.def)) {
                    self.specials.ensure_pool(id, def);
                    if let Some(energy) = self.specials.energy_mut(id) {
                        *energy = add_capped(*energy, amount, def.special_energy_max);
                    }
                }
                None
            }
            A2Effect::PsychonicBlast(fraction) => {
                if let Some([crew, _]) = self.units.get(id).and_then(|u| u.crew) {
                    self.a2_lose_crew(id, crew * fraction);
                }
                None
            }
            A2Effect::SelfDestruct(_)
            | A2Effect::Regeneration(_)
            | A2Effect::AutoAssimilator(_)
            | A2Effect::HoldingBeam { .. }
            | A2Effect::ShieldProjector(_)
            | A2Effect::ShieldInversion(_) => return,
        };
        if let Some(status) = status {
            self.specials.a2.record_effect(Effect {
                target: id,
                source: shot.source,
                weapon: w.key,
                status,
                until: self.tick.saturating_add(rule.duration.max(1)),
            });
        }
        if let Some(t) = self.units.get(id) {
            self.events.push(SimEvent::Hit {
                target: id,
                by: shot.source,
                weapon: w.key,
                facing: facing(t.heading, t.pos, shot.at),
                shield: Fixed::ZERO,
                hull: w.damage.max(Fixed::ZERO),
                at: t.pos,
            });
        }
    }

    fn a2_polaron(&mut self, db: &ContentDb, target: EntityId) -> Option<Subsystem> {
        let u = self.units.get(target)?;
        let def = db.unit(u.def)?;
        // Polaron retries random slots, and a ship with none of the rolled
        // systems keeps its existing state.
        let (indices, systems) = ([0, 1, 3, 4], SYSTEMS);
        for _ in 0..5 {
            let index = indices[(self.rng.next_u64() % indices.len() as u64) as usize];
            if def.system_hit_points[index] > Fixed::ZERO {
                return Some(systems[index]);
            }
        }
        None
    }

    fn a2_lose_crew(&mut self, id: EntityId, loss: Fixed) {
        if let Some([crew, _]) = self.units.get(id).and_then(|u| u.crew) {
            self.apply_edit(
                PlayerId::SCRIPT,
                &crate::command::Edit::UnitCrew(id, (crew - loss).max(Fixed::ZERO)),
            );
        }
    }

    fn a2_drain(&mut self, db: &ContentDb, effect: Effect) -> Option<PlayerId> {
        let A2Effect::MicroDrain { system, per_second } =
            db.weapon(effect.weapon)?.a2_special?.effect
        else {
            return None;
        };
        let amount = per_second / Fixed::from_int(TICK_HZ as i32);
        if self.specials.stasis(effect.target, self.tick) || amount <= Fixed::ZERO {
            return None;
        }
        let id = effect.target;
        match system {
            7 => self.a2_lose_crew(id, amount),
            8 => {
                let u = self.units.get_mut(id)?;
                let count = if u.shields.directional { 4 } else { 1 };
                for shield in &mut u.shields.now[..count] {
                    *shield = (*shield - amount).max(Fixed::ZERO);
                }
            }
            1 | 9 => {
                if let Some(energy) = self.specials.energy_mut(id) {
                    *energy = (*energy - amount).max(Fixed::ZERO);
                }
            }
            // Type 10 dispatches through Craft::DamageHull.
            0 | 10 if !self.specials.a2_invulnerable(id, self.tick) => {
                let u = self.units.get_mut(id)?;
                let floor = if u.flags.contains(UnitFlags::IMMORTAL) {
                    Fixed::ONE
                } else {
                    Fixed::ZERO
                };
                u.hull[0] = (u.hull[0] - amount).max(floor);
                if u.hull[0] == Fixed::ZERO {
                    let owner = u.owner;
                    return self.despawn(id, Some(effect.source)).then_some(owner);
                }
            }
            _ => {}
        }
        None
    }

    fn a2_regenerate(&mut self, db: &ContentDb, effect: Effect) -> bool {
        let Some(w) = db.weapon(effect.weapon) else {
            return false;
        };
        let Some(A2Effect::Regeneration(rate)) = w.a2_special.map(|s| s.effect) else {
            return false;
        };
        let id = effect.target;
        let Some(u) = self.units.get(id) else {
            return false;
        };
        if u.hull[0] <= Fixed::ZERO || u.flags.contains(UnitFlags::WEAPONS_OFF) {
            return false;
        }
        if self.specials.stasis(id, self.tick) {
            return true;
        }
        let ticks = Fixed::from_int(TICK_HZ as i32);
        let cost = w.energy_cost / ticks;
        if cost > Fixed::ZERO {
            let Some(energy) = self.specials.energy_mut(id) else {
                return false;
            };
            if *energy <= Fixed::ZERO {
                return false;
            }
            *energy = (*energy - cost).max(Fixed::ZERO);
        }
        self.a1_engineering(db, id, Fixed::ZERO);
        let u = self
            .units
            .get_mut(id)
            .expect("engineering repairs a live unit");
        let gain = rate / ticks;
        u.hull[0] = add_capped(u.hull[0], gain, u.hull[1]);
        let count = if u.shields.directional { 4 } else { 1 };
        let pools = &mut u.shields.now[..count];
        for now in pools.iter_mut() {
            *now = add_capped(*now, gain, u.shields.max);
        }
        let full = u.hull[0] == u.hull[1] && pools.iter().all(|&s| s == u.shields.max);
        !full
            && (cost == Fixed::ZERO || self.specials.energy_of(id).is_some_and(|e| e > Fixed::ZERO))
    }

    fn a2_self_destruct(&mut self, db: &ContentDb, effect: Effect) -> Option<PlayerId> {
        let w = db.weapon(effect.weapon)?;
        let A2Effect::SelfDestruct(radius) = w.a2_special?.effect else {
            return None;
        };
        let caster = self.units.get(effect.target)?;
        if caster.flags.contains(UnitFlags::IMMORTAL) {
            return None;
        }
        let owner = caster.owner;
        if let Some(blast) = db.unit(caster.def).and_then(|d| d.self_destruct_blast) {
            let radius = radius * blast.radius_factor;
            if radius > Fixed::ZERO {
                self.specials.a2.blasts.push(Blast {
                    source: caster.id,
                    def: caster.def,
                    weapon: w.key,
                    at: caster.pos,
                    radius,
                    reached: Fixed::ZERO,
                    hit: Vec::new(),
                });
            }
        }
        self.despawn(effect.target, Some(effect.target))
            .then_some(owner)
    }

    fn step_a2_blasts(&mut self, db: &ContentDb) {
        for mut blast in std::mem::take(&mut self.specials.a2.blasts) {
            let Some(rule) = db.unit(blast.def).and_then(|d| d.self_destruct_blast) else {
                continue;
            };
            let step = (rule.rate / Fixed::from_int(TICK_HZ as i32)).max(Fixed::from_bits(1));
            blast.reached = add_capped(blast.reached, step, blast.radius);
            for target in self.units.iter().filter(|u| {
                matches!(u.kind, EntityKind::Craft | EntityKind::Station)
                    && u.hull[0] > Fixed::ZERO
                    && a1::in_range(blast.at, u.pos, blast.reached)
            }) {
                let Err(index) = blast.hit.binary_search(&target.id) else {
                    continue;
                };
                blast.hit.insert(index, target.id);
                self.events.push(SimEvent::Hit {
                    target: target.id,
                    by: blast.source,
                    weapon: blast.weapon,
                    facing: facing(target.heading, target.pos, blast.at),
                    shield: Fixed::ZERO,
                    hull: rule.damage,
                    at: target.pos,
                });
            }
            if blast.reached < blast.radius {
                self.specials.a2.blasts.push(blast);
            }
        }
    }
}

fn add_capped(value: Fixed, amount: Fixed, max: Fixed) -> Fixed {
    Fixed::from_bits(
        value
            .to_bits()
            .saturating_add(amount.to_bits())
            .clamp(0, max.to_bits().max(0)),
    )
}
