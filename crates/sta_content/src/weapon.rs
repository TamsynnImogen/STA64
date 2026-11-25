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
//! | `look` | render only, not digested ([`ShotLook`]): ordnance `Sprite`, `spriteDuration`, `Radius`, `Length`, `lifeSpan` (every class), `travelTimeFraction` (beam classes, default 0.1), `flareSprite` / `flareRadiusMultiplier` (class `phaser`, defaults `pflare` / 1.4), `lightColor` with `lightFalloffStart` / `lightFalloffRange` (defaults 100 / 5) |
//! | hardpoint arc | `restrictFireArc = 1` with `fireArc` as the half-width in radians (our reading) |
//! | A1 specials | A1-provenance launch/ordnance classes and their class-specific keys, decoded in [`a1_special`]; launch `specialEnergyCost` precedes ordnance |

use sta_core::Fixed;
use sta_odf::Odf;

use crate::def::{A1SpecialDef, A1SpecialKind, A2Effect, A2SpecialDef, A2Target, ShotFlare, ShotLight, ShotLook, ClassLabel, FireArc, Lookup, ResourceWeaponDef, TargetMaskDef, ToHitDef, WeaponBaseDef, WeaponDef, WeaponKind};
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

/// A finite, non-negative number (the look keys are sizes and times); anything else reads as absent.
fn look_num(r: &mut Reader, key: &str) -> Option<f32> {
    r.num(key).filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as f32)
}

/// The ordnance look keys (see [`ShotLook`]). Read from data (key presence); the classes
/// that read `travelTimeFraction` and the flare keys, and their defaults, are read from the
/// executable, static.
fn shot_look(o: &mut Reader, class: &str) -> ShotLook {
    let light = o.odf.list("lightColor");
    let light = match light[..] {
        [r, g, b, ..] => {
            let n = |t: &str| t.trim_end_matches(['f', 'F', ';']).parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0);
            match (n(r), n(g), n(b)) {
                (Some(r), Some(g), Some(b)) => Some([r, g, b]),
                _ => None,
            }
        }
        _ => None,
    };
    let light = light.map(|color| ShotLight {
        color,
        falloff_start: look_num(o, "lightFalloffStart").unwrap_or(ShotLight::DEFAULT_FALLOFF_START),
        falloff_range: look_num(o, "lightFalloffRange").unwrap_or(ShotLight::DEFAULT_FALLOFF_RANGE),
    });
    ShotLook {
        sprite: o.text("sprite"),
        sprite_duration: look_num(o, "spriteDuration"),
        radius: look_num(o, "Radius"),
        length: look_num(o, "Length"),
        life_span: look_num(o, "lifeSpan"),
        travel_time_fraction: BEAM_CLASSES.contains(&class).then(|| look_num(o, "travelTimeFraction").unwrap_or(ShotLook::DEFAULT_TRAVEL_TIME_FRACTION)),
        flare: (class == "phaser").then(|| ShotFlare {
            sprite: o.text("flareSprite").unwrap_or_else(|| ShotFlare::DEFAULT_SPRITE.to_owned()),
            radius_multiplier: look_num(o, "flareRadiusMultiplier").unwrap_or(ShotFlare::DEFAULT_RADIUS_MULTIPLIER),
        }),
        light,
    }
}

/// The definition of weapon `name`, and the arc its hardpoints are limited to.
pub fn weapon_def(name: &str, odf: &Odf, ordnance: Option<&Odf>, warnings: &mut Vec<Warning>) -> (WeaponDef, Option<FireArc>) {
    weapon_def_with_a1(name, odf, ordnance, false, warnings)
}

/// Decode only for an A1-provenance load. Generic A2 launch classes are not
/// enough to identify these specials; the ordnance class is authoritative.
pub(crate) fn weapon_def_with_a1(name: &str, odf: &Odf, ordnance: Option<&Odf>, a1: bool, warnings: &mut Vec<Warning>) -> (WeaponDef, Option<FireArc>) {
    weapon_def_for_profile(name, odf, ordnance, a1, false, warnings)
}

