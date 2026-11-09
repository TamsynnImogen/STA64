//! Unit and weapon definitions: what the sim reads about a thing, in ticks and
//! [`Fixed`]. Seam types (S5); CON-03 fills them from ODFs, CON-04 from JSON5.

use std::sync::Arc;

use sta_core::{ContentKey, Fixed, Tick};

use crate::features::Features;

/// A reference from one definition to another, by ODF name as the data
/// writes it. [`ContentDb`](crate::ContentDb) resolves `key` when it is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefRef {
    pub odf: String,
    /// [`ContentKey::NONE`] until resolved.
    pub key: ContentKey,
}

impl DefRef {
    pub fn new(odf: impl Into<String>) -> DefRef {
        DefRef { odf: odf.into(), key: ContentKey::NONE }
    }
}

/// An ODF `classLabel`, lower-cased (`craft`, `starbase`, `shipyard`, ...;
/// see docs/data/odf_classes.md). Kept as text: which engine class a label
/// means is the sim's business, and many labels have no system yet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ClassLabel(pub String);

impl ClassLabel {
    pub fn new(label: &str) -> ClassLabel {
        ClassLabel(label.to_ascii_lowercase())
    }
}

/// Movement limits, from a craft's physics ODF (`physicsFile`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveStats {
    /// Cruise speed, units per second (`impulseSpeed`).
    pub max_speed: Fixed,
    /// Seconds-to-full-speed reciprocal (`forwardAccel`): speed changes by
    /// `forward_accel * max_speed` per second. Our reading of the key.
    pub forward_accel: Fixed,
    /// Turn rate, radians per second (`turnOmega`).
    pub turn_omega: Fixed,
    /// Turn acceleration, radians per second squared (`turnAlpha`).
    pub turn_alpha: Fixed,
    /// Share of the turn rate available at rest (`turnOmegaFractionAtRest`).
    pub turn_at_rest: Fixed,
}

impl MoveStats {
    pub fn from_f32(max_speed: f32, forward_accel: f32, turn_omega: f32, turn_alpha: f32, turn_at_rest: f32) -> MoveStats {
        let f = |v: f32| Fixed::from_f64(v as f64);
        MoveStats {
            max_speed: f(max_speed),
            forward_accel: f(forward_accel.max(0.01)),
            turn_omega: f(turn_omega),
            turn_alpha: f(turn_alpha.max(0.01)),
            turn_at_rest: f(turn_at_rest.clamp(0.0, 1.0)),
        }
    }

    /// Things that never move (stations, moons).
    pub const STATIC: MoveStats = MoveStats {
        max_speed: Fixed::ZERO,
        forward_accel: Fixed::ONE,
        turn_omega: Fixed::ZERO,
        turn_alpha: Fixed::ONE,
        turn_at_rest: Fixed::ZERO,
    };
}

impl Default for MoveStats {
    fn default() -> Self {
        MoveStats::from_f32(100.0, 1.0, 1.0, 1.0, 0.2)
    }
}

/// Shield strength. With [`Features::directional_shields`](crate::Features)
/// each of the four facings has `max`; without, there is one pool of `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShieldDef {
    pub max: Fixed,
    /// Regained per tick.
    pub recharge: Fixed,
}

/// Where a hardpoint can aim, relative to the unit's heading, without
/// trigonometry: a target is inside when the dot product of its direction
/// with `dir` is at least `min_dot` (the cosine of the half-width).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FireArc {
    /// Unit vector in the unit's frame, `(x, z)`, +z ahead.
    pub dir: (Fixed, Fixed),
    pub min_dot: Fixed,
}

/// A weapon mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hardpoint {
    pub weapon: DefRef,
    /// Model node the shots leave from (names only; resolved client-side).
    pub node: String,
    /// `None`: fires all round, or [`Features::fire_arcs`](crate::Features) is off.
    pub arc: Option<FireArc>,
}

/// What building a thing costs and what it can build.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildDef {
    /// Cost per resource, indexed by resource id (0 dilithium, 1 latinum,
    /// 2 metal, 3 biomatter; data-driven, so the list may grow).
    pub cost: Vec<Fixed>,
    pub ticks: Tick,
    pub officers: Fixed,
    /// What this unit can build, in button order.
    pub list: Vec<DefRef>,
}

/// Sound names a unit uses; the client resolves them (CON-10, AUD-02/03).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SoundRefs {
    pub select: Vec<String>,
    pub ack_move: Vec<String>,
    pub ack_attack: Vec<String>,
    pub explode: Option<String>,
    pub ambient: Option<String>,
}

/// Effect names a unit uses; the client resolves them (RND-09..11).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FxRefs {
    pub explosion: Option<String>,
    /// Damage effects, lightest first.
    pub damage: Vec<String>,
}

