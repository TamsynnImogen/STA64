//! Mission-set per-unit and per-player attributes that the world otherwise
//! derives from the definition (SIM-54): a unit's display name, a unit's race
//! and a player's race. A unit or player with none set has no row.
//!
//! Rules (read from the executables, static; nothing measured in game):
//! every edit is per unit, never per class; `Entity.name` / `SetName` replace
//! the unit's name (case-insensitive class name list not modelled); `SetRace`
//! changes the unit's race only (A2) and `Player.race` the seat's race (Roots);
//! an unknown race name is refused here (the original stores a null race).
//! Max hull, hull value, max crew and max shields need no state of their own:
//! the unit's `hull`, `crew` and `shields` pairs already carry the maximum.
//! In the digest and the snapshot once any row exists.

use sta_core::{EntityId, Fixed, PlayerId};

use crate::world::Fnv;
use crate::world::Table;

/// Names longest accepted (bytes); longer ones are cut at a character boundary.
pub const MAX_NAME_BYTES: usize = 127;

#[derive(Debug, Clone, Default)]
pub struct Attrs {
    names: Table<String>,
    races: Table<String>,
    player_races: Vec<(PlayerId, String)>,
}

/// What the view shows of [`Attrs`].
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct AttrsView {
    /// Script-set unit names, ascending id.
    pub names: Vec<(EntityId, String)>,
    /// Script-set unit races (the definition's race otherwise), ascending id.
    pub races: Vec<(EntityId, String)>,
    /// Script-set seat races (the setup's race otherwise), ascending player.
    pub player_races: Vec<(PlayerId, String)>,
}

/// The wire form of [`Attrs`] (snapshot suffix, format 21).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AttrsRec {
    pub version: u8,
    pub names: Vec<(EntityId, String)>,
    pub races: Vec<(EntityId, String)>,
    pub player_races: Vec<(PlayerId, String)>,
}

impl Attrs {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.races.is_empty() && self.player_races.is_empty()
    }

    pub fn name(&self, id: EntityId) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }

    pub fn race(&self, id: EntityId) -> Option<&str> {
        self.races.get(id).map(String::as_str)
    }

    pub fn player_race(&self, p: PlayerId) -> Option<&str> {
        self.player_races.binary_search_by_key(&p, |r| r.0).ok().map(|i| self.player_races[i].1.as_str())
    }

    pub(crate) fn set_name(&mut self, id: EntityId, name: &str) {
        let mut end = name.len().min(MAX_NAME_BYTES);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        self.names.insert(id, name[..end].to_owned());
    }

    pub(crate) fn set_race(&mut self, id: EntityId, race: &str) {
        self.races.insert(id, race.to_owned());
    }

    pub(crate) fn set_player_race(&mut self, p: PlayerId, race: &str) {
        match self.player_races.binary_search_by_key(&p, |r| r.0) {
            Ok(i) => self.player_races[i].1 = race.to_owned(),
            Err(i) => self.player_races.insert(i, (p, race.to_owned())),
        }
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.names.remove(id);
        self.races.remove(id);
    }

    pub fn view(&self) -> AttrsView {
        AttrsView {
            names: self.names.iter().map(|(i, s)| (i, s.clone())).collect(),
            races: self.races.iter().map(|(i, s)| (i, s.clone())).collect(),
            player_races: self.player_races.clone(),
        }
    }

    pub(crate) fn to_rec(&self) -> AttrsRec {
        let v = self.view();
        AttrsRec { version: 1, names: v.names, races: v.races, player_races: v.player_races }
    }

    /// `None` for rows out of order or empty names/races.
    pub(crate) fn from_rec(r: AttrsRec) -> Option<Attrs> {
        if r.version != 1 {
            return None;
        }
        if !r.player_races.windows(2).all(|w| w[0].0 < w[1].0) {
            return None;
        }
        Some(Attrs {
            names: Table::from_rows(r.names)?,
            races: Table::from_rows(r.races)?,
            player_races: r.player_races,
        })
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("attrs-v1");
        for (tag, rows) in [("n", &self.names), ("r", &self.races)] {
            h.str(tag);
            h.u64(rows.len() as u64);
            for (id, s) in rows.iter() {
                h.u64(id.0 as u64);
                h.str(s);
            }
        }
        h.u64(self.player_races.len() as u64);
        for (p, s) in &self.player_races {
            h.u64(u64::from(p.0));
            h.str(s);
        }
    }
}

const CAP: i64 = 1_000_000_000;

impl crate::Sim {
    fn script_unit(&self, id: EntityId) -> Option<&crate::world::Entity> {
        self.units.get(id).filter(|u| {
            u.hull[0] > Fixed::ZERO && matches!(u.kind, crate::world::EntityKind::Craft | crate::world::EntityKind::Station)
        })
    }

    /// `Entity.maxHull =`: see [`crate::command::Edit::MaxHull`].
    pub(crate) fn edit_max_hull(&mut self, id: EntityId, max: Fixed) -> bool {
        if self.script_unit(id).is_none() {
            return false;
        }
        if max >= Fixed::ZERO {
            self.units.get_mut(id).expect("live").hull[1] = max.min(Fixed::from_int(CAP as i32));
        }
        true
    }

    /// `Entity.maxCrew =`: the maximum, and the current crew down to it.
    pub(crate) fn edit_max_crew(&mut self, id: EntityId, max: Fixed) -> bool {
        if self.script_unit(id).is_none_or(|u| u.crew.is_none()) {
            return false;
        }
        let max = max.max(Fixed::ZERO).min(Fixed::from_int(CAP as i32));
        let u = self.units.get_mut(id).expect("live");
        let [now, _] = u.crew.expect("pool");
        u.crew = Some([now.min(max), max]);
        true
    }

    /// `Entity.hullValue =`: the current hull, unclamped above.
    pub(crate) fn edit_hull_value(&mut self, id: EntityId, to: Fixed) -> bool {
        if self.script_unit(id).is_none() {
            return false;
        }
        let u = self.units.get_mut(id).expect("live");
        let to = to.max(Fixed::from_bits(1)).min(Fixed::from_int(CAP as i32));
        if !(u.flags.contains(crate::view::UnitFlags::IMMORTAL) && to < u.hull[0]) {
            u.hull[0] = to;
        }
        true
    }

    /// `Entity.maxShields =` on Roots: the maximum alone.
    pub(crate) fn edit_max_shields_keep(&mut self, id: EntityId, max: Fixed) -> bool {
        if self.script_unit(id).is_none() {
            return false;
        }
        self.units.get_mut(id).expect("live").shields.max = max.max(Fixed::ZERO).min(Fixed::from_int(CAP as i32));
        true
    }

    pub(crate) fn edit_name(&mut self, id: EntityId, name: &str) -> bool {
        if self.script_unit(id).is_none() {
            return false;
        }
        self.attrs.set_name(id, name);
        true
    }

    fn known_race(&self, race: &str) -> bool {
        self.content.as_deref().is_some_and(|db| db.units().any(|d| d.race == race))
    }

    pub(crate) fn edit_unit_race(&mut self, id: EntityId, race: &str) -> bool {
        if self.script_unit(id).is_none() || !self.known_race(race) {
            return false;
        }
        self.attrs.set_race(id, race);
        true
    }

    pub(crate) fn edit_player_race(&mut self, p: PlayerId, race: &str) -> bool {
        if !self.players.iter().any(|(id, _)| *id == p) || !self.known_race(race) {
            return false;
        }
        self.attrs.set_player_race(p, race);
        true
    }
}
