//! What a campaign seat starts with (the map's team table).
//!
//! A mission's game is built bare (`Sim::from_setup`): no seat has stock, crew or an officer limit,
//! because those come from a race loadout that only a skirmish start places. The original takes a
//! mission's start values from the map itself (`RTS_CFG.h`: its `STARTING_*` values are "ignored by
//! single player - they get loaded from the map itself"), from the team table
//! [`sta_bzn::team_starts`] reads. This module turns that table into ordinary script-privileged
//! edits that the mission runtime sends before the script's first step
//! (queued while the runtime is built, so they are the script's first commands, ahead of anything it
//! queues while it loads: a `Set*` at load time wins, an `Add*` adds to the start value); `Map`, the snapshot and the replay layout
//! do not change, and a replay carries the edits as the commands they are.
//!
//! Rule STA64 follows: seat `n` (Armada team `n + 1`) gets its block's dilithium, latinum, metal and
//! biomatter ("supply" in Fleet Operations) as `Edit::Cargo` (resource ids 0 to 3), its crew as
//! `Edit::TeamCrew` and its officer limit as `Edit::MaxOfficers`, only while the seat has no crew pool and
//! no officer limit yet (a host that placed a loadout is left alone), crew and officer limit only when
//! the table gives more than 0 (what 0 means for a team is not established, so such a seat stays as it is
//! today). The table is read in both block shapes (see [`sta_bzn::team_table`]); its values are the start
//! values, not an addition to the header's `STARTING_*` defaults. Evidence: read from data for every map;
//! measured for `a2_fed01` (the table's first block equals the stock game's start bar: dilithium,
//! latinum, metal, crew); the officer limit is read from data only. The biomatter slot: read from the
//! Fleet Operations executable, static (the team loader stores it into the biomatter field after the
//! pre-load defaults, for newer file versions); the header's `STARTING_BIOMATTER` is not read by that
//! executable; for Armada II maps the same slot is applied by inference (its executable was not read).
//!
//! An Armada I map's own team table gives dilithium, crew and officer limit ([`from_a1_table`]); the install
//! header's `cfgStartingDilithium`, `cfgStartingCrew` and `cfgMaxOfficers` apply only to a map without a
//! table ([`StartStock::or_header`]). "A host placed a loadout" asks for a written crew pool or a written
//! officer limit, never the derived limit: Armada I content derives one for every seat.
//!
//! Order: the map's values first, then the script's own writes (it runs after them, so its `Set*`
//! wins and its `Add*` adds). Grade: that a map's values are in place before the script's init is not
//! measured in the original and not read from its executable; it is STA64's rule, chosen because the
//! original script calls (read from data) all set or add to a team's stock and none assumes it is empty.
//!
//! A table value of 0 for crew or the officer limit is not written (grade: not measured; what a literal 0
//! means in the original is not established): the seat keeps no crew pool or no officer limit, and in the sim an
//! unwritten officer limit means no limit, so a seat whose table says 0 is not held back by officers.

use sta_core::{Fixed, PlayerId};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::view::WorldView;
use sta_vfs::Vfs;

use crate::api::Ctx;

/// One seat's start values, whole units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatStart {
    pub seat: PlayerId,
    /// `None`: not written (a header start carries dilithium only).
    pub dilithium: Option<i32>,
    pub latinum: Option<i32>,
    pub metal: Option<i32>,
    /// Biomatter (supply).
    pub biomatter: Option<i32>,
    /// `None` when the table gives 0.
    pub crew: Option<i32>,
    /// `None` when the table gives 0.
    pub officers: Option<i32>,
}

const LIMIT: f32 = 1_000_000_000.0;

fn whole(v: f32) -> i32 {
    if v.is_nan() { 0 } else { v.clamp(0.0, LIMIT) as i32 }
}

