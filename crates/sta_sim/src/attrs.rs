//! Mission-set per-unit and per-player attributes that the world otherwise
//! derives from the definition (SIM-54): a unit's race and a player's race.
//! A unit's display name is the SIM-58 `switches` row (`Edit::Name`); the view
//! shows it in [`AttrsView::names`]. A unit or player with none set has no row.
//!
//! Rules (read from the executables, static; nothing measured in game):
//! every edit is per unit, never per class; `SetRace`
//! changes the unit's race only (A2) and `Player.race` the seat's race (Roots);
//! an unknown race name is refused here (the original stores a null race).
//! Max hull, hull value, max crew and max shields need no state of their own:
//! the unit's `hull`, `crew` and `shields` pairs already carry the maximum.
//! In the digest and the snapshot once any row exists.

use sta_core::{EntityId, Fixed, PlayerId};

use crate::world::Fnv;
use crate::world::Table;

/// Longest race name accepted on the wire and in a snapshot (bytes).
pub const MAX_RACE_BYTES: usize = 64;

#[derive(Debug, Clone, Default)]
pub struct Attrs {
    races: Table<String>,
    player_races: Vec<(PlayerId, String)>,
}

/// What the view shows of [`Attrs`].
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct AttrsView {
    /// Script-set unit names (the `switches` rows), ascending id.
    pub names: Vec<(EntityId, String)>,
    /// Script-set unit races (the definition's race otherwise), ascending id.
    pub races: Vec<(EntityId, String)>,
    /// Script-set seat races (the setup's race otherwise), ascending player.
    pub player_races: Vec<(PlayerId, String)>,
    /// What the sim derives for each seat from its race and stations, ascending
    /// player (SIM-54).
    #[serde(default)]
    pub seats: Vec<SeatView>,
    /// Craft whose impulse speed differs from their definition's (LUA-27 scripted
    /// override), ascending id; absence means the definition's value.
    #[serde(default)]
    pub impulse: Vec<(EntityId, Fixed)>,
    /// Craft whose warp speed differs from their definition's (LUA-31 scripted
    /// override), ascending id; absence means the definition's value.
    #[serde(default)]
    pub warp: Vec<(EntityId, Fixed)>,
    /// Units not at red (LUA-32), ascending id; absence means red.
    #[serde(default)]
    pub alerts: Vec<(EntityId, crate::command::AlertState)>,
    /// The tick of the last enemy hit on each unit (LUA-32), ascending id.
    #[serde(default)]
    pub enemy_hit: Vec<(EntityId, u64)>,
    /// `(team, by, ship attack orders, station attack orders)` of the ally attack counters (LUA-32), non-zero rows ascending.
    #[serde(default)]
    pub ally_attacks: Vec<(u8, u8, u32, u32)>,
    /// Craft whose special-forces flag is set (`Edit::SpecialForces`, LUA-28), ascending id.
    /// Derived from the switch rows each tick; not saved state.
    #[serde(default)]
    pub special_forces: Vec<EntityId>,
    /// The mission-wide switches (LUA-41); the default when none was sent.
    #[serde(default)]
    pub mission_switches: crate::mission_switches::MissionSwitches,
    /// The match option "warp allowed" is off for this match (#1102, #1402): derived from the sim's own flag
    /// every tick, so a client can keep its warp offer honest. False outside the profile the option counts for
    /// and when the match has warp allowed. No state of its own.
    #[serde(default)]
    pub match_warp_off: bool,
    /// The move mode each live unit's last move order named (SIM-76, ADR 0011 G4),
    /// ascending id; absence means the order took the content's default
    /// ([`crate::Sim::move_mode`] resolves it). Not a rule yet: nothing steers by it.
    #[serde(default)]
    pub move_modes: Vec<(EntityId, crate::command::MoveMode)>,
}

/// One seat's derived team numbers.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SeatView {
    pub player: PlayerId,
    /// The race now in force: the script's, else the setup's.
    pub race: Option<String>,
    /// The crew pool's capacity; `None` when the content has no crew rules.
    pub crew_capacity: Option<Fixed>,
    /// The officer limit before the race multiplier (what Roots `Player.maxOfficers` reads);
    /// the cap is `PlayerView::officers[1]`.
    pub officer_limit: Option<Fixed>,
    /// The officer limit as a setup or a script wrote it (the economy's own base), `None` when nothing was
    /// written. Unlike `officer_limit` this is never derived (Armada I content derives a limit for every
    /// seat), so a host can tell a placed loadout from a bare seat. Derived from state each tick; not saved.
    #[serde(default)]
    pub officer_base: Option<Fixed>,
}

