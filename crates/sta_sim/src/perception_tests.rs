//! Focused shared perceived-team behavior; persistence has separate tests.

use std::sync::Arc;

use sta_content::MapDef;

use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::world::NEUTRAL;
use crate::{EntityId, Fixed, MoveStats, PlayerId, Sim, SimEvent, TeamId, Vec3, WorldSource};

fn at(z: i32) -> Vec3 {
    Vec3::from_f32([2000.0, 0.0, z as f32])
}

fn scene() -> (Sim, EntityId, EntityId, EntityId) {
    let db = Arc::new(sta_content::fake::db());
    let setup = GameSetup {
        seed: 83,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: [0, 1, 2]
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Idle,
            })
            .to_vec(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let heading = (Fixed::ZERO, Fixed::ONE);
    let shooter = sim
        .spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(0), at(2000), heading)
        .unwrap();
    let ally = sim
        .spawn_def(db.key("fx_frigate").unwrap(), PlayerId(0), at(10_000), heading)
        .unwrap();
    let target = sim
        .spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), at(2350), heading)
        .unwrap();
    (sim, shooter, ally, target)
}

fn edit(player: PlayerId, seq: u32, value: Edit) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Edit(value),
    }
}

fn rejected(events: &[SimEvent]) -> bool {
    events.iter().any(|event| matches!(event, SimEvent::Rejected { .. }))
}

#[test]
fn default_override_reset_and_view_rows_are_normalized() {
    let (mut sim, shooter, ally, target) = scene();
    let baseline = sim.digest();
    assert_eq!(sim.perceived_owner(target), Some(PlayerId(1)));
    assert_eq!(sim.view().perceived_owner(target), Some(PlayerId(1)));
    assert!(sim.view().perceived_owners.is_empty());
    assert_eq!(sim.perceived_owner(EntityId(u32::MAX)), None);
    assert_eq!(sim.view().perceived_owner(EntityId(u32::MAX)), None);
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, PlayerId(0))));
    assert!(sim.apply_edit(PlayerId::BENCH, &Edit::PerceivedTeam(ally, PlayerId(2))));
    assert_eq!(sim.perceived_owner(target), Some(PlayerId(0)));
    assert_eq!(sim.unit(target).unwrap().owner, PlayerId(1));
    assert_eq!(sim.view().unit(target).unwrap().owner, PlayerId(1));
    assert_eq!(sim.view().perceived_owner(target), Some(PlayerId(0)));
    assert_eq!(
        sim.view().perceived_owners,
        vec![(ally, PlayerId(2)), (target, PlayerId(0))]
    );
    assert_ne!(sim.digest(), baseline);
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(ally, PlayerId(0))));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, PlayerId(1))));
    assert!(sim.view().perceived_owners.is_empty());
    assert_eq!(sim.digest(), baseline);
    assert_eq!(sim.perceived_owner(shooter), Some(PlayerId(0)));
}

#[test]
fn privilege_invalid_destinations_and_dying_targets_are_atomic() {
    let (mut sim, _, _, target) = scene();
    let original = sim.digest();
    for (from, id, to) in [
        (PlayerId(0), target, PlayerId(2)),
        (PlayerId::SCRIPT, EntityId(u32::MAX), PlayerId(2)),
        (PlayerId::SCRIPT, target, PlayerId(7)),
        (PlayerId::SCRIPT, target, PlayerId(8)),
        (PlayerId::SCRIPT, target, PlayerId::SCRIPT),
        (PlayerId::SCRIPT, target, PlayerId::BENCH),
    ] {
        let before = sim.digest();
        assert!(!sim.apply_edit(from, &Edit::PerceivedTeam(id, to)));
        assert_eq!(sim.digest(), before);
        assert!(sim.view().perceived_owners.is_empty());
    }
    assert_eq!(sim.digest(), original);
    let baseline = sim.unit(target).unwrap().clone();
    sim.units.get_mut(target).unwrap().flags = UnitFlags::DYING;
    let dying = sim.digest();
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, PlayerId(2))));
    assert_eq!(sim.digest(), dying);
    sim.units.get_mut(target).unwrap().flags = baseline.flags;
    assert_eq!(sim.digest(), original);
}

