//! [`UnitDef`] from a unit's resolved ODF (CON-03). See [`crate::odf`].
//!
//! Keys read (all optional; absent means zero or empty):
//!
//! | field | key |
//! |---|---|
//! | `class`, `race` | `classLabel`, `race` |
//! | `model`, `scale` | `basename` (else the ODF's own name), `ScaleSOD` |
//! | `hull` | `maxHealth` |
//! | `shields` | `maxShields` (`curShields` if that is missing), `shieldRate` per second |
//! | `crew` | `crewCost` (our reading: the crew a unit carries is what it costs) |
//! | `sensor` | `rangeScan` |
//! | `movement` | the physics ODF named by `physicsFile`, see [`movement`] |
//! | `hardpoints` | `weapon<n>` with `weaponHardPoints<n>` (first node named) |
//! | `build` | `dilithiumCost`, `latinumCost`, `metalCost`, `biomatterCost`, `buildTime` seconds, `officerCost`, `buildItem<n>` |
//! | `sounds` | `eventSelect`, `eventAcknowledge`, `ambientSound` (event and file names, resolved by the client) |
//! | `fx` | `fireball` |
//! | specials, mining | `maxSpecialEnergy`, `specialEnergyRate`, `miner`, `maxdilithium`, `miningRate`, `dilithiumTransferRate`, `resource` |

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
    MoveStats::from_f32(
        speed as f32,
        r.num("forwardAccel").unwrap_or(1.0) as f32,
        r.num("turnOmega").unwrap_or(1.0) as f32,
        r.num("turnAlpha").unwrap_or(1.0) as f32,
        r.num("turnOmegaFractionAtRest").unwrap_or(0.2) as f32,
    )
}

/// Resource id for a node's `resource` text.
fn resource_id(text: &str) -> u8 {
    let r = text.to_ascii_lowercase();
    if r.contains("latinum") {
        1
    } else if r.contains("metal") || r.contains("tritanium") {
        2
    } else if r.contains("biomatter") {
        3
    } else {
        0
    }
}

/// The definition of the unit `name`; `phys` is its resolved physics ODF.
/// Weapon and build references carry ODF names only; [`crate::odf::load_with`]
/// drops those that are not defined and fills the arcs.
pub fn unit_def(name: &str, odf: &Odf, phys: Option<&Odf>, warnings: &mut Vec<Warning>) -> UnitDef {
    let file = format!("{}.odf", stem(name));
    let class = odf.class_label().unwrap_or("");
    let mut d = UnitDef::new(name.trim().trim_end_matches(".odf"), class);
    d.class = ClassLabel::new(class.trim());
    d.raw = std::sync::Arc::new(odf.clone());
    let mut r = Reader { odf, file: &file, warnings };

    d.race = r.text("race").unwrap_or_default();
    d.model = r.text("basename").unwrap_or_else(|| stem(name));
    d.scale = match r.num("ScaleSOD") {
        Some(s) if s > 0.0 => Fixed::from_f64(s),
        _ => Fixed::ONE,
    };
    d.hull = r.fixed("maxHealth");
    let max_shields = if odf.raw("maxShields").is_some() { r.fixed("maxShields") } else { r.fixed("curShields") };
    d.shields = ShieldDef { max: max_shields, recharge: r.fixed("shieldRate") / Fixed::from_int(TICK_HZ as i32) };
    d.crew = r.fixed("crewCost");
    d.sensor = r.fixed("rangeScan");

    for (n, raw) in numbered(odf, "weapon") {
        let weapon = raw.trim().trim_matches('"').trim();
        if weapon.is_empty() || weapon.eq_ignore_ascii_case("null") {
            continue;
        }
        let node = odf
            .raw(&format!("weaponHardPoints{n}"))
            .and_then(|h| words(h).into_iter().next())
            .unwrap_or_else(|| "root".to_owned());
        d.hardpoints.push(Hardpoint { weapon: DefRef::new(weapon), node, arc: None });
    }

    let mut cost = [
        r.fixed("dilithiumCost"),
        r.fixed("latinumCost"),
        r.fixed("metalCost"),
        r.fixed("biomatterCost"),
    ]
    .to_vec();
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

    d.sounds.select = r.text("eventSelect").into_iter().collect();
    d.sounds.ack_move = r.text("eventAcknowledge").into_iter().collect();
    d.sounds.ambient = r.text("ambientSound");
    d.fx.explosion = r.text("fireball");

    d.special_energy_max = r.fixed("maxSpecialEnergy");
    d.special_energy_rate = r.fixed("specialEnergyRate");
    d.miner = d.class.0 == "freighter" || r.num("miner") == Some(1.0);
    d.max_dilithium = r.fixed("maxdilithium");
    d.mining_rate = r.fixed("miningRate");
    d.transfer_rate = r.fixed("dilithiumTransferRate");
    d.resource = r.text("resource").map(|t| resource_id(&t));

    d.movement = movement(phys, &file, warnings);
    d
}
