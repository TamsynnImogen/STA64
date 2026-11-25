//! Armada I's Federation specials. ODF class pairs choose the payload; the same
//! dispatch also covers the stock campaign variants and inherited definitions.

use serde::{Deserialize, Serialize};
use sta_content::{A1SpecialDef, A1SpecialKind as Kind, ContentDb, UnitDef, WeaponDef, WeaponKind};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TICK_HZ, Tick, Vec3};

use super::{Specials, specials_indexed};
use crate::Sim;
use crate::combat::weapons::facing;
use crate::command::Target;
use crate::event::SimEvent;
use crate::relations::Diplomacy;
use crate::view::{Relation, UnitFlags, WorldView};
use crate::world::{Entities, Entity, EntityKind, Fnv, Table};

#[derive(Debug, Clone, Default)]
pub(crate) struct State {
    holds: Table<Hold>,
    ready: Vec<(EntityId, u8, Tick)>,
    ordnance: Vec<Flight>,
    fields: Vec<Field>,
    next_effect: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
struct Hold {
    engines: Tick,
    shields: Tick,
    stasis: Tick,
    clone_until: Tick,
    reflector: Option<(u8, ContentKey)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Flight {
    id: u64,
    caster: EntityId,
    owner: PlayerId,
    weapon: ContentKey,
    at: Vec3,
    target: Option<EntityId>,
    until: Tick,
    visited: Vec<EntityId>,
}

/// Persistent render pose of a stasis field or tracked special ordnance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EffectView {
    pub id: u64,
    pub owner: PlayerId,
    pub weapon: ContentKey,
    pub at: Vec3,
    pub target: Option<EntityId>,
    pub radius: Option<Fixed>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Field {
    effect: EffectView,
    until: Tick,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Record {
    holds: Vec<(EntityId, Hold)>,
    ready: Vec<(EntityId, u8, Tick)>,
    ordnance: Vec<Flight>,
    fields: Vec<Field>,
    next_effect: u64,
}

/// Readiness and persistent effects for UI and rendering.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpecialView {
    pub active: Vec<(EntityId, u8)>,
    pub cooling: Vec<(EntityId, u8)>,
    pub stasis: Vec<EntityId>,
    pub copies: Vec<EntityId>,
    pub effects: Vec<EffectView>,
}

fn special<'a>(
    db: &'a ContentDb,
    def: &'a UnitDef,
    slot: u8,
) -> Option<(usize, &'a WeaponDef, A1SpecialDef)> {
    let (mount, _, w) = specials_indexed(db, def).nth(usize::from(slot))?;
    Some((mount, w, w.a1_special?))
}

pub(super) fn in_range(from: Vec3, to: Vec3, radius: Fixed) -> bool {
    // Widen before squaring. Stock Temporal weapons have a 99,999 unit
    // cast range, and a point received over the wire can be much farther.
    if radius < Fixed::ZERO {
        return false;
    }
    let r = i128::from(radius.to_bits());
    let dx = i128::from(from.x.to_bits()) - i128::from(to.x.to_bits());
    let dz = i128::from(from.z.to_bits()) - i128::from(to.z.to_bits());
    dx.abs() <= r && dz.abs() <= r && dx * dx <= r * r - dz * dz
}

fn target_allowed(w: &WeaponDef, kind: Kind, craft: bool, relation: Relation) -> bool {
    match (kind, relation, craft) {
        (Kind::Gemini, Relation::Ally, true) => w.base.targets.allied_craft.unwrap_or(true),
        (Kind::EngineeringTeam, Relation::Ally, true) => {
            w.base.targets.allied_craft.unwrap_or(true)
        }
        (Kind::EngineeringTeam, Relation::Ally, false) => {
            w.base.targets.allied_buildings.unwrap_or(true)
        }
        _ => false,
    }
}

fn eligible(u: &Entity, db: &ContentDb) -> bool {
    matches!(u.kind, EntityKind::Craft | EntityKind::Station)
        && u.hull[0] > Fixed::ZERO
        && !u.is_unfinished()
        && !db.unit(u.def).is_some_and(|d| d.invalid_as_target)
}

pub(crate) fn pd_enabled(db: &ContentDb, w: &WeaponDef) -> bool {
    let f = db.weapon_features(w);
    w.a1_special.is_some_and(|s| s.kind == Kind::PointDefense)
        || f.pd_energy && f.point_defense_cycles
}

fn active_enough(w: &WeaponDef, data: A1SpecialDef, energy: Fixed) -> bool {
    match data.kind {
        Kind::Reflector | Kind::PointDefense => {
            energy >= w.energy_cost / Fixed::from_int(TICK_HZ as i32)
        }
        _ => energy >= w.energy_cost,
    }
}

fn researched(db: &ContentDb, w: &WeaponDef, state: &dyn sta_content::TechState) -> bool {
    // No Specials and Science Ships Only omit unavailable weapons from
    // their trees. Ships can carry the unlisted and listed versions together.
    db.tech_tree().is_none_or(|t| {
        t.availability(&w.odf, state).is_available()
            || w.campaign_transwarp() && t.entry(&w.odf).is_none()
    })
}

pub(crate) fn researched_in_world(
    db: &ContentDb,
    units: &Entities,
    dip: Diplomacy<'_>,
    u: &Entity,
    w: &WeaponDef,
) -> bool {
    let race = db.unit(u.def).map_or("", |d| d.race.as_str());
    researched(
        db,
        w,
        &crate::tech::Owned::new(db, units, dip.players, u.owner, race),
    )
}

struct ViewTech<'a> {
    db: &'a ContentDb,
    view: &'a WorldView,
    player: PlayerId,
    race: &'a str,
}

