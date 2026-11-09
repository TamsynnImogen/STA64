//! Cloak-capable fake content, for the specials tests and the `cloak`
//! bench scenario. Add it to `fake::db()`'s content with a builder; no game
//! data. The keys sit in each definition's `raw` ODF, where [`super`] reads them.
//!
//! | name | role |
//! |---|---|
//! | `fx_cloaker` | frigate with a beam and a cloak: 600 energy (+6/s), cost 50, drain 15/s, 1 s to engage and disengage, weapons off while cloaked |
//! | `fx_cloaker_frail` | 100 energy, cost 10, drain 30/s, no regeneration: runs dry |
//! | `fx_cloaker_loud` | instant cloak that leaves weapons on: firing decloaks it, a hit decloaks it |

use std::sync::Arc;

use sta_content::{
    DefRef, FireArc, Fixed, Hardpoint, MoveStats, ShieldDef, UnitDef, WeaponDef, WeaponKind,
};

fn odf(name: &str, text: &str) -> Arc<sta_odf::Odf> {
    Arc::new(sta_odf::resolve(name, &mut |_| Some(text.to_owned())).expect("fixture ODF resolves"))
}

fn device(name: &str, keys: &str) -> WeaponDef {
    let mut w = WeaponDef::new(name, "cloakingdeviceimp", WeaponKind::Beam);
    w.raw = odf(name, keys);
    w
}

fn cloaker(name: &str, device_name: &str, energy: &str) -> UnitDef {
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
    u.raw = odf(name, energy);
    u
}

/// The units and weapons to add; `fx_beam` must come from `fake::db()`.
pub fn defs() -> (Vec<UnitDef>, Vec<WeaponDef>) {
    let weapons = vec![
        device(
            "fx_cloak",
            "effectTimer = 1.0\nenergyDrainRate = 15.0\nspecialEnergyCost = 50\n",
        ),
        device(
            "fx_cloak_frail",
            "effectTimer = 1.0\nenergyDrainRate = 30.0\nspecialEnergyCost = 10\n",
        ),
        device(
            "fx_cloak_loud",
            "effectTimer = 0\ndisableWeapons = 0\ndecloakOnHit = 1\n",
        ),
    ];
    let units = vec![
        cloaker(
            "fx_cloaker",
            "fx_cloak",
            "maxSpecialEnergy = 600\nspecialEnergyRate = 6.0\n",
        ),
        cloaker(
            "fx_cloaker_frail",
            "fx_cloak_frail",
            "maxSpecialEnergy = 100\nspecialEnergyRate = 0\n",
        ),
        cloaker("fx_cloaker_loud", "fx_cloak_loud", ""),
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
