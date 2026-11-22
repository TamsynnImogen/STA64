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
    /// `tooCloseForWarp` (Armada I physics ODFs), units: a plain move
    /// whose goal is farther than this goes to warp (SIM-64; the attack approach
    /// does not read it). Zero when the
    /// ODF has none (Armada II and later: warp on a plain move is opt-in).
    /// Like `warp_speed` the sim reads it from the definition.
    pub too_close_for_warp: Fixed,
    /// `headingCloseEnoughForWarp` (Armada I), degrees: the craft may warp
    /// while its heading is within this of the goal. Zero when absent.
    pub heading_close_for_warp: Fixed,
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
    too_close_for_warp: Fixed::ZERO,
    heading_close_for_warp: Fixed::ZERO,
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
    /// `repairRate` of a repair ship (SIM-41): its beam repairs at this many times a yard's
    /// rates (systems and hull; stock 0.25). `None` when absent.
    pub repair_rate: Option<Fixed>,
    /// `repairsShields` (FO/Roots repair ships): the beam also restores shields. `None` when
    /// absent.
    pub repairs_shields: Option<bool>,
    /// SIM-45: `recyclesInPlace` (a unit decommissioned where it stands).
    pub recycles_in_place: Option<bool>,
    /// SIM-45: `decommissionFacility` (a yard that takes decommissioned ships).
    pub decommission_facility: Option<bool>,
    /// SIM-45: a yard's `dilithiumRecyclingFraction` / `metalRecyclingFraction` (else the race's).
    pub dilithium_recycling: Option<Fixed>,
    pub metal_recycling: Option<Fixed>,
    /// SIM-45 (FO/Roots): `recycleDilithium`, `recycleLatinum`, `recycleMetal`,
    /// `recycleBiomatter` by resource slot, `Some` when any is set (the others 0).
    pub recycle_amounts: Option<[Fixed; 4]>,
    /// SIM-45 (FO/Roots): `recycleTime`, seconds.
    pub recycle_time: Option<Fixed>,
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
    /// Height of a gate aperture, relative to the builder's origin.
    pub height: Fixed,
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
        if odf.lookup_policy() == sta_odf::LookupPolicy::PrefixFirst {
            let mut any = false;
            let mut items = Vec::new();
            for index in crate::odf::prefix_numbered_indices(odf, "builditem") {
                let query = format!("builditem{index}");
                if ["availability", "provided"].iter().any(|suffix| odf.raw(&format!("{query}{suffix}")).is_some()) {
                    any = true;
                    if let Some(value) = odf.raw(&query) {
                        items.push(crate::techtree::stem(value.trim().trim_matches('"')));
                    }
                }
            }
            items.sort();
            items.dedup();
            return any.then_some(items);
        }
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
/// One ordered `podHardpoints` slot of an Armada II research station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchSlot {
    pub node: String,
    pub reserved_for: Option<String>,
    pub pose: Option<crate::RepairAttachment>,
}

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
    /// SIM-44: the cap file the unit names (`fleetCapFile`, FO/Roots), read into the same
    /// [`CapacityList`](crate::CapacityList) as the XML tech tree's `<capacitylist>`: members from
    /// `fleetCapClass#`, weights from `fleetCapClass#CapValue` (default 1), the limit from `maxCap`
    /// (absent: no limit of its own). Shared by every unit naming the same file.
    pub fleet_cap: Option<std::sync::Arc<crate::CapacityList>>,
    /// Seconds one orbit takes (`orbitTime`) for a station of class `p_mining`,
    /// the class of the orbital mining and processing facilities: `Some` marks a
    /// station that can only be built in a planet's orbit (SIM-62); zero when the
    /// ODF has no `orbitTime`. `None` for every other class.
    pub orbit_time: Option<Fixed>,
    /// Armada I Borg-physics keys from the craft's own physics file (CON-52):
    /// `Some` only under `Sta1Classic` for a craft whose `borgPhysics` is set.
    pub a1_borg: Option<crate::A1Borg>,
    /// `miningMultiplier` and `resourcesCanHandle` of a class `p_mining` station
    /// (CON-51). `None` for other classes and for a station with neither key.
    pub station_mining: Option<crate::StationMining>,
    /// The income keys of the resource node a planet names (`resource =`): mining
    /// rate, trickle, `isInfinite` and `amt<slot>` (CON-51). `None` when the node
    /// has none. Typed content only: the sim's use of it is SIM-72's.
    pub node_income: Option<crate::NodeIncome>,
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
    /// Authored `facility` flag for station recrewing (SIM-70). An explicit
    /// false is distinct from an absent key; runtime policy resolves absence.
    pub facility: Option<bool>,
    /// Authored `has_crew` flag for station recrewing (SIM-70). An explicit
    /// false is distinct from an absent key; runtime policy resolves absence.
    pub has_crew: Option<bool>,
    /// `maxCrewGain` of a station (SIM-25): what it adds to its team's crew capacity.
    pub crew_gain: Fixed,
    /// The unit's transporter and boarding keys (SIM-46 P0); typed, not read by the sim yet.
    pub transport: TransportDef,
    /// `saneCrewLossMethod` (SIM-39): above 100 crew a shield hit's loss is
    /// in crew points; without it (or at 100 or less) it is a percent of the
    /// unit's current crew.
    pub sane_crew_loss: bool,
    /// Hit points of the unit's systems (SIM-55), in the order engines,
    /// weapons, shield generator, life support, sensors: `enginesHitPoints`,
    /// `weaponsHitPoints`, `shieldGeneratorHitPoints`, `lifeSupportHitPoints`,
    /// `sensorsHitPoints`. 0 when the key is absent.
    pub system_hit_points: [Fixed; 5],
    /// The keys of the engine's crew-casualty model (SIM-39).
    pub casualty: CasualtyDef,
    /// Authored repair metadata only; no profile or runtime policy is implied.
    pub repair: RepairDef,
    /// Raw composed SOD transform of the authored repair node, when readable.
    pub repair_attachment: Option<crate::repair_geometry::RepairAttachment>,
    pub research_slots: Vec<ResearchSlot>,
    /// Station upgrade pods use the station origin instead of an external slot.
    pub research_station_upgrade: bool,
    /// Sensor range, units.
    pub sensor: Fixed,
    pub hardpoints: Vec<Hardpoint>,
    pub build: BuildDef,
    /// Where finished units leave this builder; `None`: not read (the unit
    /// appears at the builder's centre, as before SIM-16).
    pub launch: Option<LaunchPoint>,
    /// `freighterName` of a mining station: the unit (lower-cased ODF name) that
    /// appears free when a construction rig finishes the station; `None`: no key
    /// or the engine's "Unknown" default. Not resolved against the database: a
    /// name that is not a unit spawns nothing.
    pub free_unit: Option<String>,
    /// Where that unit appears: the station's dock node (`dockingHardpoint`,
    /// default `dock`), read from its model like [`launch`](UnitDef::launch).
    /// Only set when `free_unit` is.
    pub dock: Option<LaunchPoint>,
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
    /// What a `nebula_obj` does to the craft inside it (SIM-75); `None` for
    /// every other class.
    pub nebula_effects: Option<NebulaEffects>,
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
    /// The craft and object keys both stock games read (CON-55). Typed
    /// only: SIM-74 applies them.
    pub stats: UnitStatsDef,
    pub raw: Arc<sta_odf::Odf>,
}

