//! The typed rules of `RTS_CFG.h` (D9): officer limits, the path planner,
//! the keys Classic Enhanced changes, and each race's starting forces. No
//! raw header value reaches the simulation; every value here is an entry of
//! the content digest ([`Rules::digest_entries`], D10).

use crate::digest::{Canon, DigestEntry, quoted};
use crate::{Races, Warning};
use sta_assets::cfg_h::CfgH;
use sta_core::Fixed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficerLimit {
    /// Native label-map key; cosmetic, excluded from the rules hash.
    pub key: String,
    pub value: i32,
}

/// The typed rules. Which `RTS_CFG.h` keys are typed is data-driven: the ones
/// the sim reads and the ones the installed mods change (see [`CfgRules`]);
/// [`TYPED_CFG_KEYS`] lists them for the coverage report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub officer_limits: Vec<OfficerLimit>,
    starts: Vec<StartRule>,
    /// `RTS_CFG.h`'s path-planner block, when the header has any of its keys.
    pathing: Option<PathingRules>,
    /// Team crew accumulation (SIM-25), when the header or a race has a key.
    crew: Option<CrewRules>,
    /// `RTS_CFG.h`'s default craft autonomy, when the header sets either key.
    autonomy: Option<AutonomyRules>,
    /// `RTS_CFG.h`'s attack warp distance (SIM-36), when the header sets it.
    approach: Option<ApproachRules>,
    /// The other typed `RTS_CFG.h` keys.
    pub cfg: CfgRules,
}

const TYPED_LEN: usize = 4 + PathingRules::KEYS.len() + AutonomyRules::KEYS.len() + ApproachRules::KEYS.len() + CfgRules::KEYS.len();

/// Every `RTS_CFG.h` key [`Rules`] reads.
pub const TYPED_CFG_KEYS: [&str; TYPED_LEN] = {
    let mut out = [""; TYPED_LEN];
    out[0] = "MP_OFFICER_LIMIT";
    out[1] = "MP_OFFICER_LIST_SIZE";
    out[2] = "SINGLE_PLAYER_OFFICER_LIMIT";
    out[3] = "STARTING_CREW_CAPACITY";
    let mut i = 0;
    while i < PathingRules::KEYS.len() {
        out[4 + i] = PathingRules::KEYS[i];
        i += 1;
    }
    let mut a = 0;
    while a < AutonomyRules::KEYS.len() {
        out[4 + i + a] = AutonomyRules::KEYS[a];
        a += 1;
    }
    let mut w = 0;
    while w < ApproachRules::KEYS.len() {
        out[4 + i + a + w] = ApproachRules::KEYS[w];
        w += 1;
    }
    let mut j = 0;
    while j < CfgRules::KEYS.len() {
        out[4 + i + a + w + j] = CfgRules::KEYS[j];
        j += 1;
    }
    out
};

/// Team crew rules: `STARTING_CREW_CAPACITY` from `RTS_CFG.h` and each race's
/// `crewAccumulationRate` (crew per second per starbase).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CrewRules {
    /// What a team can hold with no starbase or planet (`STARTING_CREW_CAPACITY`).
    pub starting_capacity: Option<i64>,
    /// Race key (lower case, sorted) and its accumulation rate per second.
    pub races: Vec<(String, Fixed)>,
}

/// The autonomy a new craft starts with (CON-46, D9), from `RTS_CFG.h`:
/// `CRAFT_DEFAULT_MOVEMENT_AUTONOMY` and `CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY`,
/// 0 low, 1 medium, 2 high. An absent key reads as the engine's own default,
/// movement 1 and special weapon 0 (read from the executable, static). The
/// installs set 1 and 2 (Roots, Fleet Ops 4.0), 1 and 1 (Classic Enhanced)
/// and nothing (plain Classic): read from the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutonomyRules {
    pub movement: u8,
    pub special_weapon: u8,
}

impl Default for AutonomyRules {
    fn default() -> Self {
        AutonomyRules { movement: 1, special_weapon: 0 }
    }
}