impl sta_content::TechState for ViewTech<'_> {
    fn count(&self, odf: &str) -> u32 {
        let Some(key) = self.db.key(odf) else {
            return 0;
        };
        self.view
            .units
            .iter()
            .filter(|u| {
                u.owner == self.player
                    && u.progress.is_none_or(|p| p >= Fixed::ONE)
                    && self.db.research_satisfies(u.def, key)
            })
            .count() as u32
    }

    fn race(&self) -> &str {
        self.race
    }

    fn has_ally(&self, race: &str) -> bool {
        let own = self.view.player(self.player).map(|p| p.team);
        self.view.units.iter().any(|u| {
            u.owner != self.player
                && self
                    .view
                    .player(u.owner)
                    .is_some_and(|p| Some(p.team) == own)
                && u.progress.is_none_or(|p| p >= Fixed::ONE)
                && self
                    .db
                    .unit(u.def)
                    .is_some_and(|d| d.race.eq_ignore_ascii_case(race))
        })
    }
}

pub fn a1_special_researched(view: &WorldView, db: &ContentDb, unit: EntityId, slot: u8) -> bool {
    let Some(u) = view.unit(unit) else {
        return false;
    };
    let Some(def) = db.unit(u.def) else {
        return false;
    };
    let Some((_, w, _)) = special(db, def, slot) else {
        return false;
    };
    researched_in_view(view, db, unit, w)
}

pub(super) fn researched_in_view(view: &WorldView, db: &ContentDb, unit: EntityId, w: &WeaponDef) -> bool {
    let Some(u) = view.unit(unit) else { return false };
    let Some(def) = db.unit(u.def) else { return false };
    researched(db, w, &ViewTech { db, view, player: u.owner, race: &def.race })
}

pub fn a1_special_active(view: &WorldView, unit: EntityId, slot: u8) -> bool {
    view.point_defence_on(unit, slot)
        || view.a1_specials.active.binary_search(&(unit, slot)).is_ok()
}

pub fn a1_special_ready(view: &WorldView, db: &ContentDb, unit: EntityId, slot: u8) -> bool {
    let Some(u) = view.unit(unit) else {
        return false;
    };
    let Some(def) = db.unit(u.def) else {
        return false;
    };
    let Some((_, w, data)) = special(db, def, slot) else {
        return false;
    };
    a1_special_active(view, unit, slot)
        || data.kind != Kind::DetectCloak
            && u.hull[0] > Fixed::ZERO
            && u.progress.is_none_or(|p| p >= Fixed::ONE)
            && !u.flags.contains(UnitFlags::WEAPONS_OFF)
            && view.a1_specials.stasis.binary_search(&unit).is_err()
            && view.a1_specials.copies.binary_search(&unit).is_err()
            && view
                .a1_specials
                .cooling
                .binary_search(&(unit, slot))
                .is_err()
            && a1_special_researched(view, db, unit, slot)
            && active_enough(w, data, u.energy.map_or(def.special_energy_max, |e| e[0]))
}