/// Craft and object stat keys (CON-55), each `None` (a list: empty) when the
/// ODF does not set it. The key names and value types are read from the stock
/// executables, static; nothing here is measured. Not yet applied by the sim.
/// System slots are in [`UnitDef::system_hit_points`] order: engines, weapons,
/// shield generator, life support, sensors.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnitStatsDef {
    /// `intrinsicValue`: the unit's worth to target scoring and the AI.
    pub intrinsic_value: Option<Fixed>,
    /// `criticalHitPercent` (Armada I).
    pub critical_hit_percent: Option<Fixed>,
    /// `hullTargetHardpoints`, as written: an empty name keeps its place.
    pub hull_target_hardpoints: Vec<String>,
    /// `criticalTargetHardpoints`.
    pub critical_target_hardpoints: Vec<String>,
    /// `<system>TargetHardpoints` per system slot.
    pub system_target_hardpoints: [Vec<String>; 5],
    /// `<system>SparkTime` per system slot, seconds.
    pub system_spark_time: [Option<Fixed>; 5],
    /// `shipType` (Armada II), lower case.
    pub ship_type: Option<String>,
    /// `isHero`.
    pub is_hero: Option<bool>,
    /// `selfDestruct`: the ODF of the self-destruct weapon, lower case.
    pub self_destruct: Option<String>,
    /// `missileDetonator` (Armada II): an ODF name, lower case.
    pub missile_detonator: Option<String>,
    /// `defaultGreenAlert`.
    pub default_green_alert: Option<bool>,
    /// `alwaysUseSpecialWeapons`.
    pub always_use_special_weapons: Option<bool>,
    /// `damagedScan`.
    pub damaged_scan: Option<Fixed>,
    /// `avoidanceClass`.
    pub avoidance_class: Option<i32>,
    /// `collisionRadius`.
    pub collision_radius: Option<Fixed>,
    /// `distanceBelowGrid`: a shared AI build-list identity in the Roots profile when positive.
    pub distance_below_grid: Option<Fixed>,
    /// `shieldPad`.
    pub shield_pad: Option<Fixed>,
    /// `maxExtraWeapons` (Armada II).
    pub max_extra_weapons: Option<i32>,
    /// `canAttack`: 0 keeps the class from taking up a fight by itself (read by
    /// `sta_sim::movement::approach`; default 1).
    pub can_attack: Option<bool>,
    /// `canAttackPlanets` (Armada II).
    pub can_attack_planets: Option<bool>,
    /// `canOnlyAttackPlanets` (Armada II).
    pub can_only_attack_planets: Option<bool>,
    /// `recyclesInPlace` (Armada II).
    pub recycles_in_place: Option<bool>,
    /// `addToPlanner` (Armada II craft and stations): 0 keeps the unit out of
    /// the path planner (stock orbital stations). Read by `sta_sim::nav`.
    pub add_to_planner: Option<bool>,
    /// `pushBuildItem0..9`: the final-build queue entries in slot order, as class
    /// stems (gaps and `null` skipped, repeats kept); read by `sta_sim::production`.
    pub push_build_items: Vec<String>,
    /// `shipIsBuiltByEvolution` (Armada II).
    pub built_by_evolution: Option<bool>,
    /// `shipIsBuiltByShips` (Armada II).
    pub built_by_ships: Option<bool>,
    /// `healthRate` (Armada II).
    pub health_rate: Option<Fixed>,
    /// `percentNebulaWarn` (Armada II).
    pub percent_nebula_warn: Option<Fixed>,
    /// `dockingSpeedFactor` (Armada I).
    pub docking_speed_factor: Option<Fixed>,
    /// `decommissionValue` (Armada I).
    pub decommission_value: Option<i32>,
    /// `aiName`: the name the AI build lists know the unit by, lower case.
    pub ai_name: Option<String>,
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

