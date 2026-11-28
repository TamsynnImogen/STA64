//! The typed rules of `RTS_CFG.h` (D9): officer limits, the path planner,
//! the keys Classic Enhanced changes, and each race's starting forces. No
//! raw header value reaches the simulation; every value here is an
//! entry of the content digest ([`Rules::digest_entries`], D10), except
//! [`TransporterRules`] (SIM-46 P0), which the sim does not read yet.

use crate::digest::{Canon, DigestEntry, quoted};
use crate::{RaceBoarding, RaceRepair, Races, SeatRaceRule, UnitDef, Warning};
use sta_assets::cfg_h::CfgH;
use sta_core::Fixed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficerLimit {
    /// Native label-map key; cosmetic, excluded from the rules hash.
    pub key: String,
    pub value: i32,
}

/// Armada I dogfight steering values read by the native stationary-target
/// pursuit path. An unsupported angle expression uses the executable fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A1DogfightRules {
    pub pursuit_angle: Fixed,
    pub anchor_radius: Fixed,
}

impl A1DogfightRules {
    const KEYS: [&'static str; 2] = ["ANCHOR_PURSUIT_POINT_ANGLE", "ANCHOR_RADIUS"];

    fn pursuit_angle(cfg: &CfgH) -> Option<f32> {
        let raw = cfg.raw(Self::KEYS[0])?.trim();
        if let Some(value) = cfg.f32(Self::KEYS[0]) {
            return Some(value);
        }
        // The stock header spells this as `(3.14159/6)`. CfgH's float reader
        // deliberately accepts literals only, so support this local form
        // without broadening the shared header expression parser.
        let expression = raw.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or(raw);
        let (numerator, denominator) = expression.split_once('/')?;
        let value = numerator.trim().parse::<f32>().ok()? / denominator.trim().parse::<f32>().ok()?;
        (value.is_finite()).then_some(value)
    }
}

impl Default for A1DogfightRules {
    fn default() -> Self {
        A1DogfightRules {
            pursuit_angle: Fixed::from_f64(std::f64::consts::FRAC_PI_6),
            anchor_radius: Fixed::from_int(4),
        }
    }
}

/// The typed rules. Which `RTS_CFG.h` keys are typed is data-driven: the ones
/// the sim reads and the ones the installed mods change (see [`CfgRules`]);
/// [`TYPED_CFG_KEYS`] lists them for the coverage report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    upgrade_pod_maximum_tier: Option<u32>,
    pub officer_limits: Vec<OfficerLimit>,
    starts: Vec<StartRule>,
    /// `RTS_CFG.h`'s path-planner block, when the header has any of its keys.
    pathing: Option<PathingRules>,
    /// Team crew accumulation (SIM-25), when the header or a race has a key.
    crew: Option<CrewRules>,
    /// What a team's race changes about the team (SIM-54): only the races
    /// whose rule is not the identity, lower-case key, first match wins.
    seat_races: Vec<(String, SeatRaceRule)>,
    /// Race keys that disable trading with `canTrade = 0`; all others default true.
    trade_denied: Vec<String>,
    /// Authored race repair metadata by ODF stem (SIM-40), absent when unset.
    race_repairs: Vec<(String, RaceRepair)>,
    /// `(race name, ODF stem)` for the rows above: a unit's race key is the name.
    race_repair_stems: Vec<(String, String)>,
    /// `miningRateModifier` by lower-case race key (CON-51), only races that author one.
    race_mining: Vec<(String, Fixed)>,
    /// Authored race boarding keys by lower-case race key (SIM-46 P0), only
    /// the races that set any. The three the sim reads are digested (SIM-69).
    race_boarding: Vec<(String, RaceBoarding)>,
    /// `RTS_CFG.h`'s default craft autonomy, when the header sets either key.
    autonomy: Option<AutonomyRules>,
    /// `RTS_CFG.h`'s attack warp distance (SIM-36), when the header sets it.
    approach: Option<ApproachRules>,
    /// `RTS_CFG.h`'s [`WARP_KEY`], when the header sets it: see [`Rules::warp_enabled`].
    warp: Option<bool>,
    /// Planet population levels and growth (SIM-30), when the header sets all of them.
    population: Option<PopulationRules>,
    /// `RTS_CFG.h`'s transporter keys (SIM-46 P0), when the header sets any; digested (SIM-69).
    transporter: Option<TransporterRules>,
    /// The other typed `RTS_CFG.h` keys.
    pub cfg: CfgRules,
    /// The render-only shot keys (not digested).
    pub shots: ShotCfg,
    /// Warp-suppression multipliers shared by the minimap and simulation.
    pub fields: FieldCfg,
    /// Armada I's header-level start (`cfgStarting*`), see [`LegacyStart`].
    legacy_start: LegacyStart,
    /// Explicit `cfgMaxOfficers`, independently of the A2 officer limit keys.
    a1_max_officers: Option<i32>,
    a1_dogfight: A1DogfightRules,
    /// The group-movement keys (CON-59), as the header authors them.
    group: GroupCfg,
}

const TYPED_LEN: usize = 6 + A1DogfightRules::KEYS.len() + PathingRules::KEYS.len() + AutonomyRules::KEYS.len() + ApproachRules::KEYS.len() + CfgRules::KEYS.len() + LegacyStart::KEYS.len() + PopulationRules::KEYS.len() + FieldCfg::KEYS.len() + GroupCfg::KEYS.len();

/// Every `RTS_CFG.h` key [`Rules`] reads.
pub const TYPED_CFG_KEYS: [&str; TYPED_LEN] = {
    let mut out = [""; TYPED_LEN];
    out[0] = "upgradePodMaximumTier";
    out[1] = "MP_OFFICER_LIMIT";
    out[2] = "MP_OFFICER_LIST_SIZE";
    out[3] = "SINGLE_PLAYER_OFFICER_LIMIT";
    out[4] = "STARTING_CREW_CAPACITY";
    out[5] = WARP_KEY;
    let mut i = 0;
    while i < PathingRules::KEYS.len() {
        out[6 + i] = PathingRules::KEYS[i];
        i += 1;
    }
    let mut a = 0;
    while a < AutonomyRules::KEYS.len() {
        out[6 + i + a] = AutonomyRules::KEYS[a];
        a += 1;
    }
    let mut w = 0;
    while w < ApproachRules::KEYS.len() {
        out[6 + i + a + w] = ApproachRules::KEYS[w];
        w += 1;
    }
    let mut j = 0;
    while j < CfgRules::KEYS.len() {
        out[6 + i + a + w + j] = CfgRules::KEYS[j];
        j += 1;
    }
    let mut l = 0;
    while l < LegacyStart::KEYS.len() {
        out[6 + i + a + w + j + l] = LegacyStart::KEYS[l];
        l += 1;
    }
    let mut p = 0;
    while p < PopulationRules::KEYS.len() {
        out[6 + i + a + w + j + l + p] = PopulationRules::KEYS[p];
        p += 1;
    }
    let mut f = 0;
    while f < FieldCfg::KEYS.len() {
        out[6 + i + a + w + j + l + p + f] = FieldCfg::KEYS[f];
        f += 1;
    }
    let mut g = 0;
    while g < GroupCfg::KEYS.len() {
        out[6 + i + a + w + j + l + p + f + g] = GroupCfg::KEYS[g];
        g += 1;
    }
    out[TYPED_LEN - 2] = A1DogfightRules::KEYS[0];
    out[TYPED_LEN - 1] = A1DogfightRules::KEYS[1];
    out
};

/// Team crew rules: `STARTING_CREW_CAPACITY` from `RTS_CFG.h` and each race's
/// `crewAccumulationRate` (crew per second per starbase).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CrewRules {
    /// What a team can hold with no starbase or planet (`STARTING_CREW_CAPACITY`).
    pub starting_capacity: Option<i64>,
    /// `MAX_CREW`: the most a team's crew capacity can grow to, and what the
    /// A2 script call `GetMaxCrew` returns for any handle (SIM-54).
    pub max_crew: Option<i64>,
    /// Race key (lower case, sorted) and its accumulation rate per second.
    pub races: Vec<(String, Fixed)>,
}

/// The two `RTS_CFG.h` keys that only change how weapon shots are drawn. Render only: not
/// digested and not in [`TYPED_CFG_KEYS`] (D9, D10: peers that differ only in how a shot
/// looks play together). `None`: the header does not set the key; the engine's own default
/// for an absent key was not read (INTERIM: a render reads an absent colour flag as off, no
/// team colour, and invents no miss lifetime).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShotCfg {
    /// `BEAM_MISS_LIFETIME` (float, seconds): how long, counted from the shot, a beam whose
    /// target is lost keeps being drawn towards where it pointed (read from the executable,
    /// static; the value is read from data).
    pub beam_miss_lifetime: Option<Fixed>,
    /// `NORMAL_WEAPON_TEAM_COLOR` (flag): when set, phasers, pulses and photon, poleron and
    /// shield-removing torpedoes take the owner's team colour, else they are white (read
    /// from the executable, static; the value is read from data).
    pub normal_weapon_team_color: Option<bool>,
}

impl ShotCfg {
    pub const KEYS: [&'static str; 2] = ["BEAM_MISS_LIFETIME", "NORMAL_WEAPON_TEAM_COLOR"];

    /// Whether normal weapons take the owner's team colour (absent: no, INTERIM).
    pub fn team_colour(&self) -> bool {
        self.normal_weapon_team_color.unwrap_or(false)
    }

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> ShotCfg {
        let mut out = ShotCfg::default();
        if cfg.raw("BEAM_MISS_LIFETIME").is_some() {
            match cfg.f32("BEAM_MISS_LIFETIME").filter(|v| v.is_finite() && (0.0..=1.0e4).contains(v)) {
                Some(v) => out.beam_miss_lifetime = Some(Fixed::from_f64(v as f64)),
                None => warnings.push(Warning::new("RTS_CFG.h", "BEAM_MISS_LIFETIME is not a non-negative number; treated as unset")),
            }
        }
        if cfg.raw("NORMAL_WEAPON_TEAM_COLOR").is_some() {
            match cfg.f32("NORMAL_WEAPON_TEAM_COLOR").filter(|v| v.is_finite()) {
                Some(v) => out.normal_weapon_team_color = Some(v != 0.0),
                None => warnings.push(Warning::new("RTS_CFG.h", "NORMAL_WEAPON_TEAM_COLOR is not a number; treated as unset")),
            }
        }
        out
    }
}

/// The two `RTS_CFG.h` keys that size the warp-suppression fields the minimap draws: a planet's
/// bounding radius and a black hole's, each times its multiplier (read from the executable,
/// static; the values are read from data). Both keys are digested when authored and listed in
/// [`TYPED_CFG_KEYS`] because the simulation reads the field radius.
/// `None`: the header does not set the key, and no field is drawn for that class (the engine's
/// own default for an absent key was not read; INTERIM).
///
/// The whole rule (which classes have a field, and its radius) lives in one place,
/// [`Rules::warp_suppression_radius`], shared by the minimap and the no-warp rule (issue #1012).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldCfg {
    /// `cfgPLANET_DEWARP_RADIUS_MULTIPLIER`.
    pub planet_dewarp_radius_multiplier: Option<Fixed>,
    /// `cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER`.
    pub black_hole_dewarp_radius_multiplier: Option<Fixed>,
}

