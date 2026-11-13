use super::*;
use crate::WorldSource;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_content::{MapDef, Profile, UnitDef};

fn v(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
        z: Fixed::from_int(z),
    }
}

fn simulation() -> (Sim, Arc<ContentDb>) {
    let mut disabled = UnitDef::new("disabled", "craft");
    disabled.confinement.capability = Some(false);
    let db = Arc::new(
        ContentDb::builder()
            .profile(Profile::Sta2Classic)
            .unit(UnitDef::new("ordinary", "constplaceholder"))
            .unit(UnitDef::new("unsupported", "unverified_factory"))
            .unit(disabled)
            .build()
            .unwrap(),
    );
    let mut map = MapDef::empty();
    map.bounds = (v(-500, -10, -500), v(500, 10, 500));
    let setup = GameSetup {
        seed: 22,
        content: db.clone(),
        map: Arc::new(map),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "synthetic".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    for name in ["ordinary", "ordinary", "disabled", "unsupported"] {
        sim.spawn_def(
            db.key(name).unwrap(),
            PlayerId(0),
            Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    }
    (sim, db)
}

fn split(bytes: &[u8]) -> (Body, Option<(Vec3, Vec3)>, BoundaryRec, PerceptionRec) {
    let (body, rest) = postcard::take_from_bytes(&bytes[HEADER..]).unwrap();
    let (bounds, rest) = postcard::take_from_bytes(rest).unwrap();
    let (record, rest) = postcard::take_from_bytes(rest).unwrap();
    let (perception, rest) = postcard::take_from_bytes(rest).unwrap();
    let (personnel, rest): (PersonnelRec, _) = postcard::take_from_bytes(rest).unwrap();
    assert!(personnel.rows.is_empty());
    let (_, rest): (UnitCrewRec, _) = postcard::take_from_bytes(rest).unwrap();
    let (effects, rest): (StartEffectsRec, _) = postcard::take_from_bytes(rest).unwrap();
    assert!(effects.rows.is_empty());
    let (autonomy, rest): (crate::autonomy::AutonomyWire, _) = postcard::take_from_bytes(rest).unwrap();
    assert!(autonomy.rows.is_empty());
    assert!(rest.is_empty());
    (body, bounds, record, perception)
}

fn join(header: &[u8], body: &Body, bounds: Option<(Vec3, Vec3)>, record: &BoundaryRec, perception: &PerceptionRec) -> Vec<u8> {
    let mut bytes = header[..HEADER].to_vec();
    bytes.extend(postcard::to_allocvec(body).unwrap());
    bytes.extend(postcard::to_allocvec(&bounds).unwrap());
    bytes.extend(postcard::to_allocvec(record).unwrap());
    bytes.extend(postcard::to_allocvec(perception).unwrap());
    if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) >= PERSONNEL_FORMAT {
        bytes.extend(postcard::to_allocvec(&PersonnelRec { version: 1, rows: Vec::new() }).unwrap());
    }
    if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) >= CREW_FORMAT {
        bytes.extend(postcard::to_allocvec(&UnitCrewRec { version: 1, rows: Vec::new() }).unwrap());
    }
    if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) >= START_EFFECTS_FORMAT {
        bytes.extend(postcard::to_allocvec(&StartEffectsRec { version: 1, rows: Vec::new() }).unwrap());
    }
    if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) == FORMAT_VERSION {
        bytes.extend(postcard::to_allocvec(&crate::autonomy::AutonomyWire { version: 1, rows: Vec::new() }).unwrap());
    }
    bytes
}

#[test]
fn mandatory_suffix_rejects_bad_rows_versions_trailing_and_unusable_bounds() {
    let (sim, db) = simulation();
    let bytes = sim.snapshot();
    let (body, bounds, mut rec, perception) = split(&bytes);
    assert_eq!(rec.version, 1);
    assert_eq!(rec.rows, [(EntityId(1), true), (EntityId(2), true)]);
    for bad in [
        vec![], // every usable actor requires a row, even when false
        vec![(EntityId(1), false)],
        vec![
            (EntityId(1), true),
            (EntityId(1), false),
            (EntityId(2), true),
        ],
        vec![(EntityId(2), true), (EntityId(1), true)],
        vec![
            (EntityId(1), true),
            (EntityId(2), true),
            (EntityId(99), false),
        ],
        vec![
            (EntityId(1), true),
            (EntityId(2), true),
            (EntityId(3), false),
        ],
        vec![
            (EntityId(1), true),
            (EntityId(2), true),
            (EntityId(4), true),
        ],
    ] {
        rec.rows = bad;
        assert!(Sim::from_snapshot(&join(&bytes, &body, bounds, &rec, &perception), Some(db.clone())).is_err());
    }
    rec.rows = vec![(EntityId(1), false), (EntityId(2), true)];
    rec.version = 2;
    assert!(Sim::from_snapshot(&join(&bytes, &body, bounds, &rec, &perception), Some(db.clone())).is_err());
    rec.version = 1;
    for invalid in [None, Some((Vec3::ZERO, v(100, 0, 100)))] {
        assert!(Sim::from_snapshot(&join(&bytes, &body, invalid, &rec, &perception), Some(db.clone())).is_err());
    }
    let mut body_only = bytes[..HEADER].to_vec();
    body_only.extend(postcard::to_allocvec(&body).unwrap());
    assert!(Sim::from_snapshot(&body_only, Some(db.clone())).is_err());
    body_only.extend(postcard::to_allocvec(&bounds).unwrap());
    assert!(Sim::from_snapshot(&body_only, Some(db.clone())).is_err());
    for suffix in [vec![0], bytes.clone()] {
        let mut trailing = bytes.clone();
        trailing.extend(suffix);
        assert!(Sim::from_snapshot(&trailing, Some(db.clone())).is_err());
    }
}