/// The wire form of [`Attrs`] (snapshot suffix, format 22).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AttrsRec {
    pub version: u8,
    pub races: Vec<(EntityId, String)>,
    pub player_races: Vec<(PlayerId, String)>,
}

impl Attrs {
    pub fn is_empty(&self) -> bool {
        self.races.is_empty() && self.player_races.is_empty()
    }

    pub fn race(&self, id: EntityId) -> Option<&str> {
        self.races.get(id).map(String::as_str)
    }

    pub fn player_race(&self, p: PlayerId) -> Option<&str> {
        self.player_races.binary_search_by_key(&p, |r| r.0).ok().map(|i| self.player_races[i].1.as_str())
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
        self.races.remove(id);
    }

    /// The view of the races; `names` is filled by the caller from the switch rows.
    pub fn view(&self) -> AttrsView {
        AttrsView {
            names: Vec::new(),
            races: self.races.iter().map(|(i, s)| (i, s.clone())).collect(),
            player_races: self.player_races.clone(),
            seats: Vec::new(),
            special_forces: Vec::new(),
            impulse: Vec::new(),
            warp: Vec::new(),
            alerts: Vec::new(),
            enemy_hit: Vec::new(),
            ally_attacks: Vec::new(),
            mission_switches: Default::default(),
            match_warp_off: false,
            move_modes: Vec::new(),
        }
    }

    pub(crate) fn to_rec(&self) -> AttrsRec {
        let v = self.view();
        AttrsRec { version: 1, races: v.races, player_races: v.player_races }
    }

    /// Why a record is refused: an unknown version, rows out of order, an empty or over-long text.
    pub(crate) fn from_rec(r: AttrsRec) -> Result<Attrs, &'static str> {
        if r.version != 1 {
            return Err("unknown attribute record version");
        }
        if !r.player_races.windows(2).all(|w| w[0].0 < w[1].0) {
            return Err("attribute rows out of order");
        }
        let texts_ok = r.races.iter().map(|(_, s)| s).chain(r.player_races.iter().map(|(_, s)| s));
        if texts_ok.clone().any(|s| s.is_empty() || s.len() > MAX_RACE_BYTES) {
            return Err("attribute text empty or too long");
        }
        Ok(Attrs {
            races: Table::from_rows(r.races).ok_or("attribute rows out of order")?,
            player_races: r.player_races,
        })
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("attrs-v1");
        h.u64(self.races.len() as u64);
        for (id, s) in self.races.iter() {
            h.u64(id.0 as u64);
            h.str(s);
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
    /// The seats' derived numbers for the view.
    pub(crate) fn seat_views(&self) -> Vec<SeatView> {
        self.players
            .iter()
            .map(|&(player, _)| SeatView {
                player,
                race: self.seat_race(player).map(str::to_owned),
                crew_capacity: self.content.as_deref().and_then(|db| {
                    crate::economy::Economy::derived_crew_capacity(&self.units, db, &self.attrs, player)
                        .map(|c| Fixed::from_bits(c.to_bits().saturating_add(self.economy.crew_capacity_adjust(player).to_bits())))
                }),
                officer_limit: if self.content.as_deref().is_some_and(sta_content::ContentDb::a1_officers) {
                    self.officer_limit(player)
                } else {
                    self.economy.officer_base(player)
                },
                officer_base: self.economy.officer_base(player),
            })
            .collect()
    }

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
            // The sim reads a positive maximum as "this unit can be hit", so zero keeps the
            // least positive maximum (the original accepts 0).
            self.units.get_mut(id).expect("live").hull[1] = max.max(Fixed::from_bits(1)).min(Fixed::from_int(CAP as i32));
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

    /// The content's spelling of a race some definition carries (matched case-insensitively
    /// like the tech tree and `IsRace`); empty names match nothing.
    pub(crate) fn known_race(&self, race: &str) -> Option<String> {
        if race.is_empty() {
            return None;
        }
        self.content.as_deref()?.units().find(|d| d.race.eq_ignore_ascii_case(race)).map(|d| d.race.clone())
    }

    pub(crate) fn edit_unit_race(&mut self, id: EntityId, race: &str) -> bool {
        let Some(race) = self.known_race(race).filter(|_| self.script_unit(id).is_some()) else { return false };
        self.attrs.set_race(id, &race);
        true
    }

    pub(crate) fn edit_player_race(&mut self, p: PlayerId, race: &str) -> bool {
        let Some(race) = self.known_race(race).filter(|_| self.players.iter().any(|(id, _)| *id == p)) else { return false };
        self.attrs.set_player_race(p, &race);
        // The officer cap and the latinum factor follow the seat's race at once.
        self.refresh_seat_rules();
        true
    }
}