impl FieldCfg {
    pub const KEYS: [&'static str; 2] = ["cfgPLANET_DEWARP_RADIUS_MULTIPLIER", "cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER"];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> FieldCfg {
        let mut read = |key: &str| -> Option<Fixed> {
            cfg.raw(key)?;
            match cfg.f32(key).filter(|v| v.is_finite() && (0.0..=1.0e4).contains(v)) {
                Some(v) => Some(Fixed::from_f64(v as f64)),
                None => {
                    warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a non-negative number; treated as unset")));
                    None
                }
            }
        };
        FieldCfg { planet_dewarp_radius_multiplier: read(Self::KEYS[0]), black_hole_dewarp_radius_multiplier: read(Self::KEYS[1]) }
    }
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

/// Planet population rules (SIM-30), from `RTS_CFG.h`: the population a planet
/// of each level holds (`cfgPOP_SPARSE`, `cfgPOP_LIGHT`, `cfgPOP_MEDIUM`,
/// `cfgPOP_HEAVY`, people) and `cfgPLANET_CIVILIAN_RATE_MODIFIER`. The engine's
/// own defaults are overwritten by the header (read from the executable,
/// static); all five keys must be present, otherwise `None` and the colony
/// rules are inactive. The A2 family installs set 1000/2000/3000/5000 and 0.20
/// (read from data); Armada I and the Fleet Ops line have no population system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopulationRules {
    /// People of a planet at level 1..=4 (sparse, light, medium, heavy).
    pub levels: [Fixed; 4],
    /// `cfgPLANET_CIVILIAN_RATE_MODIFIER`.
    pub civilian_rate_modifier: Fixed,
}

impl PopulationRules {
    const KEYS: [&'static str; 5] =
        ["cfgPOP_SPARSE", "cfgPOP_LIGHT", "cfgPOP_MEDIUM", "cfgPOP_HEAVY", "cfgPLANET_CIVILIAN_RATE_MODIFIER"];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> Option<Self> {
        if !Self::KEYS.iter().any(|k| cfg.raw(k).is_some()) {
            return None;
        }
        let mut vals = [Fixed::ZERO; 5];
        for (v, key) in vals.iter_mut().zip(Self::KEYS) {
            match cfg.f32(key).filter(|x| x.is_finite() && *x >= 0.0) {
                Some(x) => *v = Fixed::from_f64(x.min(1.0e9) as f64),
                None => {
                    warnings.push(Warning::new("RTS_CFG.h", format!("{key} is missing or not a non-negative number; the population rules are inactive")));
                    return None;
                }
            }
        }
        Some(PopulationRules { levels: [vals[0], vals[1], vals[2], vals[3]], civilian_rate_modifier: vals[4] })
    }

    /// People a planet of `level` (0..=4) holds at most; level 0 holds none.
    pub fn cap(&self, level: u8) -> Fixed {
        match level {
            1..=4 => self.levels[usize::from(level) - 1],
            _ => Fixed::ZERO,
        }
    }
}

/// The rules key of the content-level warp switch (#1102): see [`Rules::warp_enabled`].
pub const WARP_KEY: &str = "GAME_OPTIONS_WARP";

/// Whether content has warp when its header does not set [`WARP_KEY`]: yes, for every profile.
/// The one source of that default. The data decides: content without warp says so with the key.
/// Grades: the stock Armada II profile has warp (read from the executable, and measured); the
/// other profiles keep what STA64 did before it read the key (Fleet Operations 3 and the native
/// profile: not checked).
pub const WARP_DEFAULT: bool = true;

/// [`WARP_KEY`] as the header authors it: a whole number, 0 for off and any other for on; `None`
/// when it is absent. Anything that is not a whole number warns and counts as absent.
fn warp_from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> Option<bool> {
    cfg.raw(WARP_KEY)?;
    // Whole numbers only, but `1.0` counts as 1 as for the other whole-number keys.
    match cfg.i64(WARP_KEY).or_else(|| cfg.f32(WARP_KEY).filter(|v| v.is_finite() && v.fract() == 0.0).map(|v| v as i64)) {
        Some(v) => Some(v != 0),
        None => {
            warnings.push(Warning::new("RTS_CFG.h", format!("{WARP_KEY} is not a whole number; the default is used")));
            None
        }
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
///
/// A second key, `cfgTOO_CLOSE_TO_WARP`, is the same for a warp move (#1102, D10): the craft
/// asks for warp only while it is strictly farther than this from the point it flies to,
/// centre to point (read from the executable, static; not measured). INTERIM: where the
/// header does not set it, [`ApproachRules::DEFAULT_TOO_CLOSE_TO_WARP`], the distance STA64's
/// warp move used before it read the key (the original's own default is not settled), so
/// content without the key behaves as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApproachRules {
    pub min_move_warp_distance: Fixed,
    pub too_close_to_warp: Fixed,
}

impl Default for ApproachRules {
    fn default() -> Self {
        ApproachRules { min_move_warp_distance: Fixed::from_int(1500), too_close_to_warp: Self::DEFAULT_TOO_CLOSE_TO_WARP }
    }
}

impl ApproachRules {
    const KEYS: [&'static str; 2] = ["cfg_MIN_MOVE_WARP_DISTANCE", "cfgTOO_CLOSE_TO_WARP"];

    /// INTERIM default of `cfgTOO_CLOSE_TO_WARP`: STA64's own, see the type's documentation.
    pub const DEFAULT_TOO_CLOSE_TO_WARP: Fixed = Fixed::from_int(800);

    /// `None` when the header sets neither key. A key that is set but is no distance warns
    /// and keeps its default; the other key is still read.
    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> Option<Self> {
        let mut read = |key: &str, default: &str| -> Option<Fixed> {
            cfg.raw(key)?;
            // As the header's `float` reads (f32); clamped so the fixed point holds it.
            let value = cfg.f32(key).filter(|v| v.is_finite() && *v >= 0.0).map(|v| Fixed::from_f64(v.min(1.0e9) as f64));
            if value.is_none() {
                warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a distance; {default} is used")));
            }
            value
        };
        let attack = read(Self::KEYS[0], "the engine default");
        let warp_move = read(Self::KEYS[1], "STA64's default");
        if attack.is_none() && warp_move.is_none() {
            return None;
        }
        let d = ApproachRules::default();
        Some(ApproachRules {
            min_move_warp_distance: attack.unwrap_or(d.min_move_warp_distance),
            too_close_to_warp: warp_move.unwrap_or(d.too_close_to_warp),
        })
    }
}

/// The transporter keys of `RTS_CFG.h` (SIM-46 P0): reach, delays and the
/// number of simultaneous transports. Defaults are the engine's own when the
/// header leaves a key out (read from the executable, static; Armada I's
/// executable was not analysed). The header's values are read from the data
/// and differ between games and mods.
///
/// The transport rules of the sim read all four (SIM-69), so a header that
/// sets any is in the content digest ([`Rules::digest_entries`]); a header
/// without them digests as before. The coverage report still lists a mod's
/// change as ignored until the keys join `TYPED_CFG_KEYS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransporterRules {
    /// `TRANSPORTER_RANGE` (float, 300).
    pub range: Fixed,
    /// `TRANSPORTER_DELAY`, seconds (float, 1.0).
    pub delay: Fixed,
    /// `TRANSPORTER_ACTION_DELAY`, seconds (float, 1.0).
    pub action_delay: Fixed,
    /// `TRANSPORTER_MAX` (int, 5).
    pub max: i64,
}

impl Default for TransporterRules {
    fn default() -> Self {
        TransporterRules { range: Fixed::from_int(300), delay: Fixed::ONE, action_delay: Fixed::ONE, max: 5 }
    }
}

impl TransporterRules {
    /// The header keys, in field order.
    const KEYS: [&'static str; 4] = ["TRANSPORTER_RANGE", "TRANSPORTER_DELAY", "TRANSPORTER_ACTION_DELAY", "TRANSPORTER_MAX"];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> Option<Self> {
        if !Self::KEYS.iter().any(|k| cfg.raw(k).is_some()) {
            return None;
        }
        let d = Self::default();
        let mut warn = |key: &str, what: &str| warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not {what}; the engine default is used")));
        let mut real = |key: &str, default: Fixed| {
            if cfg.raw(key).is_none() {
                return default;
            }
            // As the header's `float` reads (f32); clamped so the fixed point holds it.
            match cfg.f32(key).filter(|v| v.is_finite() && *v >= 0.0) {
                Some(v) => Fixed::from_f64(f64::from(v.min(1.0e9))),
                None => {
                    warn(key, "a non-negative number");
                    default
                }
            }
        };
        let (range, delay, action_delay) = (real(Self::KEYS[0], d.range), real(Self::KEYS[1], d.delay), real(Self::KEYS[2], d.action_delay));
        let max = if cfg.raw(Self::KEYS[3]).is_none() {
            d.max
        } else {
            match cfg.i64(Self::KEYS[3]).filter(|v| *v >= 0) {
                Some(v) => v.min(1_000_000),
                None => {
                    warnings.push(Warning::new("RTS_CFG.h", format!("{} is not a non-negative whole number; the engine default is used", Self::KEYS[3])));
                    d.max
                }
            }
        };
        Some(TransporterRules { range, delay, action_delay, max })
    }
}

/// What a seat starts with when its race ODF authors no start (`normal*`,
/// `lots*`): Armada I's `RTS_CFG.h` carries one start for every race,
/// `cfgStartingDilithium` and `cfgStartingCrew`, and its officer maximum,
/// `cfgMaxOfficers`. Armada I has dilithium and crew only (officers are a cap);
/// its race ODFs have none of Armada II's `normaldilithium` keys. Keys read
/// from the stock data and named by the executable (static);
/// that Instant Action grants exactly these is not measured on a running
/// Armada I. The executable reads the dilithium maximum when either start
/// key is missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyStart {
    /// `cfgStartingDilithium`: the dilithium stock of a seat.
    pub dilithium: Option<i64>,
    /// `cfgStartingCrew`: the team crew stock of a seat.
    pub crew: Option<i64>,
    /// `cfgMaxOfficers`: the officer cap of a seat.
    pub officers: Option<i64>,
}

impl LegacyStart {
    const KEYS: [&'static str; 4] = ["cfgStartingCrew", "cfgStartingDilithium", "cfgMaxOfficers", "cfgMaxDilithium"];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> LegacyStart {
        let mut read = |key: &str| {
            cfg.raw(key)?;
            let v = cfg.i64(key).filter(|v| *v >= 0).map(|v| v.min(1_000_000_000));
            if v.is_none() {
                warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a nonnegative integer; treated as unset")));
            }
            v
        };
        let max_dilithium = cfg.i64("cfgMaxDilithium").filter(|v| *v >= 0).map(|v| v.min(1_000_000_000));
        let mut start = |key| read(key).or_else(|| max_dilithium.filter(|_| cfg.raw(key).is_none()));
        LegacyStart { crew: start("cfgStartingCrew"), dilithium: start("cfgStartingDilithium"), // Same precedence as the officer list: `cfgMaxOfficers` counts only when no
        // Armada II style officer key is present.
        officers: if cfg.raw("MP_OFFICER_LIMIT").is_none() && cfg.raw("SINGLE_PLAYER_OFFICER_LIMIT").is_none() {
            cfg.i64("cfgMaxOfficers").filter(|v| *v >= 0).map(|v| v.min(1_000_000_000))
        } else {
            None
        } }
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
    /// `BASE_CREW_PER_SEC` (float; Enhanced only, 30). See
    /// [`yard_crew_per_sec`](CfgRules::yard_crew_per_sec).
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
    /// `DECONSTRUCT_PERCENT_TIME` (float; A2 0.67, Roots 0.3): a recycle takes this share of
    /// the unit's build time (SIM-45, read from the executable, static).
    pub deconstruct_percent_time: Option<Fixed>,
    /// `cfgBIOMATTER_RECYCLE_FRACTION` (float; A2 0, Roots 0.5): the share of the biomatter cost
    /// a recycle returns (SIM-45, read from the executable, static).
    pub biomatter_recycle_fraction: Option<Fixed>,
}

impl CfgRules {
    /// The engine's yard crew refill rate when `RTS_CFG.h` does not set
    /// `BASE_CREW_PER_SEC` (read from the executable, static).
    pub const DEFAULT_CREW_PER_SEC: i32 = 20;