/// The start values of the map `bzn` (`a2_fed01.bzn`) of `vfs`, one per seat in the table; empty when
/// the map cannot be read or carries no table.
pub fn read_start_stock(vfs: &Vfs, bzn: &str) -> StartStock {
    let Some(Ok(bytes)) = vfs.read(&format!("bzn/{bzn}")) else { return StartStock::default() };
    match sta_bzn::team_table(&bytes) {
        sta_bzn::TeamTable::Found(t) => StartStock { seats: from_table(&t), refused: None },
        sta_bzn::TeamTable::Refused(runs) => StartStock { seats: Vec::new(), refused: Some(format!("map team table refused (block runs {runs:?}): no start values")) },
        sta_bzn::TeamTable::Absent => match sta_bzn::a1::team_table(&bytes) {
            sta_bzn::a1::A1StartTable::Found(t) => StartStock { seats: from_a1_table(&t), refused: None },
            sta_bzn::a1::A1StartTable::Refused => StartStock { seats: Vec::new(), refused: Some("map team table refused (Armada I team block is not the expected shape): no start values".into()) },
            sta_bzn::a1::A1StartTable::Absent => StartStock::default(),
        },
    }
}

/// [`SeatStart`]s of an Armada I map's team table ([`sta_bzn::a1::team_starts`]): team `n + 1` for seat `n`,
/// dilithium, crew and officer limit only (the other stocks are not in that table). Crew and officer limit
/// of 0 are not written, as for the other table.
///
/// Evidence: read from data. Field meaning (dilithium, crew, officer limit): inferred from data agreement
/// and one player report (see [`sta_bzn::a1::team_starts`]), interim. Seat `n` = team `n + 1` rests on the
/// colour table's team order and the converted-mission numbering: not measured.
///
/// Interim decision, officer limit: the record is written as the limit's base, and the sim then adds the
/// officer gain of every finished starbase on top, as it does for any Armada I seat. On the maps read, the
/// record's relation to the starting stations is not uniform; whether the original shows the record itself
/// at the start is not measured, so a seat that starts with finished starbases may show one gain per
/// starting station more than the original.
// TODO: settle whether a mission's officer limit includes the starting starbases' gain (measure the original).
// TODO: the table's maximum dilithium and maximum crew records are not read (what they bound is not established).
pub fn from_a1_table(table: &[sta_bzn::a1::A1TeamStart]) -> Vec<SeatStart> {
    table
        .iter()
        .enumerate()
        .map(|(n, t)| SeatStart {
            seat: PlayerId(n as u8),
            dilithium: Some(whole(t.dilithium)),
            latinum: None,
            metal: None,
            biomatter: None,
            crew: Some(whole(t.crew)).filter(|&c| c > 0),
            officers: Some(t.officers.clamp(0, LIMIT as i32)).filter(|&o| o > 0),
        })
        .collect()
}

/// What a host read from a mission's map: the seats' start values, or why none were taken.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartStock {
    pub seats: Vec<SeatStart>,
    /// The map holds table-shaped records that are not exactly one table of eight blocks: nothing was
    /// granted, and the runtime lists this once as an unsupported use.
    pub refused: Option<String>,
}

/// [`SeatStart`]s of a team table, block `n` for seat `n`.
pub fn from_table(table: &[sta_bzn::TeamStart]) -> Vec<SeatStart> {
    table
        .iter()
        .enumerate()
        .map(|(n, t)| SeatStart {
            seat: PlayerId(n as u8),
            dilithium: Some(whole(t.dilithium)),
            latinum: Some(whole(t.latinum)),
            metal: Some(whole(t.metal)),
            biomatter: Some(whole(t.biomatter)),
            crew: Some(whole(t.crew)).filter(|&c| c > 0),
            officers: Some(t.officers.clamp(0, LIMIT as i32)).filter(|&o| o > 0),
        })
        .collect()
}

/// The start values of an install whose `RTS_CFG.h` carries one start for every team (`cfgStartingDilithium`,
/// `cfgStartingCrew`, `cfgMaxOfficers`), for each of `seats`: what a map without a team table gives its
/// seats. Empty when the header has none of the keys, so a profile whose header does not carry them (the
/// Armada II family) is unchanged by data, not by name. Each key that is absent leaves its field unset.
pub fn header_start(rules: &sta_content::Rules, seats: impl IntoIterator<Item = PlayerId>) -> Vec<SeatStart> {
    let h = rules.legacy_start();
    let whole = |v: Option<i64>| v.map(|v| v.clamp(0, i64::from(LIMIT as i32)) as i32).filter(|&v| v > 0);
    let (dilithium, crew, officers) = (h.dilithium.map(|v| v.clamp(0, i64::from(LIMIT as i32)) as i32), whole(h.crew), whole(h.officers));
    if dilithium.is_none() && crew.is_none() && officers.is_none() {
        return Vec::new();
    }
    seats.into_iter().map(|seat| SeatStart { seat, dilithium, latinum: None, metal: None, biomatter: None, crew, officers }).collect()
}

