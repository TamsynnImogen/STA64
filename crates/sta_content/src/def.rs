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
///
/// The first five fields are what every movement model reads. The rest are
/// Armada II "smooth physics" keys that only that model reads
/// (`sta_sim::kinematics`); STA64's own model ignores them. Values are the
/// ODF's own, unscaled: each model turns them into rates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveStats {
    /// Cruise speed, units per second (`impulseSpeed`).
    pub max_speed: Fixed,
    /// `forwardAccel`. Native model: a seconds-to-full-speed reciprocal
    /// (speed changes by `forward_accel * max_speed` per second; our
    /// reading). Smooth model: scaled by `combat_speed`.
    pub forward_accel: Fixed,
    /// `turnOmega`. Native model: radians per second. Smooth model: scaled
    /// by `combat_speed`.
    pub turn_omega: Fixed,
    /// `turnAlpha`. Native model: radians per second squared. Smooth model:
    /// a share of the turn rate per second.
    pub turn_alpha: Fixed,
    /// Share of the turn rate available at rest (`turnOmegaFractionAtRest`).
    pub turn_at_rest: Fixed,
    /// `combatSpeed`, units per second: the scale of the smooth model's
    /// acceleration and turn rates. Defaults to `max_speed`.
    pub combat_speed: Fixed,
    /// Share of the turn acceleration available at rest
    /// (`turnAlphaFractionAtRest`). Defaults to `turn_at_rest`.
    pub turn_alpha_at_rest: Fixed,
    /// `tooCloseToTurn`, units: inside it a craft stops rather than circling
    /// round to its point, and arrives once slow enough. Default 50 (the
    /// stock files' "usually 50").
    pub too_close_to_turn: Fixed,
    /// `pathLeadDistance`, units: how far ahead on its path a craft aims,
    /// and how near a waypoint it moves on to the next leg. Default 150 (the
    /// stock files' "usually 150").
    pub path_lead: Fixed,
    /// `turnControlAngle`, degrees: inside it the turn onto a heading is
    /// damped. Default 20.
    pub turn_control_angle: Fixed,
    /// `turnControlSquared`: damp by the square of the remaining share.
    pub turn_control_squared: bool,
    /// `forwardControlDistance`, units: inside it the approach to a stop is
    /// damped. Default 70.
    pub forward_control_distance: Fixed,
    /// `warpSpeed`, units per second; zero when the ODF has none. Read by the
    /// attack approach (SIM-36) from the unit's definition: it is not part of
    /// the smooth fields, the digest or the snapshot's stats mirror; a
    /// snapshot load re-reads it from the definition.
    pub warp_speed: Fixed,
}

impl MoveStats {
    /// The common five; the smooth-physics fields take their defaults.
    pub fn from_f32(max_speed: f32, forward_accel: f32, turn_omega: f32, turn_alpha: f32, turn_at_rest: f32) -> MoveStats {
        let f = |v: f32| Fixed::from_f64(v as f64);
        let turn_at_rest = f(turn_at_rest.clamp(0.0, 1.0));
        MoveStats {
            max_speed: f(max_speed),
            forward_accel: f(forward_accel.max(0.01)),
            turn_omega: f(turn_omega),
            turn_alpha: f(turn_alpha.max(0.01)),
            turn_at_rest,
            combat_speed: f(max_speed),
            turn_alpha_at_rest: turn_at_rest,
            ..SMOOTH_DEFAULTS
        }
    }

    /// Whether any smooth-physics field differs from what
    /// [`from_f32`](MoveStats::from_f32) gives. Hashes add those fields only
    /// then, so content without the keys hashes as before.
    pub fn has_smooth_keys(&self) -> bool {
        self.combat_speed != self.max_speed
            || self.turn_alpha_at_rest != self.turn_at_rest
            || self.too_close_to_turn != SMOOTH_DEFAULTS.too_close_to_turn
            || self.path_lead != SMOOTH_DEFAULTS.path_lead
            || self.turn_control_angle != SMOOTH_DEFAULTS.turn_control_angle
            || self.turn_control_squared != SMOOTH_DEFAULTS.turn_control_squared
            || self.forward_control_distance != SMOOTH_DEFAULTS.forward_control_distance
    }

