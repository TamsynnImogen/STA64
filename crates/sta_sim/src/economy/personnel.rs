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

    pub fn officer_limit(&self, player: PlayerId) -> Option<Fixed> {
        self.personnel
            .binary_search_by_key(&player, |r| r.0)
            .ok()
            .and_then(|i| self.personnel[i].2)
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
                || [crew, limit].into_iter().flatten().any(|v| *v < 0)
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