/// What a `nebula_obj` does to the craft inside it (SIM-75), from its ODF.
/// Key names and defaults: read from the executable, static (Armada I and
/// Armada II share them; Armada I has no `craft_max_speed_fraction`). Rates are
/// per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NebulaEffects {
    /// `disableWeapons`: weapons off inside.
    pub disable_weapons: bool,
    /// `disableShields`: shield generator off inside.
    pub disable_shields: bool,
    /// `disableCrew`: life support off inside.
    pub disable_crew: bool,
    /// `disableSensors`: sensors off inside (read, not applied yet).
    pub disable_sensors: bool,
    /// `disableCloak`: no cloak inside.
    pub disable_cloak: bool,
    /// `hideContents`: craft inside are hidden (read, not applied yet).
    pub hide_contents: bool,
    /// `damageValue`: damage per second.
    pub damage: Fixed,
    /// `damageThreshold` (default 50), `damageCrewModifier` (0.2) and
    /// `damageHullModifier` (0.1): the crew law of that damage, as an
    /// ordnance's `damageThreshold`, `shieldCrewModifier`, `hullCrewModifier`.
    pub crew_loss: OrdnanceCrewLoss,
    /// `regenValue` (whole number): shield points per second.
    pub regen: Fixed,
    /// `repairValue` (whole number): system hit points per second.
    pub repair: Fixed,
    /// `craft_speed_coefficient` (default 0.5): a multiple of the craft's speed.
    /// Applied under the Armada I profile only (interim, not measured); the
    /// Armada II family ignores it (sta_sim `nebula::speed_caps`).
    pub speed_coefficient: Fixed,
    /// `craft_speed_limit`: a speed cap; `None` for a negative value (the
    /// default -1, no cap). Applied under the Armada I profile only; ignored by
    /// the Armada II family.
    pub speed_limit: Option<Fixed>,
    /// `craft_max_speed_fraction`: above 0 it replaces the two keys above: the
    /// speed is capped at this fraction of impulse speed. The only one of the
    /// three that slows a craft in the Armada II family.
    pub max_speed_fraction: Option<Fixed>,
}