#[test]
fn live_construction_proxy_accepts_perception_without_changing_progress_or_real_owner() {
    let (mut sim, _, _, target) = scene();
    let progress = Fixed::ONE / Fixed::from_int(2);
    sim.units.get_mut(target).unwrap().progress = Some(progress);
    let real_owner = sim.unit(target).unwrap().owner;
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, PlayerId(2))));
    assert_eq!(sim.unit(target).unwrap().progress, Some(progress));
    assert_eq!(sim.unit(target).unwrap().owner, real_owner);
    assert_eq!(sim.perceived_owner(target), Some(PlayerId(2)));
    assert_eq!(sim.view().perceived_owner(target), Some(PlayerId(2)));
    let bytes = sim.snapshot();
    let loaded = Sim::from_snapshot(&bytes, sim.content().cloned()).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    assert_eq!(loaded.unit(target).unwrap().progress, Some(progress));
    assert_eq!(loaded.unit(target).unwrap().owner, real_owner);
    assert_eq!(loaded.perceived_owner(target), Some(PlayerId(2)));
}

#[test]
fn transfers_preserve_previous_perception_and_rejected_transfer_changes_nothing() {
    let (mut sim, shooter, _, target) = scene();
    // An implicit real owner becomes an explicit perceived identity on a real swap.
    let out = sim.step(&[edit(PlayerId::SCRIPT, 0, Edit::Team(target, PlayerId(2)))]);
    assert!(!rejected(&out.events));
    assert_eq!(sim.unit(target).unwrap().owner, PlayerId(2));
    assert_eq!(sim.perceived_owner(target), Some(PlayerId(1)));
    assert_eq!(sim.view().perceived_owners, vec![(target, PlayerId(1))]);
    assert!(sim.is_eliminated(PlayerId(1)));
    // A defeated seat remains a valid perceived identity.
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, PlayerId(1))));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, NEUTRAL)));
    let rows = sim.view().perceived_owners.clone();
    let failed = sim.step(&[edit(PlayerId(0), 0, Edit::Team(target, PlayerId(0)))]);
    assert!(rejected(&failed.events));
    assert_eq!(sim.perceived_owner(target), Some(NEUTRAL));
    assert_eq!(sim.view().perceived_owners, rows);
    assert_eq!(sim.unit(shooter).unwrap().owner, PlayerId(0));
    let out = sim.step(&[edit(PlayerId::BENCH, 0, Edit::Team(target, PlayerId(0)))]);
    assert!(!rejected(&out.events));
    assert_eq!(sim.unit(target).unwrap().owner, PlayerId(0));
    assert_eq!(sim.perceived_owner(target), Some(NEUTRAL));
}

#[test]
fn script_sequence_orders_perception_and_real_team_edits_within_one_batch() {
    let (mut sim, _, _, target) = scene();
    let out = sim.step(&[
        edit(PlayerId::SCRIPT, 2, Edit::PerceivedTeam(target, PlayerId(0))),
        edit(PlayerId::SCRIPT, 0, Edit::PerceivedTeam(target, PlayerId(2))),
        edit(PlayerId::SCRIPT, 1, Edit::Team(target, PlayerId(2))),
    ]);
    assert!(!rejected(&out.events));
    assert_eq!(sim.unit(target).unwrap().owner, PlayerId(2));
    assert_eq!(sim.perceived_owner(target), Some(PlayerId(0)));
    assert_eq!(out.view.perceived_owners, vec![(target, PlayerId(0))]);
}

#[test]
fn transfer_preserves_unconfigured_real_slot_as_historical_perception() {
    let (mut sim, _, _, _) = scene();
    let key = sim.content().unwrap().key("fx_frigate").unwrap();
    let id = sim
        .spawn_def(key, PlayerId(7), at(8000), (Fixed::ZERO, Fixed::ONE))
        .unwrap();
    assert_eq!(sim.perceived_owner(id), Some(PlayerId(7)));
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(id, PlayerId(7))));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(id, PlayerId(0))));
    assert_eq!(sim.unit(id).unwrap().owner, PlayerId(0));
    assert_eq!(sim.perceived_owner(id), Some(PlayerId(7)));
    assert_eq!(sim.view().perceived_owner(id), Some(PlayerId(7)));
    let bytes = sim.snapshot();
    let loaded = Sim::from_snapshot(&bytes, sim.content().cloned()).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    assert_eq!(loaded.perceived_owner(id), Some(PlayerId(7)));
    assert_eq!(loaded.unit(id).unwrap().owner, PlayerId(0));
}