impl AutonomyRules {
    const KEYS: [&'static str; 2] = ["CRAFT_DEFAULT_MOVEMENT_AUTONOMY", "CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY"];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> Option<Self> {
        if !Self::KEYS.iter().any(|k| cfg.raw(k).is_some()) {
            return None;
        }
        let stock = Self::default();
        let mut level = |key: &str, default: u8| match cfg.raw(key) {
            None => default,
            // Whole numbers only, but `2.0` counts as 2 as for the path rules.
            Some(_) => match cfg.i64(key).or_else(|| cfg.f32(key).filter(|v| v.is_finite() && v.fract() == 0.0).map(|v| v as i64)).filter(|v| (0..=2).contains(v)) {
                Some(v) => v as u8,
                None => {
                    warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not 0, 1 or 2; the engine default is used")));
                    default
                }
            },
        };
        Some(AutonomyRules {
            movement: level(Self::KEYS[0], stock.movement),
            special_weapon: level(Self::KEYS[1], stock.special_weapon),
        })
    }
}

/// How far an attacker warps toward its target (SIM-36, D9):
/// `cfg_MIN_MOVE_WARP_DISTANCE`, compared with the surface-to-surface
/// distance to the target. Above it an attack approach may warp; below it the
/// approach drops out of warp and closes in. Absent: 1500, the executable's
/// own default (read from the executable, static). The installs set 1100
/// (stock Armada II, STA2 Classic and Classic Enhanced) and 999999 (Roots,
/// Fleet Ops 4.0 and 3.2.7, which never warp on an attack): read from the
/// data; the stock and Classic values are measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApproachRules {
    pub min_move_warp_distance: Fixed,
}

impl Default for ApproachRules {
    fn default() -> Self {
        ApproachRules { min_move_warp_distance: Fixed::from_int(1500) }
    }
}

impl ApproachRules {
    const KEYS: [&'static str; 1] = ["cfg_MIN_MOVE_WARP_DISTANCE"];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> Option<Self> {
        cfg.raw(Self::KEYS[0])?;
        // As the header's `float` reads (f32); clamped so the fixed point holds it.
        match cfg.f32(Self::KEYS[0]).filter(|v| v.is_finite() && *v >= 0.0) {
            Some(v) => Some(ApproachRules { min_move_warp_distance: Fixed::from_f64(v.min(1.0e9) as f64) }),
            None => {
                warnings.push(Warning::new("RTS_CFG.h", format!("{} is not a distance; the engine default is used", Self::KEYS[0])));
                None
            }
        }
    }
}

/// The other `RTS_CFG.h` keys STA2 Classic Enhanced Gameplay changes against
/// STA2 Classic (six of the eight changed lines CON-34's coverage report
/// finds in the installed headers; the other two are [`AutonomyRules`]),
/// typed so the digest names them. `None`: the header
/// does not set the key; no default is invented where the original engine's
/// own default is not known. No sim system consumes them yet; they are
/// rules, so peers must agree on them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CfgRules {
    /// `SHIELD_DAMAGE_PROTECTION` (float; Classic 0.8, Enhanced 0.9).
    pub shield_damage_protection: Option<Fixed>,
    /// `REPAIR_SHIP_DISTANCE` (float; Classic 180, Enhanced 230).
    pub repair_ship_distance: Option<Fixed>,
    /// `BASE_CREW_PER_SEC` (float; Enhanced only, 30).
    pub base_crew_per_sec: Option<Fixed>,
    /// `cfgFREIGHTERS_PER_NEBULA` (int; Classic 1, Enhanced 3).
    pub freighters_per_nebula: Option<i64>,
    /// `HARD_AI_COST_HANDICAP` (float; Classic 0.6, Enhanced 0.5).
    pub hard_ai_cost_handicap: Option<Fixed>,
    /// `HARD_AI_SPEED_HANDICAP` (float; Classic 0.7, Enhanced 0.6).
    pub hard_ai_speed_handicap: Option<Fixed>,
    /// `cfgFACING_WEIGHT` (float; 0.2 in every Armada II family header): the
    /// share of a hit's system-location weight that comes from the distance of
    /// the hit point to each system's hardpoints (SIM-39). Read and digested;
    /// the sim does not apply it (no hardpoint positions in the sim).
    pub facing_weight: Option<Fixed>,
    /// `cfgFACING_CONSTANT` (int; 3): the distance falloff of that blend. As above.
    pub facing_constant: Option<i64>,
}