impl StartStock {
    /// The map gave no team table (and none was refused): the install header's start for `seats`.
    pub fn or_header(mut self, rules: &sta_content::Rules, seats: impl IntoIterator<Item = PlayerId>) -> StartStock {
        if self.seats.is_empty() && self.refused.is_none() {
            self.seats = header_start(rules, seats);
        }
        self
    }
}

impl SeatStart {
    /// The edits this seat needs: cargo first, then crew and officer limit (the sim takes both for any
    /// seat of the game, in command order). A seat that already has crew or an officer limit came from
    /// a placed loadout (a skirmish start) and takes nothing: whether it holds stock is not asked,
    /// because passive income can open a stock slot on the tick before a host's first think.
    pub(crate) fn edits(&self, has_loadout: bool) -> Vec<Edit> {
        if has_loadout {
            return Vec::new();
        }
        let mut out = Vec::new();
        // Resource ids: 0 dilithium, 1 latinum, 2 metal, 3 biomatter.
        for (res, v) in [(0u8, self.dilithium), (1, self.latinum), (2, self.metal), (3, self.biomatter)] {
            if let Some(v) = v {
                out.push(Edit::Cargo(self.seat, res, Fixed::from_int(v)));
            }
        }
        if let Some(c) = self.crew {
            out.push(Edit::TeamCrew(self.seat, Fixed::from_int(c)));
        }
        if let Some(o) = self.officers {
            out.push(Edit::MaxOfficers(self.seat, Fixed::from_int(o)));
        }
        out
    }
}

/// Queue the start values of `seats` as script edits and show them in the ctx's view: called once, while
/// the runtime is built, before the script loads.
pub(crate) fn queue(ctx: &mut Ctx, start: StartStock) {
    if let Some(why) = &start.refused {
        ctx.unsupported(why);
    }
    let mut shown = Vec::new();
    for seat in start.seats {
        let Some(p) = ctx.view.players.iter().find(|p| p.id == seat.seat) else { continue };
        shown.extend(seat.edits(placed_loadout(&ctx.view, p)));
    }
    show(&mut ctx.view, &shown);
    for e in &shown {
        let _ = ctx.try_push(PlayerId::SCRIPT, CommandKind::Edit(e.clone()));
    }
    ctx.start_view = shown;
}

/// Whether a host placed a loadout for `p`: a written crew pool or a written officer limit. The derived
/// limit (`PlayerView::officers`, `SeatView::officer_limit`) is not asked: Armada I content derives one for
/// every seat, written or not, so it cannot tell a placed loadout from a bare mission seat.
fn placed_loadout(view: &WorldView, p: &sta_sim::view::PlayerView) -> bool {
    p.crew.is_some() || view.attrs.seats.iter().any(|s| s.player == p.id && s.officer_base.is_some())
}

/// What the sim will hold once `edits` are applied, in `view`: the script's first step reads it.
pub(crate) fn show(view: &mut WorldView, edits: &[Edit]) {
    for e in edits {
        let seat = match e {
            Edit::Cargo(p, ..) | Edit::TeamCrew(p, _) | Edit::MaxOfficers(p, _) => *p,
            _ => continue,
        };
        let Some(pv) = view.players.iter_mut().find(|p| p.id == seat) else { continue };
        match e {
            Edit::Cargo(_, res, v) => {
                pv.resources.resize(pv.resources.len().max(4), Fixed::ZERO);
                pv.resources[usize::from(*res)] = *v;
            }
            Edit::TeamCrew(_, v) => pv.crew = Some(*v),
            Edit::MaxOfficers(_, v) => pv.officers[1] = *v,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