#[test]
fn transfer_rejects_reserved_previous_identity_without_mutation() {
    let (mut sim, _, _, _) = scene();
    let key = sim.content().unwrap().key("fx_frigate").unwrap();
    let id = sim
        .spawn_def(key, PlayerId::SCRIPT, at(8000), (Fixed::ZERO, Fixed::ONE))
        .unwrap();
    let before = sim.snapshot();
    let unit = sim.unit(id).unwrap().clone();
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(id, PlayerId(0))));
    assert_eq!(sim.unit(id).unwrap(), &unit);
    assert_eq!(sim.perceived_owner(id), Some(PlayerId::SCRIPT));
    assert_eq!(sim.snapshot(), before);
}

#[test]
fn appended_edit_tags_keep_the_previous_force_tag() {
    let id = EntityId(1);
    let force = Edit::ForceOntoMap { id, enabled: true };
    let perceived = Edit::PerceivedTeam(id, PlayerId(2));
    assert_eq!(postcard::to_allocvec(&force).unwrap(), [20, 1, 1]);
    assert_eq!(postcard::to_allocvec(&perceived).unwrap(), [21, 1, 2]);
    for value in [force, perceived] {
        let bytes = postcard::to_allocvec(&value).unwrap();
        assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), value);
    }
}

#[test]
fn despawn_clears_rows_and_seatless_synthetic_edits_keep_real_control() {
    let mut bare = Sim::new(83);
    let id = bare.spawn(0, at(0), (Fixed::ZERO, Fixed::ONE), MoveStats::STATIC);
    bare.units.get_mut(id).unwrap().hull = [Fixed::ONE; 2];
    assert!(bare.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(id, PlayerId(7))));
    assert_eq!(bare.perceived_owner(id), Some(PlayerId(7)));
    assert_eq!(bare.unit(id).unwrap().owner, PlayerId(0));
    assert!(!bare.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(id, PlayerId(8))));
    assert!(bare.despawn(id, None));
    assert_eq!(bare.perceived_owner(id), None);
    assert_eq!(bare.view().perceived_owner(id), None);
    assert!(bare.view().perceived_owners.is_empty());
    let next = bare.spawn(0, at(10), (Fixed::ZERO, Fixed::ONE), MoveStats::STATIC);
    assert_ne!(next, id);
    assert_eq!(bare.perceived_owner(next), Some(PlayerId(0)));
}

#[test]
fn automatic_fire_uses_perception_but_explicit_attack_uses_real_control() {
    let (mut sim, shooter, _, target) = scene();
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, PlayerId(0))));
    for _ in 0..3 {
        let out = sim.step(&[]);
        assert!(!out
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Fired { shooter: id, .. } if *id == shooter)));
    }
    let denied = sim.step(&[Command {
        player: PlayerId(1),
        seq: 0,
        kind: CommandKind::Order(Order::Attack {
            units: vec![shooter],
            target,
        }),
    }]);
    assert!(rejected(&denied.events));
    let ordered = sim.step(&[Command {
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::Order(Order::Attack {
            units: vec![shooter],
            target,
        }),
    }]);
    assert!(!rejected(&ordered.events));
    assert!(ordered
        .events
        .iter()
        .any(|e| matches!(e, SimEvent::Fired { shooter: id, .. } if *id == shooter)));
    let (mut ordinary, shooter, _, target) = scene();
    let fired = ordinary.step(&[]);
    assert!(fired.events.iter().any(|e| matches!(e, SimEvent::Fired { shooter: id, target: crate::event::Target::Entity(t), .. } if *id == shooter && *t == target)));
}

#[test]
fn real_enemy_cloak_blocks_explicit_fire_even_when_perceived_friendly() {
    let (mut sim, shooter, _, target) = scene();
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::PerceivedTeam(target, PlayerId(0))));
    let flags = sim.unit(target).unwrap().flags;
    sim.units.get_mut(target).unwrap().flags = flags.with(UnitFlags::CLOAKED, true);
    let attack = Command {
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::Order(Order::Attack {
            units: vec![shooter],
            target,
        }),
    };
    let hidden = sim.step(&[attack]);
    assert!(!rejected(&hidden.events));
    assert!(!hidden
        .events
        .iter()
        .any(|e| matches!(e, SimEvent::Fired { shooter: id, .. } if *id == shooter)));
    sim.units.get_mut(target).unwrap().flags = flags;
    let revealed = sim.step(&[]);
    assert!(revealed
        .events
        .iter()
        .any(|e| matches!(e, SimEvent::Fired { shooter: id, .. } if *id == shooter)));
}
