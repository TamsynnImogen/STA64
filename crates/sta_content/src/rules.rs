//! Resolved starting personnel rules. No raw header values reach the simulation.

use crate::{Races, Warning};
use sta_assets::cfg_h::CfgH;
use sta_core::Fixed;

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
    /// `RTS_CFG.h`'s path-planner block, when the header has any of its keys.
    pathing: Option<PathingRules>,
}

/// The path planner's terrain costs (SIM-23), from `RTS_CFG.h`'s PATHING
/// block. Missing keys keep the stock value, which Armada II and Fleet
/// Operations ship identically. Costs are on the scale of
/// `DEFAULT_TERRAIN_VALUE`; `IMPASSABLE_TERRAIN` marks a blocked cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathingRules {
    /// `USE_PATH_PLANNING` (1).
    pub enabled: bool,
    /// `DEFAULT_TERRAIN_VALUE` (100): open space.
    pub default_cost: i64,
    /// `IMPASSABLE_TERRAIN` (-1).
    pub impassable: i64,
    /// `OFF_MAP_PATHING_COST` (90000): outside the map, when a unit may go there.
    pub off_map_cost: i64,
    /// `NEBULAE_PATHING_COST` (300): a nebula whose ODF sets no `pathing_cost`.
    pub nebula_cost: i64,
    /// `NEBULAE_PATHING_SAFETY_COEFFICIENT` (1.8): nebula radius multiplier.
    pub nebula_safety: Fixed,
    /// `COLLISION_SAFETY_MARGIN` (1.2): how much wider than reality solids are.
    pub collision_margin: Fixed,
    /// `HIGHEST_QUADTREE_GRID_RESOLUTION` (50): the finest planner cell, units.
    pub resolution: Fixed,
}

impl Default for PathingRules {
    fn default() -> Self {
        PathingRules {
            enabled: true,
            default_cost: 100,
            impassable: -1,
            off_map_cost: 90_000,
            nebula_cost: 300,
            // As the header's `float` reads (f32), so stock and absent agree.
            nebula_safety: Fixed::from_f64(1.8f32 as f64),
            collision_margin: Fixed::from_f64(1.2f32 as f64),
            resolution: Fixed::from_int(50),
        }
    }
}

impl PathingRules {
    const KEYS: [&'static str; 8] = [
        "USE_PATH_PLANNING",
        "DEFAULT_TERRAIN_VALUE",
        "IMPASSABLE_TERRAIN",
        "OFF_MAP_PATHING_COST",
        "NEBULAE_PATHING_COST",
        "NEBULAE_PATHING_SAFETY_COEFFICIENT",
        "COLLISION_SAFETY_MARGIN",
        "HIGHEST_QUADTREE_GRID_RESOLUTION",
    ];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> Option<Self> {
        if !Self::KEYS.iter().any(|k| cfg.raw(k).is_some()) {
            return None;
        }
        let stock = Self::default();
        let mut bad = |key: &str| warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a number; the stock value is used")));
        let mut int = |key: &str, default: i64| match cfg.raw(key) {
            None => default,
            Some(_) => cfg.i64(key).or_else(|| cfg.f32(key).filter(|v| v.is_finite()).map(|v| v.round() as i64)).map(|v| v.clamp(-1_000_000_000, 1_000_000_000)).unwrap_or_else(|| { bad(key); default }),
        };
        let enabled = int("USE_PATH_PLANNING", 1) != 0;
        let default_cost = int("DEFAULT_TERRAIN_VALUE", stock.default_cost);
        let impassable = int("IMPASSABLE_TERRAIN", stock.impassable);
        let off_map_cost = int("OFF_MAP_PATHING_COST", stock.off_map_cost);
        let nebula_cost = int("NEBULAE_PATHING_COST", stock.nebula_cost);
        let mut real = |key: &str, default: Fixed| match cfg.raw(key) {
            None => default,
            Some(_) => match cfg.f32(key).filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0e6) {
                Some(v) => Fixed::from_f64(v as f64),
                None => { warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a number; the stock value is used"))); default }
            },
        };
        Some(PathingRules {
            enabled,
            default_cost,
            impassable,
            off_map_cost,
            nebula_cost,
            nebula_safety: real("NEBULAE_PATHING_SAFETY_COEFFICIENT", stock.nebula_safety),
            collision_margin: real("COLLISION_SAFETY_MARGIN", stock.collision_margin),
            resolution: real("HIGHEST_QUADTREE_GRID_RESOLUTION", stock.resolution),
        })
    }

