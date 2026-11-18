//! [`UnitDef`] from a unit's resolved ODF (CON-03). See [`crate::odf`].
//!
//! Keys read (all optional; absent means zero or empty unless noted):
//!
//! | field | key |
//! |---|---|
//! | `class`, `race` | `classLabel`, `race` |
//! | `model`, `scale` | `basename` (else the ODF's own name), `ScaleSOD` |
//! | `hull`, `initial_hull` | `maxHealth`, optional `curHealth` (otherwise maximum) |
//! | `shields`, `initial_shields` | `maxShield`/`maxShields`, `curShield`/`curShields`, `shieldRate` per second; A2/FO absent strength inherits health |
//! | `crew`, `maximum_crew` | `crewCost` (starting/build amount), optional `maximumCrew` (otherwise `crewCost`) |
//! | `transport` | `transporter`, `assault`, `boardingPartyStrength` (1.0), `disableShieldsOnTransport` (true), `weldingAnimSpeed`, `shieldDelay`, `eventTransport` (SIM-46 P0, typed only) |
//! | `facility`, `has_crew` | optional `facility`, `has_crew` flags for station recrewing; explicit false is preserved |
//! | `sensor` | `rangeScan` |
//! | `attack_power` | `attackPower` (absent: `None`) |
//! | `movement` | the physics ODF named by `physicsFile`, see [`movement`] |
//! | `hardpoints` | `weapon<n>` with `weaponHardPoints<n>` (first node named); `STA64_fireArc<n>` the slot's own arc half-width in radians |
//! | `build` | `dilithiumCost`, `latinumCost`, `metalCost`, `biomatterCost`, `buildTime` seconds, `officerCost`, `buildItem<n>` |
//! | `repair` | authored `enginesRepairTime`, `weaponsRepairTime`, `shieldGeneratorRepairTime`, `lifeSupportRepairTime`, `sensorsRepairTime` seconds per system hit point before runtime modifiers; `repairFacility`, `repairHardpoint`, `repairSide`, `canShipyardRepair` (metadata only; no effective defaults) |
//! | `sounds` | `eventSelect`, `eventAcknowledge`, `eventAttack`, `ambientSound` (event and file names, resolved by the client) |
//! | `fx` | `fireball` |
//! | specials, mining | `maxSpecialEnergy`, `specialEnergyRate`, `miner` (`miner = 0` says not a miner even for class `freighter`: [`UnitDef::mines`]), `maxdilithium`, `miningRate`, `dilithiumTransferRate`, `resource` |

use sta_core::{Fixed, TICK_HZ};
use sta_odf::Odf;

use crate::def::{ClassLabel, DefRef, Hardpoint, MoveStats, ShieldDef, UnitDef};
use crate::odf::{numbered, stem, words, Reader};
use crate::Warning;

/// Movement limits from a physics ODF: `STATIC` unless `impulseSpeed` is
/// positive. `forwardAccel` is a reciprocal of seconds to full speed: speed
/// changes by `forwardAccel * impulseSpeed` per second (our reading). Missing
/// keys take [`MoveStats::default`]'s values.
pub fn movement(phys: Option<&Odf>, file: &str, warnings: &mut Vec<Warning>) -> MoveStats {
    let Some(ph) = phys else { return MoveStats::STATIC };
    let mut r = Reader { odf: ph, file, warnings };
    let speed = r.num("impulseSpeed").unwrap_or(0.0);
    if speed <= 0.0 {
        return MoveStats::STATIC;
    }
    let m = MoveStats::from_f32(
        speed as f32,
        r.num("forwardAccel").unwrap_or(1.0) as f32,
        r.num("turnOmega").unwrap_or(1.0) as f32,
        r.num("turnAlpha").unwrap_or(1.0) as f32,
        r.num("turnOmegaFractionAtRest").unwrap_or(0.2) as f32,
    );
    let mut m = with_smooth_keys(m, |k| r.num(k));
    if let Some(v) = r.num("warpSpeed").filter(|v| *v > 0.0) {
        m.warp_speed = Fixed::from_f64(v);
    }
    // Armada I's warp-on-move keys (SIM-64); an ODF without `tooCloseForWarp` never asks.
    if let Some(v) = r.num("tooCloseForWarp").filter(|v| *v > 0.0) {
        m.too_close_for_warp = Fixed::from_f64(v);
        m.heading_close_for_warp = Fixed::from_f64(r.num("headingCloseEnoughForWarp").unwrap_or(0.0).clamp(0.0, 180.0));
    }
    m
}

