//! Immutable, profile-backed map-confinement policy and owning-ODF overrides.
//! Geometry and mutable actor flags live in the shared simulation.

use sta_core::Fixed;
use sta_odf::Odf;

use crate::{Profile, Warning};

pub const DEFAULT_INSET: Fixed = Fixed::from_int(100);
/// Exact native binary32 0.1, converted to 32.32 once at content resolution.
pub const DEFAULT_UPPER_EPSILON: Fixed = Fixed::from_bits(429496736);

/// Spatial dispatch, independently of Core actor kind or movement speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfinementSpatial {
    Native,
    Craft,
    Plain,
    Terrain,
    Unsupported,
}

/// How a new actor's flag is initialized. Position updates are separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfinementInitial {
    Disabled,
    RawXz,
    PaddedXz,
    Enabled,
}

/// Explicit values from the resolved unit ODF, or definitions built in code.
/// None means use the profile/family default; zero is never missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConfinementOverrides {
    pub capability: Option<bool>,
    pub initial: Option<ConfinementInitial>,
    pub rearm: Option<bool>,
    pub inset: Option<Fixed>,
    pub upper_epsilon: Option<Fixed>,
}

/// Effective policy, resolved once per definition by ContentDb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfinementRules {
    pub capability: bool,
    pub spatial: ConfinementSpatial,
    pub initial: ConfinementInitial,
    pub rearm: bool,
    pub inset: Fixed,
    pub upper_epsilon: Fixed,
}

impl ConfinementRules {
    /// Only represented capability can affect commands, state or hashes.
    pub fn is_enabled(self) -> bool {
        self.capability && self.spatial != ConfinementSpatial::Unsupported
    }
}

impl ConfinementSpatial {
    pub(crate) fn hash_tag(self) -> u8 {
        match self {
            Self::Native => 0,
            Self::Craft => 1,
            Self::Plain => 2,
            Self::Terrain => 3,
            Self::Unsupported => 4,
        }
    }
}

impl ConfinementInitial {
    pub(crate) fn hash_tag(self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::RawXz => 1,
            Self::PaddedXz => 2,
            Self::Enabled => 3,
        }
    }
}

const A1_CRAFT: &[&str] = &[
    "constructionrig",
    "craft",
    "freighter",
    "hover",
    "mining",
    "pod",
    "producer",
    "research",
    "scavenger",
    "sensor",
    "shipyard",
    "starbase",
    "wingman",
];
const A2_CRAFT: &[&str] = &[
    "cargo_ship",
    "colony",
    "constructionrig",
    "craft",
    "evolver",
    "freighter",
    "minearray",
    "mining",
    "p_mining",
    "planet",
    "pod",
    "producer",
    "repairship",
    "research",
    "scavenger",
    "sensor",
    "shipyard",
    "starbase",
    "trading_station",
];
const TERRAIN: &[&str] = &[
    "areaeffect_obj",
    "asteroidfield",
    "background_obj",
    "blackhole_obj",
    "i76building",
    "ionstorm_obj",
    "nebula_obj",
    "planet_obj",
    "scrap",
    "u_ball_obj",
    "wormhole_obj",
];

fn spatial(profile: Profile, class: &str) -> ConfinementSpatial {
    if profile == Profile::Native {
        return ConfinementSpatial::Native;
    }
    if class == "constplaceholder" {
        ConfinementSpatial::Plain
    } else if (if profile == Profile::Sta1Classic {
        A1_CRAFT
    } else {
        A2_CRAFT
    })
    .contains(&class)
    {
        ConfinementSpatial::Craft
    } else if TERRAIN.contains(&class) || (profile != Profile::Sta1Classic && class == "latinum") {
        ConfinementSpatial::Terrain
    } else {
        ConfinementSpatial::Unsupported
    }
}

