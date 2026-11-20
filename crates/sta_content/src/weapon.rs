//! [`WeaponDef`] from a weapon's resolved ODF and its ordnance ODF (CON-03).
//! See [`crate::odf`].
//!
//! A weapon ODF (`cannon`, `launcher`, a cloaking device, ...) says what the
//! mount does; its `ordName` ODF (`phaser`, `photontorpedo`, ...) says what
//! flies or is drawn.
//!
//! | field | key (ODF) |
//! |---|---|
//! | `class` | the ordnance's `classLabel`, else the weapon's own (so a cloaking device stays `cloakingdeviceimp`) |
//! | `kind` | `PointDefence` for `ordnancedefenseweapon` and `pointdefenselaser`; `Beam` for beam ordnance ([`BEAM_CLASSES`]) or no ordnance; else `Projectile` |
//! | `range`, `refire` | `range`, `shotDelay` (else the first `shotDelay<n>`) seconds, as ticks |
//! | `damage` | ordnance `damageBase` |
//! | `shot_speed` | ordnance `shotSpeed`, projectiles only |
//! | `duration` | ordnance `lifespan` seconds, beams and mining beams (class `minebm`) only; `STA64_duration` seconds on the weapon ODF overrides it (point defence has no lifespan) |
//! | `fire_sound`, `fx` | `fireSound`; ordnance `shotGeometry` (else `sprite`) |
//! | specials | `effectTimer`, `energyDrainRate`, `specialEnergyCost`, `disableWeapons` (default on), `decloakOnHit` |
//! | shot timing | `initialShotDelayModifier` (signed; absent stays `None`), `ignoreShotDelayEffects`; `ordnance_defence` is the weapon's own class `ordnancedefenseweapon` |
//! | `resource` (class `resourceweapon`) | `addDilithium`, `addLatinum`, `addMetal`, `addBiomatter`, `addCrew` (signed), `oneTimeMode`, `alwaysActivePlayer`, `alwaysActiveAI` |
//! | `colonize` | ordnance class `ColonizerPod`: `partySize` (default 100), `orbitRange` (SIM-30) |
//! | `base` (CON-54) | weapon: `hitChance` (+ rows), `stopToHitModifier`, `toHitStopModifier`, `impulseToHitModifier`, `toHitImpulseModifier`, `warpToHitModifier`, `toHitWarpModifier`, `slowToHitModifier`, `toHitSlowModifier`, `targetEnemyCraft`, `targetEnemyBuildings`, `targetAlliedCraft`, `targetAlliedBuildings`, `targetEnemyPlanets`, `targetEnemyPlanetsOnly`, `targetLocation`, `AItargetLocation`, `needTarget`, `beneficial`, `switchToAttack`, `cloakEnable`, `warpEnable`, `crewAffectsWeaponDelay`, `ignoreValidAsTarget`, `regularWeaponDelay`; ordnance: rows of `damageBase`, `damageVariance`, `damageThreshold`, `shieldCrewModifier`, `hullCrewModifier`, and `ignoreShield`, `forceInvalidateTarget`, `canAttackStations`, `specialValue`, `shieldDuration`, `omegaTurn`, `seekTime`, `shotAccel`, `detectRadius`, `travelTimeFraction` |
//! | hardpoint arc | `restrictFireArc = 1` with `fireArc` as the half-width in radians (our reading) |

use sta_core::Fixed;
use sta_odf::Odf;

use crate::def::{ClassLabel, FireArc, Lookup, ResourceWeaponDef, TargetMaskDef, ToHitDef, WeaponBaseDef, WeaponDef, WeaponKind};
use crate::odf::{numbered, stem, Reader};
use crate::Warning;

/// Ordnance classes that are drawn as a beam rather than flown.
pub const BEAM_CLASSES: &[&str] = &["phaser", "utribeam", "unibeam", "pdlbeam", "holding_bm", "tractor_bm"];

/// The ordnance class of a mining beam (freighters' and orbital stations').
pub const MINING_BEAM_CLASS: &str = "minebm";

/// Weapon classes that shoot down ordnance.
const POINT_DEFENCE: &[&str] = &["ordnancedefenseweapon", "pointdefenselaser"];

/// An arc ahead of the mount, `half` radians either side.
pub(crate) fn fire_arc(half: f64) -> FireArc {
    FireArc {
        dir: (Fixed::ZERO, Fixed::ONE),
        // Loader-only trigonometry; `ContentDb::hash` catches a platform
        // whose cosine differs in the last bit.
        min_dot: Fixed::from_f64(half.min(std::f64::consts::PI).cos()),
    }
}