/// A thing that can exist in the world: craft, station, yard, resource, moon.
#[derive(Debug, Clone)]
pub struct UnitDef {
    /// Set by [`ContentDb`](crate::ContentDb) when it is built.
    pub key: ContentKey,
    /// ODF name without extension; the definition's name in the database.
    pub odf: String,
    pub class: ClassLabel,
    pub race: String,
    pub model: String,
    pub scale: Fixed,
    pub movement: MoveStats,
    pub hull: Fixed,
    pub shields: ShieldDef,
    pub crew: Fixed,
    /// Sensor range, units.
    pub sensor: Fixed,
    pub hardpoints: Vec<Hardpoint>,
    pub build: BuildDef,
    pub sounds: SoundRefs,
    pub fx: FxRefs,
    /// Special energy pool (`maxSpecialEnergy`); zero: none.
    pub special_energy_max: Fixed,
    /// Special energy regained per second (`specialEnergyRate`).
    pub special_energy_rate: Fixed,
    /// A freighter that mines (`miner = 1`, or class `freighter`).
    pub miner: bool,
    /// Hold size (`maxdilithium`); zero: not set, the consumer's default.
    pub max_dilithium: Fixed,
    /// Units mined per second (`miningRate`); zero: not set.
    pub mining_rate: Fixed,
    /// Units unloaded per second (`dilithiumTransferRate`); zero: not set.
    pub transfer_rate: Fixed,
    /// Resource id a node gives (`resource`, text: latinum 1, metal or
    /// tritanium 2, biomatter 3, anything else 0); `None` when not set.
    pub resource: Option<u8>,
    /// Feature switches resolved for this unit (profile defaults, mod
    /// overrides, then the ODF's `Feature_<name>` keys); `None`: the
    /// database-wide set. Read through `ContentDb::unit_features`.
    pub features: Option<Features>,
    /// The resolved ODF, so keys no field reads yet stay reachable. Empty for
    /// definitions not made from an ODF.
    pub raw: Arc<sta_odf::Odf>,
}

impl UnitDef {
    /// A definition with everything zero or empty, to fill field by field.
    pub fn new(odf: &str, class: &str) -> UnitDef {
        let mut raw = sta_odf::Odf::default();
        raw.name = odf.to_owned();
        UnitDef {
            key: ContentKey::NONE,
            odf: odf.to_owned(),
            class: ClassLabel::new(class),
            race: String::new(),
            model: String::new(),
            scale: Fixed::ONE,
            movement: MoveStats::STATIC,
            hull: Fixed::ZERO,
            shields: ShieldDef::default(),
            crew: Fixed::ZERO,
            sensor: Fixed::ZERO,
            hardpoints: Vec::new(),
            build: BuildDef::default(),
            sounds: SoundRefs::default(),
            fx: FxRefs::default(),
            special_energy_max: Fixed::ZERO,
            special_energy_rate: Fixed::ZERO,
            miner: false,
            max_dilithium: Fixed::ZERO,
            mining_rate: Fixed::ZERO,
            transfer_rate: Fixed::ZERO,
            resource: None,
            features: None,
            raw: Arc::new(raw),
        }
    }
}

/// How a weapon delivers damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WeaponKind {
    /// Hits on firing, drawn for `duration` (phasers, disruptor beams).
    Beam,
    /// Flies at `shot_speed` and hits on arrival (torpedoes, pulses).
    Projectile,
    /// Shoots down projectiles in numbered cycles
    /// ([`Features::point_defense_cycles`](crate::Features)).
    PointDefence,
}

/// A weapon.
#[derive(Debug, Clone)]
pub struct WeaponDef {
    /// Set by [`ContentDb`](crate::ContentDb) when it is built.
    pub key: ContentKey,
    pub odf: String,
    pub class: ClassLabel,
    pub kind: WeaponKind,
    pub range: Fixed,
    /// Damage per hit.
    pub damage: Fixed,
    /// Ticks between shots.
    pub refire: Tick,
    /// Units per second; zero for beams.
    pub shot_speed: Fixed,
    /// Ticks a beam is drawn; zero for projectiles.
    pub duration: Tick,
    pub fire_sound: Option<String>,
    /// Ordnance or beam effect name (resolved client-side).
    pub fx: Option<String>,
    /// Specials: ticks to engage or disengage (`effectTimer`).
    pub effect_ticks: Tick,
    /// Specials: energy drained per second while active (`energyDrainRate`).
    pub energy_drain: Fixed,
    /// Special energy to trigger the weapon (`specialEnergyCost`).
    pub energy_cost: Fixed,
    /// Cloak: weapons are off while cloaked (`disableWeapons`, default on).
    pub disable_weapons: bool,
    /// Cloak: a hit decloaks (`decloakOnHit`, default off).
    pub decloak_on_hit: bool,
    /// Feature switches resolved for this weapon, as [`UnitDef::features`];
    /// `None`: the database-wide set. Read through
    /// `ContentDb::weapon_features`.
    pub features: Option<Features>,
    pub raw: Arc<sta_odf::Odf>,
}

impl WeaponDef {
    /// A definition with everything zero or empty, to fill field by field.
    pub fn new(odf: &str, class: &str, kind: WeaponKind) -> WeaponDef {
        let mut raw = sta_odf::Odf::default();
        raw.name = odf.to_owned();
        WeaponDef {
            key: ContentKey::NONE,
            odf: odf.to_owned(),
            class: ClassLabel::new(class),
            kind,
            range: Fixed::ZERO,
            damage: Fixed::ZERO,
            refire: 0,
            shot_speed: Fixed::ZERO,
            duration: 0,
            fire_sound: None,
            fx: None,
            effect_ticks: 0,
            energy_drain: Fixed::ZERO,
            energy_cost: Fixed::ZERO,
            disable_weapons: true,
            decloak_on_hit: false,
            features: None,
            raw: Arc::new(raw),
        }
    }
}