    /// The smooth-physics fields in declaration order, the flag as 0 or 1.
    pub fn smooth_fields(&self) -> [Fixed; 7] {
        [
            self.combat_speed,
            self.turn_alpha_at_rest,
            self.too_close_to_turn,
            self.path_lead,
            self.turn_control_angle,
            if self.turn_control_squared { Fixed::ONE } else { Fixed::ZERO },
            self.forward_control_distance,
        ]
    }

    /// Things that never move (stations, moons).
    pub const STATIC: MoveStats = MoveStats {
        max_speed: Fixed::ZERO,
        forward_accel: Fixed::ONE,
        turn_omega: Fixed::ZERO,
        turn_alpha: Fixed::ONE,
        turn_at_rest: Fixed::ZERO,
        ..SMOOTH_DEFAULTS
    };
}

/// Smooth-physics values for keys a physics ODF leaves out (the defaults
/// of `tooCloseToTurn` and `pathLeadDistance` are the stock comments'
/// "usually"; the control ones are the stock destroyer's). The first seven
/// fields are placeholders every constructor sets.
const SMOOTH_DEFAULTS: MoveStats = MoveStats {
    max_speed: Fixed::ZERO,
    forward_accel: Fixed::ONE,
    turn_omega: Fixed::ZERO,
    turn_alpha: Fixed::ONE,
    turn_at_rest: Fixed::ZERO,
    combat_speed: Fixed::ZERO,
    turn_alpha_at_rest: Fixed::ZERO,
    too_close_to_turn: Fixed::from_int(50),
    path_lead: Fixed::from_int(150),
    turn_control_angle: Fixed::from_int(20),
    turn_control_squared: false,
    forward_control_distance: Fixed::from_int(70),
    warp_speed: Fixed::ZERO,
};

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

/// Authored repair keys from a unit's resolved ODF. No repair timing or
/// eligibility defaults are inferred here; the simulation decides behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepairDef {
    /// Authored seconds per system hit point, before runtime modifiers, for
    /// engines, weapons, shield generator, life support, sensors.
    /// `None` means absent; `Some(0)` is an authored zero.
    pub system_times: [Option<Fixed>; 5],
    pub repair_facility: Option<bool>,
    pub repair_hardpoint: Option<String>,
    /// Exact signed decimal `repairSide`; malformed or out-of-range values
    /// warn and stay absent. This is metadata, not a native side policy.
    pub repair_side: Option<i32>,
    pub can_shipyard_repair: Option<bool>,
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

/// Where a finished unit leaves its builder (SIM-16), in the builder's own
/// frame (+z ahead, model units times `ScaleSOD`, XZ only). See [`crate::launch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPoint {
    /// Where the new unit appears, `(x, z)`.
    pub offset: (Fixed, Fixed),
    /// Unit vector the new unit faces and leaves along, `(x, z)`.
    pub dir: (Fixed, Fixed),
    /// Radius of a circle round the builder's centre that the unit must leave.
    pub clear: Fixed,
    /// The model node (`buildHardpoint`) that gave `offset` and `dir`;
    /// `None`: the stated fallback (builder centre, facing ahead).
    pub node: Option<String>,
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
    /// STA64_queueLimit: total station jobs, including the active head.
    /// None selects the profile default; Some(0) disables new admissions.
    pub queue_limit: Option<u32>,
    /// Items (lower-case ODF stems) whose `builditem#` slot carries a
    /// `builditem#availability` or `builditem#provided` line; `None` when no
    /// slot of this builder carries one (a menu builder). Read by
    /// `TechTree::availability_for_builder` for items the tree does not list.
    pub flagged_slots: Option<Vec<String>>,
}

