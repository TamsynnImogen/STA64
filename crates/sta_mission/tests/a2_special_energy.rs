//! Public mission-to-sim regressions for A2.SetSpecialEnergy.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::{UnitFlags, WorldView};
use sta_sim::{Sim, SimEvent, WorldSource};

fn scene() -> (Sim, Arc<ContentDb>, EntityId, EntityId) {
    scene_with_content(Arc::new(sta_sim::specials::fixture::db()))
}

fn scene_with_content(content: Arc<ContentDb>) -> (Sim, Arc<ContentDb>, EntityId, EntityId) {
    let mut map = MapDef::empty();
    map.objects = [("pool", "fx_cloaker", 0), ("plain", "fx_frigate", 2000)]
        .into_iter()
        .map(|(label, odf, x)| MapObject {
            odf: odf.into(),
            owner: Some(1),
            pos: Vec3::from_f32([x as f32, 0.0, 0.0]),
            heading: (Fixed::ONE, Fixed::ZERO),
            label: Some(label.into()),
        })
        .collect();
    let setup = GameSetup {
        seed: 31,
        map: Arc::new(map),
        content: Arc::clone(&content),
        players: vec![PlayerSetup {
            id: PlayerId(1),
            team: TeamId(1),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let sim = Sim::from_setup(&setup);
    let labeled = |name: &str| {
        sim.labels
            .iter()
            .find_map(|(id, label)| (label == name).then_some(id))
            .unwrap()
    };
    let pool = labeled("pool");
    let plain = labeled("plain");
    (sim, content, pool, plain)
}

struct OwnedOdfs(PathBuf);

impl OwnedOdfs {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sta64-energy-odfs-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create only this test's unique directory");
        std::fs::create_dir(path.join("odf")).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, body: &str) {
        std::fs::write(self.0.join("odf").join(name), body).unwrap();
    }
}

impl Drop for OwnedOdfs {
    fn drop(&mut self) {
        for name in [
            "energy_base.odf",
            "energy_override.odf",
            "energy_rate_zero.odf",
            "energy_zero.odf",
        ] {
            let _ = std::fs::remove_file(self.0.join("odf").join(name));
        }
        let _ = std::fs::remove_dir(self.0.join("odf"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn runtime(sim: &Sim, content: Arc<ContentDb>, view: WorldView) -> MissionRuntime {
    let labels = sim
        .labels
        .iter()
        .map(|(id, label)| (id, label.clone()))
        .collect::<BTreeMap<_, _>>();
    let rt = MissionRuntime::from_source_with(
        "missions/a2-special-energy.lua",
        "",
        31,
        MissionConfig {
            content: Some(content),
            labels,
            view: Some(view),
            ..MissionConfig::default()
        },
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn cloak(id: EntityId) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq: 0,
        kind: CommandKind::Order(Order::Cloak {
            units: vec![id],
            on: true,
        }),
    }
}

fn rejected(events: &[SimEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, SimEvent::Rejected { .. }))
}

#[test]
fn fractional_absolute_writes_queue_in_order_and_the_last_can_fund_cloak() {
    let (mut sim, content, pool, _) = scene();
    let mut rt = runtime(&sim, content, (*sim.view()).clone());
    rt.eval::<()>("A2.SetSpecialEnergy(EntityFinder():findOne('pool').id, 12.5); A2.SetSpecialEnergy(EntityFinder():findOne('pool').id, 50.25)")
        .unwrap();
    let mut commands = Vec::new();
    rt.think(&sim.view(), &[], &mut commands);
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].player, PlayerId::SCRIPT);
    assert_eq!(
        commands[0].kind,
        CommandKind::Edit(Edit::SetEnergy(pool, Fixed::from_f64(12.5)))
    );
    assert_eq!(
        commands[1].kind,
        CommandKind::Edit(Edit::SetEnergy(pool, Fixed::from_f64(50.25)))
    );

    let applied = sim.step(&commands);
    assert!(!rejected(&applied.events));
    let next = sim.step(&[cloak(pool)]);
    assert!(
        !rejected(&next.events),
        "last absolute value should fund the cloak"
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn low_energy_rejects_cloak_while_huge_finite_energy_is_accepted() {
    for (value, accepted) in [("49.5", false), ("1e300", true)] {
        let (mut sim, content, pool, _) = scene();
        let mut rt = runtime(&sim, content, (*sim.view()).clone());
        rt.eval::<()>(&format!(
            "A2.SetSpecialEnergy(EntityFinder():findOne('pool').id, {value})"
        ))
        .unwrap();
        let commands = rt.take_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].kind,
            CommandKind::Edit(Edit::SetEnergy(
                pool,
                Fixed::from_f64(value.parse().unwrap())
            ))
        );
        let edited = sim.step(&commands);
        assert!(!rejected(&edited.events), "setter rejected {value}");
        let ordered = sim.step(&[cloak(pool)]);
        assert_eq!(!rejected(&ordered.events), accepted, "cloak after {value}");
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    }
}

#[test]
fn malformed_and_ineligible_targets_error_without_queuing() {
    let (sim, content, pool, plain) = scene();
    let mut rt = runtime(&sim, Arc::clone(&content), (*sim.view()).clone());
    rt.eval::<()>(&format!(
        r#"
            pool = {pool}
            plain = {plain}
            local invalid = {{
                function() A2.SetSpecialEnergy() end,
                function() A2.SetSpecialEnergy(pool, 1, 2) end,
                function() A2.SetSpecialEnergy(-1, 1) end,
                function() A2.SetSpecialEnergy(0, 1) end,
                function() A2.SetSpecialEnergy(1.5, 1) end,
                function() A2.SetSpecialEnergy('1', 1) end,
                function() A2.SetSpecialEnergy(pool, nil) end,
                function() A2.SetSpecialEnergy(pool, true) end,
                function() A2.SetSpecialEnergy(pool, '1') end,
                function() A2.SetSpecialEnergy(pool, math.huge) end,
                function() A2.SetSpecialEnergy(pool, -math.huge) end,
                function() A2.SetSpecialEnergy(pool, 0/0) end,
                function() A2.SetSpecialEnergy(99999, 1) end,
                function() A2.SetSpecialEnergy(plain, 1) end
            }}
            for _, call in ipairs(invalid) do assert(not pcall(call)) end
            "#,
        pool = pool.0,
        plain = plain.0,
    ))
    .unwrap();
    assert!(rt.take_commands().is_empty());

    for dead_view in [
        {
            let mut view = (*sim.view()).clone();
            view.units.iter_mut().find(|u| u.id == pool).unwrap().flags = UnitFlags::DYING;
            view
        },
        {
            let mut view = (*sim.view()).clone();
            view.units.iter_mut().find(|u| u.id == pool).unwrap().hull[0] = Fixed::ZERO;
            view
        },
    ] {
        let mut dead_rt = runtime(&sim, Arc::clone(&content), dead_view);
        assert!(
            dead_rt
                .eval::<bool>(&format!(
                    "return not pcall(function() A2.SetSpecialEnergy({}, 100) end)",
                    pool.0
                ))
                .unwrap()
        );
        assert!(dead_rt.take_commands().is_empty());
    }
}

#[test]
fn file_inherited_energy_defaults_respect_explicit_child_overrides() {
    let files = OwnedOdfs::new();
    files.write(
        "energy_base.odf",
        "maxSpecialEnergy = 600\nspecialEnergyRate = 6\n",
    );
    files.write(
        "energy_override.odf",
        "#include \"energy_base.odf\"\nmaxSpecialEnergy = 20\nspecialEnergyRate = 0\n",
    );
    files.write(
        "energy_rate_zero.odf",
        "#include \"energy_base.odf\"\nspecialEnergyRate = 0\n",
    );
    files.write(
        "energy_zero.odf",
        "#include \"energy_base.odf\"\nmaxSpecialEnergy = 0\n",
    );
    let mut vfs = sta_vfs::Vfs::default();
    vfs.push_dir("owned special energy test files".into(), files.0.clone())
        .unwrap();
    let mut legacy = sta_content::legacy::Legacy::default();

    for (name, max, rate) in [
        ("energy_override", 20.0, 0.0),
        ("energy_rate_zero", 600.0, 0.0),
        ("energy_zero", 0.0, 6.0),
    ] {
        let raw = legacy
            .odf(&vfs, name)
            .expect("child ODF resolves from files");
        assert_eq!(raw.f32("maxSpecialEnergy"), Some(max));
        assert_eq!(raw.f32("specialEnergyRate"), Some(rate));

        let fake = sta_content::fake::db();
        let (mut units, weapons) = sta_sim::specials::fixture::defs();
        let unit = units.iter_mut().find(|u| u.odf == "fx_cloaker").unwrap();
        // The typed fields the ODF loader fills from the resolved keys; the
        // sim reads no raw ODF (D9).
        let typed = |key: &str| Fixed::from_f64(raw.f32(key).unwrap_or(0.0).max(0.0) as f64);
        unit.special_energy_max = typed("maxSpecialEnergy");
        unit.special_energy_rate = typed("specialEnergyRate");
        unit.raw = raw;
        let mut builder = ContentDb::builder()
            .profile(fake.profile())
            .features(fake.features().clone());
        for weapon in fake.weapons().cloned().chain(weapons) {
            builder = builder.weapon(weapon);
        }
        for unit in fake.units().cloned().chain(units) {
            builder = builder.unit(unit);
        }
        let content = Arc::new(builder.build().unwrap());
        let (mut sim, content, pool, _) = scene_with_content(content);
        let mut rt = runtime(&sim, content, (*sim.view()).clone());
        let amount = if name == "energy_rate_zero" {
            49.9
        } else {
            1e300
        };
        let call = format!("A2.SetSpecialEnergy({}, {amount})", pool.0);

        if max == 0.0 {
            assert!(
                rt.eval::<bool>(&format!("return not pcall(function() {call} end)"))
                    .unwrap()
            );
            assert!(
                rt.take_commands().is_empty(),
                "explicit zero removes the pool"
            );
        } else {
            rt.eval::<()>(&call).unwrap();
            let commands = rt.take_commands();
            assert_eq!(commands.len(), 1);
            assert_eq!(
                commands[0].kind,
                CommandKind::Edit(Edit::SetEnergy(pool, Fixed::from_f64(amount)))
            );
            assert!(!rejected(&sim.step(&commands).events));
            assert!(
                rejected(&sim.step(&[cloak(pool)]).events),
                "file override must block cloak for {name}: max20 caps below cost50, rate0 leaves 49.9 below cost50"
            );
        }
    }
}
