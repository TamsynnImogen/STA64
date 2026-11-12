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
//! | `duration` | ordnance `lifespan` seconds, beams only; `STA64_duration` seconds on the weapon ODF overrides it (point defence has no lifespan) |
//! | `fire_sound`, `fx` | `fireSound`; ordnance `shotGeometry` (else `sprite`) |
//! | specials | `effectTimer`, `energyDrainRate`, `specialEnergyCost`, `disableWeapons` (default on), `decloakOnHit` |
//! | shot timing | `initialShotDelayModifier` (signed; absent stays `None`), `ignoreShotDelayEffects`; `ordnance_defence` is the weapon's own class `ordnancedefenseweapon` |
//! | hardpoint arc | `restrictFireArc = 1` with `fireArc` as the half-width in radians (our reading) |

use sta_core::Fixed;
use sta_odf::Odf;

use crate::def::{ClassLabel, FireArc, WeaponDef, WeaponKind};
use crate::odf::{numbered, stem, Reader};
use crate::Warning;

/// Ordnance classes that are drawn as a beam rather than flown.
pub const BEAM_CLASSES: &[&str] = &["phaser", "utribeam", "unibeam", "pdlbeam", "holding_bm", "tractor_bm"];

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

    let arc = if r.flag("restrictFireArc") == Some(true) {
        r.num("fireArc").filter(|a| *a > 0.0).map(fire_arc)
    } else {
        None
    };

    if let Some(ord) = ordnance {
        let mut o = Reader { odf: ord, file: &file, warnings };
        d.damage = o.fixed("damageBase");
        d.fx = o.text("shotGeometry").or_else(|| o.text("sprite"));
        match kind {
            WeaponKind::Beam => d.duration = o.ticks("lifespan"),
            _ => d.shot_speed = o.fixed("shotSpeed"),
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