impl CfgRules {
    /// The keys, in the order of [`texts`](CfgRules::texts).
    pub const KEYS: [&'static str; 8] = [
        "BASE_CREW_PER_SEC",
        "HARD_AI_COST_HANDICAP",
        "HARD_AI_SPEED_HANDICAP",
        "REPAIR_SHIP_DISTANCE",
        "SHIELD_DAMAGE_PROTECTION",
        "cfgFREIGHTERS_PER_NEBULA",
        "cfgFACING_WEIGHT",
        "cfgFACING_CONSTANT",
    ];

    /// Each value as the digest renders it (`unset` when absent), in
    /// [`KEYS`](CfgRules::KEYS) order.
    fn texts(&self) -> [String; 8] {
        let real = |v: Option<Fixed>| v.map_or_else(|| "unset".to_owned(), crate::digest::fixed_text);
        let int = |v: Option<i64>| v.map_or_else(|| "unset".to_owned(), |v| v.to_string());
        [
            real(self.base_crew_per_sec),
            real(self.hard_ai_cost_handicap),
            real(self.hard_ai_speed_handicap),
            real(self.repair_ship_distance),
            real(self.shield_damage_protection),
            int(self.freighters_per_nebula),
            real(self.facing_weight),
            int(self.facing_constant),
        ]
    }

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> CfgRules {
        let mut warn = |key: &str, what: &str| warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not {what}; treated as unset")));
        let mut real = |key: &str| {
            cfg.raw(key)?;
            // As the header's `float` reads (f32), so equal text is equal everywhere.
            let v = cfg.f32(key).filter(|v| v.is_finite() && v.abs() <= 1.0e9).map(|v| Fixed::from_f64(v as f64));
            if v.is_none() {
                warn(key, "a number");
            }
            v
        };
        let shield_damage_protection = real("SHIELD_DAMAGE_PROTECTION");
        let repair_ship_distance = real("REPAIR_SHIP_DISTANCE");
        let base_crew_per_sec = real("BASE_CREW_PER_SEC");
        let hard_ai_cost_handicap = real("HARD_AI_COST_HANDICAP");
        let hard_ai_speed_handicap = real("HARD_AI_SPEED_HANDICAP");
        let facing_weight = real("cfgFACING_WEIGHT");
        let mut int = |key: &str| {
            cfg.raw(key)?;
            let v = cfg.i64(key).map(|v| v.clamp(-1_000_000_000, 1_000_000_000));
            if v.is_none() {
                warn(key, "a whole number");
            }
            v
        };
        CfgRules {
            shield_damage_protection,
            repair_ship_distance,
            base_crew_per_sec,
            freighters_per_nebula: int("cfgFREIGHTERS_PER_NEBULA"),
            hard_ai_cost_handicap,
            hard_ai_speed_handicap,
            facing_weight,
            facing_constant: int("cfgFACING_CONSTANT"),
        }
    }
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
            resolution: Fixed::from_int(50),
        }
    }
}