/// `m` with the smooth-physics keys (SIM-15) a physics ODF sets; keys it
/// leaves out keep `m`'s values. `num` reads one key as a number.
pub(crate) fn with_smooth_keys(mut m: MoveStats, mut num: impl FnMut(&str) -> Option<f64>) -> MoveStats {
    let mut get = |key: &str, at_least: f64| num(key).map(|v| Fixed::from_f64(v.max(at_least)));
    if let Some(v) = get("combatSpeed", 0.0).filter(|v| *v > Fixed::ZERO) {
        m.combat_speed = v;
    }
    if let Some(v) = get("turnAlphaFractionAtRest", 0.0) {
        m.turn_alpha_at_rest = v.min(Fixed::ONE);
    }
    if let Some(v) = get("tooCloseToTurn", 0.0) {
        m.too_close_to_turn = v;
    }
    if let Some(v) = get("pathLeadDistance", 0.0) {
        m.path_lead = v;
    }
    if let Some(v) = get("turnControlAngle", 0.0) {
        m.turn_control_angle = v;
    }
    if let Some(v) = get("turnControlSquared", 0.0) {
        m.turn_control_squared = v != Fixed::ZERO;
    }
    if let Some(v) = get("forwardControlDistance", 0.0) {
        m.forward_control_distance = v;
    }
    m
}

/// Resource id for a node's `resource` text when nothing better is known
/// (stock names; [`crate::odf::load_with`] replaces it with the node ODF's
/// `amt<slot>` keys and the profile's own words).
fn resource_id(text: &str) -> u8 {
    crate::ResourceModel::default().slot_of_text(text).unwrap_or(0) as u8
}

/// The definition of the unit `name`; `phys` is its resolved physics ODF.
/// Weapon and build references carry ODF names only; [`crate::odf::load_with`]
/// drops those that are not defined and fills the arcs.
pub fn unit_def(name: &str, odf: &Odf, phys: Option<&Odf>, warnings: &mut Vec<Warning>) -> UnitDef {
    unit_def_for_profile(name, odf, phys, crate::Profile::Native, warnings)
}

