use sta_content::{ContentDb, Fixed, Overrides, Profile, TradeStationDef, odf::load_with};
use sta_vfs::Vfs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(can_trade: Option<&str>) -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sta64-trade-policy-{}-{id}", std::process::id()));
        std::fs::create_dir_all(path.join("odf")).unwrap();
        std::fs::write(
            path.join("odf/races.odf"),
            "numberOfRaces = 1\nrace0 = \"RaceA.odf\"\n",
        )
        .unwrap();
        let flag = can_trade.map_or(String::new(), |value| format!("canTrade = {value}\n"));
        std::fs::write(
            path.join("odf/RaceA.odf"),
            format!("name = \"Alpha\"\n{flag}"),
        )
        .unwrap();
        std::fs::write(
            path.join("odf/unit.odf"),
            "unitName = \"Unit\"\nclassLabel = \"craft\"\nrace = \"alpha\"\n",
        )
        .unwrap();
        Self(path)
    }

    fn load(&self) -> ContentDb {
        let mut vfs = Vfs::default();
        vfs.push_dir("fixture".into(), self.0.clone()).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        ContentDb::load_as(&vfs, Profile::Sta2Classic, &index)
            .unwrap()
            .0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn load(station: &str, cargo: &str) -> ContentDb {
    load_with(
        Profile::Sta2Classic,
        &Overrides::default(),
        &["station".into(), "cargo".into()],
        &mut |name, version| match (name, version) {
            ("station.odf", 0) => Some(station.to_owned()),
            ("cargo.odf", 0) => Some(cargo.to_owned()),
            _ => None,
        },
    )
    .unwrap()
    .0
}

fn unit<'a>(db: &'a ContentDb, name: &str) -> &'a sta_content::UnitDef {
    db.unit(db.key(name).unwrap()).unwrap()
}

#[test]
fn trade_unit_classes_get_effective_defaults_and_authored_values() {
    let db = load(
        "unitName = \"Station\"\nclassLabel = \"trading_station\"\ncargoKey1 = \"good_a\"\ncargoValue1 = 0\ncargoKey2 = \"good_b\"\ncargoKey4 = \"ignored\"\n",
        "unitName = \"Cargo\"\nclassLabel = \"cargo_ship\"\nm_maxResource = 220\n",
    );
    assert_eq!(
        unit(&db, "station").trade_station,
        Some(TradeStationDef {
            transfer_rate: Fixed::from_int(15),
            profit_share: Fixed::from_f64(0.1),
            goods: vec![
                sta_content::TradeGood {
                    key: "good_a".into(),
                    value: Fixed::ZERO
                },
                sta_content::TradeGood {
                    key: "good_b".into(),
                    value: Fixed::ONE
                },
            ],
        }),
    );
    assert_eq!(
        unit(&db, "cargo").trade_cargo_capacity,
        Some(Fixed::from_int(220))
    );
}

#[test]
fn trade_rules_change_only_their_unit_hashes() {
    let station = "unitName = \"Station\"\nclassLabel = \"trading_station\"\n";
    let cargo = "unitName = \"Cargo\"\nclassLabel = \"cargo_ship\"\n";
    let defaults = load(station, cargo);
    assert_eq!(
        unit(&defaults, "station")
            .trade_station
            .as_ref()
            .unwrap()
            .transfer_rate,
        Fixed::from_int(15)
    );
    assert_eq!(
        unit(&defaults, "station")
            .trade_station
            .as_ref()
            .unwrap()
            .profit_share,
        Fixed::from_f64(0.1)
    );
    assert_eq!(
        unit(&defaults, "cargo").trade_cargo_capacity,
        Some(Fixed::from_int(150))
    );
    assert_ne!(
        defaults.hash(),
        load(&format!("{station}cargoTransferRate = 20\n"), cargo).hash()
    );
    assert_ne!(
        defaults.hash(),
        load(&format!("{station}percentTradeProfit = 15\n"), cargo).hash()
    );
    assert_ne!(
        defaults.hash(),
        load(&format!("{station}cargoKey1 = \"good\"\n"), cargo).hash()
    );
    assert_ne!(
        defaults.hash(),
        load(station, &format!("{cargo}m_maxResource = 151\n")).hash()
    );
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}

#[test]
fn race_can_trade_defaults_true_and_denials_are_exposed_and_digested() {
    let default = Fixture::new(None).load();
    let denied = Fixture::new(Some("0")).load();
    assert!(default.can_trade("alpha"));
    assert!(default.can_trade("unknown"));
    assert!(
        default.can_trade("ALPHA"),
        "race lookup is case-insensitive"
    );
    assert!(!denied.can_trade("Alpha"));
    assert_ne!(
        default.hash(),
        denied.hash(),
        "a denied race changes the effective rules digest"
    );
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}