impl PathingRules {
    const KEYS: [&'static str; 7] = [
        "USE_PATH_PLANNING",
        "DEFAULT_TERRAIN_VALUE",
        "IMPASSABLE_TERRAIN",
        "OFF_MAP_PATHING_COST",
        "NEBULAE_PATHING_COST",
        "NEBULAE_PATHING_SAFETY_COEFFICIENT",
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
            resolution: real("HIGHEST_QUADTREE_GRID_RESOLUTION", stock.resolution),
        })
    }

    /// Each value as the digest renders it, in [`KEYS`](PathingRules::KEYS) order.
    fn texts(&self) -> [String; 7] {
        let f = crate::digest::fixed_text;
        [
            u8::from(self.enabled).to_string(),
            self.default_cost.to_string(),
            self.impassable.to_string(),
            self.off_map_cost.to_string(),
            self.nebula_cost.to_string(),
            f(self.nebula_safety),
            f(self.resolution),
        ]
    }
}

/// A race's starting forces, as the sim gives them (`Races::loadout_with_mode`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct StartRule {
    /// The race ODF's stem: unique, unlike the race key (`observer` and
    /// `federation` share `federation`).
    stem: String,
    race: String,
    /// `instantActionSlot`; which race a key means depends on it.
    slot: Option<i64>,
    normal: [i64; 4],
    lots: [i64; 4],
    crew: [Option<i64>; 2],
    /// `standardUnits`, as the race ODF lists them.
    units: Vec<String>,
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
        if let Some(cap) = cfg.i64("STARTING_CREW_CAPACITY").filter(|c| *c >= 0) {
            rules.crew = Some(CrewRules { starting_capacity: Some(cap.min(1_000_000_000)), races: Vec::new() });
        }
        rules.autonomy = AutonomyRules::from_cfg(&cfg, &mut warnings);
        rules.approach = ApproachRules::from_cfg(&cfg, &mut warnings);
        rules.cfg = CfgRules::from_cfg(&cfg, &mut warnings);
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

    /// The team crew rules, when the content has any.
    pub fn crew(&self) -> Option<&CrewRules> {
        self.crew.as_ref()
    }

    /// Set the crew rules (tests and content built in code).
    pub fn with_crew(mut self, crew: CrewRules) -> Self {
        self.crew = Some(crew);
        self
    }

    /// The default craft autonomy: the header's, else the engine's.
    pub fn autonomy(&self) -> AutonomyRules {
        self.autonomy.unwrap_or_default()
    }

    /// Set the default autonomy (tests and content built in code).
    pub fn with_autonomy(mut self, autonomy: AutonomyRules) -> Self {
        self.autonomy = Some(autonomy);
        self
    }

    /// The attack warp distance: the header's, else the engine's.
    pub fn approach(&self) -> ApproachRules {
        self.approach.unwrap_or_default()
    }

    /// Set the attack warp distance (tests and content built in code).
    pub fn with_approach(mut self, approach: ApproachRules) -> Self {
        self.approach = Some(approach);
        self
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
                stem: race.stem.clone(),
                race: race.name.clone(),
                slot: race.slot,
                normal: race.stock.map(|n| n.clamp(0, 1_000_000_000)),
                lots: race.lots_stock.map(|n| n.clamp(0, 1_000_000_000)),
                crew: [race.normal_crew, race.lots_crew].map(|v| v.map(|n| n.clamp(0, 1_000_000_000))),
                units: race.standard_units.clone(),
            })
            .collect();
        // Stems are ODF names, unique per install; lookups go stem first.
        rules.starts.sort_by(|a, b| a.stem.cmp(&b.stem));
        rules.starts.dedup_by(|a, b| a.stem == b.stem);
        let mut rates: Vec<(String, Fixed)> = races.crew_rates().map(|(k, c)| (k.to_owned(), c.accumulation_rate)).collect();
        if !rates.is_empty() {
            rates.sort_by(|a, b| a.0.cmp(&b.0));
            rates.dedup_by(|a, b| a.0 == b.0);
            rules.crew.get_or_insert_with(CrewRules::default).races = rates;
        }
        Ok((rules, warnings))
    }

    pub fn officer_limit(&self, choice: usize) -> Option<i32> {
        self.officer_limits.get(choice).map(|limit| limit.value)
    }

    /// The rules as digest entries (D10): `RTS_CFG.h:<KEY>` for every typed
    /// header key, with the resolved value (the stock value where the header
    /// sets none and one is known, `unset` where not), and `race:<name>` for
    /// each race's starting forces. Officer labels are display text and left
    /// out; `SINGLE_PLAYER_OFFICER_LIMIT` resolves into `MP_OFFICER_LIMIT`.
    pub fn digest_entries(&self) -> Vec<DigestEntry> {
        let mut out = vec![DigestEntry::new(
            "RTS_CFG.h:MP_OFFICER_LIMIT",
            format!("[{}]", self.officer_limits.iter().map(|l| l.value.to_string()).collect::<Vec<_>>().join(",")),
        )];
        for (key, value) in PathingRules::KEYS.iter().zip(self.pathing().texts()) {
            out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value));
        }
        // Resolved values: an absent key and the engine default digest alike.
        let a = self.autonomy();
        for (key, value) in AutonomyRules::KEYS.iter().zip([a.movement, a.special_weapon]) {
            out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value.to_string()));
        }
        // Only when it differs from the engine default, so an absent key and
        // the default digest alike and content without it keeps its hash.
        if self.approach() != ApproachRules::default() {
            out.push(DigestEntry::new(
                format!("RTS_CFG.h:{}", ApproachRules::KEYS[0]),
                crate::digest::fixed_text(self.approach().min_move_warp_distance),
            ));
        }
        // The two facing keys (SIM-39) only when set, so content without them keeps its hash.
        for (i, (key, value)) in CfgRules::KEYS.iter().zip(self.cfg.texts()).enumerate() {
            if i >= 6 && value == "unset" {
                continue;
            }
            out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value));
        }
        let crew = self.crew.as_ref();
        out.push(DigestEntry::new(
            "RTS_CFG.h:STARTING_CREW_CAPACITY",
            crew.and_then(|c| c.starting_capacity).map_or_else(|| "unset".to_owned(), |c| c.to_string()),
        ));
        let rates = crew.map(|c| c.races.iter().map(|(k, r)| format!("{}:{}", quoted(k), crate::digest::fixed_text(*r))).collect::<Vec<_>>()).unwrap_or_default();
        out.push(DigestEntry::new("crew:accumulation", format!("[{}]", rates.join(","))));
        for s in &self.starts {
            let opt = |v: Option<i64>| v.map_or_else(|| "-".to_owned(), |v| v.to_string());
            let nums = |v: [i64; 4]| format!("[{}]", v.map(|n| n.to_string()).join(","));
            let value = Canon::new()
                .text("race", &s.race)
                .raw("slot", &opt(s.slot))
                .raw("normal", &nums(s.normal))
                .raw("lots", &nums(s.lots))
                .raw("crew", &opt(s.crew[0]))
                .raw("lots_crew", &opt(s.crew[1]))
                .list("units", s.units.iter().map(|u| quoted(&u.to_ascii_lowercase())))
                .finish();
            out.push(DigestEntry::new(crate::digest::key("race:", &s.stem), value));
        }
        out
    }
}