    fn hash_values(&self) -> Vec<u64> {
        vec![
            u64::from(self.enabled),
            self.default_cost as u64,
            self.impassable as u64,
            self.off_map_cost as u64,
            self.nebula_cost as u64,
            self.nebula_safety.to_bits() as u64,
            self.collision_margin.to_bits() as u64,
            self.resolution.to_bits() as u64,
        ]
    }
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
        rules.pathing = PathingRules::from_cfg(&cfg, &mut warnings);
        (rules, warnings)
    }

    /// The path planner's rules: the header's, else the stock values.
    pub fn pathing(&self) -> PathingRules {
        self.pathing.unwrap_or_default()
    }

    /// Set the path planner's rules (tests and content built in code).
    pub fn with_pathing(mut self, pathing: PathingRules) -> Self {
        self.pathing = Some(pathing);
        self
    }

    /// The path planner's values for the content hash; empty when the
    /// header had none, so content without them keeps its hash.
    pub(crate) fn pathing_hash_values(&self) -> Vec<u64> {
        self.pathing.as_ref().map(PathingRules::hash_values).unwrap_or_default()
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

#[cfg(test)]
mod pathing_tests {
    use super::*;

    /// The PATHING block as Armada II and Fleet Operations ship it.
    const STOCK: &str = "// PATHING\nint OFF_MAP_PATHING_COST = 90000;\nfloat HIGHEST_QUADTREE_GRID_RESOLUTION = 50.0;\n\
        int USE_PATH_PLANNING = 1;\nint IMPASSABLE_TERRAIN = -1;\nint DEFAULT_TERRAIN_VALUE = 100;\n\
        float NEBULAE_PATHING_COST = 300.0;\nfloat NEBULAE_PATHING_SAFETY_COEFFICIENT = 1.8;\n\
        float COLLISION_SAFETY_MARGIN = 1.2;\n";

    #[test]
    fn the_stock_header_matches_the_defaults() {
        let (rules, warnings) = Rules::from_header(STOCK);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(rules.pathing(), PathingRules::default());
        assert!(!rules.pathing_hash_values().is_empty());
    }

    #[test]
    fn a_header_without_the_block_keeps_the_hash_and_the_stock_values() {
        let (rules, _) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        assert_eq!(rules.pathing(), PathingRules::default());
        assert!(rules.pathing_hash_values().is_empty());
    }

    #[test]
    fn a_mod_changes_what_it_sets_and_keeps_the_rest() {
        let (rules, warnings) = Rules::from_header(
            "int USE_PATH_PLANNING = 0;\nfloat NEBULAE_PATHING_COST = 450.0;\nint DEFAULT_TERRAIN_VALUE = 100;\nint OFF_MAP_PATHING_COST = DEFAULT_TERRAIN_VALUE * 4;\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let p = rules.pathing();
        assert!(!p.enabled);
        assert_eq!(p.nebula_cost, 450);
        assert_eq!(p.off_map_cost, 400);
        assert_eq!(p.nebula_safety, PathingRules::default().nebula_safety);
        let (bad, warnings) = Rules::from_header("float COLLISION_SAFETY_MARGIN = wide;\n");
        assert_eq!(bad.pathing().collision_margin, PathingRules::default().collision_margin);
        assert_eq!(warnings.len(), 1);
    }
}