    /// Crew per second a yard refills (`BASE_CREW_PER_SEC`, D9): the header's
    /// value, else 20. A negative value reads as 0. No sim system refills
    /// crew at a yard yet; this is the rate it will use.
    pub fn yard_crew_per_sec(&self) -> Fixed {
        self.base_crew_per_sec.unwrap_or(Fixed::from_int(Self::DEFAULT_CREW_PER_SEC)).max(Fixed::ZERO)
    }

    /// The keys, in the order of [`texts`](CfgRules::texts).
    pub const KEYS: [&'static str; 10] = [
        "BASE_CREW_PER_SEC",
        "HARD_AI_COST_HANDICAP",
        "HARD_AI_SPEED_HANDICAP",
        "REPAIR_SHIP_DISTANCE",
        "SHIELD_DAMAGE_PROTECTION",
        "cfgFREIGHTERS_PER_NEBULA",
        "cfgFACING_WEIGHT",
        "cfgFACING_CONSTANT",
        "DECONSTRUCT_PERCENT_TIME",
        "cfgBIOMATTER_RECYCLE_FRACTION",
    ];

    /// Each value as the digest renders it (`unset` when absent), in
    /// [`KEYS`](CfgRules::KEYS) order.
    fn texts(&self) -> [String; 10] {
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
            real(self.deconstruct_percent_time),
            real(self.biomatter_recycle_fraction),
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
        let deconstruct_percent_time = real("DECONSTRUCT_PERCENT_TIME");
        let biomatter_recycle_fraction = real("cfgBIOMATTER_RECYCLE_FRACTION");
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
            deconstruct_percent_time,
            biomatter_recycle_fraction,
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

/// The group-movement keys of `RTS_CFG.h` (CON-59, ADR 0011 G12) as the header
/// authors them: `None` where it sets none (the profile's default then applies,
/// see [`GroupRules`]). Read from data; the keys are digested when they differ
/// from the profile's default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupCfg {
    /// `FORMATION_SPACING`: the unit formation files are written in, world units.
    pub formation_spacing: Option<Fixed>,
    /// `FORMATION_LEADER_SLOWDOWN`: the factor the group leader's speed is held
    /// to (G7).
    pub leader_slowdown: Option<Fixed>,
    /// `cfgDEFAULT_TO_FORMATION_MOVE`: the default move mode of the Armada II
    /// family, true for the formation move (G4). Read by every profile but
    /// Armada I.
    pub default_to_formation_move: Option<bool>,
    /// `DEFAULT_TO_FORMATION_MOVE`: the same rule under Armada I's spelling
    /// (G4); read by Armada I only, the other spelling is ignored there.
    pub a1_default_to_formation_move: Option<bool>,
    /// `FORMATIONS_RELATIVE_TO_SHIP`: Armada I's rule (ADR 0011 G8, a profile
    /// rule under ADR 0004 D2); read and digested under Armada I only. INTERIM:
    /// whether another profile has the rule is not established.
    pub formations_relative_to_ship: Option<bool>,
}

impl GroupCfg {
    pub const KEYS: [&'static str; 5] = [
        "FORMATION_SPACING",
        "FORMATION_LEADER_SLOWDOWN",
        "cfgDEFAULT_TO_FORMATION_MOVE",
        "DEFAULT_TO_FORMATION_MOVE",
        "FORMATIONS_RELATIVE_TO_SHIP",
    ];

    fn from_cfg(cfg: &CfgH, warnings: &mut Vec<Warning>) -> GroupCfg {
        let mut real = |key: &str, max: f32| -> Option<Fixed> {
            cfg.raw(key)?;
            match cfg.f32(key).filter(|v| v.is_finite() && *v > 0.0 && *v <= max) {
                Some(v) => Some(Fixed::from_f64(f64::from(v))),
                None => {
                    warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a positive number; the profile default is used")));
                    None
                }
            }
        };
        let formation_spacing = real(Self::KEYS[0], 1.0e4);
        let leader_slowdown = real(Self::KEYS[1], 100.0);
        let mut flag = |key: &str| -> Option<bool> {
            cfg.raw(key)?;
            match cfg.f32(key).filter(|v| v.is_finite()) {
                Some(v) => Some(v != 0.0),
                None => {
                    warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a number; the profile default is used")));
                    None
                }
            }
        };
        let default_to_formation_move = flag(Self::KEYS[2]);
        let a1_default_to_formation_move = flag(Self::KEYS[3]);
        let formations_relative_to_ship = flag(Self::KEYS[4]);
        GroupCfg { formation_spacing, leader_slowdown, default_to_formation_move, a1_default_to_formation_move, formations_relative_to_ship }
    }
}

/// The group-movement rules the simulation reads (ADR 0011 G12), resolved for a
/// profile: the header's keys ([`GroupCfg`]) over the profile's defaults, and
/// the constants the executables carry, which no header sets.
///
/// Evidence of the defaults: the leader slowdown and default move mode per
/// profile are the values ADR 0011 G4 and G7 name (read from data). The spacing
/// default is needed so that formation files can be scaled when a header sets
/// no spacing; its number was read from data while building CON-59, not from
/// the ADR. What the engine does for an absent key was not read, so all key
/// defaults are INTERIM. The constants are the ones ADR 0011 names (read from
/// the executable, static; not measured) and are the same for every profile
/// until TOOL-26 and TOOL-27 say otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupRules {
    /// `FORMATION_SPACING`, world units.
    pub formation_spacing: Fixed,
    /// `FORMATION_LEADER_SLOWDOWN`.
    pub leader_slowdown: Fixed,
    /// The default move mode: true is the formation move (G4).
    pub default_to_formation_move: bool,
    /// Armada I's `FORMATIONS_RELATIVE_TO_SHIP` (G8); false under every other
    /// profile whatever the header says (interim, see [`GroupCfg`]).
    pub formations_relative_to_ship: bool,
    /// A craft joins a cluster whose centre is this near (G1).
    pub cluster_radius: Fixed,
    /// A follower farther than this from its slot point (G6)...
    pub stuck_far_distance: Fixed,
    /// ...that gained less than this...
    pub stuck_far_progress: Fixed,
    /// ...in this many seconds plans for itself.
    pub stuck_far_seconds: u32,
    /// Or farther than this with no progress...
    pub stuck_near_distance: Fixed,
    /// ...in this many seconds.
    pub stuck_near_seconds: u32,
    /// A follower plans again when its slot point has moved more than this.
    pub replan_distance: Fixed,
    /// The group is dissolved when this share, in percent, of its members...
    pub dissolve_share_percent: u32,
    /// ...is within this distance of the destination (G10).
    pub dissolve_distance: Fixed,
}

impl GroupRules {
    /// The defaults of `profile`: the key defaults (INTERIM, see the type) and
    /// the executables' constants. Native takes the Armada II family's, as
    /// STA64's own rules start from them.
    pub fn defaults(profile: crate::Profile) -> GroupRules {
        use crate::Profile::{Fo3, Sta1Classic};
        GroupRules {
            // INTERIM, read from data (see the type); not a measured rule.
            formation_spacing: Fixed::from_int(100),
            leader_slowdown: if profile == Sta1Classic { Fixed::from_f64(0.9f32 as f64) } else { Fixed::ONE },
            default_to_formation_move: !matches!(profile, Fo3 | Sta1Classic),
            formations_relative_to_ship: false,
            cluster_radius: Fixed::from_int(1200),
            stuck_far_distance: Fixed::from_int(1000),
            stuck_far_progress: Fixed::from_int(100),
            stuck_far_seconds: 8,
            stuck_near_distance: Fixed::from_int(500),
            stuck_near_seconds: 16,
            replan_distance: Fixed::from_int(1500),
            dissolve_share_percent: 75,
            dissolve_distance: Fixed::from_int(700),
        }
    }