#[cfg(test)]
mod approach_tests {
    use super::*;

    #[test]
    fn the_warp_distance_is_typed_and_digested_only_off_the_engine_default() {
        let (none, _) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        assert_eq!(none.approach().min_move_warp_distance, Fixed::from_int(1500));
        let (stock, w) = Rules::from_header("float cfg_MIN_MOVE_WARP_DISTANCE = 1100.0;\n");
        assert!(w.is_empty());
        assert_eq!(stock.approach().min_move_warp_distance, Fixed::from_int(1100));
        let (fo, _) = Rules::from_header("float cfg_MIN_MOVE_WARP_DISTANCE = 999999.0;\n");
        assert_eq!(fo.approach().min_move_warp_distance, Fixed::from_int(999_999));
        let (bad, w) = Rules::from_header("float cfg_MIN_MOVE_WARP_DISTANCE = far;\n");
        assert_eq!(w.len(), 1);
        assert_eq!(bad.approach(), ApproachRules::default());
        let key = "RTS_CFG.h:cfg_MIN_MOVE_WARP_DISTANCE";
        let (explicit, _) = Rules::from_header("float cfg_MIN_MOVE_WARP_DISTANCE = 1500.0;\n");
        let digest = |r: &Rules| crate::digest::ContentDigest::from_entries(r.digest_entries());
        assert!(digest(&none).get(key).is_none());
        assert_eq!(digest(&none), digest(&explicit), "absent and the default digest alike");
        assert_eq!(digest(&stock).get(key), Some("1100"));
        assert_ne!(digest(&stock), digest(&fo));
    }
}

