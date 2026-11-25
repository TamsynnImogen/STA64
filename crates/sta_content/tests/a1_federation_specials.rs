//! Authored A1 special key shapes, based on the installed A1 class/key audit.
use sta_content::{
    A1SpecialDef, A1SpecialKind as K, ContentDb, Fixed, Overrides, Profile, odf::load_with,
};

fn load(profile: Profile, launch: &str, ordnance: Option<&str>) -> ContentDb {
    let launch = format!("wpnName = \"Fixture\"\n{launch}");
    let files = [
        ("fixture.odf", launch.as_str()),
        ("shot.odf", ordnance.unwrap_or("")),
    ];
    load_with(
        profile,
        &Overrides::default(),
        &["fixture".into()],
        &mut |name, version| {
            files
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name) && version == 0)
                .map(|(_, text)| (*text).to_owned())
        },
    )
    .unwrap()
    .0
}

fn payload(db: &ContentDb) -> A1SpecialDef {
    db.weapon(db.key("fixture").unwrap())
        .unwrap()
        .a1_special
        .unwrap()
}

fn ticks(seconds: f64) -> u32 {
    sta_core::seconds_to_ticks(seconds) as u32
}

#[test]
fn ordnance_classes_read_their_distinct_keys() {
    let cases = [
        ("minelayer", "antimine", "detectRadius = 250\n", K::Mines),
        (
            "minelayer",
            "chainpulsar",
            "chainRadius = 300\nmaxHits = 5\nchainDamageModifier = 20\n",
            K::ChainPulsar,
        ),
        (
            "areacannon",
            "warpcoreoverload",
            "specialValue = 25\n",
            K::EngineOverload,
        ),
        ("launcher", "manheimeffect", "effectTimer = 30\n", K::Gemini),
        (
            "areamissile",
            "shieldh",
            "specialValue = 15\n",
            K::ShieldDisruptor,
        ),
        (
            "cannon",
            "fedrepairteam",
            "shieldsReturned = 0.5\n",
            K::EngineeringTeam,
        ),
        (
            "cannon",
            "temporal",
            "lifeSpan = 45\nRadius = 400\n",
            K::Stasis,
        ),
    ];
    for (launch_class, ord_class, keys, kind) in cases {
        let launch = format!(
            "classLabel = \"{launch_class}\"\nordName = \"shot\"\nmaxTargets = 4\nhitCondition = 2\n"
        );
        let ord = format!("classLabel = \"{ord_class}\"\nspecialEnergyCost = 600\n{keys}");
        let db = load(Profile::Sta1Classic, &launch, Some(&ord));
        let s = payload(&db);
        assert_eq!(s.kind, kind);
        assert_eq!(
            db.weapon(db.key("fixture").unwrap()).unwrap().energy_cost,
            Fixed::from_int(600)
        );
        if kind == K::ChainPulsar {
            assert_eq!(
                (s.radius, s.limit, s.bonus_damage),
                (Fixed::from_int(300), 5, Fixed::from_int(20))
            );
        }
        if kind == K::Mines {
            assert_eq!(s.radius, Fixed::from_int(250));
        }
        if kind == K::EngineOverload {
            assert_eq!(
                (s.duration_ticks, s.limit, s.hit_condition),
                (ticks(25.0), 4, 2)
            );
        }
        if kind == K::Gemini {
            assert_eq!(s.duration_ticks, ticks(30.0));
        }
        if kind == K::ShieldDisruptor {
            assert_eq!(
                (s.duration_ticks, s.limit, s.hit_condition),
                (ticks(15.0), 4, 2)
            );
        }
        if kind == K::EngineeringTeam {
            assert_eq!(s.shields_returned, Fixed::from_f64(0.5));
        }
        if kind == K::Stasis {
            assert_eq!(
                (s.duration_ticks, s.radius),
                (ticks(45.0), Fixed::from_int(400))
            );
        }
    }
}

#[test]
fn launch_only_forms_and_energy_precedence() {
    let reflector = load(
        Profile::Sta1Classic,
        "classLabel = \"reflectweapon\"\nreflectMissile = 1\nreflectBeam = 1\nspecialEnergyCost = 60\n",
        None,
    );
    assert_eq!(payload(&reflector).kind, K::Reflector);
    assert!(payload(&reflector).reflect_missile && payload(&reflector).reflect_beam);
    let detect = load(
        Profile::Sta1Classic,
        "classLabel = \"detectcloak\"\ndetectClass = 3\ndetectTimer = 10\nspecialEnergyCost = 2\n",
        None,
    );
    assert_eq!(
        (
            payload(&detect).kind,
            payload(&detect).detect_class,
            payload(&detect).duration_ticks
        ),
        (K::DetectCloak, 3, ticks(10.0))
    );
    let pd = load(
        Profile::Sta1Classic,
        "classLabel = \"PointDefenseLaser\"\nordName = \"shot\"\nspecialEnergyCost = 40\n",
        Some("classLabel = \"PDLBeam\"\n"),
    );
    assert_eq!(payload(&pd).kind, K::PointDefense);
    let temporal = load(
        Profile::Sta1Classic,
        "classLabel = \"cannon\"\nordName = \"shot\"\nspecialEnergyCost = 5000\n",
        Some("classLabel = \"temporal\"\nspecialEnergyCost = 100\nlifeSpan = 45\nRadius = 400\n"),
    );
    assert_eq!(
        temporal
            .weapon(temporal.key("fixture").unwrap())
            .unwrap()
            .energy_cost,
        Fixed::from_int(5000)
    );
}

#[test]
fn a1_provenance_gates_the_payload_and_digest() {
    let launch = "classLabel = \"reflectweapon\"\nreflectMissile = 1\n";
    let a1 = load(Profile::Sta1Classic, launch, None);
    let a2 = load(Profile::Sta2Classic, launch, None);
    assert_eq!(payload(&a1).kind, K::Reflector);
    assert!(
        a2.weapon(a2.key("fixture").unwrap())
            .unwrap()
            .a1_special
            .is_none()
    );
    let other = load(
        Profile::Sta1Classic,
        "classLabel = \"reflectweapon\"\nreflectMissile = 0\n",
        None,
    );
    assert_ne!(
        a1.digest().get("weapon:fixture"),
        other.digest().get("weapon:fixture")
    );
    let normal = load(Profile::Sta1Classic, "classLabel = \"cannon\"\n", None);
    assert!(
        normal
            .weapon(normal.key("fixture").unwrap())
            .unwrap()
            .a1_special
            .is_none()
    );
    assert!(
        !normal
            .digest()
            .get("weapon:fixture")
            .unwrap()
            .contains("a1_special")
    );
}