    fn resolved(cfg: &GroupCfg, profile: crate::Profile) -> GroupRules {
        let d = GroupRules::defaults(profile);
        GroupRules {
            formation_spacing: cfg.formation_spacing.unwrap_or(d.formation_spacing),
            leader_slowdown: cfg.leader_slowdown.unwrap_or(d.leader_slowdown),
            default_to_formation_move: if profile == crate::Profile::Sta1Classic { cfg.a1_default_to_formation_move } else { cfg.default_to_formation_move }
                .unwrap_or(d.default_to_formation_move),
            formations_relative_to_ship: profile == crate::Profile::Sta1Classic && cfg.formations_relative_to_ship.unwrap_or(d.formations_relative_to_ship),
            ..d
        }
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
    /// Maximum effective ship-system pod tier (A2FO extension, default 6).
    pub fn upgrade_pod_maximum_tier(&self) -> u32 {
        self.upgrade_pod_maximum_tier.unwrap_or(6)
    }

    fn read_upgrade_pod_maximum_tier(&mut self, cfg: &CfgH, warnings: &mut Vec<Warning>) {
        let key = "upgradePodMaximumTier";
        if cfg.raw(key).is_none() { return; }
        match cfg.i64(key).filter(|v| (3..=16).contains(v)) {
            Some(value) => self.upgrade_pod_maximum_tier = Some(value as u32),
            None => warnings.push(Warning::new("RTS_CFG.h", format!("{key} is not a whole number from 3 to 16; last valid value retained"))),
        }
    }

    pub fn from_header(text: &str) -> (Self, Vec<Warning>) {
        let cfg = CfgH::parse(text);
        let mut warnings = Vec::new();
        let mut rules = Self::default();
        rules.read_upgrade_pod_maximum_tier(&cfg, &mut warnings);
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
        rules.legacy_start = LegacyStart::from_cfg(&cfg, &mut warnings);
        let mut dogfight = A1DogfightRules::default();
        if cfg.raw(A1DogfightRules::KEYS[0]).is_some() {
            match A1DogfightRules::pursuit_angle(&cfg).filter(|v| *v >= 0.0) {
                Some(v) => dogfight.pursuit_angle = Fixed::from_f64(f64::from(v).min(1.0e9)),
                None => warnings.push(Warning::new("RTS_CFG.h", format!("{} is not a non-negative numeric angle; using the executable default", A1DogfightRules::KEYS[0]))),
            }
        }
        if cfg.raw(A1DogfightRules::KEYS[1]).is_some() {
            match cfg.f32(A1DogfightRules::KEYS[1]).filter(|v| v.is_finite() && *v >= 0.0) {
                Some(v) => dogfight.anchor_radius = Fixed::from_f64(f64::from(v).min(1.0e9)),
                None => warnings.push(Warning::new("RTS_CFG.h", format!("{} is not a non-negative numeric literal; using the executable default", A1DogfightRules::KEYS[1]))),
            }
        }
        rules.a1_dogfight = dogfight;
        rules.a1_max_officers = cfg.raw("cfgMaxOfficers")
            .and_then(|_| cfg.i64("cfgMaxOfficers"))
            .and_then(|n| i32::try_from(n).ok())
            .filter(|n| *n >= 0);
        // Armada I has no multiplayer officer list: `cfgMaxOfficers` is the one cap.
        if rules.officer_limits.is_empty() && cfg.raw("MP_OFFICER_LIMIT").is_none() && cfg.raw("SINGLE_PLAYER_OFFICER_LIMIT").is_none() && cfg.raw("cfgMaxOfficers").is_some() {
            match cfg.i64("cfgMaxOfficers").and_then(|v| i32::try_from(v).ok()).filter(|v| *v >= 0) {
                Some(value) => rules.officer_limits.push(OfficerLimit { key: "ol_normal".into(), value }),
                None => warnings.push(Warning::new("RTS_CFG.h", "invalid cfgMaxOfficers; officer maximum unavailable")),
            }
        }
        rules.pathing = PathingRules::from_cfg(&cfg, &mut warnings);
        let starting = cfg.i64("STARTING_CREW_CAPACITY").filter(|c| *c >= 0).map(|c| c.min(1_000_000_000));
        let max_crew = cfg.i64("MAX_CREW").filter(|c| *c >= 0).map(|c| c.min(1_000_000_000));
        if starting.is_some() || max_crew.is_some() {
            rules.crew = Some(CrewRules { starting_capacity: starting, max_crew, races: Vec::new() });
        }
        rules.population = PopulationRules::from_cfg(&cfg, &mut warnings);
        rules.autonomy = AutonomyRules::from_cfg(&cfg, &mut warnings);
        rules.approach = ApproachRules::from_cfg(&cfg, &mut warnings);
        rules.warp = warp_from_cfg(&cfg, &mut warnings);
        rules.cfg = CfgRules::from_cfg(&cfg, &mut warnings);
        rules.shots = ShotCfg::from_cfg(&cfg, &mut warnings);
        rules.fields = FieldCfg::from_cfg(&cfg, &mut warnings);
        rules.group = GroupCfg::from_cfg(&cfg, &mut warnings);
        rules.transporter = TransporterRules::from_cfg(&cfg, &mut warnings);
        (rules, warnings)
    }

    /// The group-movement rules for `profile`: the header's keys over the
    /// profile's defaults (CON-59, ADR 0011 G12).
    pub fn group_rules(&self, profile: crate::Profile) -> GroupRules {
        GroupRules::resolved(&self.group, profile)
    }

    /// The group-movement keys as the header authors them.
    pub fn group_cfg(&self) -> GroupCfg {
        self.group
    }

    /// Set the authored group-movement keys (tests and content built in code).
    pub fn with_group_cfg(mut self, group: GroupCfg) -> Self {
        self.group = group;
        self
    }

    /// The group-movement rules as digest entries (D10), for `profile`: one
    /// `RTS_CFG.h:<KEY>` entry per rule whose value is not the profile's
    /// default, so an absent key and the default digest alike and content that
    /// leaves them out keeps its hash. The default-move key is named
    /// `cfgDEFAULT_TO_FORMATION_MOVE` whichever spelling the profile reads;
    /// the relative-to-ship key digests under Armada I only.
    pub fn group_digest_entries(&self, profile: crate::Profile) -> Vec<DigestEntry> {
        let (d, r) = (GroupRules::defaults(profile), self.group_rules(profile));
        let mut out = Vec::new();
        let mut put = |key: &str, changed: bool, value: String| {
            if changed {
                out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value));
            }
        };
        put(GroupCfg::KEYS[0], r.formation_spacing != d.formation_spacing, crate::digest::fixed_text(r.formation_spacing));
        put(GroupCfg::KEYS[1], r.leader_slowdown != d.leader_slowdown, crate::digest::fixed_text(r.leader_slowdown));
        put(GroupCfg::KEYS[2], r.default_to_formation_move != d.default_to_formation_move, u8::from(r.default_to_formation_move).to_string());
        put(GroupCfg::KEYS[4], r.formations_relative_to_ship != d.formations_relative_to_ship, u8::from(r.formations_relative_to_ship).to_string());
        out
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

    /// What a team of `race` (a unit race key, any case) changes about the
    /// team: the identity for a race with no authored or derived rule.
    pub fn seat_race(&self, race: &str) -> SeatRaceRule {
        let race = race.trim();
        self.seat_races.iter().find(|(k, _)| k.eq_ignore_ascii_case(race)).map_or_else(SeatRaceRule::default, |(_, r)| *r)
    }

    /// Set the seat race rules (tests and content built in code).
    pub fn with_seat_races(mut self, rules: Vec<(String, SeatRaceRule)>) -> Self {
        self.seat_races = rules;
        self
    }

    /// Whether the race can trade; races without an explicit denial default true.
    pub fn can_trade(&self, race: &str) -> bool {
        let race = race.trim();
        !self.trade_denied.iter().any(|k| k.eq_ignore_ascii_case(race))
    }

    /// Set races explicitly denied from trading (tests and content built in code).
    pub fn with_trade_denied(mut self, races: Vec<String>) -> Self {
        let mut races: Vec<_> = races.into_iter().map(|k| k.trim().to_ascii_lowercase()).filter(|k| !k.is_empty()).collect();
        races.sort();
        races.dedup();
        self.trade_denied = races;
        self
    }

    /// The race's `miningRateModifier`, `None` when the race authors none (the
    /// engine default was not read). Content only: the sim's use of it is SIM-72's.
    pub fn race_mining_rate_modifier(&self, race: &str) -> Option<Fixed> {
        let race = race.trim();
        self.race_mining.iter().find(|(k, _)| k.eq_ignore_ascii_case(race)).map(|(_, v)| *v)
    }

    /// Set the race mining modifiers (tests and content built in code).
    pub fn with_race_mining(mut self, rows: Vec<(String, Fixed)>) -> Self {
        self.race_mining = rows;
        self
    }

    /// Authored repair metadata for an exact race ODF stem, not a unit race key.
    pub fn race_repair(&self, stem: &str) -> Option<RaceRepair> {
        let stem = crate::odf::stem(stem);
        self.race_repairs.iter().find(|(key, _)| *key == stem).map(|(_, repair)| *repair)
    }

    /// Repair metadata of the race a unit carries (its race key, the ODF's `name`):
    /// through the name's file stem when the race is known, else the key as a stem.
    pub fn race_repair_of(&self, race: &str) -> Option<RaceRepair> {
        let name = race.trim().to_ascii_lowercase();
        match self.race_repair_stems.iter().find(|(n, _)| *n == name) {
            Some((_, stem)) => self.race_repair(stem),
            None => self.race_repair(&name),
        }
    }

    /// Set which race name has which repair stem (tests and content built in code).
    pub fn with_race_repair_stems(mut self, rows: Vec<(String, String)>) -> Self {
        self.race_repair_stems = rows.into_iter().map(|(n, s)| (n.trim().to_ascii_lowercase(), crate::odf::stem(&s))).collect();
        self
    }