#[cfg(test)]
mod autonomy_tests {
    use super::*;

    #[test]
    fn absent_keys_read_as_the_engine_defaults() {
        let (rules, warnings) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        assert!(warnings.is_empty());
        assert_eq!(rules.autonomy(), AutonomyRules { movement: 1, special_weapon: 0 });
    }

    #[test]
    fn the_installed_headers_values_are_read() {
        for (text, want) in [
            ("int CRAFT_DEFAULT_MOVEMENT_AUTONOMY = 1;\nint CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY = 2;\n", (1, 2)),
            ("int CRAFT_DEFAULT_MOVEMENT_AUTONOMY = 1;\nint CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY = 1;\n", (1, 1)),
            // One key only: the other keeps the engine default.
            ("int CRAFT_DEFAULT_MOVEMENT_AUTONOMY = 2;\n", (2, 0)),
        ] {
            let (rules, warnings) = Rules::from_header(text);
            assert!(warnings.is_empty(), "{warnings:?}");
            let a = rules.autonomy();
            assert_eq!((a.movement, a.special_weapon), want, "{text}");
            let d = crate::digest::ContentDigest::from_entries(rules.digest_entries());
            assert_eq!(d.get("RTS_CFG.h:CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY"), Some(want.1.to_string().as_str()));
        }
    }

    #[test]
    fn one_valid_and_one_invalid_key_keep_the_valid_one() {
        let (rules, warnings) = Rules::from_header("int CRAFT_DEFAULT_MOVEMENT_AUTONOMY = 2;\nint CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY = 9;\n");
        assert_eq!(warnings.len(), 1);
        assert_eq!(rules.autonomy(), AutonomyRules { movement: 2, special_weapon: 0 });
        let (rules, _) = Rules::from_header("int CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY = 2;\n");
        assert_eq!(rules.autonomy(), AutonomyRules { movement: 1, special_weapon: 2 });
    }

    #[test]
    fn expressions_and_whole_floats_are_read() {
        let (rules, warnings) = Rules::from_header("int BASE = 1;\nint CRAFT_DEFAULT_MOVEMENT_AUTONOMY = BASE + 1;\nfloat CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY = 2.0;\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(rules.autonomy(), AutonomyRules { movement: 2, special_weapon: 2 });
    }

    #[test]
    fn the_content_hash_sees_values_not_presence() {
        // An absent key and the engine default are the same rule (D10).
        let plain = sta_content_fake_hash(Rules::default());
        assert_eq!(plain, sta_content_fake_hash(Rules::default().with_autonomy(AutonomyRules::default())));
        assert_ne!(
            sta_content_fake_hash(Rules::default().with_autonomy(AutonomyRules { movement: 1, special_weapon: 1 })),
            sta_content_fake_hash(Rules::default().with_autonomy(AutonomyRules { movement: 1, special_weapon: 2 })),
        );
    }

    fn sta_content_fake_hash(rules: Rules) -> u64 {
        let fake = crate::fake::db();
        let mut b = crate::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            b = b.unit(u.clone());
        }
        b.build().unwrap().with_rules(rules).hash()
    }

    #[test]
    fn a_bad_level_warns_and_keeps_the_default() {
        let (rules, warnings) = Rules::from_header("int CRAFT_DEFAULT_MOVEMENT_AUTONOMY = 7;\nint CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY = high;\n");
        assert_eq!(warnings.len(), 2);
        assert_eq!(rules.autonomy(), AutonomyRules::default());
    }
}