pub fn a1_special_target_ok(
    view: &WorldView,
    db: &ContentDb,
    unit: EntityId,
    slot: u8,
    target: Target,
) -> bool {
    let Some(u) = view.unit(unit) else {
        return false;
    };
    let Some((_, w, data)) = db.unit(u.def).and_then(|d| special(db, d, slot)) else {
        return false;
    };
    match (data.kind, target) {
        (Kind::Stasis, Target::Point(at)) => in_range(u.pos, at, w.range),
        (Kind::Gemini | Kind::EngineeringTeam, Target::Entity(id)) => {
            view.unit(id).is_some_and(|t| {
                let relation = if u.owner == t.owner {
                    Relation::Ally
                } else {
                    view.player(u.owner)
                        .and_then(|p| p.relations.iter().find(|r| r.0 == t.owner))
                        .map_or(Relation::Neutral, |r| r.1)
                };
                t.hull[0] > Fixed::ZERO
                    && t.progress.is_none_or(|p| p >= Fixed::ONE)
                    && db.unit(t.def).is_some_and(|d| {
                        let kind = EntityKind::of(&d.class, &d.movement);
                        matches!(kind, EntityKind::Craft | EntityKind::Station)
                            && !d.invalid_as_target
                            && target_allowed(w, data.kind, kind == EntityKind::Craft, relation)
                    })
                    && in_range(u.pos, t.pos, w.range)
                    && (data.kind != Kind::Gemini
                        || view.a1_specials.copies.binary_search(&id).is_err())
            })
        }
        (
            Kind::Mines
            | Kind::ChainPulsar
            | Kind::EngineOverload
            | Kind::ShieldDisruptor
            | Kind::Reflector
            | Kind::PointDefense,
            Target::None,
        ) => true,
        _ => false,
    }
}

impl State {
    pub(super) fn is_empty(&self) -> bool {
        self.holds.is_empty()
            && self.ready.is_empty()
            && self.ordnance.is_empty()
            && self.fields.is_empty()
            && self.next_effect == 0
    }

    pub(super) fn to_rec(&self) -> Record {
        Record {
            holds: self.holds.iter().map(|(id, hold)| (id, *hold)).collect(),
            ready: self.ready.clone(),
            ordnance: self.ordnance.clone(),
            fields: self.fields.clone(),
            next_effect: self.next_effect,
        }
    }

    pub(super) fn from_rec(r: Record) -> Option<Self> {
        if r.ready
            .windows(2)
            .any(|p| (p[0].0, p[0].1) >= (p[1].0, p[1].1))
        {
            return None;
        }
        Some(Self {
            holds: Table::from_rows(r.holds)?,
            ready: r.ready,
            ordnance: r.ordnance,
            fields: r.fields,
            next_effect: r.next_effect,
        })
    }