    /// Set synthetic race repair metadata; canonical stems keep the first row.
    pub fn with_race_repairs(mut self, rows: Vec<(String, RaceRepair)>) -> Self {
        let mut rows: Vec<_> = rows
            .into_iter()
            .map(|(stem, repair)| (crate::odf::stem(&stem), repair))
            .filter(|(stem, repair)| !stem.is_empty() && *repair != RaceRepair::default())
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows.dedup_by(|a, b| a.0 == b.0);
        self.race_repairs = rows;
        self
    }

    /// The boarding keys of a race by its key (`unitedfederation`), any case:
    /// the race ODF's values with the engine defaults for absent keys, and all
    /// defaults for a race that sets none (SIM-46 P0; digested when it differs from the defaults, SIM-69).
    pub fn race_boarding(&self, name: &str) -> RaceBoarding {
        let n = name.trim().to_ascii_lowercase();
        self.race_boarding.iter().find(|(k, _)| *k == n).map(|(_, b)| b.clone()).unwrap_or_default()
    }

    /// Set the race boarding keys (tests and content built in code).
    pub fn with_race_boarding(mut self, rows: Vec<(String, RaceBoarding)>) -> Self {
        self.race_boarding = rows.into_iter().map(|(k, b)| (k.trim().to_ascii_lowercase(), b)).collect();
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

    /// The planet population rules, when the content has them (SIM-30).
    pub fn population(&self) -> Option<&PopulationRules> {
        self.population.as_ref()
    }

    /// Set the population rules (tests and content built in code).
    pub fn with_population(mut self, population: PopulationRules) -> Self {
        self.population = Some(population);
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

    /// The transporter rules: the header's values, the engine's defaults for
    /// the keys it leaves out (SIM-46 P0; the sim does not read them yet).
    pub fn transporter(&self) -> TransporterRules {
        self.transporter.unwrap_or_default()
    }

    /// Set the transporter rules (tests and content built in code).
    pub fn with_transporter(mut self, transporter: TransporterRules) -> Self {
        self.transporter = Some(transporter);
        self
    }

    /// The attack warp distance: the header's, else the engine's.
    pub fn approach(&self) -> ApproachRules {
        self.approach.unwrap_or_default()
    }

    /// Whether this content has warp at all (#1102, ADR 0004 D10): [`WARP_KEY`] of the layered
    /// rules header when it is set (0: off; any other whole number: on), else [`WARP_DEFAULT`].
    /// One value for the whole content; a mod switches warp off by shipping the key in its own
    /// header. Off, no craft enters or stays at warp by any path that asks the sim's warp
    /// permission; Armada I's own cruise model is another mechanism and does not ask it.
    ///
    /// This reading of the key is STA64's own rule; what the original games do with the key is
    /// not established. Grade for the key's name and form: read from data.
    pub fn warp_enabled(&self) -> bool {
        self.warp.unwrap_or(WARP_DEFAULT)
    }

    /// Set the content-level warp switch (tests and content built in code); `None`: as a header
    /// without the key.
    pub fn with_warp(mut self, on: Option<bool>) -> Self {
        self.warp = on;
        self
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
        // Parse each mounted copy, parent to child. Invalid child values do
        // not erase the last valid inherited extension value.
        let mut inherited = Self::default();
        for version in (1..vfs.resolve_all("RTS_CFG.h").len()).rev() {
            match crate::layered::resolve(vfs, "RTS_CFG.h", version) {
                Ok(Some(layer)) => inherited.read_upgrade_pod_maximum_tier(&CfgH::parse(&layer.text), &mut warnings),
                Ok(None) => {}
                Err(e) => warnings.push(Warning::new("RTS_CFG.h", format!("could not read inherited upgradePodMaximumTier: {e}"))),
            }
        }
        rules.upgrade_pod_maximum_tier = rules.upgrade_pod_maximum_tier.or(inherited.upgrade_pod_maximum_tier);
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
        rules.seat_races = races.seat_rules().map(|(k, r)| (k.to_owned(), r)).collect();
        rules.trade_denied = races.trade_denied_races().map(str::to_owned).collect();
        rules.race_mining = races.mining_rate_modifiers().map(|(k, v)| (k.to_owned(), v)).collect();
        rules = rules.with_race_repairs(races.repair_rules().map(|(stem, repair)| (stem.to_owned(), repair)).collect());
        rules.race_repair_stems = races.repair_stems().map(|(n, s)| (n.to_owned(), s.to_owned())).collect();
        rules.race_boarding = races.boarding_rules().map(|(k, b)| (k.to_owned(), b.clone())).collect();
        Ok((rules, warnings))
    }

    /// The warp-suppression multipliers, as the header sets them.
    pub fn fields(&self) -> FieldCfg {
        self.fields
    }

    /// The radius of the warp-suppression field of a unit of this definition, world units: its
    /// bounding radius (`UnitDef::radius`) times the class's multiplier; `None` for a class with no
    /// field, an unset multiplier or an unread radius.
    ///
    /// Read from Armada II's `PlanetClass::GetWarpSuppressionRadius` (0x004a9920)
    /// and `BlackHoleClass::GetWarpSuppressionRadius` (0x0049d470). The multipliers
    /// come from the active header. Armada I's executable rule was not checked.
    pub fn warp_suppression_radius(&self, def: &UnitDef) -> Option<Fixed> {
        let multiplier = match def.raw.class_label()?.to_ascii_lowercase().as_str() {
            "planet" => self.fields.planet_dewarp_radius_multiplier,
            "blackhole_obj" => self.fields.black_hole_dewarp_radius_multiplier,
            _ => None,
        }?;
        (def.radius > Fixed::ZERO).then(|| def.radius * multiplier)
    }

    /// Armada I's header-level start, for a race that authors none.
    pub fn legacy_start(&self) -> LegacyStart {
        self.legacy_start
    }

    /// Explicit `cfgMaxOfficers`; an inherited A2 cap is not used here.
    pub fn a1_max_officers(&self) -> Option<i32> {
        self.a1_max_officers
    }

    /// Armada I dogfight pursuit steering configuration, with executable defaults.
    pub fn a1_dogfight(&self) -> A1DogfightRules {
        self.a1_dogfight
    }

    pub(crate) fn without_a1_dogfight(mut self) -> Self {
        self.a1_dogfight = A1DogfightRules::default();
        self
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
        if self.upgrade_pod_maximum_tier() != 6 {
            out.push(DigestEntry::new("RTS_CFG.h:upgradePodMaximumTier", self.upgrade_pod_maximum_tier().to_string()));
        }
        for (key, value) in PathingRules::KEYS.iter().zip(self.pathing().texts()) {
            out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value));
        }
        // Resolved values: an absent key and the engine default digest alike.
        let a = self.autonomy();
        for (key, value) in AutonomyRules::KEYS.iter().zip([a.movement, a.special_weapon]) {
            out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value.to_string()));
        }
        // The warp switch: only when it differs from the default, so an absent key and one that
        // restates the default digest alike and content without it keeps its hash.
        if self.warp_enabled() != WARP_DEFAULT {
            out.push(DigestEntry::new(format!("RTS_CFG.h:{WARP_KEY}"), u8::from(self.warp_enabled()).to_string()));
        }
        // The two warp distances: each only when it differs from its default, so an absent key
        // and the default digest alike and content without it keeps its hash.
        let (approach, approach_default) = (self.approach(), ApproachRules::default());
        if approach.min_move_warp_distance != approach_default.min_move_warp_distance {
            out.push(DigestEntry::new(
                format!("RTS_CFG.h:{}", ApproachRules::KEYS[0]),
                crate::digest::fixed_text(approach.min_move_warp_distance),
            ));
        }
        if approach.too_close_to_warp != approach_default.too_close_to_warp {
            out.push(DigestEntry::new(
                format!("RTS_CFG.h:{}", ApproachRules::KEYS[1]),
                crate::digest::fixed_text(approach.too_close_to_warp),
            ));
        }
        if self.a1_dogfight.pursuit_angle != A1DogfightRules::default().pursuit_angle {
            out.push(DigestEntry::new(
                format!("RTS_CFG.h:{}", A1DogfightRules::KEYS[0]),
                crate::digest::fixed_text(self.a1_dogfight.pursuit_angle),
            ));
        }
        if self.a1_dogfight.anchor_radius != A1DogfightRules::default().anchor_radius {
            out.push(DigestEntry::new(
                format!("RTS_CFG.h:{}", A1DogfightRules::KEYS[1]),
                crate::digest::fixed_text(self.a1_dogfight.anchor_radius),
            ));
        }
        // The facing keys (SIM-39) and the recycle keys (SIM-45) only when set, so content
        // without them keeps its hash.
        for (i, (key, value)) in CfgRules::KEYS.iter().zip(self.cfg.texts()).enumerate() {
            if i >= 6 && value == "unset" {
                continue;
            }
            out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value));
        }
        // Only when set, so content without the keys keeps its hash.
        for (key, value) in [("cfgStartingCrew", self.legacy_start.crew), ("cfgStartingDilithium", self.legacy_start.dilithium)] {
            if let Some(value) = value {
                out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), value.to_string()));
            }
        }
        // SIM-30: only when set, so content without the keys keeps its hash.
        if let Some(p) = &self.population {
            for (key, value) in PopulationRules::KEYS.iter().zip(p.levels.into_iter().chain([p.civilian_rate_modifier])) {
                out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), crate::digest::fixed_text(value)));
            }
        }
        // Only authored multipliers contribute, preserving the digest of content without them.
        for (key, value) in FieldCfg::KEYS.iter().zip([self.fields.planet_dewarp_radius_multiplier, self.fields.black_hole_dewarp_radius_multiplier]) {
            if let Some(value) = value {
                out.push(DigestEntry::new(format!("RTS_CFG.h:{key}"), crate::digest::fixed_text(value)));
            }
        }
        let crew = self.crew.as_ref();
        out.push(DigestEntry::new(
            "RTS_CFG.h:STARTING_CREW_CAPACITY",
            crew.and_then(|c| c.starting_capacity).map_or_else(|| "unset".to_owned(), |c| c.to_string()),
        ));
        let rates = crew.map(|c| c.races.iter().map(|(k, r)| format!("{}:{}", quoted(k), crate::digest::fixed_text(*r))).collect::<Vec<_>>()).unwrap_or_default();
        out.push(DigestEntry::new("crew:accumulation", format!("[{}]", rates.join(","))));
        // New entries only when set, so content with neither key keeps its digest.
        if let Some(max) = crew.and_then(|c| c.max_crew) {
            out.push(DigestEntry::new("RTS_CFG.h:MAX_CREW", max.to_string()));
        }
        if !self.seat_races.is_empty() {
            let rows = self
                .seat_races
                .iter()
                .map(|(k, r)| format!("{}:{}:{}", quoted(k), crate::digest::fixed_text(r.officer_multiplier), crate::digest::fixed_text(r.latinum_scale)))
                .collect::<Vec<_>>();
            out.push(DigestEntry::new("race:seat-rules", format!("[{}]", rows.join(","))));
        }
        for race in &self.trade_denied {
            out.push(DigestEntry::new(crate::digest::key("race-trade:", race), "can_trade=0"));
        }
        // SIM-69: the transport rules read these (sim-visible), only when set, so content without the keys keeps its digest.
        if let Some(t) = self.transporter {
            let value = Canon::new().fixed("range", t.range).fixed("delay", t.delay).fixed("action_delay", t.action_delay).int("max", t.max).finish();
            out.push(DigestEntry::new("RTS_CFG.h:TRANSPORTER", value));
        }
        for (race, b) in &self.race_boarding {
            let d = RaceBoarding::default();
            if (b.boarding_strength, b.can_commandeer, b.can_be_commandeered) != (d.boarding_strength, d.can_commandeer, d.can_be_commandeered) {
                let value = Canon::new()
                    .fixed("boarding_strength", b.boarding_strength)
                    .flag("can_commandeer", b.can_commandeer)
                    .flag("can_be_commandeered", b.can_be_commandeered)
                    .finish();
                out.push(DigestEntry::new(crate::digest::key("race-boarding:", race), value));
            }
        }
        // CON-51: only when a race authors one, so content without the key keeps its hash.
        if !self.race_mining.is_empty() {
            let rows = self.race_mining.iter().map(|(k, v)| format!("{}:{}", quoted(k), crate::digest::fixed_text(*v))).collect::<Vec<_>>();
            out.push(DigestEntry::new("race:mining-rate", format!("[{}]", rows.join(","))));
        }
        for (stem, repair) in &self.race_repairs {
            let mut canon = Canon::new();
            canon
                .opt_fixed("strength", repair.strength)
                .opt_fixed("yellow", repair.yellow)
                .opt_fixed("red", repair.red)
                .opt_fixed("crew_yellow_status", repair.crew_yellow_status)
                .opt_fixed("crew_red_status", repair.crew_red_status);
            // SIM-74: only when the race sets them, so a race without the keys keeps its text.
            if repair.weapon_yellow.is_some() || repair.weapon_red.is_some() {
                canon.opt_fixed("weapon_yellow", repair.weapon_yellow).opt_fixed("weapon_red", repair.weapon_red);
            }
            // SIM-45: only when set, so races without them keep their hash.
            if repair.recycle_dilithium.is_some() || repair.recycle_metal.is_some() {
                canon.opt_fixed("recycle_dilithium", repair.recycle_dilithium).opt_fixed("recycle_metal", repair.recycle_metal);
            }
            let value = canon.finish();
            out.push(DigestEntry::new(crate::digest::key("race-repair:", stem), value));
        }
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
mod shot_cfg_tests {
    use super::*;

