//! Resolved starting personnel rules. No raw header values reach the simulation.

use crate::{Races, Warning};
use sta_assets::cfg_h::CfgH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficerLimit {
    /// Native label-map key; cosmetic, excluded from the rules hash.
    pub key: String,
    pub value: i32,
}

/// Narrow first slice of the typed rules seam; other RTS rules are not implied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub officer_limits: Vec<OfficerLimit>,
    starts: Vec<StartRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartRule {
    race: String,
    normal: [i64; 4],
    lots: [i64; 4],
    crew: [Option<i64>; 2],
}

impl Rules {
    pub fn from_header(text: &str) -> (Self, Vec<Warning>) {
        let cfg = CfgH::parse(text);
        let mut warnings = Vec::new();
        let mut rules = Self::default();
        if cfg.raw("MP_OFFICER_LIMIT").is_some() {
            match (cfg.i64("MP_OFFICER_LIST_SIZE"), cfg.integer_pairs("MP_OFFICER_LIMIT")) {
                (Some(size), Some(rows)) if size >= 0 && usize::try_from(size).ok() == Some(rows.len())
                    && rows.iter().all(|(_, value)| i32::try_from(*value).is_ok() && *value >= 0) => {
                    rules.officer_limits = rows.into_iter().map(|(key, value)| OfficerLimit { key, value: value as i32 }).collect();
                }
                _ => warnings.push(Warning::new("RTS_CFG.h", "invalid MP_OFFICER_LIMIT/count or unresolved/nonnegative integer expression; officer maximum unavailable")),
            }
        } else if let Some(value) = cfg.i64("SINGLE_PLAYER_OFFICER_LIMIT") {
            if let Some(value) = i32::try_from(value).ok().filter(|v| *v >= 0) {
                rules.officer_limits.push(OfficerLimit {
                    key: "ol_normal".into(),
                    value,
                });
            } else {
                warnings.push(Warning::new(
                    "RTS_CFG.h",
                    "invalid SINGLE_PLAYER_OFFICER_LIMIT; officer maximum unavailable",
                ));
            }
        }
        (rules, warnings)
    }

    /// Read `RTS_CFG.h` as the layers inherit it (D11: a header may include its
    /// own name from the layer below, see [`crate::layered`]). An include
    /// cycle is an error.
    pub(crate) fn load(vfs: &sta_vfs::Vfs, races: &Races) -> Result<(Self, Vec<Warning>), crate::ContentError> {
        let resolved = crate::layered::resolve(vfs, "RTS_CFG.h", 0).map_err(|e| crate::ContentError::IncludeCycle(e.0))?.unwrap_or_default();
        let mut warnings = resolved.warnings;
        let (mut rules, parsed) = Self::from_header(&resolved.text);
        warnings.extend(parsed);
        rules.starts = races
            .iter()
            .map(|race| StartRule {
                race: race.name.clone(),
                normal: race.stock.map(|n| n.clamp(0, 1_000_000_000)),
                lots: race.lots_stock.map(|n| n.clamp(0, 1_000_000_000)),
                crew: [race.normal_crew, race.lots_crew].map(|v| v.map(|n| n.clamp(0, 1_000_000_000))),
            })
            .collect();
        rules.starts.sort_by(|a, b| a.race.cmp(&b.race));
        Ok((rules, warnings))
    }

    pub fn officer_limit(&self, choice: usize) -> Option<i32> {
        self.officer_limits.get(choice).map(|limit| limit.value)
    }

    /// Explicit numeric encoding; cosmetic labels and Rust native layouts are absent.
    pub(crate) fn hash_values(&self) -> Vec<u64> {
        if self.officer_limits.is_empty() && self.starts.is_empty() {
            return Vec::new();
        }
        let mut out = vec![self.officer_limits.len() as u64];
        out.extend(self.officer_limits.iter().map(|limit| limit.value as u64));
        out.push(self.starts.len() as u64);
        for start in &self.starts {
            // Faction identity is a simulation lookup key, unlike a display caption.
            out.push(start.race.len() as u64);
            out.extend(start.race.bytes().map(u64::from));
            out.extend(start.normal.map(|n| n as u64));
            out.extend(start.lots.map(|n| n as u64));
            for crew in start.crew {
                out.push(u64::from(crew.is_some()));
                out.push(crew.unwrap_or(0) as u64);
            }
        }
        out
    }
}