    pub(super) fn hash_into(&self, h: &mut Fnv) {
        if self.is_empty() {
            return;
        }
        h.str("a1-specials-v1");
        let bytes = postcard::to_allocvec(&self.to_rec()).expect("A1 special state serializes");
        h.u64(bytes.len() as u64);
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            h.u64(u64::from_le_bytes(word));
        }
    }

    pub(crate) fn valid(&self, now: Tick, units: &Entities, db: Option<&ContentDb>) -> bool {
        let active_weapon = |id, slot| {
            let db = db?;
            let def = db.unit(units.get(id)?.def)?;
            special(db, def, slot)
        };
        let timer = |until| until == 0 || until >= now;
        let mut ids: Vec<_> = self
            .ordnance
            .iter()
            .map(|f| f.id)
            .chain(self.fields.iter().map(|f| f.effect.id))
            .collect();
        ids.sort_unstable();
        self.next_effect < 1_u64 << 63
            && ids.iter().all(|&id| id != 0 && id <= self.next_effect)
            && ids.windows(2).all(|pair| pair[0] < pair[1])
            && self.holds.iter().all(|(id, h)| {
                units.contains(id)
                    && *h != Hold::default()
                    && timer(h.engines)
                    && timer(h.shields)
                    && timer(h.stasis)
                    && timer(h.clone_until)
                    && h.reflector.is_none_or(|(slot, key)| {
                        active_weapon(id, slot)
                            .is_some_and(|(_, w, d)| w.key == key && d.kind == Kind::Reflector)
                    })
            })
            && self.ready.iter().all(|&(id, slot, until)| {
                timer(until) && until != 0 && active_weapon(id, slot).is_some()
            })
            && self.ordnance.iter().all(|f| {
                timer(f.until)
                    && f.until != 0
                    && db
                        .and_then(|db| db.weapon(f.weapon))
                        .and_then(|w| w.a1_special)
                        .is_some_and(|d| {
                            matches!(
                                d.kind,
                                Kind::Mines
                                    | Kind::ChainPulsar
                                    | Kind::ShieldDisruptor
                                    | Kind::Gemini
                            )
                        })
            })
            && self.fields.iter().all(|f| {
                timer(f.until)
                    && f.until != 0
                    && db
                        .and_then(|db| db.weapon(f.effect.weapon))
                        .and_then(|w| w.a1_special)
                        .is_some_and(|d| {
                            d.kind == Kind::Stasis && f.effect.radius == Some(d.radius)
                        })
            })
    }

    fn allocate_effect(&mut self) -> u64 {
        self.next_effect = self.next_effect.saturating_add(1);
        self.next_effect
    }

    fn launch(&mut self, mut flight: Flight) {
        flight.id = self.allocate_effect();
        self.ordnance.push(flight);
    }

    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.holds.remove(id);
        self.ready.retain(|r| r.0 != id);
    }

    pub(crate) fn disabled_units(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.holds.iter().map(|(id, _)| id)
    }

    fn edit_hold(&mut self, id: EntityId, edit: impl FnOnce(&mut Hold)) {
        let mut hold = self.holds.get(id).copied().unwrap_or_default();
        edit(&mut hold);
        if hold == Hold::default() {
            self.holds.remove(id);
        } else {
            self.holds.insert(id, hold);
        }
    }

    fn cooling(&self, id: EntityId, slot: u8, now: Tick) -> bool {
        self.ready
            .binary_search_by_key(&(id, slot), |r| (r.0, r.1))
            .is_ok_and(|i| self.ready[i].2 > now)
    }

    fn cooldown(&mut self, id: EntityId, slot: u8, until: Tick) {
        match self.ready.binary_search_by_key(&(id, slot), |r| (r.0, r.1)) {
            Ok(i) => self.ready[i].2 = until,
            Err(i) => self.ready.insert(i, (id, slot, until)),
        }
    }

    pub(crate) fn view(&self, now: Tick, units: &Entities, db: Option<&ContentDb>) -> SpecialView {
        SpecialView {
            active: self
                .holds
                .iter()
                .filter_map(|(id, h)| h.reflector.map(|(slot, _)| (id, slot)))
                .collect(),
            cooling: self
                .ready
                .iter()
                .filter(|r| r.2 > now)
                .map(|r| (r.0, r.1))
                .collect(),
            stasis: self
                .holds
                .iter()
                .filter(|(_, h)| h.stasis > now)
                .map(|(id, _)| id)
                .collect(),
            copies: self
                .holds
                .iter()
                .filter(|(_, h)| h.clone_until != 0)
                .map(|(id, _)| id)
                .collect(),
            effects: self
                .fields
                .iter()
                .filter(|f| f.until > now)
                .map(|f| f.effect)
                .chain(
                    self.ordnance
                        .iter()
                        .filter(|f| f.until > now)
                        .map(|f| EffectView {
                            id: f.id,
                            owner: f.owner,
                            weapon: f.weapon,
                            at: f.at,
                            target: f.target,
                            radius: None,
                        }),
                )
                .chain(self.holds.iter().filter_map(|(id, hold)| {
                    let (_, weapon) = hold.reflector?;
                    let u = units.get(id)?;
                    let radius = db?.unit(u.def)?.radius;
                    Some(EffectView {
                        id: (1_u64 << 63) | u64::from(id.0),
                        owner: u.owner,
                        weapon,
                        at: u.pos,
                        target: Some(id),
                        radius: Some(radius),
                    })
                }))
                .collect(),
        }
    }
}

