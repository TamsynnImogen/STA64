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
//! Rule STA64 follows: seat `n` (Armada team `n + 1`) gets its block's dilithium, latinum and metal as
//! `Edit::Cargo`, its crew as `Edit::TeamCrew` and its officer limit as `Edit::MaxOfficers`, only
//! while the seat has no crew pool and no officer limit yet (a host that placed a loadout is left
//! alone), crew and officer limit only when the table gives more than 0 (what 0 means for a team is not established,
//! so such a seat stays as it is today). Biomatter is not in the table as far as read. Evidence: read
//! from data for every map; measured for `a2_fed01` (the table's first block equals the stock game's
//! start bar: dilithium, latinum, metal, crew); the officer limit is read from data only.
//!
//! A table value of 0 for crew or the officer limit is not written (what a literal 0 means in the
//! original is not established): the seat keeps no crew pool or no officer limit, and in the sim an
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
    /// The edits this seat needs: cargo first, then crew and officer limit (the sim takes both for any
    /// seat of the game, in command order). A seat that already has crew or an officer limit came from
    /// a placed loadout (a skirmish start) and takes nothing: whether it holds stock is not asked,
    /// because passive income can open a stock slot on the tick before a host's first think.
    pub(crate) fn edits(&self, has_loadout: bool) -> Vec<Edit> {
        if has_loadout {
            return Vec::new();
        }
        let mut out = Vec::new();
        // Resource ids: 0 dilithium, 1 latinum, 2 metal.
        for (res, v) in [(0u8, self.dilithium), (1, self.latinum), (2, self.metal)] {
            out.push(Edit::Cargo(self.seat, res, Fixed::from_int(v)));
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
pub(crate) fn queue(ctx: &mut Ctx, seats: Vec<SeatStart>) {
    let mut shown = Vec::new();
    for seat in seats {
        let Some(p) = ctx.view.players.iter().find(|p| p.id == seat.seat) else { continue };
        shown.extend(seat.edits(p.crew.is_some() || p.officers[1] > Fixed::ZERO));
    }
    show(&mut ctx.view, &shown);
    for e in &shown {
        let _ = ctx.try_push(PlayerId::SCRIPT, CommandKind::Edit(e.clone()));
    }
    ctx.start_view = shown;
}

/// What the sim will hold once `edits` are applied, in `view`: the script's first step reads it.
pub(crate) fn show(view: &mut WorldView, edits: &[Edit]) {
    for e in edits {
        let (seat, apply): (PlayerId, Box<dyn FnOnce(&mut sta_sim::view::PlayerView)>) = match e {
            Edit::Cargo(p, res, v) => {
                let (res, v) = (usize::from(*res), *v);
                (*p, Box::new(move |pv| {
                    pv.resources.resize(pv.resources.len().max(4), Fixed::ZERO);
                    pv.resources[res] = v;
                }))
            }
            Edit::TeamCrew(p, v) => {
                let v = *v;
                (*p, Box::new(move |pv| pv.crew = Some(v)))
            }
            Edit::MaxOfficers(p, v) => {
                let v = *v;
                (*p, Box::new(move |pv| pv.officers[1] = v))
            }
            _ => continue,
        };
        if let Some(pv) = view.players.iter_mut().find(|p| p.id == seat) {
            apply(pv);
        }
    }
}

#[cfg(test)]
mod tests;