    fn parse(text: &str) -> (ShotCfg, usize) {
        let mut w = Vec::new();
        let c = ShotCfg::from_cfg(&CfgH::parse(text), &mut w);
        (c, w.len())
    }

    #[test]
    fn absent_keys_are_unset_without_a_warning() {
        let (c, w) = parse("");
        assert_eq!((c, w, c.team_colour()), (ShotCfg::default(), 0, false));
    }

    #[test]
    fn the_miss_lifetime_is_a_bounded_number() {
        let (c, w) = parse("float BEAM_MISS_LIFETIME = 0.3f;\n");
        assert_eq!((c.beam_miss_lifetime, w), (Some(Fixed::from_f64(0.3f32 as f64)), 0));
        for bad in ["-1", "x", "1e5"] {
            let (c, w) = parse(&format!("float BEAM_MISS_LIFETIME = {bad};\n"));
            assert_eq!((c.beam_miss_lifetime, w), (None, 1), "{bad}");
        }
    }

    #[test]
    fn the_team_colour_flag_reads_both_values_and_rejects_text() {
        let (c, w) = parse("int NORMAL_WEAPON_TEAM_COLOR = 1;\n");
        assert_eq!((c.normal_weapon_team_color, c.team_colour(), w), (Some(true), true, 0));
        let (c, w) = parse("int NORMAL_WEAPON_TEAM_COLOR = 0;\n");
        assert_eq!((c.normal_weapon_team_color, c.team_colour(), w), (Some(false), false, 0));
        let (c, w) = parse("int NORMAL_WEAPON_TEAM_COLOR = x;\n");
        assert_eq!((c.normal_weapon_team_color, w), (None, 1));
    }

    #[test]
    fn the_keys_stay_out_of_the_digest() {
        let (set, _) = parse("float BEAM_MISS_LIFETIME = 0.3f;\nint NORMAL_WEAPON_TEAM_COLOR = 1;\n");
        assert_ne!(set, ShotCfg::default());
        assert_eq!(Rules::default().digest_entries(), Rules { shots: set, ..Rules::default() }.digest_entries());
        assert!(ShotCfg::KEYS.iter().all(|k| !TYPED_CFG_KEYS.contains(k)));
    }
}

#[cfg(test)]
mod field_cfg_tests {
    use super::*;

    fn parse(text: &str) -> (FieldCfg, usize) {
        let mut w = Vec::new();
        let c = FieldCfg::from_cfg(&CfgH::parse(text), &mut w);
        (c, w.len())
    }

    #[test]
    fn absent_keys_are_unset_without_a_warning() {
        assert_eq!(parse(""), (FieldCfg::default(), 0));
    }

    #[test]
    fn both_multipliers_are_read_and_bad_values_are_unset() {
        let (c, w) = parse("float cfgPLANET_DEWARP_RADIUS_MULTIPLIER = 4.5f;\nint cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER = 100;\n");
        assert_eq!((c.planet_dewarp_radius_multiplier, c.black_hole_dewarp_radius_multiplier, w), (Some(Fixed::from_f64(4.5)), Some(Fixed::from_int(100)), 0));
        let (c, w) = parse("float cfgPLANET_DEWARP_RADIUS_MULTIPLIER = -2;\nint cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER = x;\n");
        assert_eq!((c, w), (FieldCfg::default(), 2));
        let (rules, _) = Rules::from_header("int cfgPLANET_DEWARP_RADIUS_MULTIPLIER = 3;\n");
        assert_eq!(rules.fields().planet_dewarp_radius_multiplier, Some(Fixed::from_int(3)));
    }

    #[test]
    fn the_field_radius_is_the_bounding_radius_times_the_class_multiplier() {
        let (rules, _) = Rules::from_header("int cfgPLANET_DEWARP_RADIUS_MULTIPLIER = 4;\nint cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER = 100;\n");
        let def = |class: &str, radius: i32| {
            let mut d = UnitDef::new("t", class);
            d.raw = std::sync::Arc::new(sta_odf::resolve("t", &mut |_| Some(format!("classLabel = \"{class}\"\n"))).unwrap());
            d.radius = Fixed::from_int(radius);
            d
        };
        assert_eq!(rules.warp_suppression_radius(&def("planet", 260)), Some(Fixed::from_int(1040)));
        assert_eq!(rules.warp_suppression_radius(&def("blackhole_obj", 10)), Some(Fixed::from_int(1000)));
        assert_eq!(rules.warp_suppression_radius(&def("craft", 10)), None);
        assert_eq!(rules.warp_suppression_radius(&def("planet", 0)), None, "an unread radius");
        assert_eq!(Rules::default().warp_suppression_radius(&def("planet", 260)), None, "an unset key");
    }