impl Specials {
    pub(crate) fn stasis(&self, id: EntityId, now: Tick) -> bool {
        self.a1.holds.get(id).is_some_and(|h| h.stasis > now)
    }

    pub(crate) fn engines_held(&self, id: EntityId, now: Tick) -> bool {
        self.a1
            .holds
            .get(id)
            .is_some_and(|h| h.engines > now || h.stasis > now)
            || self.a2_held(id, crate::command::Subsystem::Engines, now)
    }

    pub(crate) fn shields_held(&self, id: EntityId, now: Tick) -> bool {
        self.a1.holds.get(id).is_some_and(|h| h.shields > now)
            || self.a2_held(id, crate::command::Subsystem::Shields, now)
    }

    pub(crate) fn is_gemini(&self, id: EntityId) -> bool {
        self.a1.holds.get(id).is_some_and(|h| h.clone_until != 0)
    }

    pub(crate) fn a1_detector(
        db: &ContentDb,
        units: &Entities,
        dip: Diplomacy<'_>,
        u: &Entity,
    ) -> bool {
        u.hull[0] > Fixed::ZERO
            && !u.is_unfinished()
            && db.unit(u.def).is_some_and(|def| {
                def.hardpoints.iter().any(|hp| {
                    db.weapon(hp.weapon.key).is_some_and(|w| {
                        w.a1_special
                            .is_some_and(|s| s.kind == Kind::DetectCloak && s.detect_class == 3)
                            && researched_in_world(db, units, dip, u, w)
                    })
                })
            })
    }
}

impl Sim {
    pub(crate) fn order_a1_special(
        &mut self,
        db: &ContentDb,
        player: PlayerId,
        id: EntityId,
        slot: u8,
        target: Target,
    ) -> Option<bool> {
        let u = self.units.get(id)?;
        let def = db.unit(u.def)?;
        let (mount, w, data) = special(db, def, slot)?;
        if (player.is_player() && u.owner != player)
            || u.hull[0] <= Fixed::ZERO
            || u.is_unfinished()
        {
            return Some(false);
        }
        let caster = u.clone();
        if data.kind == Kind::Reflector
            && self
                .specials
                .a1
                .holds
                .get(id)
                .is_some_and(|h| h.reflector.is_some())
        {
            self.specials.a1.edit_hold(id, |h| h.reflector = None);
            return Some(true);
        }
        let pd_on = data.kind == Kind::PointDefense && self.weapons.pd_is_on(id, mount);
        if !pd_on
            && (caster.flags.contains(UnitFlags::WEAPONS_OFF)
                || self.specials.stasis(id, self.tick)
                || self.specials.a2_held(id, crate::command::Subsystem::Weapons, self.tick)
                || self.specials.is_gemini(id)
                || self.specials.a1.cooling(id, slot, self.tick)
                || !researched_in_world(
                    db,
                    &self.units,
                    Diplomacy::new(&self.players, &self.relations),
                    &caster,
                    w,
                ))
        {
            return Some(false);
        }
        let now = self
            .specials
            .energy_of(id)
            .unwrap_or(def.special_energy_max);
        if !pd_on && !active_enough(w, data, now) {
            return Some(false);
        }
        if data.kind == Kind::PointDefense {
            return Some(
                target == Target::None
                    && self.weapons.order_special(
                        self.tick,
                        db,
                        &self.units,
                        &mut self.specials,
                        &mut self.rng,
                        &mut self.events,
                        player,
                        &[id],
                        slot,
                    ),
            );
        }
        if !self.cast_a1(db, &caster, w, data, slot, target) {
            return Some(false);
        }
        self.specials.ensure_pool(id, def);
        if data.kind != Kind::Reflector
            && let Some(energy) = self.specials.energy_mut(id)
        {
            *energy = *energy - w.energy_cost;
        }
        if w.refire > 0 {
            self.specials
                .a1
                .cooldown(id, slot, self.tick.saturating_add(w.refire));
        }
        self.events.push(SimEvent::Fired {
            shooter: id,
            weapon: w.key,
            hardpoint: mount as u8,
            target,
        });
        Some(true)
    }