impl UnitDef {
    /// Armada II mobile producers such as Mother are authored as ships
    /// with movement physics and no fixed-facility flag (read from ODFs).
    pub fn allows_station_motion(&self) -> bool {
        matches!(self.class.0.as_str(), "starbase" | "shipyard" | "buildyard" | "research" | "mining" | "sensor" | "trading_station")
            && self.movement.max_speed > Fixed::ZERO
            && !self.is_starbase
            && self.raw.bool("ship") == Some(true)
    }

    /// Wormhole opening duration. Armada II reads AnimationTime in seconds.
    pub fn wormhole_open_ticks(&self) -> u64 {
        let seconds = self.raw.f32("AnimationTime").filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(2.0);
        ((f64::from(seconds) * f64::from(sta_core::TICK_HZ)).round() as u64).max(1)
    }

    /// Armada II's embryo producer, consumed when its product finishes.
    pub fn is_evolver(&self) -> bool {
        self.class.0 == "evolver"
    }

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
            fleet_cap: None,
            orbit_time: None,
            a1_borg: None,
            station_mining: None,
            node_income: None,
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
            facility: None,
            has_crew: None,
            crew_gain: Fixed::ZERO,
            transport: TransportDef::default(),
            sane_crew_loss: false,
            system_hit_points: [Fixed::ZERO; 5],
            casualty: CasualtyDef::default(),
            repair: RepairDef::default(),
            repair_attachment: None,
            research_slots: Vec::new(),
            research_station_upgrade: false,
            sensor: Fixed::ZERO,
            hardpoints: Vec::new(),
            build: BuildDef::default(),
            launch: None,
            free_unit: None,
            dock: None,
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
            nebula_effects: None,
            avoid_me: false,
            hidden: false,
            invalid_as_target: false,
            attack_power: None,
            life: None,
            stats: UnitStatsDef::default(),
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
    /// A mining beam's drawn size: ordnance `radiusScale` and `lengthScale` (read from data;
    /// the client scales the `shotGeometry` model by `(radius, radius, length / lengthScale)`).
    /// `None` for every other weapon and for a mining beam with neither key. Render only: not
    /// digested.
    pub shot_scale: Option<ShotScale>,
    /// What the stock look draws the shot from (ordnance look keys). Render only: not digested.
    pub look: ShotLook,
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
    /// `damageVariance` of the ordnance (SIM-39): each hit does `damageBase`
    /// plus or minus up to this much, uniformly. 0 when absent.
    pub damage_variance: Fixed,
    /// Feature switches resolved for this weapon, as [`UnitDef::features`];
    /// `None`: the database-wide set. Read through
    /// `ContentDb::weapon_features`.
    pub features: Option<Features>,
    /// `resourceweapon` class only (SIM-24): what the weapon adds to its
    /// owner's stock and who it works for. `None` for every other class.
    pub resource: Option<ResourceWeaponDef>,
    /// A `replaceweapon`, including manual activation (SIM-28); `None` for other weapons.
    pub replace: Option<crate::lifecycle::ReplaceDef>,
    /// The colonizer pod this weapon launches (SIM-30): set when the ordnance's
    /// class is `ColonizerPod`; `None` for every other weapon.
    pub colonize: Option<ColonizePodDef>,
    /// The weapon and ordnance base keys both stock games read (CON-54).
    /// Typed only: the sim rules that apply them are SIM-73.
    pub base: WeaponBaseDef,
    pub raw: Arc<sta_odf::Odf>,
}

/// A mining beam's `radiusScale` and `lengthScale` (ordnance keys, read from data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShotScale {
    pub radius: Fixed,
    pub length: Fixed,
}