    #[test]
    fn authored_multipliers_change_the_digest_but_absent_keys_keep_it() {
        use crate::digest::ContentDigest;

        let baseline = ContentDigest::from_entries(Rules::default().digest_entries());
        let (absent, _) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        let absent_digest = ContentDigest::from_entries(absent.digest_entries());
        assert_eq!(absent_digest, baseline);
        for key in FieldCfg::KEYS {
            assert!(TYPED_CFG_KEYS.contains(&key));
            assert!(baseline.get(&format!("RTS_CFG.h:{key}")).is_none());
        }

        let (set, warnings) = Rules::from_header("int cfgPLANET_DEWARP_RADIUS_MULTIPLIER = 3;\nint cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER = 100;\n");
        assert!(warnings.is_empty());
        let digest = ContentDigest::from_entries(set.digest_entries());
        assert_ne!(digest, baseline);
        assert_eq!(digest.get("RTS_CFG.h:cfgPLANET_DEWARP_RADIUS_MULTIPLIER"), Some("3"));
        assert_eq!(digest.get("RTS_CFG.h:cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER"), Some("100"));
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use crate::digest::ContentDigest;
    use crate::Profile;

    fn digest(rules: &Rules, profile: Profile) -> ContentDigest {
        ContentDigest::from_entries(rules.group_digest_entries(profile))
    }

    #[test]
    fn absent_keys_give_the_profiles_defaults() {
        let (rules, warnings) = Rules::from_header("int UNRELATED = 3;\n");
        assert!(warnings.is_empty());
        for profile in [Profile::Native, Profile::Sta2Classic, Profile::Fo4, Profile::Fo3, Profile::Sta1Classic] {
            assert_eq!(rules.group_rules(profile), GroupRules::defaults(profile), "{profile:?}");
            assert!(rules.group_digest_entries(profile).is_empty(), "{profile:?}");
        }
        let (a2, a1, fo3) = (GroupRules::defaults(Profile::Sta2Classic), GroupRules::defaults(Profile::Sta1Classic), GroupRules::defaults(Profile::Fo3));
        assert_eq!((a2.default_to_formation_move, fo3.default_to_formation_move, a1.default_to_formation_move), (true, false, false));
        assert_eq!((a2.leader_slowdown, a1.leader_slowdown), (Fixed::ONE, Fixed::from_f64(0.9f32 as f64)));
        assert!(!a1.formations_relative_to_ship);
    }

    #[test]
    fn the_constants_of_adr_0011_are_the_defaults_of_every_profile() {
        for profile in [Profile::Native, Profile::Sta2Classic, Profile::Fo4, Profile::Fo3, Profile::Sta1Classic] {
            let g = GroupRules::defaults(profile);
            assert_eq!(
                (g.cluster_radius, g.stuck_far_distance, g.stuck_far_progress, g.stuck_far_seconds, g.stuck_near_distance, g.stuck_near_seconds),
                (Fixed::from_int(1200), Fixed::from_int(1000), Fixed::from_int(100), 8, Fixed::from_int(500), 16),
            );
            assert_eq!((g.replan_distance, g.dissolve_share_percent, g.dissolve_distance), (Fixed::from_int(1500), 75, Fixed::from_int(700)));
        }
    }

    #[test]
    fn the_header_overrides_what_it_sets_and_keeps_the_rest() {
        let (rules, warnings) = Rules::from_header("float FORMATION_SPACING = 120.0;\nfloat FORMATION_LEADER_SLOWDOWN = 0.75;\nint cfgDEFAULT_TO_FORMATION_MOVE = 0;\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        let g = rules.group_rules(Profile::Sta2Classic);
        assert_eq!((g.formation_spacing, g.leader_slowdown, g.default_to_formation_move), (Fixed::from_int(120), Fixed::from_f64(0.75), false));
        assert_eq!(g.cluster_radius, Fixed::from_int(1200));
        assert!(!g.formations_relative_to_ship);
    }

    #[test]
    fn each_profile_reads_its_own_spelling_of_the_default_move_key() {
        let (a1, _) = Rules::from_header("int DEFAULT_TO_FORMATION_MOVE = 1;\nint FORMATIONS_RELATIVE_TO_SHIP = 1;\n");
        let g = a1.group_rules(Profile::Sta1Classic);
        assert!(g.default_to_formation_move && g.formations_relative_to_ship);
        // The other spelling is ignored: Armada II family profiles keep their default.
        assert_eq!(a1.group_rules(Profile::Sta2Classic).default_to_formation_move, GroupRules::defaults(Profile::Sta2Classic).default_to_formation_move);
        assert!(!a1.group_rules(Profile::Fo3).default_to_formation_move);
        let (both, _) = Rules::from_header("int cfgDEFAULT_TO_FORMATION_MOVE = 0;\nint DEFAULT_TO_FORMATION_MOVE = 1;\n");
        assert!(!both.group_rules(Profile::Sta2Classic).default_to_formation_move);
        assert!(both.group_rules(Profile::Sta1Classic).default_to_formation_move);
        for key in GroupCfg::KEYS {
            assert!(TYPED_CFG_KEYS.contains(&key), "{key}");
        }
    }

    #[test]
    fn a_bad_value_warns_and_keeps_the_default() {
        let (rules, warnings) = Rules::from_header("float FORMATION_SPACING = wide;\nfloat FORMATION_LEADER_SLOWDOWN = -1.0;\nint cfgDEFAULT_TO_FORMATION_MOVE = yes;\n");
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(rules.group_cfg(), GroupCfg::default());
        assert_eq!(rules.group_rules(Profile::Fo3), GroupRules::defaults(Profile::Fo3));
    }

    #[test]
    fn the_digest_holds_a_value_that_is_not_the_profiles_default_under_one_key_each() {
        let (rules, _) = Rules::from_header("float FORMATION_SPACING = 120.0;\nint cfgDEFAULT_TO_FORMATION_MOVE = 0;\nfloat FORMATION_LEADER_SLOWDOWN = 1.0;\n");
        let d = digest(&rules, Profile::Sta2Classic);
        assert_eq!(d.get("RTS_CFG.h:FORMATION_SPACING"), Some("120"));
        assert_eq!(d.get("RTS_CFG.h:cfgDEFAULT_TO_FORMATION_MOVE"), Some("0"));
        assert_eq!(d.get("RTS_CFG.h:FORMATION_LEADER_SLOWDOWN"), None, "the profile's own value digests like an absent key");
        // The same header under Fleet Operations 3.2.7 differs in other keys: values, not presence.
        let fo3 = digest(&rules, Profile::Fo3);
        assert_eq!(fo3.get("RTS_CFG.h:cfgDEFAULT_TO_FORMATION_MOVE"), None);
        assert_eq!(rules.group_digest_entries(Profile::Sta1Classic).len(), 2, "spacing and slowdown differ from the profile defaults; the mode equals its default");
    }

    #[test]
    fn each_key_changes_the_resolved_rules_and_so_the_digest() {
        let base = digest(&Rules::default(), Profile::Sta2Classic);
        for header in [
            "float FORMATION_SPACING = 90.0;\n",
            "float FORMATION_LEADER_SLOWDOWN = 0.9;\n",
            "int cfgDEFAULT_TO_FORMATION_MOVE = 0;\n",
        ] {
            let (rules, w) = Rules::from_header(header);
            assert!(w.is_empty(), "{w:?}");
            assert_ne!(digest(&rules, Profile::Sta2Classic), base, "{header}");
        }
    }

    #[test]
    fn the_relative_to_ship_key_counts_under_armada_i_only() {
        let (rules, w) = Rules::from_header("int FORMATIONS_RELATIVE_TO_SHIP = 1;\n");
        assert!(w.is_empty(), "{w:?}");
        assert_ne!(digest(&rules, Profile::Sta1Classic), digest(&Rules::default(), Profile::Sta1Classic));
        for profile in [Profile::Native, Profile::Sta2Classic, Profile::Fo4, Profile::Fo3] {
            assert!(!rules.group_rules(profile).formations_relative_to_ship, "{profile:?}");
            assert_eq!(digest(&rules, profile), digest(&Rules::default(), profile), "{profile:?}");
        }
    }
}

#[cfg(test)]
mod approach_tests {
    use super::*;

    /// [`WARP_KEY`] (#1102): typed, 0 is off and any other whole number is on, absent is the
    /// default, and it is digested only off the default. Authored headers.
    #[test]
    fn the_warp_switch_is_typed_and_digested_only_off_its_default() {
        let digest = |r: &Rules| crate::digest::ContentDigest::from_entries(r.digest_entries());
        let key = format!("RTS_CFG.h:{WARP_KEY}");
        let (none, w) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        assert!(w.is_empty());
        assert_eq!(none.warp_enabled(), WARP_DEFAULT);
        assert!(digest(&none).get(&key).is_none());

        let (off, w) = Rules::from_header("int GAME_OPTIONS_WARP = 0;\n");
        assert!(w.is_empty());
        assert!(!off.warp_enabled());
        assert_eq!(digest(&off).get(&key), Some("0"));
        assert_ne!(digest(&off).hash64(), digest(&none).hash64());

        // any other whole number is on, and digests like a header without the key
        for text in ["int GAME_OPTIONS_WARP = 1;\n", "int GAME_OPTIONS_WARP = 7;\n", "float GAME_OPTIONS_WARP = 1.0;\n"] {
            let (on, w) = Rules::from_header(text);
            assert!(w.is_empty(), "{text}");
            assert!(on.warp_enabled(), "{text}");
            assert_eq!(digest(&on).hash64(), digest(&none).hash64(), "{text}");
        }
        // not a whole number: a warning, and the default
        for text in ["int GAME_OPTIONS_WARP = maybe;\n", "float GAME_OPTIONS_WARP = 0.5;\n"] {
            let (bad, w) = Rules::from_header(text);
            assert_eq!(w.len(), 1, "{text}");
            assert_eq!(bad.warp_enabled(), WARP_DEFAULT, "{text}");
        }
        assert!(crate::TYPED_CFG_KEYS.contains(&WARP_KEY));
        // set in code, as a test or content built in code does
        assert!(!Rules::default().with_warp(Some(false)).warp_enabled());
        assert!(Rules::default().with_warp(None).warp_enabled());
    }

    /// `cfgTOO_CLOSE_TO_WARP` (#1102): typed, read beside the attack key without disturbing it,
    /// and digested only off its default. Values authored for the test.
    #[test]
    fn the_warp_move_distance_is_typed_and_digested_only_off_its_default() {
        let digest = |r: &Rules| crate::digest::ContentDigest::from_entries(r.digest_entries());
        let key = "RTS_CFG.h:cfgTOO_CLOSE_TO_WARP";
        let (none, _) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        assert_eq!(none.approach().too_close_to_warp, ApproachRules::DEFAULT_TOO_CLOSE_TO_WARP);
        assert!(digest(&none).get(key).is_none());

        let (set, w) = Rules::from_header("float cfgTOO_CLOSE_TO_WARP = 640.0;\n");
        assert!(w.is_empty());
        assert_eq!(set.approach().too_close_to_warp, Fixed::from_int(640));
        assert_eq!(set.approach().min_move_warp_distance, ApproachRules::default().min_move_warp_distance, "the attack key keeps its default");
        assert_eq!(digest(&set).get(key), Some(crate::digest::fixed_text(Fixed::from_int(640)).as_str()));
        assert!(digest(&set).get("RTS_CFG.h:cfg_MIN_MOVE_WARP_DISTANCE").is_none());
        assert_ne!(digest(&set).hash64(), digest(&none).hash64());

        // Set to the default: the same digest as a header without the key.
        let default = ApproachRules::DEFAULT_TOO_CLOSE_TO_WARP.to_f32();
        let (explicit, _) = Rules::from_header(&format!("float cfgTOO_CLOSE_TO_WARP = {default:.1};\n"));
        assert_eq!(digest(&explicit).hash64(), digest(&none).hash64());

        // Both keys, and a bad value for one: the other is still read.
        let (both, w) = Rules::from_header("float cfg_MIN_MOVE_WARP_DISTANCE = 1200.0;\nfloat cfgTOO_CLOSE_TO_WARP = 640.0;\n");
        assert!(w.is_empty());
        assert_eq!((both.approach().min_move_warp_distance, both.approach().too_close_to_warp), (Fixed::from_int(1200), Fixed::from_int(640)));
        let (bad, w) = Rules::from_header("float cfg_MIN_MOVE_WARP_DISTANCE = 1200.0;\nfloat cfgTOO_CLOSE_TO_WARP = near;\n");
        assert_eq!(w.len(), 1);
        assert_eq!((bad.approach().min_move_warp_distance, bad.approach().too_close_to_warp), (Fixed::from_int(1200), ApproachRules::DEFAULT_TOO_CLOSE_TO_WARP));
        assert!(crate::TYPED_CFG_KEYS.contains(&"cfgTOO_CLOSE_TO_WARP"));
    }

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
        assert_eq!(c.cfg.yard_crew_per_sec(), Fixed::from_int(20), "absent key: the engine default");
        assert_eq!(e.cfg.yard_crew_per_sec(), Fixed::from_int(30));
        let neg = CfgRules { base_crew_per_sec: Some(Fixed::from_int(-5)), ..CfgRules::default() };
        assert_eq!(neg.yard_crew_per_sec(), Fixed::ZERO);
        assert_eq!(e.autonomy(), AutonomyRules { movement: 1, special_weapon: 1 });
        // Every changed line is a differing digest key.
        let (dc, de) = (ContentDigest::from_entries(c.digest_entries()), ContentDigest::from_entries(e.digest_entries()));
        let keys: Vec<String> = dc.differences(&de).into_iter().map(|d| d.key).collect();
        let mut want: Vec<String> = CfgRules::KEYS[..6].iter().chain(&["CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY"]).map(|k| format!("RTS_CFG.h:{k}")).collect();
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
        // The two facing keys are digested only when set (SIM-39).
        let mut rules = Rules::default().with_approach(ApproachRules { min_move_warp_distance: Fixed::from_int(1100), too_close_to_warp: Fixed::from_int(900) }).with_warp(Some(false));
        rules.upgrade_pod_maximum_tier = Some(9);
        rules.legacy_start = LegacyStart { dilithium: Some(1), crew: Some(2), officers: None };
        rules.cfg.facing_weight = Some(Fixed::ONE);
        rules.cfg.facing_constant = Some(3);
        rules.cfg.deconstruct_percent_time = Some(Fixed::ONE);
        rules.cfg.biomatter_recycle_fraction = Some(Fixed::ONE);
        rules.fields.planet_dewarp_radius_multiplier = Some(Fixed::ONE);
        rules.fields.black_hole_dewarp_radius_multiplier = Some(Fixed::ONE);
        rules.a1_dogfight = A1DogfightRules { pursuit_angle: Fixed::ONE, anchor_radius: Fixed::from_int(5) };
        let pop = PopulationRules { levels: [Fixed::from_int(1000); 4], civilian_rate_modifier: Fixed::ONE };
        let d = ContentDigest::from_entries(rules.with_population(pop).digest_entries());
        // The dilithium maximum contributes through the effective starting amounts.
        // The group keys (CON-59) resolve against the profile, so they digest through
        // `group_digest_entries` (see `group_tests`).
        for k in TYPED_CFG_KEYS.iter().filter(|k| !matches!(**k, "MP_OFFICER_LIST_SIZE" | "SINGLE_PLAYER_OFFICER_LIMIT" | "cfgMaxOfficers" | "cfgMaxDilithium") && !GroupCfg::KEYS.contains(k)) {
            assert!(d.get(&format!("RTS_CFG.h:{k}")).is_some(), "{k}");
        }
    }
}

#[cfg(test)]
mod legacy_start_tests {
    use super::*;
    use crate::digest::ContentDigest;

