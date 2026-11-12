use std::sync::Arc;

use sta_content::{Features, MapDef, MoveStats, UnitDef};

use crate::command::Edit;
use crate::setup::{GameOptions, GameSetup};
use crate::world::{Entity, EntityKind};
use crate::{Fixed, PlayerId, Sim, Vec3, WorldSource};

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// Test-only control: retain every saved state field except the newly added
/// crew pools, so existing gameplay goldens must still match byte-for-byte.
/// `fake::db()`'s content hash before CON-35's keyed digest (FNV-1a over
/// the old encoding). Goldens recorded before then fold it in.
pub(crate) const LEGACY_FAKE_CONTENT_HASH: u64 = 0xc0d4_3c3a_94f1_650e;

/// The digest as it was before unit crew and before CON-35: crew removed,
/// and `fake::db()` content identified by its old hash, so state goldens
/// recorded then still check the same state.
pub(crate) fn digest_before_crew(sim: &Sim) -> u64 {
    let mut control = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
    let ids: Vec<_> = control.units.iter().map(|unit| unit.id).collect();
    for id in ids {
        control.units.get_mut(id).unwrap().crew = None;
    }
    let hash = control.content.as_ref().map_or(0, |c| c.hash());
    let fake = sta_content::fake::db().hash();
    control.digest_with_content_hash(if hash == fake { LEGACY_FAKE_CONTENT_HASH } else { hash })
}

#[test]
fn bare_and_content_entities_keep_distinct_crew_presence() {
    let bare = Entity::new(
        EntityKind::Craft,
        sta_content::ContentKey::NONE,
        PlayerId(0),
        Vec3::ZERO,
        AHEAD,
        MoveStats::default(),
    );
    assert_eq!(bare.crew, None);
    assert_eq!(bare.view().crew, None);
    assert_eq!(bare.view().energy, None);

    for (class, kind, expected) in [
        ("craft", EntityKind::Craft, Some([Fixed::from_int(9); 2])),
        (
            "shipyard",
            EntityKind::Station,
            Some([Fixed::from_int(9); 2]),
        ),
        ("moon", EntityKind::Resource, None),
    ] {
        let mut def = UnitDef::new("status_test", class);
        def.crew = Fixed::from_int(9);
        let entity = Entity::from_def(&def, &Features::default(), PlayerId(0), Vec3::ZERO, AHEAD);
        assert_eq!(entity.kind, kind);
        assert_eq!(entity.crew, expected);
        assert_eq!(entity.view().crew, expected);
    }

    let mut def = UnitDef::new("zero_crew", "craft");
    def.crew = Fixed::from_int(-5);
    let negative = Entity::from_def(&def, &Features::default(), PlayerId(0), Vec3::ZERO, AHEAD);
    assert_eq!(negative.crew, Some([Fixed::ZERO; 2]));
    def.crew = Fixed::ZERO;
    let zero = Entity::from_def(&def, &Features::default(), PlayerId(0), Vec3::ZERO, AHEAD);
    assert_eq!(zero.crew, Some([Fixed::ZERO; 2]));
}

#[test]
fn crew_is_live_canonical_state_and_hashes_only_when_known() {
    let mut sim = Sim::new(7);
    let id = sim.spawn(0, Vec3::ZERO, AHEAD, MoveStats::default());
    let unknown_digest = sim.digest();
    assert_eq!(sim.view().unit(id).unwrap().crew, None);

    sim.units.get_mut(id).unwrap().crew = Some([Fixed::ZERO; 2]);
    let zero_digest = sim.digest();
    assert_ne!(zero_digest, unknown_digest);
    assert_eq!(sim.view().unit(id).unwrap().crew, Some([Fixed::ZERO; 2]));

    sim.units.get_mut(id).unwrap().crew = Some([Fixed::from_int(3), Fixed::from_int(8)]);
    assert_ne!(sim.digest(), zero_digest);
    assert_eq!(
        sim.view().unit(id).unwrap().crew,
        Some([Fixed::from_int(3), Fixed::from_int(8)])
    );

    sim.units.get_mut(id).unwrap().crew = None;
    assert_eq!(sim.digest(), unknown_digest);
    assert!(sim.despawn(id, None));
    assert_eq!(sim.view().unit(id), None);
}

#[test]
fn content_spawn_crew_survives_transfer_and_despawn_removes_view() {
    let db = Arc::new(sta_content::fake::db());
    let setup = GameSetup {
        seed: 8,
        map: Arc::new(MapDef::empty()),
        content: Arc::clone(&db),
        players: Vec::new(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim
        .spawn_def(
            db.key("fx_cruiser").unwrap(),
            PlayerId(0),
            Vec3::ZERO,
            AHEAD,
        )
        .unwrap();
    let expected = Some([Fixed::from_int(300); 2]);
    assert_eq!(sim.unit(id).unwrap().crew, expected);
    assert_eq!(sim.view().unit(id).unwrap().crew, expected);
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(id, PlayerId(1))));
    assert_eq!(sim.unit(id).unwrap().crew, expected);
    assert_eq!(sim.view().unit(id).unwrap().crew, expected);
    assert!(sim.despawn(id, None));
    assert!(sim.view().unit(id).is_none());
}

#[test]
fn energy_view_joins_only_initialized_live_energy_without_mutating_it() {
    let db = Arc::new(crate::specials::fixture::db());
    let setup = GameSetup {
        seed: 9,
        map: Arc::new(MapDef::empty()),
        content: Arc::clone(&db),
        players: Vec::new(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim
        .spawn_def(
            db.key("fx_cloaker").unwrap(),
            PlayerId(0),
            Vec3::ZERO,
            AHEAD,
        )
        .unwrap();
    let before = sim.digest();
    assert_eq!(sim.specials.energy_of(id), None);
    assert_eq!(sim.view().unit(id).unwrap().energy, None);
    assert_eq!(sim.digest(), before);
    assert_eq!(sim.specials.energy_of(id), None);

    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::SetEnergy(id, Fixed::from_int(17))));
    let after = sim.digest();
    assert_eq!(
        sim.view().unit(id).unwrap().energy,
        Some([Fixed::from_int(17), Fixed::from_int(600)])
    );
    assert_eq!(sim.specials.energy_of(id), Some(Fixed::from_int(17)));
    assert_eq!(sim.digest(), after);

    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::SetEnergy(id, Fixed::from_int(4))));
    assert_eq!(
        sim.view().unit(id).unwrap().energy,
        Some([Fixed::from_int(4), Fixed::from_int(600)])
    );
}