pub(crate) fn unit_def_for_profile(name: &str, odf: &Odf, phys: Option<&Odf>, profile: crate::Profile, warnings: &mut Vec<Warning>) -> UnitDef {
    let file = format!("{}.odf", stem(name));
    let class = odf.class_label().unwrap_or("");
    let mut d = UnitDef::new(name.trim().trim_end_matches(".odf"), class);
    d.class = ClassLabel::new(class.trim());
    d.raw = std::sync::Arc::new(odf.clone());
    d.hidden = crate::lifecycle::hidden(odf);
    d.invalid_as_target = crate::lifecycle::invalid_as_target(odf);
    d.life = crate::lifecycle::life_ticks(odf);
    d.confinement = crate::confinement::from_odf(odf, &file, warnings);
    let mut r = Reader { odf, file: &file, warnings };

    d.race = r.text("race").unwrap_or_default();
    d.model = r.text("basename").unwrap_or_else(|| stem(name));
    d.scale = match r.num("ScaleSOD") {
        Some(s) if s > 0.0 => Fixed::from_f64(s),
        _ => Fixed::ONE,
    };
    d.hull = r.fixed("maxHealth");
    d.initial_hull = odf.raw("curHealth").map(|_| r.fixed("curHealth"));
    // Test presence, not value: an authored zero must never inherit health.
    let shield_key = |singular, plural| {
        if odf.raw(singular).is_some() { Some(singular) }
        else if odf.raw(plural).is_some() { Some(plural) }
        else { None }
    };
    let max_key = shield_key("maxShield", "maxShields");
    let current_key = shield_key("curShield", "curShields");
    d.initial_shields = current_key.map(|key| r.fixed(key));
    let max_shields = match max_key {
        Some(key) => r.fixed(key),
        None => d.initial_shields.unwrap_or_default(),
    };
    d.shields = ShieldDef { max: max_shields, recharge: r.fixed("shieldRate") / Fixed::from_int(TICK_HZ as i32) };
    if matches!(profile, crate::Profile::Sta2Classic | crate::Profile::Fo4 | crate::Profile::Fo3)
        && max_key.is_none() && current_key.is_none()
    {
        d.shields.max = d.hull;
        d.initial_shields = d.initial_hull;
    }
    d.crew = r.fixed("crewCost");
    d.maximum_crew = odf.raw("maximumCrew").map(|_| r.fixed("maximumCrew"));
    d.facility = r.flag("facility");
    d.has_crew = r.flag("has_crew");
    // Planets carry their own typed `planet_crew_gain` and ignore this key here.
    if !matches!(d.class.0.as_str(), "planet" | "planet_obj" | "moon") {
        d.crew_gain = r.fixed("maxCrewGain").min(Fixed::from_int(1_000_000_000));
    }
    d.sane_crew_loss = r.flag("saneCrewLossMethod").unwrap_or(false);
    // SIM-46 P0: typed, not read by the sim yet. A non-number warns and keeps the default; negatives read as zero.
    d.transport = crate::def::TransportDef {
        transporter: r.flag("transporter"),
        assault: r.flag("assault").unwrap_or(false),
        boarding_party_strength: r.num("boardingPartyStrength").map_or(Fixed::ONE, |v| Fixed::from_f64(v.max(0.0))),
        disable_shields_on_transport: r.flag("disableShieldsOnTransport").unwrap_or(true),
        welding_anim_speed: r.num("weldingAnimSpeed").map(|v| Fixed::from_f64(v.max(0.0))),
        shield_delay: r.num("shieldDelay").map(|v| Fixed::from_f64(v.max(0.0))),
        event_transport: r.text("eventTransport"),
    };
    d.system_hit_points = [
        r.fixed("enginesHitPoints"),
        r.fixed("weaponsHitPoints"),
        r.fixed("shieldGeneratorHitPoints"),
        r.fixed("lifeSupportHitPoints"),
        r.fixed("sensorsHitPoints"),
    ];
    d.casualty = crate::def::CasualtyDef {
        shield_protection: r.odf.raw("shieldProtection").map(|_| r.fixed("shieldProtection")),
        system_hit_percent: [
            r.fixed("enginesHitPercent"),
            r.fixed("weaponsHitPercent"),
            r.fixed("shieldGeneratorHitPercent"),
            r.fixed("lifeSupportHitPercent"),
            r.fixed("sensorsHitPercent"),
        ],
        crew_hit_percent: r.fixed("crewHitPercent"),
        hull_hit_percent: r.odf.raw("hullHitPercent").map(|_| r.fixed("hullHitPercent")),
        system_crew_loss: [
            r.fixed("enginesCrewLoss"),
            r.fixed("weaponsCrewLoss"),
            r.fixed("shieldGeneratorCrewLoss"),
            r.fixed("lifeSupportCrewLoss"),
            r.fixed("sensorsCrewLoss"),
        ],
        life_support_loss: r.fixed("lifeSupportLoss"),
        recrew_rate: r.odf.raw("recrewRate").map(|_| r.fixed("recrewRate")),
    };
    for (slot, key) in [
        "enginesRepairTime", "weaponsRepairTime", "shieldGeneratorRepairTime",
        "lifeSupportRepairTime", "sensorsRepairTime",
    ].into_iter().enumerate() {
        d.repair.system_times[slot] = match r.num(key) {
            Some(seconds) if seconds >= 0.0 => Some(Fixed::from_f64(seconds)),
            Some(_) => {
                r.warnings.push(Warning::new(&file, format!("{key} is negative; ignored")));
                None
            }
            None => None,
        };
    }
    d.repair.repair_facility = r.flag("repairFacility");
    d.repair.repair_hardpoint = r.text("repairHardpoint");
    // Parse the authored decimal exactly: Reader::num converts through f32
    // and clamps large values, which would silently change a side number.
    d.repair.repair_side = odf.raw("repairSide").and_then(|_| match r.text("repairSide").and_then(|s| s.parse::<i32>().ok()) {
        Some(side) => Some(side),
        None => {
            r.warnings.push(Warning::new(&file, "repairSide is not a signed whole number; ignored"));
            None
        }
    });
    d.repair.can_shipyard_repair = r.flag("canShipyardRepair");
    d.repair.repairs_shields = r.flag("repairsShields");
    d.repair.repair_rate = odf.raw("repairRate").map(|_| r.fixed("repairRate").max(Fixed::ZERO).min(Fixed::from_int(1_000)));
    d.sensor = r.fixed("rangeScan");
    d.attack_power = odf.raw("attackPower").map(|_| r.fixed("attackPower").max(Fixed::ZERO).min(Fixed::from_int(1_000_000)));

    for (n, raw) in numbered(odf, "weapon") {
        let weapon = raw.trim().trim_matches('"').trim();
        if weapon.is_empty() || weapon.eq_ignore_ascii_case("null") {
            continue;
        }
        let node = odf
            .raw(&format!("weaponHardPoints{n}"))
            .and_then(|h| words(h).into_iter().next())
            .unwrap_or_else(|| "root".to_owned());
        // `STA64_fireArc<N>`: this slot's own half-width (radians), where stock ODFs
        // only limit a weapon wherever it is mounted (`restrictFireArc`, `fireArc`).
        let arc_key = format!("STA64_fireArc{n}");
        let arc = odf.raw(&arc_key).and_then(|t| {
            // f64, not the ODF number reader's f32: the angle's cosine is hashed.
            let text = t.trim().trim_end_matches([';', 'f', 'F']).trim().trim_matches('"');
            match text.parse::<f64>() {
                Ok(half) if half.is_finite() && half > 0.0 => Some(crate::weapon::fire_arc(half)),
                _ => {
                    r.warnings.push(Warning::new(&file, format!("{arc_key} = {t} is not a positive angle; no arc")));
                    None
                }
            }
        });
        d.hardpoints.push(Hardpoint { weapon: DefRef::new(weapon), node, arc });
    }

    let mut cost: Vec<Fixed> = (0..crate::resources::SLOTS).map(|s| r.fixed(&crate::resources::cost_key(s))).collect();
    while cost.last() == Some(&Fixed::ZERO) {
        cost.pop();
    }
    d.build.cost = cost;
    d.build.ticks = r.ticks("buildTime");
    d.build.officers = r.fixed("officerCost");
    for (_, raw) in numbered(odf, "builditem") {
        let item = raw.trim().trim_matches('"').trim();
        // `NULL` leaves a gap in the button grid; it is not a reference.
        if !item.is_empty() && !item.eq_ignore_ascii_case("null") {
            d.build.list.push(DefRef::new(item));
        }
    }
    d.build.flagged_slots = crate::BuildDef::flagged_slots_of(odf);
    d.avoid_me = odf.bool("avoidMe").unwrap_or(false);

    d.sounds.select = r.text("eventSelect").into_iter().collect();
    d.sounds.ack_move = r.text("eventAcknowledge").into_iter().collect();
    d.sounds.ack_attack = r.text("eventAttack").into_iter().collect();
    d.sounds.ambient = r.text("ambientSound");
    d.fx.explosion = r.text("fireball");

    d.special_energy_max = r.fixed("maxSpecialEnergy");
    d.special_energy_rate = r.fixed("specialEnergyRate");
    let miner_key = r.num("miner");
    d.miner = miner_key.is_some_and(|v| v != 0.0);
    d.miner_off = miner_key == Some(0.0);
    d.max_dilithium = r.fixed("maxdilithium");
    d.mining_rate = r.fixed("miningRate");
    d.transfer_rate = r.fixed("dilithiumTransferRate");
    d.resource = r.text("resource").map(|t| resource_id(&t));
    // SIM-23: a nebula's path-planner cost (`pathing_cost`, a whole number
    // in every stock and FO file; a fraction rounds) and `addToPathPlanner`.
    if d.class.0.eq_ignore_ascii_case("nebula_obj") {
        d.nebula_pathing = Some(crate::NebulaPathing {
            cost: r.num("pathing_cost").map(|v| v.round() as i64),
            planned: r.num("addToPathPlanner").is_none_or(|v| v != 0.0),
        });
    }

    d.stats = stats(odf, &file, warnings);
    d.movement = movement(phys, &file, warnings);
    d
}

