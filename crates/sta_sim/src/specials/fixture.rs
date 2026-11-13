//! Cloak-capable fake content, for the specials tests and the `cloak`
//! bench scenario. Add it to `fake::db()`'s content with a builder; no game
//! data. The values sit in the typed fields the ODF loader fills from the
//! keys named in [`super`].
//!
//! | name | role |
//! |---|---|
//! | `fx_cloaker` | frigate with a beam and a cloak: 600 energy (+6/s), cost 50, drain 15/s, 1 s to engage and disengage, weapons off while cloaked |
//! | `fx_cloaker_frail` | 100 energy, cost 10, drain 30/s, no regeneration: runs dry |
//! | `fx_cloaker_loud` | instant cloak that leaves weapons on: firing decloaks it, a hit decloaks it |

use sta_content::{
    DefRef, FireArc, Fixed, Hardpoint, MoveStats, ShieldDef, UnitDef, WeaponDef, WeaponKind,
};

/// A cloaking device: `effectTimer` seconds, `energyDrainRate`,
/// `specialEnergyCost`, `disableWeapons`, `decloakOnHit`.
fn device(name: &str, seconds: f64, drain: i32, cost: i32, disable_weapons: bool, decloak_on_hit: bool) -> WeaponDef {
    let mut w = WeaponDef::new(name, "cloakingdeviceimp", WeaponKind::Beam);
    w.effect_ticks = sta_core::seconds_to_ticks(seconds);
    w.energy_drain = Fixed::from_int(drain);
    w.energy_cost = Fixed::from_int(cost);
    w.disable_weapons = disable_weapons;
    w.decloak_on_hit = decloak_on_hit;
    w
}

/// `energy`: `maxSpecialEnergy` and `specialEnergyRate`.
fn cloaker(name: &str, device_name: &str, energy: (Fixed, Fixed)) -> UnitDef {
    let mut u = UnitDef::new(name, "craft");
    u.race = "fx".into();
    u.model = "fx_frigate".into();
    u.movement = MoveStats::from_f32(200.0, 1.5, 1.6, 1.6, 0.3);
    u.hull = Fixed::from_int(300);
    u.shields = ShieldDef {
        max: Fixed::from_int(150),
        recharge: Fixed::from_f64(0.25),
    };
    u.crew = Fixed::from_int(80);
    u.sensor = Fixed::from_int(1200);
    let fore = FireArc {
        dir: (Fixed::ZERO, Fixed::ONE),
        min_dot: Fixed::from_f64(std::f64::consts::FRAC_1_SQRT_2),
    };
    u.hardpoints = vec![
        Hardpoint {
            weapon: DefRef::new("fx_beam"),
            node: "hp01".into(),
            arc: Some(fore),
        },
        Hardpoint {
            weapon: DefRef::new(device_name),
            node: "hp02".into(),
            arc: None,
        },
    ];
    (u.special_energy_max, u.special_energy_rate) = energy;
    u
}

/// The units and weapons to add; `fx_beam` must come from `fake::db()`.
pub fn defs() -> (Vec<UnitDef>, Vec<WeaponDef>) {
    let weapons = vec![
        device("fx_cloak", 1.0, 15, 50, true, false),
        device("fx_cloak_frail", 1.0, 30, 10, true, false),
        device("fx_cloak_loud", 0.0, 0, 0, false, true),
    ];
    let int = Fixed::from_int;
    let units = vec![
        cloaker("fx_cloaker", "fx_cloak", (int(600), int(6))),
        cloaker("fx_cloaker_frail", "fx_cloak_frail", (int(100), Fixed::ZERO)),
        cloaker("fx_cloaker_loud", "fx_cloak_loud", (Fixed::ZERO, Fixed::ZERO)),
    ];
    (units, weapons)
}

/// `fake::db()` plus [`defs`].
pub fn db() -> sta_content::ContentDb {
    let fake = sta_content::fake::db();
    let (units, weapons) = defs();
    let mut b = sta_content::ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for w in fake.weapons().cloned().chain(weapons) {
        b = b.weapon(w);
    }
    for u in fake.units().cloned().chain(units) {
        b = b.unit(u);
    }
    b.build().expect("fake content plus cloakers is consistent")
}