/// The data a stock shot is drawn from: the ordnance ODF's look keys, plus the defaults
/// STA64 applies where a key is absent. Render only: nothing here is read by the sim or
/// digested (D10: peers that differ only in how a shot looks play together). Every
/// `Option` is `None` when the key is absent and no default is known. Lengths are in
/// world units, times in seconds. Evidence grades: key names and which classes read them
/// "read from the executable, static"; presence of a key "read from data".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShotLook {
    /// `Sprite`: the sprite's name in the sprite tables (any ordnance class).
    pub sprite: Option<String>,
    /// `spriteDuration`: when above 0 and the sprite is animated, it replaces the length of
    /// the sprite's first animation channel. The sprite is shared between shots, so which
    /// class wins when several name it is INTERIM: class load order not determined.
    pub sprite_duration: Option<f32>,
    /// `Radius`: a ribbon's full width, or a billboard's half size (class dependent).
    pub radius: Option<f32>,
    /// `Length`: a pulse's streak length.
    pub length: Option<f32>,
    /// `lifeSpan`: the shot's life, every ordnance class (the sim-facing `WeaponDef::duration`
    /// stays beams-only).
    pub life_span: Option<f32>,
    /// `travelTimeFraction`, beam classes only ([`BEAM_CLASSES`](crate::weapon::BEAM_CLASSES)):
    /// the share of `lifeSpan` the beam takes to grow out and to retract. The key, else
    /// [`ShotLook::DEFAULT_TRAVEL_TIME_FRACTION`]. `None` for other classes.
    pub travel_time_fraction: Option<f32>,
    /// The muzzle flare, class `phaser` only.
    pub flare: Option<ShotFlare>,
    /// The point light the shot carries (`lightColor`); `None` without that key.
    pub light: Option<ShotLight>,
}

impl ShotLook {
    /// `travelTimeFraction` when the key is absent (read from the executable, static).
    pub const DEFAULT_TRAVEL_TIME_FRACTION: f32 = 0.1;
}

/// A phaser's muzzle flare: `flareSprite` and `flareRadiusMultiplier` of the ordnance ODF.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotFlare {
    /// `flareSprite`, else [`ShotFlare::DEFAULT_SPRITE`].
    pub sprite: String,
    /// `flareRadiusMultiplier`, else [`ShotFlare::DEFAULT_RADIUS_MULTIPLIER`]; the flare's
    /// half size is `Radius` times this.
    pub radius_multiplier: f32,
}

impl ShotFlare {
    /// `flareSprite` when absent (read from the executable, static).
    pub const DEFAULT_SPRITE: &'static str = "pflare";
    /// `flareRadiusMultiplier` when absent (read from the executable, static).
    pub const DEFAULT_RADIUS_MULTIPLIER: f32 = 1.4;
}

/// A shot's point light: `lightColor`, `lightFalloffStart`, `lightFalloffRange`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotLight {
    /// `lightColor`'s three numbers as authored. Their scale (0..1 or 0..255) is INTERIM:
    /// not determined; the existing recipe reading treats a maximum above 1 as 0..255.
    pub color: [f32; 3],
    /// `lightFalloffStart`, else [`ShotLight::DEFAULT_FALLOFF_START`].
    pub falloff_start: f32,
    /// `lightFalloffRange`, else [`ShotLight::DEFAULT_FALLOFF_RANGE`]. How start and range
    /// shape the attenuation is INTERIM: the formula was not read.
    pub falloff_range: f32,
}

impl ShotLight {
    /// `lightFalloffStart` when absent (read from the executable, static).
    pub const DEFAULT_FALLOFF_START: f32 = 100.0;
    /// `lightFalloffRange` when absent (read from the executable, static).
    pub const DEFAULT_FALLOFF_RANGE: f32 = 5.0;
}

/// A value with per-target rows (CON-54): `key = base` followed by
/// `"name" value` lines. Which target a row's name selects is the sim's rule
/// (SIM-73); here the names are kept lower-cased as written, in file order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lookup {
    pub base: Fixed,
    /// Shared between the weapons whose tables are equal: Fleet Operations
    /// repeats the same few tables over millions of rows.
    pub rows: LookupRows,
}

/// The rows of a [`Lookup`]: `(name, value)` in file order.
pub type LookupRows = Arc<[(Arc<str>, Fixed)]>;

impl Lookup {
    /// The rows as `(name, value)`, in file order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Fixed)> {
        self.rows.iter().map(|(name, v)| (&**name, *v))
    }
}

