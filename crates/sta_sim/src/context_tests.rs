//! The right-click group vote ([`super::context_order`]) and what the sim
//! does with the winning order for units that cannot carry it out.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef};

use super::context_order;
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{EntityId, Fixed, PlayerId, Sim, TeamId, Vec3, WorldSource};

struct Scene {
    sim: Sim,
    db: Arc<ContentDb>,
    miner: EntityId,
    cruiser: EntityId,
    cruiser2: EntityId,
    moon: EntityId,
    enemy: EntityId,
}

fn scene() -> Scene {
    let db = Arc::new(sta_content::fake::db());
    let setup = GameSetup {
        seed: 5,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: [0, 1]
            .map(|id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle })
            .to_vec(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let h = (Fixed::ZERO, Fixed::ONE);
    let at = |x: f32, z: f32| Vec3::from_f32([x, 0.0, z]);
    let mut spawn = |odf: &str, owner: u8, x: f32, z: f32| sim.spawn_def(db.key(odf).unwrap(), PlayerId(owner), at(x, z), h).unwrap();
    let miner = spawn("fx_miner", 0, 900.0, 1000.0);
    let cruiser = spawn("fx_cruiser", 0, 1100.0, 1000.0);
    let cruiser2 = spawn("fx_cruiser", 0, 1300.0, 1000.0);
    let moon = spawn("fx_moon", 255, 1000.0, 2200.0);
    let enemy = spawn("fx_frigate", 1, 1000.0, 6000.0);
    Scene { sim, db, miner, cruiser, cruiser2, moon, enemy }
}

#[test]
fn mixed_group_on_a_resource_sends_mine_to_everyone() {
    let s = scene();
    let v = s.sim.view();
    let own = [s.cruiser, s.miner];
    assert_eq!(
        context_order(Some(&s.db), &v, PlayerId(0), &own, s.moon),
        Some(Order::Mine { units: own.to_vec(), node: s.moon })
    );
}

#[test]
fn nobody_votes_gives_a_plain_move() {
    let s = scene();
    let v = s.sim.view();
    // Warships on a moon, an unarmed miner on an enemy: no vote, a move.
    assert_eq!(context_order(Some(&s.db), &v, PlayerId(0), &[s.cruiser, s.cruiser2], s.moon), None);
    assert_eq!(context_order(Some(&s.db), &v, PlayerId(0), &[s.miner], s.enemy), None);
}

#[test]
fn attack_outvotes_mine_and_goes_to_unarmed_units_too() {
    let s = scene();
    let v = s.sim.view();
    let own = [s.miner, s.cruiser];
    assert_eq!(
        context_order(Some(&s.db), &v, PlayerId(0), &own, s.enemy),
        Some(Order::Attack { units: own.to_vec(), target: s.enemy })
    );
}

#[test]
fn units_that_cannot_carry_the_winning_order_keep_their_own() {
    let mut s = scene();
    let order = |seq, o| Command { player: PlayerId(0), seq, kind: CommandKind::Order(o) };
    let away = Vec3::from_f32([1100.0, 0.0, 9000.0]);
    s.sim.step(&[order(1, Order::Move { units: vec![s.cruiser], to: away, queue: false })]);
    let v = s.sim.view();
    let mine = context_order(Some(&s.db), &v, PlayerId(0), &[s.cruiser, s.miner], s.moon).unwrap();
    s.sim.step(&[order(2, mine)]);
    for _ in 0..90 {
        s.sim.step(&[]);
    }
    let v = s.sim.view();
    let pos = |id| v.unit(id).unwrap().pos;
    // The cruiser still heads for its point, past the moon; the miner for the moon.
    assert!(pos(s.cruiser).z > Fixed::from_int(1100), "cruiser kept its move: {:?}", pos(s.cruiser));
    assert_eq!(pos(s.cruiser).x, Fixed::from_int(1100), "straight on its course");
    assert!(pos(s.miner).z > Fixed::from_int(1000), "miner went to mine: {:?}", pos(s.miner));
}

/// A freighter whose mining beam is a hardpoint, and a construction rig with
/// a tractor beam (ordnance class `tractor_bm`), as stock ODFs give them: neither votes
/// Attack, so on an enemy they get a plain move.
#[test]
fn mining_and_tractor_beams_do_not_vote_attack() {
    use sta_content::{DefRef, Hardpoint, UnitDef, WeaponDef, WeaponKind};
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    let mut mine = WeaponDef::new("fx_minebeam", "minebm", WeaponKind::Projectile);
    mine.damage = Fixed::from_int(15);
    let tractor = WeaponDef::new("fx_tractor", "tractor_bm", WeaponKind::Beam);
    let hp = |w: &str| Hardpoint { weapon: DefRef::new(w), node: "hp01".into(), arc: None };
    let mut miner = fake.unit(fake.key("fx_miner").unwrap()).unwrap().clone();
    miner.odf = "fx_armed_miner".into();
    miner.hardpoints = vec![hp("fx_minebeam")];
    let mut rig = UnitDef::new("fx_tractor_rig", "constructionrig");
    rig.movement = miner.movement;
    rig.hull = Fixed::from_int(100);
    rig.hardpoints = vec![hp("fx_tractor")];
    let db = Arc::new(b.weapon(mine).weapon(tractor).unit(miner).unit(rig).build().unwrap());
    let setup = GameSetup {
        seed: 5,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: [0, 1]
            .map(|id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle })
            .to_vec(),
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let h = (Fixed::ZERO, Fixed::ONE);
    let at = |x: f32| Vec3::from_f32([x, 0.0, 0.0]);
    let m = sim.spawn_def(db.key("fx_armed_miner").unwrap(), PlayerId(0), at(0.0), h).unwrap();
    let r = sim.spawn_def(db.key("fx_tractor_rig").unwrap(), PlayerId(0), at(200.0), h).unwrap();
    let c = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(0), at(400.0), h).unwrap();
    let e = sim.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), at(3000.0), h).unwrap();
    let v = sim.view();
    assert_eq!(context_order(Some(&db), &v, PlayerId(0), &[m, r], e), None);
    let attack = context_order(Some(&db), &v, PlayerId(0), &[m, r, c], e).unwrap();
    assert_eq!(attack, Order::Attack { units: vec![m, r, c], target: e });
    // The sim takes it for the cruiser only; the miner and the rig keep theirs.
    sim.step(&[Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(attack.clone()) }]);
    assert_eq!((sim.weapons.attack_of(m), sim.weapons.attack_of(r), sim.weapons.attack_of(c)), (None, None, Some(e)));
    assert_eq!(super::takers(Some(&db), &v, &attack), vec![c]);
}
