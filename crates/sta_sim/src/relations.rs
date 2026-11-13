//! Mission-set relations between seats (SIM-63).
//!
//! Rule (read from the executables, static; Armada I and II): a relation is
//! one of Enemy / Neutral / Ally and is **directional**: `view(a, b)` is seat
//! a's own view of seat b. A mission's `SetRelation(a, b, r)` writes only that
//! one direction. Two readings exist:
//!
//! - *unconfirmed* (`view(a, b)`): a's view only. Auto-acquire, auto-fire,
//!   assist and the world view's per-player relation list use it, with the
//!   shooter's view of the target's perceived owner.
//! - *confirmed* (`confirmed(a, b)`): the worse of both views (Enemy < Neutral
//!   < Ally). Shared vision, point-defence cover and victory use it: only a
//!   mutual ally shares sight.
//!
//! Defaults, unchanged from before this module: seats of one team are Ally,
//! seats of different teams Enemy, a seat is Ally to itself, any pair with an
//! owner that is not a seat (the neutral owner) is Neutral. Only pairs a
//! mission changed are stored (an override equal to the default is removed),
//! so a game no mission touches has an empty table, an unchanged digest and
//! an unchanged snapshot body apart from the empty record.
//!
//! Not modelled: explicit attack orders are never refused for relation (as in
//! the original); a commanded attack is dropped when the shooter's view of the
//! target changes after the order (see `Weapons::relation_changed`).

use sta_core::{PlayerId, TeamId};

use crate::view::Relation;
use crate::world::Fnv;

/// Wire / snapshot code of a relation: 0 Enemy, 1 Neutral, 2 Ally (the
/// executable's numbering, which `SetRelation` passes unchanged).
pub fn code(r: Relation) -> u8 {
    match r {
        Relation::Enemy => 0,
        Relation::Neutral => 1,
        Relation::Ally => 2,
    }
}

/// Inverse of [`code`]; `None` for anything above 2.
pub fn from_code(c: u8) -> Option<Relation> {
    match c {
        0 => Some(Relation::Enemy),
        1 => Some(Relation::Neutral),
        2 => Some(Relation::Ally),
        _ => None,
    }
}

/// Snapshot row: `(from, to, code)`.
pub type RelationRows = Vec<(u8, u8, u8)>;

/// The mission overrides, ascending by `(from, to)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relations {
    rows: Vec<((PlayerId, PlayerId), Relation)>,
}

impl Relations {
    pub const fn new() -> Self {
        Relations { rows: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    fn get(&self, from: PlayerId, to: PlayerId) -> Option<Relation> {
        self.rows.binary_search_by_key(&(from, to), |r| r.0).ok().map(|i| self.rows[i].1)
    }

    /// Store `from`'s view of `to`, dropping the row when it equals the
    /// default (`default`), so equal states have one representation.
    pub(crate) fn set(&mut self, from: PlayerId, to: PlayerId, rel: Relation, default: Relation) {
        match self.rows.binary_search_by_key(&(from, to), |r| r.0) {
            Ok(i) if rel == default => {
                self.rows.remove(i);
            }
            Ok(i) => self.rows[i].1 = rel,
            Err(i) if rel != default => self.rows.insert(i, ((from, to), rel)),
            Err(_) => {}
        }
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("relations");
        h.u64(self.rows.len() as u64);
        for ((a, b), r) in &self.rows {
            h.u64(u64::from(a.0));
            h.u64(u64::from(b.0));
            h.u64(u64::from(code(*r)));
        }
    }

    pub fn to_rows(&self) -> RelationRows {
        self.rows.iter().map(|((a, b), r)| (a.0, b.0, code(*r))).collect()
    }

    /// Rebuild from snapshot rows; `None` unless the rows ascend strictly,
    /// carry a known code and name two distinct seats of `players`, and none
    /// equals the default.
    pub fn from_rows(rows: &[(u8, u8, u8)], players: &[(PlayerId, TeamId)]) -> Option<Self> {
        let mut out = Relations::new();
        let mut last = None;
        for &(a, b, c) in rows {
            let key = (PlayerId(a), PlayerId(b));
            let rel = from_code(c)?;
            let seat = |p: PlayerId| players.iter().any(|s| s.0 == p);
            if a == b || !seat(key.0) || !seat(key.1) || last.is_some_and(|l| l >= key) {
                return None;
            }
            if rel == default_view(players, key.0, key.1) {
                return None;
            }
            last = Some(key);
            out.rows.push((key, rel));
        }
        Some(out)
    }
}

fn team_of(players: &[(PlayerId, TeamId)], p: PlayerId) -> Option<TeamId> {
    players.iter().find(|s| s.0 == p).map(|s| s.1)
}

/// The relation with no mission override.
pub fn default_view(players: &[(PlayerId, TeamId)], a: PlayerId, b: PlayerId) -> Relation {
    if a == b {
        return Relation::Ally;
    }
    match (team_of(players, a), team_of(players, b)) {
        (Some(x), Some(y)) if x == y => Relation::Ally,
        (Some(_), Some(_)) => Relation::Enemy,
        _ => Relation::Neutral,
    }
}

fn rank(r: Relation) -> u8 {
    code(r)
}

/// The seats and their relations, as the consumers ask for them.
#[derive(Clone, Copy)]
pub struct Diplomacy<'a> {
    pub players: &'a [(PlayerId, TeamId)],
    pub rel: &'a Relations,
}

static NO_RELATIONS: Relations = Relations::new();

impl<'a> Diplomacy<'a> {
    pub fn new(players: &'a [(PlayerId, TeamId)], rel: &'a Relations) -> Self {
        Diplomacy { players, rel }
    }

    /// Seats only, no mission overrides (standalone steps and tests).
    pub fn plain(players: &'a [(PlayerId, TeamId)]) -> Self {
        Diplomacy { players, rel: &NO_RELATIONS }
    }

    pub fn is_seat(&self, p: PlayerId) -> bool {
        team_of(self.players, p).is_some()
    }

    /// `a`'s own view of `b` (unconfirmed).
    pub fn view(&self, a: PlayerId, b: PlayerId) -> Relation {
        if a == b {
            return Relation::Ally;
        }
        self.rel.get(a, b).unwrap_or_else(|| default_view(self.players, a, b))
    }

    /// The worse of both views (confirmed).
    pub fn confirmed(&self, a: PlayerId, b: PlayerId) -> Relation {
        let (x, y) = (self.view(a, b), self.view(b, a));
        if rank(x) <= rank(y) { x } else { y }
    }

    /// `a` may pick `b` on its own: both are seats and a's view of b is Enemy.
    pub fn is_enemy(&self, a: PlayerId, b: PlayerId) -> bool {
        self.is_seat(a) && self.is_seat(b) && self.view(a, b) == Relation::Enemy
    }

    /// Mutual allies, which share sight and count as friends in victory.
    pub fn confirmed_ally(&self, a: PlayerId, b: PlayerId) -> bool {
        a == b || self.confirmed(a, b) == Relation::Ally
    }
}

#[cfg(test)]
#[path = "relations_tests.rs"]
mod tests;