    const A1: &str = "int cfgMaxDilithium = 888888;\nint cfgMaxOfficers = 77;\nint cfgMaxCrew = 55555;\nint cfgStartingDilithium = 1234;\nint cfgStartingCrew = 2345;\n";

    #[test]
    fn armada_one_header_start_and_officer_cap_are_typed() {
        let (r, w) = Rules::from_header(A1);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(r.legacy_start(), LegacyStart { dilithium: Some(1234), crew: Some(2345), officers: Some(77) });
        assert_eq!(r.officer_limit(0), Some(77));
        let d = ContentDigest::from_entries(r.digest_entries());
        assert_eq!(d.get("RTS_CFG.h:cfgStartingDilithium"), Some("1234"));
        assert_eq!(d.get("RTS_CFG.h:cfgStartingCrew"), Some("2345"));
    }

    #[test]
    fn missing_armada_one_start_uses_the_dilithium_maximum() {
        let (rules, warnings) = Rules::from_header("int cfgMaxDilithium = 900;\nint cfgStartingCrew = 0;\n");
        assert!(warnings.is_empty());
        assert_eq!(rules.legacy_start().dilithium, Some(900));
        assert_eq!(rules.legacy_start().crew, Some(0));
    }

    #[test]
    fn a_header_without_the_keys_keeps_its_digest_and_an_explicit_officer_list_wins() {
        let (none, _) = Rules::from_header("int SOMETHING_ELSE = 1;\n");
        assert_eq!(none.legacy_start(), LegacyStart::default());
        assert_eq!(none.officer_limit(0), None);
        let d = ContentDigest::from_entries(none.digest_entries());
        assert!(d.get("RTS_CFG.h:cfgStartingDilithium").is_none() && d.get("RTS_CFG.h:cfgStartingCrew").is_none());
        let (r, _) = Rules::from_header("int SINGLE_PLAYER_OFFICER_LIMIT = 7;\nint cfgMaxOfficers = 33;\n");
        assert_eq!(r.officer_limits.len(), 1);
        assert_eq!(r.officer_limit(0), Some(7));
        assert_eq!(r.legacy_start().officers, None, "an explicit officer key wins: cfgMaxOfficers is not a second source");
    }

    #[test]
    fn bad_values_warn_and_stay_unset() {
        let (r, w) = Rules::from_header("int cfgStartingDilithium = lots;\nint cfgStartingCrew = -5;\nint cfgMaxOfficers = many;\n");
        assert_eq!(r.legacy_start(), LegacyStart::default());
        assert_eq!(r.officer_limit(0), None);
        assert_eq!(w.len(), 3);
    }

    // The literal mirrors the typed-in header text below and its expected parse; it is not meant as pi.
    #[allow(clippy::approx_constant)]
    #[test]
    fn a1_dogfight_defaults_and_nondefaults_are_typed_and_digested() {
        let (defaults, warnings) = Rules::from_header("");
        assert!(warnings.is_empty());
        assert_eq!(defaults.a1_dogfight(), A1DogfightRules::default());
        assert!(defaults.digest_entries().iter().all(|e| !e.key.contains("ANCHOR_")));

        // The stock parenthesized division remains supported locally even
        // though CfgH::f32 itself only accepts numeric literals.
        let (rules, warnings) = Rules::from_header(
            "float ANCHOR_PURSUIT_POINT_ANGLE = (3.14159/6);\nfloat ANCHOR_RADIUS = 5.0;\n",
        );
        assert_eq!(rules.a1_dogfight().pursuit_angle, Fixed::from_f64(f64::from(3.14159f32 / 6.0f32)));
        assert_eq!(rules.a1_dogfight().anchor_radius, Fixed::from_int(5));
        assert!(warnings.is_empty());
        let entries = rules.digest_entries();
        assert!(entries.iter().any(|e| e.key == "RTS_CFG.h:ANCHOR_RADIUS"));
        assert!(entries.iter().any(|e| e.key == "RTS_CFG.h:ANCHOR_PURSUIT_POINT_ANGLE"));
        assert!(TYPED_CFG_KEYS.contains(&"ANCHOR_PURSUIT_POINT_ANGLE"));
        assert!(TYPED_CFG_KEYS.contains(&"ANCHOR_RADIUS"));
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
    fn max_crew_and_seat_rules_are_read_and_digested_only_when_set() {
        let d = digest("int STARTING_CREW_CAPACITY = 250;\nint MAX_CREW = 99999;\n");
        assert_eq!(Rules::from_header("int MAX_CREW = 99999;\n").0.crew().unwrap().max_crew, Some(99999));
        assert_eq!(d.get("RTS_CFG.h:MAX_CREW"), Some("99999"));
        let none = ContentDigest::from_entries(Rules::default().digest_entries());
        assert_eq!(none.get("RTS_CFG.h:MAX_CREW"), None);
        assert_eq!(none.get("race:seat-rules"), None);
        let rule = SeatRaceRule { officer_multiplier: Fixed::from_f64(0.5), latinum_scale: Fixed::ONE };
        let set = Rules::default().with_seat_races(vec![("x".into(), rule)]);
        assert_eq!(set.seat_race("X"), rule);
        assert_eq!(set.seat_race("y"), SeatRaceRule::default());
        assert_eq!(none.first_difference(&ContentDigest::from_entries(set.digest_entries())).unwrap().key, "race:seat-rules");
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
            Rules::default().with_crew(CrewRules { starting_capacity: None, max_crew: None, races: r }).digest_entries(),
        );
        let a = rates(vec![("fed".into(), Fixed::from_int(2))]);
        let b = rates(vec![("fed".into(), Fixed::from_int(3))]);
        assert_eq!(a.first_difference(&b).unwrap().key, "crew:accumulation");
        assert_eq!(a.get("crew:accumulation"), Some("[\"fed\":2]"));
    }
}

#[cfg(test)]
mod population_tests {
    use super::*;
    use crate::digest::ContentDigest;

    const HEADER: &str = "int cfgPOP_SPARSE = 1000;\nint cfgPOP_LIGHT = 2000;\nint cfgPOP_MEDIUM = 3000;\nint cfgPOP_HEAVY = 5000;\nfloat cfgPLANET_CIVILIAN_RATE_MODIFIER = 0.20;\n";

    #[test]
    fn the_population_keys_are_read_and_digested() {
        let (rules, w) = Rules::from_header(HEADER);
        assert!(w.is_empty(), "{w:?}");
        let p = rules.population().unwrap();
        assert_eq!([1u8, 2, 3, 4].map(|l| p.cap(l)), [1000, 2000, 3000, 5000].map(Fixed::from_int));
        assert_eq!(p.cap(0), Fixed::ZERO);
        assert_eq!(p.civilian_rate_modifier, Fixed::from_f64(0.2f32 as f64));
        let d = ContentDigest::from_entries(rules.digest_entries());
        assert_eq!(d.get("RTS_CFG.h:cfgPOP_HEAVY"), Some("5000"));
        let other = Rules::from_header(&HEADER.replace("5000", "6000")).0;
        assert_ne!(d, ContentDigest::from_entries(other.digest_entries()));
    }

    #[test]
    fn absent_or_partial_keys_leave_the_rules_inactive_and_the_digest_alone() {
        let (none, w) = Rules::from_header("int SOMETHING_ELSE = 3;\n");
        assert!(none.population().is_none() && w.is_empty());
        let d = ContentDigest::from_entries(none.digest_entries());
        assert!(d.get("RTS_CFG.h:cfgPOP_SPARSE").is_none());
        let (part, w) = Rules::from_header("int cfgPOP_SPARSE = 1000;\n");
        assert!(part.population().is_none());
        assert_eq!(w.len(), 1);
    }
}