pub(crate) fn weapon_def_for_profile(name: &str, odf: &Odf, ordnance: Option<&Odf>, a1: bool, a2: bool, warnings: &mut Vec<Warning>) -> (WeaponDef, Option<FireArc>) {
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
        d.look = shot_look(&mut o, ord_class.as_deref().unwrap_or(""));
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
    if a1 || a2 {
        if a1 || d.base.special == Some(true) || own == "detectcloak" {
            d.a1_special = a1_special(odf, ordnance, &own, ord_class.as_deref(), &file, warnings);
            if a2 && let Some(data) = d.a1_special.as_mut().filter(|s| s.kind == A1SpecialKind::Reflector) {
                let mut l = Reader { odf, file: &file, warnings };
                let chance = |r: &mut Reader<'_>, key| {
                    Fixed::from_f64(r.num(key).unwrap_or(1.0).clamp(0.0, 1.0))
                };
                data.reflect_chance = Some((chance(&mut l, "reflectionRatio"), chance(&mut l, "reflectHitOwnerRatio")));
            }
        }
        if a2 {
            d.a2_special = a2_special(odf, ordnance, &own, ord_class.as_deref(), &file, warnings);
            if let Some(special) = d.a2_special.filter(|s| s.effect.sustained_beam()) {
                d.kind = WeaponKind::Beam;
                d.duration = special.flight;
            }
        }
        if (d.a1_special.is_some() || d.a2_special.is_some())
            && odf.raw("specialEnergyCost").is_none()
            && let Some(ord) = ordnance
        {
            let mut o = Reader { odf: ord, file: &file, warnings };
            d.energy_cost = o.fixed("specialEnergyCost");
        }
    }
    (d, arc)
}

fn a2_special(launch: &Odf, ordnance: Option<&Odf>, own: &str, ord_class: Option<&str>, file: &str, warnings: &mut Vec<Warning>) -> Option<A2SpecialDef> {
    let mut l = Reader { odf: launch, file, warnings };
    if l.flag("special") != Some(true) {
        return None;
    }
    let effect = match (own, ord_class) {
        ("selfdestruct", _) => A2Effect::SelfDestruct(l.fixed("radius").max(Fixed::ZERO)),
        ("regeneration", _) => A2Effect::Regeneration(Fixed::from_f64(
            l.num("regenModifier").unwrap_or(10.0).max(0.0),
        )),
        (_, Some("klingoncommandoteam")) => A2Effect::Commando(Fixed::ZERO),
        (_, Some("plasmacannon")) => A2Effect::Plasma,
        ("toggleweapon", Some("holding_bm")) => A2Effect::HoldingBeam { delay: 0, interval: 0, party: Fixed::ZERO },
        ("toggleweapon", Some("borgbore")) => A2Effect::AutoAssimilator(Fixed::ZERO),
        ("toggleweapon", Some("esconverter")) => A2Effect::ShieldProjector(Fixed::ZERO),
        ("toggleweapon", Some("shieldinv")) => A2Effect::ShieldInversion(Fixed::ZERO),
        (_, Some("deathchant")) => {
            let o = ordnance?;
            let mut o = Reader { odf: o, file, warnings: l.warnings };
            let factor = |r: &mut Reader<'_>, key| Fixed::from_f64(r.num(key).unwrap_or(1.0).clamp(0.0, 100.0));
            A2Effect::DeathChant {
                fire: factor(&mut o, "FireModifier"), repair: factor(&mut o, "RepairModifier"),
                turn: factor(&mut o, "TurnModifier"), damage: factor(&mut o, "DamageModifier"),
                speed: factor(&mut o, "SpeedModifier"), hit: factor(&mut o, "HitModifier"),
            }
        }
        (_, Some("myotron")) => A2Effect::Myotron,
        (_, Some("posijammer")) => A2Effect::SensorJammer,
        (_, Some("polerontorpedo")) => A2Effect::Polaron,
        (_, Some("fedshieldemitter")) => A2Effect::RestoreShields(Fixed::from_int(100)),
        (_, Some("borgenergyemitter")) => A2Effect::RestoreEnergy(Fixed::ZERO),
        (_, Some("remodulation")) => A2Effect::Remodulate,
        (_, Some("psychonicblast")) => A2Effect::PsychonicBlast(Fixed::ZERO),
        (_, Some("micro")) => A2Effect::MicroDrain { system: 0, per_second: Fixed::ZERO },
        _ => return None,
    };
    let target = if l.flag("targetLocation") == Some(true) {
        A2Target::Point
    } else if l.flag("needTarget") == Some(true) {
        A2Target::Entity
    } else {
        A2Target::Immediate
    };
    let max_targets = l.num("maxTargets").filter(|n| *n >= 0.0).map_or(u32::MAX, |n| n.min(u32::MAX as f64) as u32);
    let hit_condition = l.num("hitCondition").filter(|n| *n >= 0.0).map_or(0, |n| n.min(u32::MAX as f64) as u32);
    let projectile = matches!(own, "launcher" | "areamissile");
    let mut data = A2SpecialDef { target, effect, duration: 0, flight: 0, max_targets, hit_condition, projectile };
    if matches!(effect, A2Effect::SelfDestruct(_)) {
        data.duration = l.ticks("timer");
    }
    if let Some(ord) = ordnance {
        let mut o = Reader { odf: ord, file, warnings };
        data.flight = o.ticks("lifeSpan");
        match effect {
            A2Effect::Myotron | A2Effect::SensorJammer | A2Effect::Polaron | A2Effect::Remodulate |
            A2Effect::DeathChant { .. } => {
                data.duration = o.ticks("specialValue");
            }
            A2Effect::RestoreEnergy(_) => data.effect = A2Effect::RestoreEnergy(o.fixed("specialValue")),
            A2Effect::PsychonicBlast(_) => data.effect = A2Effect::PsychonicBlast(
                o.fixed("percentage").max(Fixed::ZERO).min(Fixed::ONE),
            ),
            A2Effect::Commando(_) => data.effect = A2Effect::Commando(o.fixed("partySize").max(Fixed::ZERO)),
            A2Effect::HoldingBeam { .. } => {
                data.effect = A2Effect::HoldingBeam {
                    delay: o.ticks("boardingDelay"), interval: o.ticks("boardingInterval").max(1),
                    party: o.fixed("boardingSize").max(Fixed::ZERO),
                };
            }
            A2Effect::AutoAssimilator(_) => {
                data.effect = A2Effect::AutoAssimilator(o.fixed("specialValue").max(Fixed::ZERO));
                if data.flight == 0 {
                    data.flight = o.ticks("duration");
                }
            }
            A2Effect::ShieldProjector(_) => data.effect = A2Effect::ShieldProjector(o.fixed("specialValue").max(Fixed::ZERO)),
            A2Effect::ShieldInversion(_) => data.effect = A2Effect::ShieldInversion(o.fixed("drainRate").max(Fixed::ZERO)),
            A2Effect::MicroDrain { .. } => {
                let seconds = o.num("DrainTime").filter(|s| *s > 0.0).unwrap_or(0.0);
                data.duration = sta_core::seconds_to_ticks(seconds);
                let total = o.num("DrainRate").unwrap_or(0.0).max(0.0);
                data.effect = A2Effect::MicroDrain {
                    system: o.num("affectedSystem").filter(|n| *n >= 0.0).map_or(0, |n| n.min(u32::MAX as f64) as u32),
                    // TimedDrain divides the authored total by its duration.
                    per_second: Fixed::from_f64(if seconds > 0.0 { total / seconds } else { 0.0 }),
                };
            }
            A2Effect::SelfDestruct(_) | A2Effect::RestoreShields(_) | A2Effect::Regeneration(_) | A2Effect::Plasma => {}
        }
    }
    Some(data)
}