    fn cast_a1(
        &mut self,
        db: &ContentDb,
        u: &Entity,
        w: &WeaponDef,
        data: A1SpecialDef,
        slot: u8,
        target: Target,
    ) -> bool {
        let until = self
            .tick
            .saturating_add(u64::from(data.duration_ticks.max(1)));
        let launch_until = self
            .tick
            .saturating_add(u64::from(data.flight_ticks.max(1)));
        let new_flight = |target, until| Flight {
            id: 0,
            caster: u.id,
            owner: u.owner,
            weapon: w.key,
            at: u.pos,
            target,
            until,
            visited: Vec::new(),
        };
        match (data.kind, target) {
            (Kind::Reflector, Target::None) => self
                .specials
                .a1
                .edit_hold(u.id, |h| h.reflector = Some((slot, w.key))),
            (Kind::Mines, Target::None) => self.specials.a1.launch(new_flight(None, launch_until)),
            (Kind::ChainPulsar, Target::None) => {
                let first = self
                    .a1_area_targets(db, u.owner, u.id, u.pos, w.range, 2)
                    .first()
                    .copied();
                self.specials.a1.launch(new_flight(first, launch_until));
            }
            (Kind::EngineOverload | Kind::ShieldDisruptor, Target::None) => {
                let targets =
                    self.a1_area_targets(db, u.owner, u.id, u.pos, w.range, data.hit_condition);
                for target in targets.into_iter().take(data.limit as usize) {
                    if data.kind == Kind::EngineOverload {
                        self.specials
                            .a1
                            .edit_hold(target, |h| h.engines = h.engines.max(until));
                        self.a1_hit(w.key, target, u.id, u.pos, Fixed::ZERO);
                    } else {
                        self.specials
                            .a1
                            .launch(new_flight(Some(target), launch_until));
                    }
                }
            }
            (Kind::Gemini | Kind::EngineeringTeam, Target::Entity(id)) => {
                let dip = Diplomacy::new(&self.players, &self.relations);
                if !self.units.get(id).is_some_and(|t| {
                    eligible(t, db)
                        && target_allowed(
                            w,
                            data.kind,
                            t.kind == EntityKind::Craft,
                            dip.view(u.owner, t.owner),
                        )
                        && in_range(u.pos, t.pos, w.range)
                        && (data.kind != Kind::Gemini || !self.specials.is_gemini(id))
                }) {
                    return false;
                }
                if data.kind == Kind::EngineeringTeam {
                    self.a1_engineering(db, id, data.shields_returned);
                } else {
                    self.specials.a1.launch(new_flight(Some(id), launch_until));
                }
            }
            (Kind::Stasis, Target::Point(at))
                if in_range(u.pos, at, w.range)
                    && self.bounds.is_none_or(|(min, max)| {
                        at.x >= min.x && at.x <= max.x && at.z >= min.z && at.z <= max.z
                    }) =>
            {
                let ids = self.a1_area_targets(db, u.owner, u.id, at, data.radius, 2);
                for id in ids {
                    self.specials
                        .a1
                        .edit_hold(id, |h| h.stasis = h.stasis.max(until));
                }
                let id = self.specials.a1.allocate_effect();
                self.specials.a1.fields.push(Field {
                    effect: EffectView {
                        id,
                        owner: u.owner,
                        weapon: w.key,
                        at,
                        target: None,
                        radius: Some(data.radius),
                    },
                    until,
                });
            }
            _ => return false,
        }
        true
    }