/// A signed number, `None` when the key is absent or not a number (the latter warns).
fn signed(r: &mut Reader, key: &str) -> Option<Fixed> {
    r.num(key).map(Fixed::from_f64)
}

/// `key` with its table rows (`"name" value`). `None` when the key is absent.
/// A row that is not a quoted name and a number warns and is skipped.
fn lookup(r: &mut Reader, key: &str) -> Option<Lookup> {
    r.odf.raw(key)?;
    let base = signed(r, key).unwrap_or(Fixed::ZERO);
    let mut rows = Vec::new();
    let odf = r.odf;
    for row in odf.rows(key) {
        let parsed = row.trim().strip_prefix('"').and_then(|rest| rest.split_once('"')).and_then(|(name, value)| {
            let number = value.split_whitespace().next()?.trim_end_matches(';').trim_end_matches(['f', 'F']);
            let v = number.parse::<f64>().ok().filter(|v| v.is_finite())?;
            let name = name.trim().to_ascii_lowercase();
            (!name.is_empty()).then(|| (std::sync::Arc::<str>::from(name), Fixed::from_f64(v.clamp(-1.0e9, 1.0e9))))
        });
        match parsed {
            Some(pair) => rows.push(pair),
            None => r.warnings.push(Warning::new(r.file, format!("{key} row {row} is not a quoted name and a number; skipped"))),
        }
    }
    Some(Lookup { base, rows: rows.into() })
}

/// As [`lookup`], kept only when the key has rows.
fn rows_only(r: &mut Reader, key: &str) -> Option<Lookup> {
    if r.odf.has_rows(key) { lookup(r, key).filter(|l| !l.rows.is_empty()) } else { None }
}

/// Interned rows of one weapon lookup table.
type LookupRows = std::sync::Arc<[(std::sync::Arc<str>, Fixed)]>;

/// Shares equal tables and names between weapons (CON-54): an ordnance is read
/// once per weapon that fires it, and Fleet Operations pastes the same table
/// into thousands of files.
#[derive(Default)]
pub(crate) struct LookupPool {
    names: std::collections::HashSet<std::sync::Arc<str>>,
    tables: std::collections::HashSet<LookupRows>,
}

impl LookupPool {
    pub fn share(&mut self, b: &mut WeaponBaseDef) {
        let tables = [&mut b.hit_chance, &mut b.damage_base, &mut b.damage_variance, &mut b.damage_threshold, &mut b.shield_crew_modifier, &mut b.hull_crew_modifier];
        for l in tables.into_iter().flatten() {
            if let Some(shared) = self.tables.get(&*l.rows) {
                l.rows = std::sync::Arc::clone(shared);
                continue;
            }
            let rows: Vec<_> = l.rows.iter().map(|(name, v)| {
                let name = match self.names.get(&**name) {
                    Some(n) => std::sync::Arc::clone(n),
                    None => {
                        self.names.insert(std::sync::Arc::clone(name));
                        std::sync::Arc::clone(name)
                    }
                };
                (name, *v)
            }).collect();
            l.rows = rows.into();
            self.tables.insert(std::sync::Arc::clone(&l.rows));
        }
    }
}

/// The weapon's own base keys (CON-54).
fn weapon_base(r: &mut Reader) -> WeaponBaseDef {
    WeaponBaseDef {
        hit_chance: lookup(r, "hitChance"),
        to_hit: ToHitDef {
            stop_to_hit: signed(r, "stopToHitModifier"),
            to_hit_stop: signed(r, "toHitStopModifier"),
            impulse_to_hit: signed(r, "impulseToHitModifier"),
            to_hit_impulse: signed(r, "toHitImpulseModifier"),
            warp_to_hit: signed(r, "warpToHitModifier"),
            to_hit_warp: signed(r, "toHitWarpModifier"),
            slow_to_hit: signed(r, "slowToHitModifier"),
            to_hit_slow: signed(r, "toHitSlowModifier"),
        },
        targets: TargetMaskDef {
            enemy_craft: r.flag("targetEnemyCraft"),
            enemy_buildings: r.flag("targetEnemyBuildings"),
            allied_craft: r.flag("targetAlliedCraft"),
            allied_buildings: r.flag("targetAlliedBuildings"),
            enemy_planets: r.flag("targetEnemyPlanets"),
            enemy_planets_only: r.flag("targetEnemyPlanetsOnly"),
            location: r.flag("targetLocation"),
            ai_location: r.flag("AItargetLocation"),
        },
        need_target: r.flag("needTarget"),
        beneficial: r.flag("beneficial"),
        switch_to_attack: r.flag("switchToAttack"),
        cloak_enable: r.flag("cloakEnable"),
        warp_enable: r.flag("warpEnable"),
        special: r.flag("special"),
        crew_affects_weapon_delay: r.flag("crewAffectsWeaponDelay"),
        ignore_valid_as_target: r.flag("ignoreValidAsTarget"),
        regular_weapon_delay: signed(r, "regularWeaponDelay"),
        ..WeaponBaseDef::default()
    }
}

