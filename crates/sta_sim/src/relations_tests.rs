//! Mission relations (SIM-63) against `fake::db()`.

use crate::command::{Command, CommandKind, Edit, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::Relation;
use crate::{Sim, WorldSource, WorldView};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn at(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// Seats 1 and 2 on different teams, 3 on a third. A cruiser each for 1 and 2,
/// 350 apart (inside weapon range); seat 3's frigate far away.
struct Fx {
    sim: Sim,
    a: EntityId,
    b: EntityId,
    far: EntityId,
}

fn fixture() -> Fx {
    let db = sta_content::fake::db();
    let (cruiser, frigate) = (db.key("fx_cruiser").unwrap(), db.key("fx_frigate").unwrap());
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1), seat(2), seat(3)],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let a = sim.spawn_def(cruiser, PlayerId(1), at(2000, 2000), AHEAD).unwrap();
    let b = sim.spawn_def(cruiser, PlayerId(2), at(2000, 2350), (Fixed::ZERO, -Fixed::ONE)).unwrap();
    let far = sim.spawn_def(frigate, PlayerId(3), at(9000, 9000), AHEAD).unwrap();
    Fx { sim, a, b, far }
}

fn rel(from: u8, to: u8, relation: Relation) -> Command {
    Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::SetRelation { from: PlayerId(from), to: PlayerId(to), relation }) }
}

fn attack(player: u8, shooter: EntityId, target: EntityId) -> Command {
    Command { player: PlayerId(player), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![shooter], target }) }
}

/// Who fired over `ticks` ticks.
fn shooters(sim: &mut Sim, ticks: u32, first: &[Command]) -> Vec<EntityId> {
    let mut out = Vec::new();
    for i in 0..ticks {
        let o = sim.step(if i == 0 { first } else { &[] });
        for e in &o.events {
            if let SimEvent::Fired { shooter, .. } = e {
                out.push(*shooter);
            }
        }
    }
    out
}

fn sees(v: &WorldView, p: u8, id: EntityId) -> bool {
    v.visible_to(PlayerId(p)).iter().any(|u| u.id == id)
}

#[test]
fn untouched_game_keeps_team_relations_and_fires_both_ways() {
    let mut f = fixture();
    let v = f.sim.view();
    assert_eq!(v.players[0].relations, [(PlayerId(2), Relation::Enemy), (PlayerId(3), Relation::Enemy)]);
    let fired = shooters(&mut f.sim, 60, &[]);
    assert!(fired.contains(&f.a) && fired.contains(&f.b));
}

#[test]
fn neutral_stops_auto_fire_only_in_the_setters_direction() {
    let mut f = fixture();
    let fired = shooters(&mut f.sim, 60, &[rel(1, 2, Relation::Neutral)]);
    assert!(!fired.contains(&f.a), "seat 1 no longer picks seat 2");
    assert!(fired.contains(&f.b), "seat 2's own view is still Enemy");
    let v = f.sim.view();
    assert_eq!(v.players[0].relations[0], (PlayerId(2), Relation::Neutral));
    assert_eq!(v.players[1].relations[0], (PlayerId(1), Relation::Enemy));
}

#[test]
fn both_neutral_or_ally_means_no_shots_and_ally_needs_both_ways_for_sight() {
    for r in [Relation::Neutral, Relation::Ally] {
        let mut f = fixture();
        let fired = shooters(&mut f.sim, 60, &[rel(1, 2, r), rel(2, 1, r)]);
        assert!(fired.is_empty(), "{r:?}: {fired:?}");
    }
    // Vision: seat 3's frigate is out of everyone's sensor range except seat 3.
    let mut f = fixture();
    f.sim.step(&[rel(3, 1, Relation::Ally)]);
    assert!(!sees(&f.sim.view(), 1, f.far), "one-sided Ally shares nothing");
    f.sim.step(&[rel(1, 3, Relation::Ally)]);
    let v = f.sim.view();
    assert!(sees(&v, 1, f.far) && sees(&v, 3, f.a), "mutual Ally shares sight");
    assert!(!sees(&v, 2, f.far));
    f.sim.step(&[rel(1, 3, Relation::Enemy)]);
    assert!(!sees(&f.sim.view(), 1, f.far), "sight ends with the relation");
}