#[test]
fn true_outside_flag_is_preserved_without_reinitializing_or_projecting() {
    let (mut sim, db) = simulation();
    let id = sim
        .spawn_def(
            db.key("ordinary").unwrap(),
            PlayerId(0),
            v(600, 50, 0),
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    assert_eq!(sim.force_onto_map(id), Some(false));
    let before = sim.unit(id).unwrap().clone();
    let command = Command {
        player: PlayerId::SCRIPT,
        seq: 0,
        kind: CommandKind::Edit(Edit::ForceOntoMap { id, enabled: true }),
    };
    assert!(sim.apply(&command));
    assert_eq!(
        sim.unit(id).unwrap(),
        &before,
        "flag-only application must not mutate the entity"
    );
    let saved = sim.snapshot();
    let mut loaded = Sim::from_snapshot(&saved, Some(db)).unwrap();
    assert_eq!(loaded.force_onto_map(id), Some(true));
    assert_eq!(loaded.unit(id).unwrap().pos, before.pos);
    assert_eq!(loaded.snapshot(), saved);
    for _ in 0..30 {
        assert_eq!(loaded.step(&[]), sim.step(&[]));
    }
    assert_eq!(
        loaded.unit(id).unwrap().pos,
        before.pos,
        "plain objects have no unconditional tick"
    );
}

#[test]
fn header_downgrades_and_forged_old_content_hashes_cannot_hide_state() {
    for sim in [Sim::new(22), simulation().0] {
        let bytes = sim.snapshot();
        for version in [8_u16, 9, 10] {
            let mut bad = bytes.clone();
            bad[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(Sim::from_snapshot(&bad, sim.content.clone()).is_err());
        }
    }
    // These remain genuine old writer outputs except for the deliberately forged
    // hash in this negative test. Matching a new content hash cannot supply flags.
    let (_, enabled) = simulation();
    for original in [
        include_bytes!("../../tests/fixtures/mining-v8.stas").as_slice(),
        include_bytes!("../../tests/fixtures/native-control-v9.stas").as_slice(),
    ] {
        let mut bad = original.to_vec();
        bad[10..18].copy_from_slice(&enabled.hash().to_le_bytes());
        assert!(Sim::from_snapshot(&bad, Some(enabled.clone())).is_err());
    }
}

#[test]
fn genuine_native_v9_controls_continue_with_the_old_digests() {
    let db = Arc::new(sta_content::fake::db());
    assert_eq!(db.hash(), 0xc0d4_3c3a_94f1_650e);
    let mut control = Sim::from_snapshot(
        include_bytes!("../../tests/fixtures/native-control-v9.stas"),
        Some(db.clone()),
    )
    .unwrap();
    assert_eq!(control.tick, 300);
    assert_eq!(control.digest(), 0xace6_bda2_8dd6_531f);
    assert_eq!(control.force_onto_map(EntityId(1)), None);
    for tick in 300..900 {
        let commands = if tick == 500 {
            vec![Command {
                player: PlayerId(0),
                seq: 2,
                kind: CommandKind::Order(Order::Stop {
                    units: vec![EntityId(1)],
                }),
            }]
        } else {
            vec![]
        };
        control.step(&commands);
        if control.tick == 600 {
            assert_eq!(control.digest(), 0xe551_eb38_7ec7_a1ee);
        }
    }
    assert_eq!(control.digest(), 0x2b96_65c4_c00e_0f3b);
    let mut idle = Sim::from_snapshot(
        include_bytes!("../../tests/fixtures/mining-idle-v9.stas"),
        Some(db),
    )
    .unwrap();
    assert_eq!(idle.tick, 101);
    assert_eq!(idle.digest(), 0x6493_8e8c_1b12_f06c);
    for _ in 0..800 {
        idle.step(&[]);
    }
    assert_eq!(idle.digest(), 0x2077_c2e5_597d_d8e5);
}

#[test]
fn tag20_is_appended_and_flag_only_rejections_are_atomic() {
    for (enabled, last) in [(false, 0), (true, 1)] {
        let edit = Edit::ForceOntoMap {
            id: EntityId(42),
            enabled,
        };
        let bytes = postcard::to_allocvec(&edit).unwrap();
        assert_eq!(bytes, [20, 42, last]);
        assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
    }
    let (mut sim, _) = simulation();
    for player in [
        PlayerId(0),
        PlayerId(100),
        PlayerId::SCRIPT,
        PlayerId::BENCH,
    ] {
        for id in [EntityId::NONE, EntityId(3), EntityId(4), EntityId(99)] {
            for enabled in [false, true] {
                let before = sim.snapshot();
                assert!(!sim.apply(&Command {
                    player,
                    seq: 0,
                    kind: CommandKind::Edit(Edit::ForceOntoMap { id, enabled })
                }));
                assert_eq!(sim.snapshot(), before);
            }
        }
    }
    let mut native = Sim::new(22);
    let id = native.spawn(
        0,
        Vec3::ZERO,
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::default(),
    );
    let before = native.snapshot();
    assert!(!native.apply(&Command {
        player: PlayerId::SCRIPT,
        seq: 0,
        kind: CommandKind::Edit(Edit::ForceOntoMap { id, enabled: false })
    }));
    assert_eq!(native.snapshot(), before);
}