/// The ordnance's base keys into `b` (CON-54).
fn ordnance_base(o: &mut Reader, b: &mut WeaponBaseDef) {
    b.damage_base = rows_only(o, "damageBase");
    b.damage_variance = rows_only(o, "damageVariance");
    b.damage_threshold = rows_only(o, "damageThreshold");
    b.shield_crew_modifier = rows_only(o, "shieldCrewModifier");
    b.hull_crew_modifier = rows_only(o, "hullCrewModifier");
    b.ignore_shield = o.flag("ignoreShield");
    b.force_invalidate_target = o.flag("forceInvalidateTarget");
    b.can_attack_stations = o.flag("canAttackStations");
    b.special_value = signed(o, "specialValue");
    b.shield_duration = signed(o, "shieldDuration");
    b.omega_turn = signed(o, "omegaTurn");
    b.seek_time = signed(o, "seekTime");
    b.shot_accel = signed(o, "shotAccel");
    b.detect_radius = signed(o, "detectRadius");
    b.travel_time_fraction = signed(o, "travelTimeFraction");
}

/// The definition of weapon `name`, and the arc its hardpoints are limited to.
pub fn weapon_def(name: &str, odf: &Odf, ordnance: Option<&Odf>, warnings: &mut Vec<Warning>) -> (WeaponDef, Option<FireArc>) {
    let file = format!("{}.odf", stem(name));
    let own = odf.class_label().unwrap_or("").trim().to_ascii_lowercase();
    let ord_class = ordnance.and_then(|o| o.class_label()).map(|c| c.trim().to_ascii_lowercase());
    let kind = if POINT_DEFENCE.contains(&own.as_str()) {
        WeaponKind::PointDefence
    } else {
        match &ord_class {
            Some(c) if !BEAM_CLASSES.contains(&c.as_str()) => WeaponKind::Projectile,
            _ => WeaponKind::Beam,
        }
    };
    let class = ord_class.as_deref().filter(|c| !c.is_empty()).unwrap_or(&own).to_owned();
    let mut d = WeaponDef::new(name.trim().trim_end_matches(".odf"), &class, kind);
    d.class = ClassLabel::new(&class);
    d.raw = std::sync::Arc::new(odf.clone());
    d.replace = crate::lifecycle::replace_def(odf);

    let mut r = Reader { odf, file: &file, warnings };
    d.range = r.fixed("range");
    d.refire = if odf.raw("shotDelay").is_some() {
        r.ticks("shotDelay")
    } else {
        match numbered(odf, "shotdelay").first() {
            Some((n, _)) => r.ticks(&format!("shotDelay{n}")),
            None => 0,
        }
    };
    d.fire_sound = r.text("fireSound");
    d.effect_ticks = r.ticks("effectTimer");
    d.energy_drain = r.fixed("energyDrainRate");
    d.energy_cost = r.fixed("specialEnergyCost");
    // `STA64_duration`: seconds the weapon stays active, for kinds whose ordnance
    // gives none (point defence). Overrides the ordnance's `lifespan`.
    let own_duration = r.num("STA64_duration").filter(|s| *s >= 0.0).map(sta_core::seconds_to_ticks);
    d.disable_weapons = r.flag("disableWeapons").unwrap_or(true);
    d.decloak_on_hit = r.flag("decloakOnHit").unwrap_or(false);
    d.initial_shot_delay_modifier = r.num("initialShotDelayModifier").map(Fixed::from_f64);
    d.ignore_shot_delay_effects = r.flag("ignoreShotDelayEffects").unwrap_or(false);
    d.ordnance_defence = own == "ordnancedefenseweapon";
    d.activate_on_team_change = r.flag("activateOnTeamChange").unwrap_or(false);
    d.base = weapon_base(&mut r);

    if own == "resourceweapon" {
        let mut add = [Fixed::ZERO; crate::resources::SLOTS];
        for (slot, key) in crate::resources::KEYS.iter().enumerate() {
            // Signed: the Roots Tal Shiar weapons take (`addDilithium = -100`).
            if let Some(v) = r.num(&format!("add{key}")) {
                add[slot] = Fixed::from_f64(v);
            }
        }
        d.resource = Some(ResourceWeaponDef {
            add,
            crew: r.num("addCrew").map_or(Fixed::ZERO, Fixed::from_f64),
            one_time: r.flag("oneTimeMode").unwrap_or(false),
            always_player: r.flag("alwaysActivePlayer").unwrap_or(false),
            always_ai: r.flag("alwaysActiveAI").unwrap_or(false),
        });
    }

    let arc = if r.flag("restrictFireArc") == Some(true) {
        r.num("fireArc").filter(|a| *a > 0.0).map(fire_arc)
    } else {
        None
    };

    if let Some(ord) = ordnance {
        let mut o = Reader { odf: ord, file: &file, warnings };
        d.damage = o.fixed("damageBase");
        ordnance_base(&mut o, &mut d.base);
        let loss = crate::def::OrdnanceCrewLoss {
            threshold: o.fixed("damageThreshold"),
            shield: o.fixed("shieldCrewModifier"),
            hull: o.fixed("hullCrewModifier"),
        };
        d.crew_loss = (loss != crate::def::OrdnanceCrewLoss::default()).then_some(loss);
        if ord_class.as_deref() == Some("colonizerpod") {
            // SIM-30: read from data (A2 colonizer pod ordnance: 100 and 150).
            let party_size = o.num("partySize").filter(|n| *n >= 0.0).unwrap_or(100.0);
            d.colonize = Some(crate::def::ColonizePodDef { party_size: Fixed::from_f64(party_size), orbit_range: o.fixed("orbitRange") });
        }
        d.damage_variance = o.fixed("damageVariance");
        d.fx = o.text("shotGeometry").or_else(|| o.text("sprite"));
        match kind {
            WeaponKind::Beam => d.duration = o.ticks("lifespan"),
            _ => {
                d.shot_speed = o.fixed("shotSpeed");
                // A mining beam (class `minebm`) is not one of the drawn beam classes above and stays a
                // projectile kind, but its `lifeSpan` is how long each pulse lives: the orbital income of
                // the sim and the drawn beam both read it (read from data; the kind is unchanged).
                if ord_class.as_deref() == Some(MINING_BEAM_CLASS) {
                    d.duration = o.ticks("lifespan");
                    // Its drawn size: `radiusScale` (default 1) and `lengthScale` (a length divisor; 1 if
                    // absent or not positive) of the ordnance, read from data.
                    let radius = o.num("radiusScale").filter(|v| *v > 0.0).unwrap_or(1.0);
                    let length = o.num("lengthScale").filter(|v| *v > 0.0).unwrap_or(1.0);
                    d.shot_scale = Some(crate::def::ShotScale { radius: Fixed::from_f64(radius), length: Fixed::from_f64(length) });
                }
            }
        }
    } else if kind != WeaponKind::Beam || d.range > Fixed::ZERO && d.effect_ticks == 0 && d.energy_cost == Fixed::ZERO {
        // A weapon that shoots but has no ordnance to read damage from.
        warnings.push(Warning::new(&file, "no ordName (or its ODF is missing): damage 0"));
    }
    if let Some(ticks) = own_duration {
        d.duration = ticks;
    }
    (d, arc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> WeaponDef {
        let odf = sta_odf::resolve("w", &mut |_| Some(text.to_owned())).unwrap();
        weapon_def("w", &odf, None, &mut Vec::new()).0
    }

    fn with_ordnance(weapon: &str, ordnance: &str) -> (WeaponDef, Vec<Warning>) {
        let w = sta_odf::resolve("w", &mut |_| Some(weapon.to_owned())).unwrap();
        let o = sta_odf::resolve("o", &mut |_| Some(ordnance.to_owned())).unwrap();
        let mut warnings = Vec::new();
        let def = weapon_def("w", &w, Some(&o), &mut warnings).0;
        (def, warnings)
    }

    #[test]
    fn base_keys_absent_stay_unset() {
        let (w, warnings) = with_ordnance("classLabel = \"cannon\"\nordName = \"o\"\n", "classLabel = \"photontorpedo\"\ndamageBase = 20\n");
        assert_eq!(w.base, WeaponBaseDef::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn weapon_base_keys_are_read() {
        let w = parse(
            "classLabel = \"cannon\"\nhitChance = 0.9f\n\"FBattle.odf\" 0.5\n\"fscout.odf\"\t0.25f;\n\
             stopToHitModifier = -0.1\ntoHitStopModifier = 0.2\nimpulseToHitModifier = 0.05\ntoHitImpulseModifier = -0.3\n\
             warpToHitModifier = 1\ntoHitWarpModifier = 2\nslowToHitModifier = 3\ntoHitSlowModifier = 4\n\
             targetEnemyCraft = 1\ntargetEnemyBuildings = 0\ntargetAlliedCraft = 0\ntargetAlliedBuildings = 1\n\
             targetEnemyPlanets = 1\ntargetEnemyPlanetsOnly = 0\ntargetLocation = 1\nAItargetLocation = 0\n\
             needTarget = 1\nbeneficial = 0\nswitchToAttack = 1\ncloakEnable = 1\nwarpEnable = 0\n\
             crewAffectsWeaponDelay = 0\nignoreValidAsTarget = 1\nregularWeaponDelay = 1.5\nspecial = 1\n",
        );
        let b = &w.base;
        let hit = b.hit_chance.as_ref().unwrap();
        assert_eq!(hit.base, Fixed::from_f64(0.9f32 as f64));
        assert_eq!(hit.iter().collect::<Vec<_>>(), [("fbattle.odf", Fixed::from_f64(0.5)), ("fscout.odf", Fixed::from_f64(0.25))]);
        assert_eq!(b.to_hit.stop_to_hit, Some(Fixed::from_f64(-0.1f32 as f64)));
        assert_eq!(b.to_hit.to_hit_impulse, Some(Fixed::from_f64(-0.3f32 as f64)));
        assert_eq!((b.to_hit.warp_to_hit, b.to_hit.to_hit_slow), (Some(Fixed::ONE), Some(Fixed::from_int(4))));
        let t = b.targets;
        assert_eq!((t.enemy_craft, t.enemy_buildings, t.allied_craft, t.allied_buildings), (Some(true), Some(false), Some(false), Some(true)));
        assert_eq!((t.enemy_planets, t.enemy_planets_only, t.location, t.ai_location), (Some(true), Some(false), Some(true), Some(false)));
        assert_eq!((b.need_target, b.beneficial, b.switch_to_attack, b.cloak_enable, b.warp_enable), (Some(true), Some(false), Some(true), Some(true), Some(false)));
        assert_eq!((b.crew_affects_weapon_delay, b.ignore_valid_as_target), (Some(false), Some(true)));
        assert_eq!(b.regular_weapon_delay, Some(Fixed::from_f64(1.5)));
        assert_eq!(b.special, Some(true));
    }

    #[test]
    fn a_mining_beam_reads_its_radius_and_length_scale_and_other_classes_do_not() {
        let w = "classLabel = \"cannon\"\nordName = \"o\"\n";
        let (m, _) = with_ordnance(w, "classLabel = \"minebm\"\nlifeSpan = 1\nradiusScale = 2.5\nlengthScale = 5\nshotGeometry = \"x.sod\"\n");
        assert_eq!(m.shot_scale, Some(crate::def::ShotScale { radius: Fixed::from_f64(2.5), length: Fixed::from_int(5) }));
        let (bare, _) = with_ordnance(w, "classLabel = \"minebm\"\nlifeSpan = 1\n");
        assert_eq!(bare.shot_scale.map(|s| (s.radius, s.length)), Some((Fixed::ONE, Fixed::ONE)));
        let (t, _) = with_ordnance(w, "classLabel = \"photontorpedo\"\nradiusScale = 2.5\nlengthScale = 5\n");
        assert_eq!(t.shot_scale, None);
    }

    #[test]
    fn ordnance_base_keys_and_damage_rows_are_read() {
        let (w, warnings) = with_ordnance(
            "classLabel = \"cannon\"\nordName = \"o\"\n",
            "classLabel = \"photontorpedo\"\ndamageBase = 20\n\"zmama.odf\" 2\ndamageVariance = 3\nhullCrewModifier = 1\n\"a.odf\" 0\n\
             ignoreShield = 1\nforceInvalidateTarget = 1\ncanAttackStations = 0\nspecialValue = 7\nshieldDuration = 0.5\n\
             omegaTurn = 1.25\nseekTime = 2\nshotAccel = 30\ndetectRadius = 150\ntravelTimeFraction = 0.75\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let b = &w.base;
        // The plain values stay where they were.
        assert_eq!((w.damage, w.damage_variance), (Fixed::from_int(20), Fixed::from_int(3)));
        let damage = b.damage_base.as_ref().unwrap();
        assert_eq!((damage.base, damage.iter().collect::<Vec<_>>()), (Fixed::from_int(20), vec![("zmama.odf", Fixed::from_int(2))]));
        // No rows: not repeated here.
        assert_eq!(b.damage_variance, None);
        assert_eq!(b.hull_crew_modifier.as_ref().map(|l| l.rows.len()), Some(1));
        assert_eq!((b.ignore_shield, b.force_invalidate_target, b.can_attack_stations), (Some(true), Some(true), Some(false)));
        assert_eq!((b.special_value, b.shield_duration), (Some(Fixed::from_int(7)), Some(Fixed::from_f64(0.5))));
        assert_eq!((b.omega_turn, b.seek_time, b.shot_accel), (Some(Fixed::from_f64(1.25)), Some(Fixed::from_int(2)), Some(Fixed::from_int(30))));
        assert_eq!((b.detect_radius, b.travel_time_fraction), (Some(Fixed::from_int(150)), Some(Fixed::from_f64(0.75))));
    }

    #[test]
    fn malformed_base_values_warn_and_are_skipped() {
        let odf = sta_odf::resolve("w", &mut |_| Some("classLabel = \"cannon\"\nhitChance = 0.5\n\"a.odf\" lots\n\"b.odf\" 1\nneedTarget = maybe\nstopToHitModifier = fast\n".to_owned())).unwrap();
        let mut warnings = Vec::new();
        let w = weapon_def("w", &odf, None, &mut warnings).0;
        assert_eq!(w.base.hit_chance.as_ref().unwrap().iter().collect::<Vec<_>>(), [("b.odf", Fixed::ONE)]);
        assert_eq!((w.base.need_target, w.base.to_hit.stop_to_hit), (None, None));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn equal_tables_are_shared() {
        let text = "classLabel = \"cannon\"\nhitChance = 0.5\n\"a.odf\" 1\n\"b.odf\" 2\n";
        let (mut one, mut two) = (parse(text), parse(text));
        let mut other = parse("classLabel = \"cannon\"\nhitChance = 0.5\n\"a.odf\" 3\n");
        let mut pool = LookupPool::default();
        pool.share(&mut one.base);
        pool.share(&mut two.base);
        pool.share(&mut other.base);
        let rows = |w: &WeaponDef| std::sync::Arc::clone(&w.base.hit_chance.as_ref().unwrap().rows);
        assert!(std::sync::Arc::ptr_eq(&rows(&one), &rows(&two)));
        assert!(!std::sync::Arc::ptr_eq(&rows(&one), &rows(&other)));
        assert!(std::sync::Arc::ptr_eq(&rows(&one)[0].0, &rows(&other)[0].0), "names are shared across tables");
        assert_eq!(one.base, parse(text).base, "sharing changes no value");
    }

    #[test]
    fn shot_timing_keys() {
        let w = parse("[WeaponClass]\nclassLabel = \"cannon\"\nshotDelay = 2.0\n");
        assert_eq!(w.initial_shot_delay_modifier, None);
        assert!(!w.ignore_shot_delay_effects && !w.ordnance_defence);
        let w = parse("[WeaponClass]\nclassLabel = \"cannon\"\ninitialShotDelayModifier = 0.0\nignoreShotDelayEffects = 1\n");
        assert_eq!(w.initial_shot_delay_modifier, Some(Fixed::ZERO));
        assert!(w.ignore_shot_delay_effects);
        let w = parse("[WeaponClass]\nclassLabel = \"cannon\"\ninitialShotDelayModifier = -0.5\n");
        assert_eq!(w.initial_shot_delay_modifier, Some(Fixed::from_f64(-0.5)));
        let w = parse("[WeaponClass]\nclassLabel = \"ordnancedefenseweapon\"\n");
        assert!(w.ordnance_defence && w.kind == WeaponKind::PointDefence);
        let w = parse("[WeaponClass]\nclassLabel = \"pointdefenselaser\"\n");
        assert!(!w.ordnance_defence);
    }
}