#[test]
fn setting_mid_combat_stops_and_resumes_fire() {
    let mut f = fixture();
    assert!(shooters(&mut f.sim, 40, &[]).contains(&f.a));
    assert!(!shooters(&mut f.sim, 40, &[rel(1, 2, Relation::Neutral)]).contains(&f.a));
    assert!(shooters(&mut f.sim, 40, &[rel(1, 2, Relation::Enemy)]).contains(&f.a));
}

#[test]
fn explicit_attack_is_allowed_on_a_neutral_and_dropped_when_the_view_changes() {
    let mut f = fixture();
    f.sim.step(&[rel(1, 2, Relation::Neutral)]);
    assert!(shooters(&mut f.sim, 60, &[attack(1, f.a, f.b)]).contains(&f.a), "no relation check on a commanded attack");
    assert_eq!(f.sim.weapons.attack_of(f.a), Some(f.b));
    f.sim.step(&[rel(1, 2, Relation::Ally)]);
    assert_eq!(f.sim.weapons.attack_of(f.a), None, "the view of the target changed");
    assert!(!shooters(&mut f.sim, 40, &[]).contains(&f.a));
}

#[test]
fn rejected_edits_and_default_normalisation() {
    let mut f = fixture();
    let d0 = f.sim.digest();
    for bad in [rel(1, 1, Relation::Neutral), rel(1, 9, Relation::Neutral), rel(9, 1, Relation::Enemy)] {
        assert!(!f.sim.apply_edit(PlayerId::SCRIPT, match &bad.kind { CommandKind::Edit(e) => e, _ => unreachable!() }));
    }
    assert!(!f.sim.apply_edit(PlayerId(1), &Edit::SetRelation { from: PlayerId(1), to: PlayerId(2), relation: Relation::Neutral }), "players may not edit");
    assert_eq!(f.sim.digest(), d0);
    // Setting the default is a no-op for the digest; a real change is not.
    f.sim.step(&[rel(1, 2, Relation::Enemy)]);
    let mut g = fixture();
    g.sim.step(&[]);
    assert_eq!(f.sim.digest(), g.sim.digest());
    f.sim.step(&[rel(1, 2, Relation::Ally)]);
    g.sim.step(&[]);
    assert_ne!(f.sim.digest(), g.sim.digest());
    f.sim.step(&[rel(1, 2, Relation::Enemy)]);
    g.sim.step(&[]);
    assert_eq!(f.sim.digest(), g.sim.digest(), "back to the default is the untouched state");
}

#[test]
fn snapshot_round_trip_continues_identically_and_replays_deterministically() {
    let cmds = [rel(1, 2, Relation::Neutral), rel(3, 1, Relation::Ally)];
    let run = |snap_at: Option<u32>| {
        let mut f = fixture();
        f.sim.step(&cmds);
        let mut sim = f.sim;
        for i in 0..80 {
            if snap_at == Some(i) {
                let bytes = sim.snapshot();
                sim = Sim::from_snapshot(&bytes, Some(Arc::new(sta_content::fake::db()))).unwrap();
            }
            sim.step(&[]);
        }
        sim.digest()
    };
    assert_eq!(run(None), run(None), "same commands, same digest");
    assert_eq!(run(None), run(Some(30)), "a snapshot mid-run continues identically");
}

#[test]
fn corrupt_relation_rows_are_refused() {
    use crate::relations::Relations;
    let players = [(PlayerId(1), TeamId(1)), (PlayerId(2), TeamId(2))];
    assert!(Relations::from_rows(&[(1, 2, 1)], &players).is_some());
    for bad in [vec![(1, 2, 3)], vec![(1, 1, 1)], vec![(1, 5, 1)], vec![(1, 2, 0)], vec![(2, 1, 1), (1, 2, 1)], vec![(1, 2, 1), (1, 2, 2)]] {
        assert!(Relations::from_rows(&bad, &players).is_none(), "{bad:?}");
    }
}
