//! Initialized crew stock and officer maximum, separate from mined resources.

use super::*;

pub(crate) type PersonnelRows = Vec<(PlayerId, Option<i64>, Option<i64>)>;

impl Economy {
    pub fn crew_of(&self, player: PlayerId) -> Option<Fixed> {
        self.personnel
            .binary_search_by_key(&player, |r| r.0)
            .ok()
            .and_then(|i| self.personnel[i].1)
    }

    /// The limit as the setup or a script wrote it (`Team+0x164`), before the
    /// seat race's multiplier.
    pub fn officer_base(&self, player: PlayerId) -> Option<Fixed> {
        self.personnel
            .binary_search_by_key(&player, |r| r.0)
            .ok()
            .and_then(|i| self.personnel[i].2)
    }

    /// The officer cap: `floor(maxOfficerMultiplier * limit)` of the seat's
    /// current race (`Team::GetMaxOfficers`, read from the executable, static;
    /// the multiplier is 1 unless the race ODF sets it), so a re-race takes effect at once.
    pub fn officer_limit(&self, player: PlayerId) -> Option<Fixed> {
        let base = self.officer_base(player)?;
        let m = self.seat_rule(player).officer_multiplier;
        if m == Fixed::ONE {
            return Some(base);
        }
        let product = (i128::from(base.to_bits()) * i128::from(m.to_bits())) >> 32;
        Some(Fixed::from_bits(((product >> 32) << 32).clamp(0, i128::from(i64::MAX)) as i64))
    }

    /// `Edit::MaxOfficers` for a seat: the original writes the field raw
    /// (read from the executable, static), so a seat whose data gave no limit
    /// takes the script's value as its limit.
    pub(crate) fn edit_officer_limit(&mut self, player: PlayerId, value: Fixed) -> bool {
        let value = value.max(Fixed::ZERO).min(Fixed::from_int(1_000_000_000));
        match self.personnel.binary_search_by_key(&player, |r| r.0) {
            Ok(i) => self.personnel[i].2 = Some(value),
            Err(i) => self.personnel.insert(i, (player, None, Some(value))),
        }
        true
    }

    /// `Edit::TeamCrew` for a seat: a raw write in the original (read from the
    /// executable, static), so a seat with no stock yet starts one.
    pub(crate) fn edit_team_crew(&mut self, player: PlayerId, value: Fixed) -> bool {
        let value = value.max(Fixed::ZERO).min(Fixed::from_int(1_000_000_000));
        match self.personnel.binary_search_by_key(&player, |r| r.0) {
            Ok(i) => self.personnel[i].1 = Some(value),
            Err(i) => self.personnel.insert(i, (player, Some(value), None)),
        }
        true
    }

    pub(crate) fn set_personnel(&mut self, player: PlayerId, crew: Option<Fixed>, limit: Option<Fixed>) {
        if crew.is_none() && limit.is_none() {
            return;
        }
        let row = (player, crew, limit);
        match self.personnel.binary_search_by_key(&player, |r| r.0) {
            Ok(i) => self.personnel[i] = row,
            Err(i) => self.personnel.insert(i, row),
        }
    }

    pub(super) fn hash_personnel(&self, h: &mut Fnv) {
        if self.personnel.is_empty() {
            return;
        }
        h.u64(0x7065_7273_6f6e_7631);
        h.u64(self.personnel.len() as u64);
        for &(player, crew, limit) in &self.personnel {
            h.u64(u64::from(player.0));
            for value in [crew, limit] {
                h.u64(u64::from(value.is_some()));
                if let Some(value) = value {
                    h.fixed(value);
                }
            }
        }
    }

    pub(crate) fn personnel_rec(&self) -> PersonnelRows {
        self.personnel
            .iter()
            .map(|&(player, crew, limit)| (player, crew.map(Fixed::to_bits), limit.map(Fixed::to_bits)))
            .collect()
    }

    pub(crate) fn restore_personnel(
        &mut self,
        rows: PersonnelRows,
        players: &[(PlayerId, sta_core::TeamId)],
    ) -> Result<(), &'static str> {
        if rows.windows(2).any(|w| w[0].0 >= w[1].0) {
            return Err("personnel rows out of order/duplicate");
        }
        if rows.iter().any(|(player, crew, limit)| {
            !players.iter().any(|p| p.0 == *player)
                || (crew.is_none() && limit.is_none())
                // The team pool has no floor at 0 (SIM-54), the limit does.
                || limit.is_some_and(|v| v < 0)
        }) {
            return Err("invalid personnel player/value");
        }
        self.personnel = rows
            .into_iter()
            .map(|(p, c, l)| (p, c.map(Fixed::from_bits), l.map(Fixed::from_bits)))
            .collect();
        Ok(())
    }
}
