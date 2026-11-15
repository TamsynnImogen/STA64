//! What a campaign seat starts with (the map's team table).
//!
//! A mission's game is built bare (`Sim::from_setup`): no seat has stock, crew or an officer limit,
//! because those come from a race loadout that only a skirmish start places. The original takes a
//! mission's start values from the map itself (`RTS_CFG.h`: its `STARTING_*` values are "ignored by
//! single player - they get loaded from the map itself"), from the team table
//! [`sta_bzn::team_starts`] reads. This module turns that table into ordinary script-privileged
//! edits that the mission runtime sends before the script's first step
//! ([`crate::runtime::MissionRuntime::set_start_stock`]); `Map`, the snapshot and the replay layout
//! do not change, and a replay carries the edits as the commands they are.
//!
//! Rule STA64 follows: seat `n` (Armada team `n + 1`) gets its block's dilithium, latinum and metal as
//! `Edit::Cargo`, its crew as `Edit::TeamCrew` and its officer limit as `Edit::MaxOfficers`, each
//! only while the seat has none of that yet (a host that placed a loadout is left alone), crew and
//! officer limit only when the table gives more than 0 (what 0 means for a team is not established,
//! so such a seat stays as it is today). Biomatter is not in the table as far as read. Evidence: read
//! from data for every map; measured for `a2_fed01` (the table's first block equals the stock game's
//! start bar: dilithium, latinum, metal, crew); the officer limit is read from data only.

use sta_core::{Fixed, PlayerId};
use sta_sim::command::Edit;
use sta_vfs::Vfs;

/// One seat's start values, whole units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatStart {
    pub seat: PlayerId,
    pub dilithium: i32,
    pub latinum: i32,
    pub metal: i32,
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
pub fn read_start_stock(vfs: &Vfs, bzn: &str) -> Vec<SeatStart> {
    let Some(Ok(bytes)) = vfs.read(&format!("bzn/{bzn}")) else { return Vec::new() };
    from_table(&sta_bzn::team_starts(&bytes))
}

/// [`SeatStart`]s of a team table, block `n` for seat `n`.
pub fn from_table(table: &[sta_bzn::TeamStart]) -> Vec<SeatStart> {
    table
        .iter()
        .enumerate()
        .map(|(n, t)| SeatStart {
            seat: PlayerId(n as u8),
            dilithium: whole(t.dilithium),
            latinum: whole(t.latinum),
            metal: whole(t.metal),
            crew: Some(whole(t.crew)).filter(|&c| c > 0),
            officers: Some(t.officers.clamp(0, LIMIT as i32)).filter(|&o| o > 0),
        })
        .collect()
}

impl SeatStart {
    /// The edits this seat needs given what the view already holds for it: cargo first, then crew and
    /// officer limit (the sim takes both for any seat of the game, in command order).
    pub(crate) fn edits(&self, has_stock: bool, has_crew: bool, has_officers: bool) -> Vec<Edit> {
        let mut out = Vec::new();
        if !has_stock {
            // Resource ids: 0 dilithium, 1 latinum, 2 metal.
            for (res, v) in [(0u8, self.dilithium), (1, self.latinum), (2, self.metal)] {
                out.push(Edit::Cargo(self.seat, res, Fixed::from_int(v)));
            }
        }
        if let Some(c) = self.crew.filter(|_| !has_crew) {
            out.push(Edit::TeamCrew(self.seat, Fixed::from_int(c)));
        }
        if let Some(o) = self.officers.filter(|_| !has_officers) {
            out.push(Edit::MaxOfficers(self.seat, Fixed::from_int(o)));
        }
        out
    }
}

#[cfg(test)]
mod tests;