/// To-hit modifiers of a weapon (CON-54), signed, `None` when the ODF has no
/// such key. `<state>ToHit` is the shooter's own movement state, `toHit<state>`
/// the target's (our reading of the key names; not measured). `warp` and
/// `slow` are Armada I's states, `impulse` Armada II's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToHitDef {
    pub stop_to_hit: Option<Fixed>,
    pub to_hit_stop: Option<Fixed>,
    pub impulse_to_hit: Option<Fixed>,
    pub to_hit_impulse: Option<Fixed>,
    pub warp_to_hit: Option<Fixed>,
    pub to_hit_warp: Option<Fixed>,
    pub slow_to_hit: Option<Fixed>,
    pub to_hit_slow: Option<Fixed>,
}

/// What a weapon may be aimed at (CON-54). `None`: the key is absent and the
/// weapon class's own default applies (not typed here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TargetMaskDef {
    /// `targetEnemyCraft`.
    pub enemy_craft: Option<bool>,
    /// `targetEnemyBuildings`.
    pub enemy_buildings: Option<bool>,
    /// `targetAlliedCraft`.
    pub allied_craft: Option<bool>,
    /// `targetAlliedBuildings`.
    pub allied_buildings: Option<bool>,
    /// `targetEnemyPlanets` (Armada II).
    pub enemy_planets: Option<bool>,
    /// `targetEnemyPlanetsOnly` (Armada II).
    pub enemy_planets_only: Option<bool>,
    /// `targetLocation`: aimed at a point, not a unit.
    pub location: Option<bool>,
    /// `AItargetLocation`: the same for computer players.
    pub ai_location: Option<bool>,
}

/// Weapon and ordnance base keys (CON-54), each `None` when the ODF does not
/// set it. Evidence: the key names and value types are read from the stock
/// executables, static; nothing here is measured. Not yet applied by the sim.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WeaponBaseDef {
    /// `hitChance` with its per-target rows.
    pub hit_chance: Option<Lookup>,
    /// The ordnance's `damageBase`, `damageVariance`, `damageThreshold`,
    /// `shieldCrewModifier` and `hullCrewModifier`, only when the key has
    /// per-target rows (the plain values are `WeaponDef::damage`,
    /// `damage_variance` and `crew_loss`).
    pub damage_base: Option<Lookup>,
    pub damage_variance: Option<Lookup>,
    pub damage_threshold: Option<Lookup>,
    pub shield_crew_modifier: Option<Lookup>,
    pub hull_crew_modifier: Option<Lookup>,
    pub to_hit: ToHitDef,
    pub targets: TargetMaskDef,
    /// `needTarget`.
    pub need_target: Option<bool>,
    /// `beneficial`: the weapon helps its target.
    pub beneficial: Option<bool>,
    /// `switchToAttack`.
    pub switch_to_attack: Option<bool>,
    /// `cloakEnable`: usable while cloaked.
    pub cloak_enable: Option<bool>,
    /// `special`: the weapon is a special weapon (read by `sta_sim::movement::approach`).
    pub special: Option<bool>,
    /// `warpEnable` (Armada II): usable at warp.
    pub warp_enable: Option<bool>,
    /// `crewAffectsWeaponDelay`.
    pub crew_affects_weapon_delay: Option<bool>,
    /// `ignoreValidAsTarget` (Armada II).
    pub ignore_valid_as_target: Option<bool>,
    /// `regularWeaponDelay`, seconds.
    pub regular_weapon_delay: Option<Fixed>,
    /// Ordnance `ignoreShield`.
    pub ignore_shield: Option<bool>,
    /// Ordnance `forceInvalidateTarget` (Armada II).
    pub force_invalidate_target: Option<bool>,
    /// Ordnance `canAttackStations` (Armada II mines).
    pub can_attack_stations: Option<bool>,
    /// Ordnance `specialValue`.
    pub special_value: Option<Fixed>,
    /// Ordnance `shieldDuration`, seconds.
    pub shield_duration: Option<Fixed>,
    /// Ordnance `omegaTurn`: a seeking shot's turn rate.
    pub omega_turn: Option<Fixed>,
    /// Ordnance `seekTime`, seconds.
    pub seek_time: Option<Fixed>,
    /// Ordnance `shotAccel`.
    pub shot_accel: Option<Fixed>,
    /// Ordnance `detectRadius` (mines).
    pub detect_radius: Option<Fixed>,
    /// Ordnance `travelTimeFraction` (Armada II beams).
    pub travel_time_fraction: Option<Fixed>,
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
    /// `shieldCrewModifier`: the crew law's factor for the shield part over the threshold.
    pub shield: Fixed,
    /// `hullCrewModifier`: crew lost by a hit that lands on the crew location.
    pub hull: Fixed,
}