/// The ten A1 Federation special forms, including campaign launch aliases.
/// Key locations are read from the A1 ODF data; zero means a key is absent.
fn a1_special(launch: &Odf, ordnance: Option<&Odf>, own: &str, ord_class: Option<&str>, file: &str, warnings: &mut Vec<Warning>) -> Option<A1SpecialDef> {
    fn ticks(r: &mut Reader<'_>, key: &str) -> u32 {
        r.ticks(key).min(u32::MAX as u64) as u32
    }
    let kind = match (own, ord_class) {
        (_, Some("antimine")) => A1SpecialKind::Mines,
        (_, Some("chainpulsar")) => A1SpecialKind::ChainPulsar,
        (_, Some("warpcoreoverload")) => A1SpecialKind::EngineOverload,
        ("reflectweapon", _) => A1SpecialKind::Reflector,
        ("detectcloak", _) => A1SpecialKind::DetectCloak,
        (_, Some("manheimeffect")) => A1SpecialKind::Gemini,
        (_, Some("shieldh")) => A1SpecialKind::ShieldDisruptor,
        (_, Some("fedrepairteam")) => A1SpecialKind::EngineeringTeam,
        ("pointdefenselaser", _) => A1SpecialKind::PointDefense,
        (_, Some("temporal")) => A1SpecialKind::Stasis,
        _ => return None,
    };
    let mut s = A1SpecialDef::new(kind);
    let mut l = Reader { odf: launch, file, warnings };
    s.limit = l.num("maxTargets").filter(|n| *n >= 0.0).map_or(0, |n| n.min(u32::MAX as f64) as u32);
    s.hit_condition = l.num("hitCondition").filter(|n| *n >= 0.0).map_or(0, |n| n.min(u32::MAX as f64) as u32);
    s.reflect_missile = l.flag("reflectMissile").unwrap_or(false);
    s.reflect_beam = l.flag("reflectBeam").unwrap_or(false);
    s.detect_class = l.num("detectClass").filter(|n| *n >= 0.0).map_or(0, |n| n.min(u32::MAX as f64) as u32);
    if kind == A1SpecialKind::DetectCloak {
        s.duration_ticks = ticks(&mut l, "detectTimer");
    }
    if let Some(ord) = ordnance {
        let mut o = Reader { odf: ord, file, warnings };
        s.flight_ticks = ticks(&mut o, "lifeSpan");
        match kind {
            A1SpecialKind::Mines => s.radius = o.fixed("detectRadius"),
            A1SpecialKind::ChainPulsar => {
                s.radius = o.fixed("chainRadius");
                s.limit = o.num("maxHits").filter(|n| *n >= 0.0).map_or(0, |n| n.min(u32::MAX as f64) as u32);
                s.bonus_damage = o.fixed("chainDamageModifier");
            }
            A1SpecialKind::EngineOverload | A1SpecialKind::ShieldDisruptor => s.duration_ticks = ticks(&mut o, "specialValue"),
            A1SpecialKind::Gemini => s.duration_ticks = ticks(&mut o, "effectTimer"),
            A1SpecialKind::EngineeringTeam => s.shields_returned = o.fixed("shieldsReturned"),
            A1SpecialKind::Stasis => { s.duration_ticks = ticks(&mut o, "lifeSpan"); s.radius = o.fixed("Radius"); }
            A1SpecialKind::Reflector | A1SpecialKind::DetectCloak | A1SpecialKind::PointDefense => {}
        }
    }
    Some(s)
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

#[cfg(test)]
mod look_tests {
    use super::*;

    fn look(class: &str, body: &str) -> ShotLook {
        let w = sta_odf::resolve("w", &mut |_| Some("classLabel = \"cannon\"\nordName = \"o\"\n".to_owned())).unwrap();
        let o = sta_odf::resolve("o", &mut |_| Some(format!("classLabel = \"{class}\"\n{body}"))).unwrap();
        weapon_def("w", &w, Some(&o), &mut Vec::new()).0.look
    }

    #[test]
    fn keys_are_read_for_every_class() {
        let l = look("photontorpedo", "Sprite = \"sp\"\nspriteDuration = 0.7\nRadius = 7.5\nLength = 33\nlifeSpan = 4\n");
        assert_eq!((l.sprite.as_deref(), l.sprite_duration, l.radius, l.length, l.life_span), (Some("sp"), Some(0.7), Some(7.5), Some(33.0), Some(4.0)));
        assert_eq!((l.travel_time_fraction, l.flare, l.light), (None, None, None));
        assert_eq!(look("photontorpedo", ""), ShotLook::default());
    }

    #[test]
    fn beam_classes_default_the_travel_time_fraction() {
        assert_eq!(look("phaser", "").travel_time_fraction, Some(0.1));
        assert_eq!(look("pdlbeam", "travelTimeFraction = 0.01\n").travel_time_fraction, Some(0.01));
        assert_eq!(look("pphaser", "travelTimeFraction = 0.5\n").travel_time_fraction, None);
    }

    #[test]
    fn only_phasers_have_a_flare_with_defaults_and_overrides() {
        let f = look("phaser", "").flare.unwrap();
        assert_eq!((f.sprite.as_str(), f.radius_multiplier), ("pflare", 1.4));
        let f = look("phaser", "flareSprite = \"x\"\nflareRadiusMultiplier = 2\n").flare.unwrap();
        assert_eq!((f.sprite.as_str(), f.radius_multiplier), ("x", 2.0));
        assert_eq!(look("pdlbeam", "flareSprite = \"x\"\n").flare, None);
        assert_eq!(look("pphaser", "").flare, None);
    }

    #[test]
    fn a_light_needs_a_colour_and_defaults_its_falloff() {
        let l = look("pphaser", "lightColor = 210 90 40\n").light.unwrap();
        assert_eq!((l.color, l.falloff_start, l.falloff_range), ([210.0, 90.0, 40.0], 100.0, 5.0));
        let l = look("photontorpedo", "lightColor = 0.5f 0.25 1\nlightFalloffStart = 60\nlightFalloffRange = 9\n").light.unwrap();
        assert_eq!((l.color, l.falloff_start, l.falloff_range), ([0.5, 0.25, 1.0], 60.0, 9.0));
        assert_eq!(look("pphaser", "lightFalloffStart = 60\n").light, None);
        assert_eq!(look("pphaser", "lightColor = 1 2\n").light, None);
    }

    #[test]
    fn keys_are_inherited_through_includes_and_overridden() {
        let mut files = std::collections::HashMap::new();
        files.insert("base", "classLabel = \"phaser\"\nRadius = 3\nlifeSpan = 1.5\nflareRadiusMultiplier = 2\nlightColor = 1 1 1\n");
        files.insert("child", "#include \"base.odf\"\nRadius = 5.5\nflareSprite = \"y\"\n");
        let w = sta_odf::resolve("w", &mut |_| Some("classLabel = \"cannon\"\nordName = \"o\"\n".to_owned())).unwrap();
        let o = sta_odf::resolve("child", &mut |n| files.get(n.trim_end_matches(".odf")).map(|s| (*s).to_owned())).unwrap();
        let l = weapon_def("w", &w, Some(&o), &mut Vec::new()).0.look;
        assert_eq!((l.radius, l.life_span), (Some(5.5), Some(1.5)));
        let f = l.flare.unwrap();
        assert_eq!((f.sprite.as_str(), f.radius_multiplier), ("y", 2.0));
        assert!(l.light.is_some());
    }

    #[test]
    fn negative_and_malformed_values_read_as_absent() {
        let l = look("phaser", "Radius = -3\nlifeSpan = soon\nLength = inf\nspriteDuration = -inf\n");
        assert_eq!((l.radius, l.life_span, l.length, l.sprite_duration), (None, None, None, None));
    }

    #[test]
    fn a2_area_special_reads_ordnance_cost_and_effect_without_changing_a1() {
        let launcher = sta_odf::resolve("gmyotron", &mut |_| Some(
            "classLabel = \"areamissile\"\nspecial = 1\nordName = \"myotron\"\nmaxTargets = 4\nhitCondition = 2\nrange = 400\n".to_owned()
        )).unwrap();
        let ordnance = sta_odf::resolve("myotron", &mut |_| Some(
            "classLabel = \"myotron\"\nspecialEnergyCost = 1000\nspecialValue = 15\nlifeSpan = 10\n".to_owned()
        )).unwrap();
        let mut warnings = Vec::new();
        let a2 = weapon_def_for_profile("gmyotron", &launcher, Some(&ordnance), false, true, &mut warnings).0;
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(a2.energy_cost, Fixed::from_int(1000));
        assert_eq!(a2.a1_special, None);
        assert_eq!(a2.a2_special, Some(A2SpecialDef {
            target: A2Target::Immediate,
            effect: A2Effect::Myotron,
            duration: sta_core::seconds_to_ticks(15.0),
            flight: sta_core::seconds_to_ticks(10.0),
            max_targets: 4,
            hit_condition: 2,
            projectile: true,
        }));
        let a1 = weapon_def_for_profile("gmyotron", &launcher, Some(&ordnance), true, false, &mut Vec::new()).0;
        assert_eq!(a1.a2_special, None);
    }

    #[test]
    fn stock_a2_federation_class_pairs_use_shared_special_payload() {
        let launcher = sta_odf::resolve("gwcover", &mut |_| Some(
            "classLabel = \"areacannon\"\nspecial = 1\nordName = \"wcover\"\nmaxTargets = 4\nhitCondition = 2\n".to_owned()
        )).unwrap();
        let ordnance = sta_odf::resolve("wcover", &mut |_| Some(
            "classLabel = \"warpcoreoverload\"\nspecialEnergyCost = 600\nspecialValue = 25\n".to_owned()
        )).unwrap();
        let weapon = weapon_def_for_profile("gwcover", &launcher, Some(&ordnance), false, true, &mut Vec::new()).0;
        assert_eq!(weapon.a1_special.map(|s| s.kind), Some(A1SpecialKind::EngineOverload));
        assert_eq!(weapon.energy_cost, Fixed::from_int(600));
    }
}