impl BuildDef {
    /// [`flagged_slots`](BuildDef::flagged_slots) as a builder's ODF gives them.
    pub fn flagged_slots_of(odf: &sta_odf::Odf) -> Option<Vec<String>> {
        let mut any = false;
        let mut items = Vec::new();
        for k in odf.keys() {
            let l = k.to_ascii_lowercase();
            let Some(rest) = l.strip_prefix("builditem") else { continue };
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || !matches!(&rest[digits..], "availability" | "provided") {
                continue;
            }
            any = true;
            if let Some(v) = odf.raw(&format!("builditem{}", &rest[..digits])) {
                items.push(crate::techtree::stem(v.trim().trim_matches('"')));
            }
        }
        items.sort();
        items.dedup();
        any.then_some(items)
    }
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
    /// Resolved legacy `is_starbase` ODF flag, default false. This is
    /// independent of the simulation's actor kind or producer behavior.
    pub is_starbase: bool,
    /// Resolved planet rule family, independent of actor kind and colony state.
    /// `None` retains the existing non-planet-metadata path; Native content
    /// opts in explicitly. Filled by the profile-aware ODF loader, or by an author.
    pub planet_role: Option<crate::planet::PlanetRole>,
    /// Effective signed `maxCrewGain` contribution for a Producer planet.
    /// Zero for other roles; this is not a crew stock or population count.
    pub planet_crew_gain: i32,
    /// Effective `crewModifier` of a Background planet (CON-37). `None` for
    /// other roles and where neither the ODF nor a measured profile fallback
    /// (Armada I: 1.2) supplies one. An authored zero is `Some(0)`.
    pub planet_crew_modifier: Option<Fixed>,
    /// Effective `isColonizable` of a Producer planet (CON-37); false for other roles.
    pub planet_colonizable: bool,
    /// Effective `civilianPopulationModifier` of a Producer planet (CON-37);
    /// zero for other roles.
    pub planet_civilian_population_modifier: Fixed,
    /// Population level cap of a Producer planet (SIM-30): the planet ODF's
    /// `maxPopulation` word, 0 unpopulated, 1 sparse, 2 light, 3 medium, 4 heavy
    /// (read from the executable, static). Zero for other roles.
    pub planet_max_population: u8,
    /// `canHaveOrbitalStation` of the resource node a planet names (`resource =`,
    /// e.g. `ResourcePlanet.odf`): an orbital station may be built in its orbit
    /// (SIM-62). False for moons, rocks and everything without the key.
    pub can_have_orbital_station: bool,
    /// Seconds one orbit takes (`orbitTime`) for a station of class `p_mining`,
    /// the class of the orbital mining and processing facilities: `Some` marks a
    /// station that can only be built in a planet's orbit (SIM-62); zero when the
    /// ODF has no `orbitTime`. `None` for every other class.
    pub orbit_time: Option<Fixed>,
    pub race: String,
    pub model: String,
    pub scale: Fixed,
    pub movement: MoveStats,
    /// Maximum hull strength (`maxHealth`).
    pub hull: Fixed,
    pub shields: ShieldDef,
    /// Authored starting hull; `None` starts at the hull maximum.
    pub initial_hull: Option<Fixed>,
    /// Authored starting shield strength per pool; `None` starts at maximum.
    pub initial_shields: Option<Fixed>,
    /// Starting crew and build cost (`crewCost`), separate from capacity.
    pub crew: Fixed,
    /// `maximumCrew`; absent means the starting crew amount.
    pub maximum_crew: Option<Fixed>,
    /// `maxCrewGain` of a station (SIM-25): what it adds to its team's crew capacity.
    pub crew_gain: Fixed,
    /// `saneCrewLossMethod` (SIM-39): a hit's crew loss is in absolute crew
    /// points; without it the loss is a percent of the unit's maximum crew.
    pub sane_crew_loss: bool,
    /// Hit points of the unit's systems (SIM-55), in the order engines,
    /// weapons, shield generator, life support, sensors: `enginesHitPoints`,
    /// `weaponsHitPoints`, `shieldGeneratorHitPoints`, `lifeSupportHitPoints`,
    /// `sensorsHitPoints`. 0 when the key is absent.
    pub system_hit_points: [Fixed; 5],
    /// Authored repair metadata only; no profile or runtime policy is implied.
    pub repair: RepairDef,
    /// Sensor range, units.
    pub sensor: Fixed,
    pub hardpoints: Vec<Hardpoint>,
    pub build: BuildDef,
    /// Where finished units leave this builder; `None`: not read (the unit
    /// appears at the builder's centre, as before SIM-16).
    pub launch: Option<LaunchPoint>,
    /// Bounding radius of the model times `ScaleSOD`, units (SIM-21); zero:
    /// not read, the unit takes no room.
    pub radius: Fixed,
    pub sounds: SoundRefs,
    pub fx: FxRefs,
    /// Special energy pool (`maxSpecialEnergy`); zero: none.
    pub special_energy_max: Fixed,
    /// Special energy regained per second (`specialEnergyRate`).
    pub special_energy_rate: Fixed,
    /// The ODF's `miner` key is set and non-zero. Class `freighter` also mines:
    /// read [`UnitDef::mines`], not this, to ask whether a unit mines.
    pub miner: bool,
    /// The ODF says `miner = 0`: not a miner even if class `freighter` (the Data
    /// Miner of stock Armada II).
    pub miner_off: bool,
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
    /// Owning-unit map-confinement overrides; resolved once through ContentDb.
    pub confinement: crate::ConfinementOverrides,
    /// Path-planner rules of a `nebula_obj` (SIM-23); `None` for every other
    /// class, and for content that predates them.
    pub nebula_pathing: Option<NebulaPathing>,
    /// `avoidMe = 1`: other units steer around and keep out of this one
    /// (moons and balls in Fleet Operations, SIM-21).
    pub avoid_me: bool,
    /// `hidden = 1`: the player never sees or picks it (start helpers).
    pub hidden: bool,
    /// `invalidAsTarget = 1`: automatic target selection skips it.
    pub invalid_as_target: bool,
    /// `attackPower` (SIM-36): how much enemies want to shoot it; automatic
    /// target choice prefers the highest `attackPower / current shields`.
    /// `None` when the ODF has no such key.
    pub attack_power: Option<Fixed>,
    /// `autoExpireTime` (else `lifeTimer`) in ticks; the object removes itself
    /// that long after it appears (SIM-28).
    pub life: Option<Tick>,
    /// The resolved ODF, so loaders and tools can reach keys no field reads.
    /// The simulation never reads it (D9): what it needs is a typed field.
    /// Empty for definitions not made from an ODF.
    pub raw: Arc<sta_odf::Odf>,
}