pub(crate) fn resolve(
    profile: Profile,
    class: &str,
    own: ConfinementOverrides,
    file: &str,
    warnings: &mut Vec<Warning>,
) -> ConfinementRules {
    let spatial = spatial(profile, class);
    let represented = spatial != ConfinementSpatial::Unsupported;
    if !represented {
        warnings.push(Warning::new(
            file,
            format!(
                "map confinement: no verified spatial policy for class {class:?} in {profile:?}"
            ),
        ));
    }
    let initial = if profile == Profile::Native || !represented {
        ConfinementInitial::Disabled
    } else if profile == Profile::Sta1Classic {
        if spatial == ConfinementSpatial::Terrain {
            ConfinementInitial::Enabled
        } else {
            ConfinementInitial::PaddedXz
        }
    } else {
        ConfinementInitial::RawXz
    };
    let mut rules = ConfinementRules {
        capability: own
            .capability
            .unwrap_or(profile != Profile::Native && represented),
        spatial,
        initial: own.initial.unwrap_or(initial),
        rearm: own.rearm.unwrap_or(
            profile == Profile::Sta1Classic
                && spatial != ConfinementSpatial::Terrain
                && represented,
        ),
        inset: nonnegative(
            own.inset,
            DEFAULT_INSET,
            "STA64_mapConfinementInset",
            file,
            warnings,
        ),
        upper_epsilon: nonnegative(
            own.upper_epsilon,
            DEFAULT_UPPER_EPSILON,
            "STA64_mapConfinementUpperEpsilon",
            file,
            warnings,
        ),
    };
    if spatial == ConfinementSpatial::Terrain && rules.rearm {
        warnings.push(Warning::new(file,
            "STA64_mapConfinementRearm: finished terrain bypasses ordinary rearm; using false for finished actors (construction keeps authored rearm)"));
        rules.rearm = false;
    }
    if !rules.is_enabled()
        && (own
            .initial
            .is_some_and(|v| v != ConfinementInitial::Disabled)
            || own.rearm == Some(true))
    {
        warnings.push(Warning::new(file,
            "map confinement initialization/rearm settings are inactive while capability is disabled or unsupported"));
    }
    rules
}

/// Construction proxies are plain GameObjects, even for terrain/Craft products.
/// Reuse sanitized dimensions and immutable capability, but not the finished
/// constructor exception or spatial override. Authored values apply to both.
pub(crate) fn construction(
    profile: Profile,
    finished: ConfinementRules,
    own: ConfinementOverrides,
) -> ConfinementRules {
    if finished.spatial == ConfinementSpatial::Unsupported {
        return finished;
    }
    ConfinementRules {
        spatial: ConfinementSpatial::Plain,
        initial: own.initial.unwrap_or(match profile {
            Profile::Native => ConfinementInitial::Disabled,
            Profile::Sta1Classic => ConfinementInitial::PaddedXz,
            Profile::Sta2Classic | Profile::Fo4 => ConfinementInitial::RawXz,
        }),
        rearm: own.rearm.unwrap_or(profile == Profile::Sta1Classic),
        ..finished
    }
}

fn nonnegative(
    own: Option<Fixed>,
    default: Fixed,
    key: &str,
    file: &str,
    warnings: &mut Vec<Warning>,
) -> Fixed {
    match own {
        Some(value) if value >= Fixed::ZERO => value,
        Some(_) => {
            warnings.push(Warning::new(
                file,
                format!("{key}: expected a nonnegative value; using default"),
            ));
            default
        }
        None => default,
    }
}

pub(crate) fn from_odf(odf: &Odf, file: &str, warnings: &mut Vec<Warning>) -> ConfinementOverrides {
    let capability = flag(odf, "STA64_mapConfinement", file, warnings);
    let rearm = flag(odf, "STA64_mapConfinementRearm", file, warnings);
    let initial = odf.str("STA64_mapConfinementInitial").and_then(|value| {
        match value.trim().to_ascii_lowercase().as_str() {
            "0" | "disabled" => Some(ConfinementInitial::Disabled),
            "raw_xz" => Some(ConfinementInitial::RawXz),
            "padded_xz" => Some(ConfinementInitial::PaddedXz),
            "enabled" => Some(ConfinementInitial::Enabled),
            _ => {
                warnings.push(Warning::new(file,
                    "STA64_mapConfinementInitial: expected disabled/raw_xz/padded_xz/enabled; using default"));
                None
            }
        }
    });
    ConfinementOverrides {
        capability,
        initial,
        rearm,
        inset: number(odf, "STA64_mapConfinementInset", file, warnings),
        upper_epsilon: number(odf, "STA64_mapConfinementUpperEpsilon", file, warnings),
    }
}

fn flag(odf: &Odf, key: &str, file: &str, warnings: &mut Vec<Warning>) -> Option<bool> {
    odf.raw(key)?;
    let value = odf.bool(key);
    if value.is_none() {
        warnings.push(Warning::new(
            file,
            format!("{key}: expected a boolean; using default"),
        ));
    }
    value
}

fn number(odf: &Odf, key: &str, file: &str, warnings: &mut Vec<Warning>) -> Option<Fixed> {
    odf.raw(key)?;
    // Preserve the owning ODF's f32/suffix semantics, without Reader's unrelated
    // saturation limit. 2^31 world units is the first unrepresentable positive
    // Fixed value; using i64::MAX as f64 would round to that same boundary.
    if let Some(value) = odf.f32(key)
        && value.is_finite()
        && value >= 0.0
        && f64::from(value) < 2147483648.0
    {
        return Some(Fixed::from_f64(f64::from(value)));
    }
    warnings.push(Warning::new(
        file,
        format!("{key}: expected a finite nonnegative representable number; using default"),
    ));
    None
}