#[cfg(test)]
mod pathing_tests {
    use super::*;

    /// The PATHING block as Armada II and Fleet Operations ship it.
    const STOCK: &str = "// PATHING\nint OFF_MAP_PATHING_COST = 90000;\nfloat HIGHEST_QUADTREE_GRID_RESOLUTION = 50.0;\n\
        int USE_PATH_PLANNING = 1;\nint IMPASSABLE_TERRAIN = -1;\nint DEFAULT_TERRAIN_VALUE = 100;\n\
        float NEBULAE_PATHING_COST = 300.0;\nfloat NEBULAE_PATHING_SAFETY_COEFFICIENT = 1.8;\n";

    #[test]
    fn the_stock_header_matches_the_defaults() {
        let (rules, warnings) = Rules::from_header(STOCK);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(rules.pathing(), PathingRules::default());
    }

    #[test]
    fn a_header_without_the_block_digests_like_the_stock_block() {
        // Values, not presence: absent and stock resolve alike.
        let (rules, _) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        assert_eq!(rules.pathing(), PathingRules::default());
        let (stock, _) = Rules::from_header(STOCK);
        assert_eq!(rules.digest_entries(), stock.digest_entries());
        let d = crate::digest::ContentDigest::from_entries(rules.digest_entries());
        assert_eq!(d.get("RTS_CFG.h:OFF_MAP_PATHING_COST"), Some("90000"));
        assert_eq!(d.get("RTS_CFG.h:USE_PATH_PLANNING"), Some("1"));
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
        let (bad, warnings) = Rules::from_header("float NEBULAE_PATHING_SAFETY_COEFFICIENT = wide;\n");
        assert_eq!(bad.pathing().nebula_safety, PathingRules::default().nebula_safety);
        assert_eq!(warnings.len(), 1);
    }
}

#[cfg(test)]
mod cfg_tests {
    use super::*;
    use crate::digest::ContentDigest;

    /// The eight lines STA2 Classic Enhanced Gameplay changes, as both
    /// headers write them (comments as in the installed files).
    const CLASSIC: &str = "float HARD_AI_COST_HANDICAP = 0.6;\nfloat HARD_AI_SPEED_HANDICAP = 0.7;\n\
        int cfgFREIGHTERS_PER_NEBULA = 1;\nfloat SHIELD_DAMAGE_PROTECTION = 0.8;\nfloat REPAIR_SHIP_DISTANCE = 180;\n";
    const ENHANCED: &str = "float HARD_AI_COST_HANDICAP = 0.5; // JanB: decreased from 0.6\n\
        float HARD_AI_SPEED_HANDICAP = 0.6; // JanB: decreased from 0.7\nint cfgFREIGHTERS_PER_NEBULA = 3;\n\
        float SHIELD_DAMAGE_PROTECTION = 0.9; // JanB: increased from 0.8\nfloat BASE_CREW_PER_SEC = 30.0;\n\
        float REPAIR_SHIP_DISTANCE = 230;\nint CRAFT_DEFAULT_MOVEMENT_AUTONOMY = 1;\nint CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY = 1;\n";