/// The ODF key stems of the five systems, in `system_hit_points` order.
const SYSTEMS: [&str; 5] = ["engines", "weapons", "shieldGenerator", "lifeSupport", "sensors"];

/// A list of quoted names as written; an empty `""` keeps its place. Text
/// outside quotes (a bare word) is taken as one name.
fn names(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = raw.trim();
    while !rest.is_empty() {
        if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            out.push(quoted[..end].trim().to_owned());
            rest = quoted.get(end + 1..).unwrap_or("").trim_start();
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push(rest[..end].trim_end_matches(';').to_owned());
            rest = rest[end..].trim_start();
        }
    }
    out
}

/// The CON-55 stat keys of a unit ODF. A value that does not parse warns and stays unset.
fn stats(odf: &Odf, file: &str, warnings: &mut Vec<Warning>) -> crate::def::UnitStatsDef {
    let mut r = Reader { odf, file, warnings };
    let signed = |r: &mut Reader, key: &str| r.num(key).map(Fixed::from_f64);
    let whole = |r: &mut Reader, key: &str| r.num(key).map(|v| v.round() as i32);
    let name = |r: &Reader, key: &str| r.text(key).map(|t| t.to_ascii_lowercase()).filter(|t| !t.eq_ignore_ascii_case("null"));
    let list = |key: &str| odf.raw(key).map(names).unwrap_or_default();
    crate::def::UnitStatsDef {
        intrinsic_value: signed(&mut r, "intrinsicValue"),
        critical_hit_percent: signed(&mut r, "criticalHitPercent"),
        hull_target_hardpoints: list("hullTargetHardpoints"),
        critical_target_hardpoints: list("criticalTargetHardpoints"),
        system_target_hardpoints: SYSTEMS.map(|s| list(&format!("{s}TargetHardpoints"))),
        system_spark_time: SYSTEMS.map(|s| signed(&mut r, &format!("{s}SparkTime"))),
        ship_type: name(&r, "shipType"),
        is_hero: r.flag("isHero"),
        self_destruct: name(&r, "selfDestruct"),
        missile_detonator: name(&r, "missileDetonator"),
        default_green_alert: r.flag("defaultGreenAlert"),
        always_use_special_weapons: r.flag("alwaysUseSpecialWeapons"),
        damaged_scan: signed(&mut r, "damagedScan"),
        avoidance_class: whole(&mut r, "avoidanceClass"),
        collision_radius: signed(&mut r, "collisionRadius"),
        shield_pad: signed(&mut r, "shieldPad"),
        max_extra_weapons: whole(&mut r, "maxExtraWeapons"),
        can_attack_planets: r.flag("canAttackPlanets"),
        can_only_attack_planets: r.flag("canOnlyAttackPlanets"),
        recycles_in_place: r.flag("recyclesInPlace"),
        add_to_planner: r.flag("addToPlanner"),
        built_by_evolution: r.flag("shipIsBuiltByEvolution"),
        built_by_ships: r.flag("shipIsBuiltByShips"),
        health_rate: signed(&mut r, "healthRate"),
        percent_nebula_warn: signed(&mut r, "percentNebulaWarn"),
        docking_speed_factor: signed(&mut r, "dockingSpeedFactor"),
        decommission_value: whole(&mut r, "decommissionValue"),
        ai_name: name(&r, "aiName"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn odf(text: &str) -> Odf {
        let text = text.to_owned();
        sta_odf::resolve("p.odf", &mut |_| Some(text.clone())).unwrap()
    }

    /// Armada I's warp-on-move keys (SIM-64), as `fastphys.odf` writes them
    /// (trailing `;` included); Armada II's physics files have neither.
    fn unit(text: &str) -> (UnitDef, Vec<Warning>) {
        let mut w = Vec::new();
        let d = unit_def("u", &odf(text), None, &mut w);
        (d, w)
    }

    #[test]
    fn stat_keys_absent_stay_unset() {
        let (d, w) = unit("classLabel = \"craft\"\nmaxHealth = 100\n");
        assert_eq!(d.stats, crate::def::UnitStatsDef::default());
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn stat_keys_are_read() {
        let (d, w) = unit(
            "classLabel = \"craft\"\nintrinsicValue = 0.45f\ncriticalHitPercent = 5\n\
             hullTargetHardpoints = \"hp01\" \"\" \"hp03\"\ncriticalTargetHardpoints = \"hp10\"\n\"hp11\" \"hp12\"\n\
             enginesTargetHardpoints = \"e1\" \"e2\"\nsensorsTargetHardpoints = \"s1\"\nweaponsSparkTime = 1.5\n\
             shipType = \"F\"\nisHero = 1\nselfDestruct = \"GSelfD\"\nmissileDetonator = \"gdeton\"\ndefaultGreenAlert = 0\n\
             alwaysUseSpecialWeapons = 1\ndamagedScan = 250\navoidanceClass = 3\ncollisionRadius = 12.5\nshieldPad = -2\n\
             maxExtraWeapons = 2\ncanAttackPlanets = 1\ncanOnlyAttackPlanets = 0\nrecyclesInPlace = 1\n\
             shipIsBuiltByEvolution = 1\nshipIsBuiltByShips = 0\nhealthRate = 0.2\npercentNebulaWarn = 0.5\n\
             dockingSpeedFactor = 0.25\ndecommissionValue = 40\naiName = \"Fed_Destroyer\"\n",
        );
        assert!(w.is_empty(), "{w:?}");
        let s = &d.stats;
        assert_eq!(s.intrinsic_value, Some(Fixed::from_f64(0.45f32 as f64)));
        assert_eq!(s.critical_hit_percent, Some(Fixed::from_int(5)));
        assert_eq!(s.hull_target_hardpoints, ["hp01", "", "hp03"], "an empty name keeps its place");
        assert_eq!(s.critical_target_hardpoints, ["hp10", "hp11", "hp12"], "a continued list is one list");
        assert_eq!(s.system_target_hardpoints[0], ["e1", "e2"]);
        assert_eq!(s.system_target_hardpoints[4], ["s1"]);
        assert!(s.system_target_hardpoints[1].is_empty());
        assert_eq!(s.system_spark_time, [None, Some(Fixed::from_f64(1.5)), None, None, None]);
        assert_eq!((s.ship_type.as_deref(), s.self_destruct.as_deref(), s.missile_detonator.as_deref()), (Some("f"), Some("gselfd"), Some("gdeton")));
        assert_eq!((s.is_hero, s.default_green_alert, s.always_use_special_weapons), (Some(true), Some(false), Some(true)));
        assert_eq!((s.damaged_scan, s.avoidance_class, s.max_extra_weapons), (Some(Fixed::from_int(250)), Some(3), Some(2)));
        assert_eq!((s.collision_radius, s.shield_pad), (Some(Fixed::from_f64(12.5)), Some(Fixed::from_int(-2))));
        assert_eq!((s.can_attack_planets, s.can_only_attack_planets, s.recycles_in_place), (Some(true), Some(false), Some(true)));
        assert_eq!((s.built_by_evolution, s.built_by_ships), (Some(true), Some(false)));
        assert_eq!((s.health_rate, s.percent_nebula_warn), (Some(Fixed::from_f64(0.2f32 as f64)), Some(Fixed::from_f64(0.5))));
        assert_eq!((s.docking_speed_factor, s.decommission_value, s.ai_name.as_deref()), (Some(Fixed::from_f64(0.25)), Some(40), Some("fed_destroyer")));
    }

    #[test]
    fn malformed_stat_values_warn_and_stay_unset() {
        let (d, w) = unit("classLabel = \"craft\"\nintrinsicValue = lots\nisHero = maybe\navoidanceClass = big\nselfDestruct = \"NULL\"\n");
        assert_eq!((d.stats.intrinsic_value, d.stats.is_hero, d.stats.avoidance_class, d.stats.self_destruct.as_deref()), (None, None, None, None));
        assert_eq!(w.len(), 3, "{w:?}");
    }

    #[test]
    fn armada_one_warp_keys_are_read_and_absent_keys_leave_them_zero() {
        let ph = odf("impulseSpeed = 20\nwarpSpeed = 100\ntooCloseForWarp = 10;\nheadingCloseEnoughForWarp = 30;\n");
        let m = movement(Some(&ph), "p.odf", &mut Vec::new());
        assert_eq!((m.too_close_for_warp, m.heading_close_for_warp), (Fixed::from_int(10), Fixed::from_int(30)));
        assert!(m.warp_speed > m.max_speed);
        let none = movement(Some(&odf("impulseSpeed = 20\nwarpSpeed = 100\n")), "p.odf", &mut Vec::new());
        assert_eq!((none.too_close_for_warp, none.heading_close_for_warp), (Fixed::ZERO, Fixed::ZERO));
        // A heading band without a distance asks for nothing.
        let band = movement(Some(&odf("impulseSpeed = 20\nheadingCloseEnoughForWarp = 30\n")), "p.odf", &mut Vec::new());
        assert_eq!(band.heading_close_for_warp, Fixed::ZERO);
    }

    /// The smooth-physics keys of a stock-shaped physics ODF (values and the
    /// trailing `;` as stock `sbattphys.odf` writes them).
    #[test]
    fn smooth_keys_are_read() {
        let ph = odf(
            "physics = \"smooth\"\ncombatSpeed = 40\nimpulseSpeed = 80\ntooCloseToTurn = 50;\npathLeadDistance = 150;\n\
             forwardAccel = 3;\nturnOmega = 1;\nturnAlpha = 1;\nturnOmegaFractionAtRest = .0;\nturnAlphaFractionAtRest = .0;\n\
             turnControlSquared = 1;\nturnControlAngle = 30;\nforwardControlDistance = 100;\n",
        );
        let mut w = Vec::new();
        let m = movement(Some(&ph), "p.odf", &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(m.combat_speed, Fixed::from_int(40));
        assert_eq!(m.max_speed, Fixed::from_int(80));
        assert_eq!(m.turn_alpha_at_rest, Fixed::ZERO);
        assert_eq!(m.too_close_to_turn, Fixed::from_int(50));
        assert_eq!(m.path_lead, Fixed::from_int(150));
        assert_eq!(m.turn_control_angle, Fixed::from_int(30));
        assert!(m.turn_control_squared);
        assert_eq!(m.forward_control_distance, Fixed::from_int(100));
        assert!(m.has_smooth_keys());
    }

    /// Without the keys, the defaults (and no change to hashes).
    #[test]
    fn smooth_keys_default() {
        let mut w = Vec::new();
        let m = movement(Some(&odf("impulseSpeed = 120\nturnOmegaFractionAtRest = 1\n")), "p.odf", &mut w);
        assert_eq!(m.combat_speed, m.max_speed);
        assert_eq!(m.turn_alpha_at_rest, Fixed::ONE);
        assert!(!m.has_smooth_keys());
        assert!(!MoveStats::default().has_smooth_keys());
    }
}