    fn a1_area_targets(
        &self,
        db: &ContentDb,
        owner: PlayerId,
        caster: EntityId,
        at: Vec3,
        radius: Fixed,
        condition: u32,
    ) -> Vec<EntityId> {
        let dip = Diplomacy::new(&self.players, &self.relations);
        let mut targets: Vec<_> = self
            .units
            .iter()
            .filter(|t| {
                t.id != caster
                    && eligible(t, db)
                    && !t.flags.contains(UnitFlags::CLOAKED)
                    && match condition {
                        1 => dip.view(owner, t.owner) == Relation::Ally,
                        2 => dip.is_enemy(owner, t.owner),
                        _ => true,
                    }
                    && in_range(at, t.pos, radius)
            })
            .map(|t| (crate::combat::weapons::flat(at, t.pos).1, t.id))
            .collect();
        targets.sort_unstable();
        targets.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn step_a1_timers(&mut self, db: &ContentDb) {
        if self.specials.a1.is_empty() {
            return;
        }
        let tick = self.tick;
        self.specials
            .a1
            .ready
            .retain(|r| r.2 > tick && self.units.contains(r.0));
        self.specials.a1.fields.retain(|f| f.until > tick);
        let ids: Vec<_> = self.specials.a1.holds.iter().map(|(id, _)| id).collect();
        for id in ids {
            let Some(u) = self.units.get(id) else {
                self.specials.a1.holds.remove(id);
                continue;
            };
            let hold = *self.specials.a1.holds.get(id).expect("listed hold");
            if hold.clone_until != 0 && hold.clone_until <= tick {
                self.specials.a1.holds.remove(id);
                self.retire(id);
                continue;
            }
            let mut reflector = hold.reflector;
            if let Some((_, key)) = reflector {
                let drain = db.weapon(key).map_or(Fixed::ZERO, |w| {
                    w.energy_cost / Fixed::from_int(TICK_HZ as i32)
                });
                let energy = self.specials.energy_of(id).unwrap_or(Fixed::ZERO);
                if energy < drain || u.flags.contains(UnitFlags::WEAPONS_OFF) {
                    reflector = None;
                } else if hold.stasis <= tick
                    && let Some(e) = self.specials.energy_mut(id)
                {
                    *e = *e - drain;
                }
            }
            self.specials.a1.edit_hold(id, |h| {
                if h.engines <= tick {
                    h.engines = 0;
                }
                if h.shields <= tick {
                    h.shields = 0;
                }
                if h.stasis <= tick {
                    h.stasis = 0;
                }
                h.reflector = reflector;
            });
        }
    }

    pub(crate) fn step_a1_ordnance(&mut self, db: &ContentDb) {
        if self.specials.a1.ordnance.is_empty() {
            return;
        }
        for mut flight in std::mem::take(&mut self.specials.a1.ordnance) {
            if flight.until <= self.tick {
                continue;
            }
            let Some(w) = db.weapon(flight.weapon) else {
                continue;
            };
            let Some(data) = w.a1_special else { continue };
            if self.step_a1_flight(db, &mut flight, w, data) {
                self.specials.a1.ordnance.push(flight);
            }
        }
    }

    fn step_a1_flight(
        &mut self,
        db: &ContentDb,
        f: &mut Flight,
        w: &WeaponDef,
        data: A1SpecialDef,
    ) -> bool {
        if f.target.is_none() && matches!(data.kind, Kind::Mines | Kind::ChainPulsar) {
            let radius = if data.kind == Kind::Mines || !f.visited.is_empty() {
                data.radius
            } else {
                w.range
            };
            f.target = self
                .a1_area_targets(db, f.owner, f.caster, f.at, radius, 2)
                .into_iter()
                .find(|id| !f.visited.contains(id));
        }
        let Some(id) = f.target else {
            return f.visited.is_empty();
        };
        let Some(target) = self.units.get(id).filter(|t| eligible(t, db)) else {
            f.target = None;
            return matches!(data.kind, Kind::Mines | Kind::ChainPulsar);
        };
        let (dir, distance) = crate::combat::weapons::flat(f.at, target.pos);
        let speed = w.shot_speed / Fixed::from_int(TICK_HZ as i32);
        if speed > Fixed::ZERO && distance > speed {
            f.at.x = f.at.x + dir.0 * speed;
            f.at.z = f.at.z + dir.1 * speed;
            return true;
        }
        let at = target.pos;
        let until = self
            .tick
            .saturating_add(u64::from(data.duration_ticks.max(1)));
        match data.kind {
            Kind::Mines | Kind::ChainPulsar => {
                // The original adds n times the modifier after hit n.
                let n = f.visited.len() as i128;
                let steps = n * (n + 1) / 2;
                let bits = i128::from(w.damage.to_bits())
                    .saturating_add(i128::from(data.bonus_damage.to_bits()).saturating_mul(steps));
                let damage = Fixed::from_bits(bits.clamp(0, i128::from(i64::MAX)) as i64);
                self.a1_hit(w.key, id, f.caster, f.at, damage);
                if data.kind == Kind::ChainPulsar && f.visited.len() + 1 < data.limit as usize {
                    f.visited.push(id);
                    f.at = at;
                    f.target = None;
                    return true;
                }
            }
            Kind::ShieldDisruptor => {
                self.specials
                    .a1
                    .edit_hold(id, |h| h.shields = h.shields.max(until));
                self.a1_hit(w.key, id, f.caster, f.at, Fixed::ZERO);
            }
            Kind::Gemini => {
                let dip = Diplomacy::new(&self.players, &self.relations);
                if dip.view(f.owner, target.owner) == Relation::Ally
                    && !self.specials.is_gemini(id)
                    && let Some(copy) = self.a1_gemini(db, f.caster, id)
                {
                    self.specials.a1.edit_hold(copy, |h| h.clone_until = until);
                    self.a1_hit(w.key, id, f.caster, f.at, Fixed::ZERO);
                }
            }
            _ => {}
        }
        false
    }

    fn a1_hit(
        &mut self,
        weapon: ContentKey,
        id: EntityId,
        caster: EntityId,
        from: Vec3,
        damage: Fixed,
    ) {
        if let Some(t) = self.units.get(id) {
            self.events.push(SimEvent::Hit {
                target: id,
                by: caster,
                weapon,
                facing: facing(t.heading, t.pos, from),
                shield: Fixed::ZERO,
                hull: damage,
                at: t.pos,
            });
        }
    }

    pub(crate) fn reflect_special_hits(&mut self, db: &ContentDb, from: usize) {
        if self.specials.a1.holds.is_empty() {
            return;
        }
        let dip = Diplomacy::new(&self.players, &self.relations);
        for event in &mut self.events[from..] {
            let SimEvent::Hit {
                target,
                by,
                weapon,
                facing: face,
                hull,
                at,
                ..
            } = event
            else {
                continue;
            };
            let Some((_, key)) = self
                .specials
                .a1
                .holds
                .get(*target)
                .and_then(|h| h.reflector)
            else {
                continue;
            };
            if *hull <= Fixed::ZERO || self.specials.stasis(*target, self.tick) {
                continue;
            }
            let Some(rule) = db.weapon(key).and_then(|w| w.a1_special) else {
                continue;
            };
            let Some(incoming) = db.weapon(*weapon) else {
                continue;
            };
            let reflects = match incoming.kind {
                WeaponKind::Projectile => rule.reflect_missile,
                WeaponKind::Beam => rule.reflect_beam,
                _ => false,
            };
            if !reflects {
                continue;
            }
            let Some(source) = self.units.get(*by) else {
                continue;
            };
            let Some(defender) = self.units.get(*target) else {
                continue;
            };
            if !dip.is_enemy(defender.owner, source.owner) {
                continue;
            }
            if let Some((reflect, return_to_source)) = rule.reflect_chance {
                let roll = |rng: &mut crate::Rng| Fixed::from_bits((rng.next_u64() >> 32) as i64);
                if roll(&mut self.rng) >= reflect {
                    continue;
                }
                let return_it = roll(&mut self.rng) < return_to_source;
                // Missiles can be thrown off course; beams are aimed back at
                // the source after a successful reflection.
                if incoming.kind == WeaponKind::Projectile && !return_it {
                    *weapon = key;
                    *hull = Fixed::ZERO;
                    continue;
                }
            }
            (*target, *by) = (*by, *target);
            *face = facing(source.heading, source.pos, defender.pos);
            *at = source.pos;
        }
    }
}