/// What a nebula costs the path planner (SIM-23), from its ODF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NebulaPathing {
    /// `pathing_cost`, on the scale of `RTS_CFG.h`'s `DEFAULT_TERRAIN_VALUE`;
    /// `None`: not set, `NEBULAE_PATHING_COST` applies.
    pub cost: Option<i64>,
    /// `addToPathPlanner` (default 1): 0 leaves the nebula out of planning.
    pub planned: bool,
}

impl UnitDef {
    /// Whether the unit mines. The `miner` key decides when the ODF has one
    /// (stock `freight.odf` says `miner = 1`; the Data Miner, `fdata.odf`, is
    /// class `freighter` and says `miner = 0`); without it a `freighter` mines.
    pub fn mines(&self) -> bool {
        !self.miner_off && (self.miner || self.class.0 == "freighter")
    }

    /// [`avoid_me`](UnitDef::avoid_me) (the field).
    pub fn avoid_me(&self) -> bool {
        self.avoid_me
    }

    /// A definition with everything zero or empty, to fill field by field.
    pub fn new(odf: &str, class: &str) -> UnitDef {
        let mut raw = sta_odf::Odf::default();
        raw.name = odf.to_owned();
        UnitDef {
            key: ContentKey::NONE,
            odf: odf.to_owned(),
            class: ClassLabel::new(class),
            is_starbase: false,
            planet_role: None,
            planet_crew_gain: 0,
            planet_crew_modifier: None,
            planet_colonizable: false,
            planet_civilian_population_modifier: Fixed::ZERO,
            planet_max_population: 0,
            can_have_orbital_station: false,
            orbit_time: None,
            race: String::new(),
            model: String::new(),
            scale: Fixed::ONE,
            movement: MoveStats::STATIC,
            hull: Fixed::ZERO,
            shields: ShieldDef::default(),
            initial_hull: None,
            initial_shields: None,
            crew: Fixed::ZERO,
            maximum_crew: None,
            crew_gain: Fixed::ZERO,
            sane_crew_loss: false,
            system_hit_points: [Fixed::ZERO; 5],
            repair: RepairDef::default(),
            sensor: Fixed::ZERO,
            hardpoints: Vec::new(),
            build: BuildDef::default(),
            launch: None,
            radius: Fixed::ZERO,
            sounds: SoundRefs::default(),
            fx: FxRefs::default(),
            special_energy_max: Fixed::ZERO,
            special_energy_rate: Fixed::ZERO,
            miner: false,
            miner_off: false,
            max_dilithium: Fixed::ZERO,
            mining_rate: Fixed::ZERO,
            transfer_rate: Fixed::ZERO,
            resource: None,
            features: None,
            confinement: crate::ConfinementOverrides::default(),
            nebula_pathing: None,
            avoid_me: false,
            hidden: false,
            invalid_as_target: false,
            attack_power: None,
            life: None,
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
    /// `initialShotDelayModifier` (a Roots key): `None` when absent, which
    /// the engine treats as -1 (see `sta_sim::combat::weapons`).
    pub initial_shot_delay_modifier: Option<Fixed>,
    /// `ignoreShotDelayEffects` (FO): no shot jitter, no team modifier.
    pub ignore_shot_delay_effects: bool,
    /// The weapon's own class is `ordnancedefenseweapon` (FO's point-defence
    /// class, as opposed to `pointdefenselaser`).
    pub ordnance_defence: bool,
    /// `activateOnTeamChange` (an FO mod key, default 0): the weapon's special
    /// is engaged for its new owner after a completed team change (SIM-29).
    pub activate_on_team_change: bool,
    /// The ordnance's crew-loss keys (SIM-39); `None` when it has none of them.
    pub crew_loss: Option<OrdnanceCrewLoss>,
    /// Feature switches resolved for this weapon, as [`UnitDef::features`];
    /// `None`: the database-wide set. Read through
    /// `ContentDb::weapon_features`.
    pub features: Option<Features>,
    /// `resourceweapon` class only (SIM-24): what the weapon adds to its
    /// owner's stock and who it works for. `None` for every other class.
    pub resource: Option<ResourceWeaponDef>,
    /// An instant `replaceweapon` (SIM-28); `None` for every other weapon.
    pub replace: Option<crate::lifecycle::ReplaceDef>,
    /// The colonizer pod this weapon launches (SIM-30): set when the ordnance's
    /// class is `ColonizerPod`; `None` for every other weapon.
    pub colonize: Option<ColonizePodDef>,
    pub raw: Arc<sta_odf::Odf>,
}

/// The keys of a `ColonizerPod` ordnance (SIM-30), read from the ordnance ODF
/// of a weapon (the launcher's `shotDelay` is the weapon's `refire`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColonizePodDef {
    /// `partySize`: colonists one pod carries (default 100).
    pub party_size: Fixed,
    /// `orbitRange`: the colonize range of the ship, from the planet surface.
    pub orbit_range: Fixed,
}

/// What a hit of this ordnance does to the target's crew (SIM-39): its
/// `damageThreshold`, `shieldCrewModifier` and `hullCrewModifier`. Absent keys
/// read as 0 (no loss).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OrdnanceCrewLoss {
    /// `damageThreshold`: damage below it costs no crew.
    pub threshold: Fixed,
    /// `shieldCrewModifier`: crew lost per point of shield damage over the threshold.
    pub shield: Fixed,
    /// `hullCrewModifier`: the same for hull damage (read, not applied: not measured).
    pub hull: Fixed,
}

/// The keys of a `resourceweapon` (Fleet Operations 3.x and later; stock
/// Armada I and II have none), read by [`crate::weapon::weapon_def`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceWeaponDef {
    /// Signed amount per resource slot (dilithium, latinum, metal,
    /// biomatter): `addDilithium`, `addLatinum`, `addMetal`, `addBiomatter`.
    pub add: [Fixed; 4],
    /// Signed crew per second (`addCrew`), onto the team's crew stock.
    pub crew: Fixed,
    /// `oneTimeMode`: a lump sum, not a rate. Default off.
    pub one_time: bool,
    /// `alwaysActivePlayer`: works for a human seat without being triggered.
    pub always_player: bool,
    /// `alwaysActiveAI`: the same for an AI seat.
    pub always_ai: bool,
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
            initial_shot_delay_modifier: None,
            ignore_shot_delay_effects: false,
            ordnance_defence: false,
            activate_on_team_change: false,
            crew_loss: None,
            features: None,
            resource: None,
            replace: None,
            colonize: None,
            raw: Arc::new(raw),
        }
    }
}