/// A unit's transporter and boarding keys (SIM-46 P0). Read, typed and
/// exposed only: no simulation system applies them yet and they are not in
/// the content digest until one does. Absent keys keep the engine's default
/// (read from the executable, static); `None` marks a key the ODF does not
/// carry where its absence means something other than a default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportDef {
    /// `transporter`: the unit can transport. `None` when no ODF in its
    /// include chain has the key (Armada I has none).
    pub transporter: Option<bool>,
    /// `assault`: the unit may be ordered to capture (stock and Fleet Operations).
    pub assault: bool,
    /// `boardingPartyStrength`: multiplier on the carrier's boarding parties;
    /// 1.0 when absent, an explicit zero is kept.
    pub boarding_party_strength: Fixed,
    /// `disableShieldsOnTransport`: the carrier's own shields drop while it
    /// transports; true when absent, an explicit zero is kept.
    pub disable_shields_on_transport: bool,
    /// `weldingAnimSpeed` (Roots): only zero against non-zero matters to the
    /// transport-attack rule; `None` when absent.
    pub welding_anim_speed: Option<Fixed>,
    /// `shieldDelay`, seconds: how long a unit's shields stay down after a
    /// hit; `None` when absent.
    pub shield_delay: Option<Fixed>,
    /// `eventTransport`: the event a transport plays (presentation).
    pub event_transport: Option<String>,
}

impl Default for TransportDef {
    fn default() -> Self {
        TransportDef {
            transporter: None,
            assault: false,
            boarding_party_strength: Fixed::ONE,
            disable_shields_on_transport: true,
            welding_anim_speed: None,
            shield_delay: None,
            event_transport: None,
        }
    }
}

/// A unit's crew-casualty keys (SIM-39). Arrays are in the order of
/// [`UnitDef::system_hit_points`]: engines, weapons, shield generator, life
/// support, sensors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CasualtyDef {
    /// `shieldProtection`; `None`: the `RTS_CFG.h` `SHIELD_DAMAGE_PROTECTION`.
    pub shield_protection: Option<Fixed>,
    /// `enginesHitPercent`, `weaponsHitPercent`, `shieldGeneratorHitPercent`,
    /// `lifeSupportHitPercent`, `sensorsHitPercent`: the weight of the hit-location roll.
    pub system_hit_percent: [Fixed; 5],
    /// `crewHitPercent`.
    pub crew_hit_percent: Fixed,
    /// `hullHitPercent`; `None`: what the others leave of 100.
    pub hull_hit_percent: Option<Fixed>,
    /// `enginesCrewLoss`, `weaponsCrewLoss`, `shieldGeneratorCrewLoss`,
    /// `lifeSupportCrewLoss`, `sensorsCrewLoss`: percent of crew a dead system kills.
    pub system_crew_loss: [Fixed; 5],
    /// `lifeSupportLoss`: percent of crew per second lost while life support is down.
    pub life_support_loss: Fixed,
    /// `recrewRate`: crew per second a station recrews; `None` is the engine's 10.
    pub recrew_rate: Option<Fixed>,
}

impl Default for CasualtyDef {
    fn default() -> Self {
        CasualtyDef {
            shield_protection: None,
            system_hit_percent: [Fixed::ZERO; 5],
            crew_hit_percent: Fixed::ZERO,
            hull_hit_percent: None,
            system_crew_loss: [Fixed::ZERO; 5],
            life_support_loss: Fixed::ZERO,
            recrew_rate: None,
        }
    }
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
            shot_scale: None,
            look: ShotLook::default(),
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
            damage_variance: Fixed::ZERO,
            features: None,
            resource: None,
            replace: None,
            colonize: None,
            base: WeaponBaseDef::default(),
            raw: Arc::new(raw),
        }
    }
}