    #[test]
    fn the_enhanced_keys_are_typed() {
        let (c, w) = Rules::from_header(CLASSIC);
        assert!(w.is_empty(), "{w:?}");
        let (e, w) = Rules::from_header(ENHANCED);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.cfg.freighters_per_nebula, Some(1));
        assert_eq!(e.cfg.freighters_per_nebula, Some(3));
        assert_eq!(e.cfg.shield_damage_protection, Some(Fixed::from_f64(0.9f32 as f64)));
        assert_eq!(e.cfg.repair_ship_distance, Some(Fixed::from_int(230)));
        assert_eq!(c.cfg.base_crew_per_sec, None);
        assert_eq!(e.cfg.base_crew_per_sec, Some(Fixed::from_int(30)));
        assert_eq!(e.autonomy(), AutonomyRules { movement: 1, special_weapon: 1 });
        // Every changed line is a differing digest key.
        let (dc, de) = (ContentDigest::from_entries(c.digest_entries()), ContentDigest::from_entries(e.digest_entries()));
        let keys: Vec<String> = dc.differences(&de).into_iter().map(|d| d.key).collect();
        let mut want: Vec<String> = CfgRules::KEYS.iter().chain(&["CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY"]).map(|k| format!("RTS_CFG.h:{k}")).collect();
        want.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        // Movement autonomy: Enhanced sets 1, the engine default.
        assert_eq!(keys, want);
        let first = dc.first_difference(&de).unwrap();
        assert_eq!((first.key.as_str(), first.ours.as_deref(), first.theirs.as_deref()), ("RTS_CFG.h:BASE_CREW_PER_SEC", Some("unset"), Some("30")));
    }

    #[test]
    fn bad_values_warn_and_stay_unset() {
        let (r, w) = Rules::from_header("float SHIELD_DAMAGE_PROTECTION = lots;\nint cfgFREIGHTERS_PER_NEBULA = 2.5;\n");
        assert_eq!((r.cfg.shield_damage_protection, r.cfg.freighters_per_nebula), (None, None));
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn every_typed_key_is_listed_once() {
        let mut keys = TYPED_CFG_KEYS.to_vec();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), TYPED_CFG_KEYS.len());
        assert!(TYPED_CFG_KEYS.iter().all(|k| !k.is_empty()));
        // The digest has one entry per typed key but the two officer keys
        // that resolve into MP_OFFICER_LIMIT, and the attack warp distance,
        // which is digested only when it differs from the engine default.
        let d = ContentDigest::from_entries(Rules::default().with_approach(ApproachRules { min_move_warp_distance: Fixed::from_int(1100) }).digest_entries());
        for k in TYPED_CFG_KEYS.iter().filter(|k| !matches!(**k, "MP_OFFICER_LIST_SIZE" | "SINGLE_PLAYER_OFFICER_LIMIT")) {
            assert!(d.get(&format!("RTS_CFG.h:{k}")).is_some(), "{k}");
        }
    }
}

#[cfg(test)]
mod crew_tests {
    use super::*;
    use crate::digest::ContentDigest;

    fn digest(text: &str) -> ContentDigest {
        ContentDigest::from_entries(Rules::from_header(text).0.digest_entries())
    }

    #[test]
    fn starting_crew_capacity_is_read_and_digested() {
        let (rules, _) = Rules::from_header("int STARTING_CREW_CAPACITY = 250;\n");
        assert_eq!(rules.crew().unwrap().starting_capacity, Some(250));
        let d = digest("int STARTING_CREW_CAPACITY = 250;\n");
        assert_eq!(d.get("RTS_CFG.h:STARTING_CREW_CAPACITY"), Some("250"));
        assert_ne!(d, digest("int STARTING_CREW_CAPACITY = 251;\n"));
        let (none, _) = Rules::from_header("int SOMETHING_ELSE = 1;\n");
        assert!(none.crew().is_none());
        assert_eq!(digest("int SOMETHING_ELSE = 1;\n").get("RTS_CFG.h:STARTING_CREW_CAPACITY"), Some("unset"));
    }

    #[test]
    fn race_accumulation_rates_are_digested() {
        let rates = |r: Vec<(String, Fixed)>| ContentDigest::from_entries(
            Rules::default().with_crew(CrewRules { starting_capacity: None, races: r }).digest_entries(),
        );
        let a = rates(vec![("fed".into(), Fixed::from_int(2))]);
        let b = rates(vec![("fed".into(), Fixed::from_int(3))]);
        assert_eq!(a.first_difference(&b).unwrap().key, "crew:accumulation");
        assert_eq!(a.get("crew:accumulation"), Some("[\"fed\":2]"));
    }
}
