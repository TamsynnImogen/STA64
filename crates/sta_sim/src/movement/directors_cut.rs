//! Armada I Director's Cut keeps attacking craft in motion. The original's
//! dogfight steering pursues an angularly offset point around an anchor
//! (Armada.exe, static). In STA64's planar movement model the anchor follows
//! the enemy. Steering room accounts for unit radii and the craft's turn
//! radius, so even the small stock anchor radius yields a flyable orbit.

use sta_content::A1DogfightRules;
use sta_core::{Fixed, Vec3};

use crate::Sim;
use crate::kinematics;
use crate::world::{Entity, Fnv};

impl Sim {
    pub(crate) fn hash_directors_cut(&self, hash: &mut Fnv) {
        if self.directors_cut {
            hash.str("a1-directors-cut");
        }
    }
}

pub(super) fn pursuit_point(
    craft: &Entity,
    enemy: &Entity,
    distance: Fixed,
    clearance: Fixed,
    turn_radius: Fixed,
    rules: A1DogfightRules,
) -> Vec3 {
    let radial = if distance > Fixed::ZERO {
        (
            (craft.pos.x - enemy.pos.x) / distance,
            (craft.pos.z - enemy.pos.z) / distance,
        )
    } else {
        (-craft.heading.0, -craft.heading.1)
    };
    let angle = rules
        .pursuit_angle
        .clamp(-kinematics::pi(), kinematics::pi());
    let offset = kinematics::rotate_by(radial, -angle);
    let radius = rules
        .anchor_radius
        .max(clearance + kinematics::arrive_distance())
        .max(turn_radius * Fixed::from_int(2));
    Vec3 {
        x: enemy.pos.x + offset.0 * radius,
        y: enemy.pos.y,
        z: enemy.pos.z + offset.1 * radius,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::{
        ContentDb, DefRef, Hardpoint, MapDef, MoveStats, Profile, UnitDef, WeaponDef, WeaponKind,
    };
    use sta_core::{EntityId, PlayerId, TeamId};

    use super::*;
    use crate::WorldSource;
    use crate::command::{Command, CommandKind, Order};
    use crate::replay::{Recorder, Replay};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};

    fn setup(profile: Profile, enabled: bool) -> GameSetup {
        let play = crate::kinematics::a1_tests::play();
        let mut weapon = WeaponDef::new("beam", "phaser", WeaponKind::Beam);
        weapon.range = Fixed::from_int(300);
        weapon.damage = Fixed::ONE;
        weapon.refire = 30;
        let mut ship = UnitDef::new("ship", "craft");
        ship.radius = Fixed::from_int(4);
        ship.sensor = Fixed::from_int(600);
        ship.movement = MoveStats::from_f32(24.0, 1.0, 3.0, 1.0, 2.0);
        ship.a1_combat_dynamics = Some(sta_content::A1Dynamics {
            turn_radius: Fixed::from_int(13),
            ..play
        });
        ship.hardpoints = vec![Hardpoint {
            weapon: DefRef::new("beam"),
            node: "hp01".into(),
            arc: None,
        }];
        let mut station = UnitDef::new("station", "starbase");
        station.radius = Fixed::from_int(12);
        station.hull = Fixed::from_int(10_000);
        let db = ContentDb::builder()
            .profile(profile)
            .a1_play_physics(Some(play))
            .weapon(weapon)
            .unit(ship)
            .unit(station)
            .build()
            .unwrap();
        GameSetup {
            seed: 68,
            map: Arc::new(MapDef::empty()),
            content: Arc::new(db),
            players: [0, 1]
                .into_iter()
                .map(|seat| PlayerSetup {
                    id: PlayerId(seat),
                    team: TeamId(seat),
                    race: "test".into(),
                    control: Control::Idle,
                })
                .collect(),
            options: GameOptions {
                directors_cut: enabled,
                ..GameOptions::default()
            },
        }
    }

    fn spawn(sim: &mut Sim) -> (EntityId, EntityId) {
        let db = sim.content.clone().unwrap();
        let heading = (Fixed::ZERO, Fixed::ONE);
        let ship = sim
            .spawn_def(
                db.key("ship").unwrap(),
                PlayerId(0),
                Vec3::from_f32([0.0, 0.0, -100.0]),
                heading,
            )
            .unwrap();
        let enemy = sim
            .spawn_def(db.key("station").unwrap(), PlayerId(1), Vec3::ZERO, heading)
            .unwrap();
        (ship, enemy)
    }

    #[test]
    fn directors_cut_moves_the_attacker_and_snapshot_keeps_the_same_flight() {
        let profiles = |enabled| setup(Profile::Sta1Classic, enabled);
        let mut dogfight = Sim::from_setup(&profiles(true));
        let mut normal = Sim::from_setup(&profiles(false));
        assert_ne!(dogfight.digest(), normal.digest());
        assert_ne!(
            dogfight.section_digests().movement,
            normal.section_digests().movement
        );
        let (ship, enemy) = spawn(&mut dogfight);
        assert_eq!(spawn(&mut normal), (ship, enemy));
        let initial = dogfight.unit(ship).unwrap().pos;
        let order = Order::Attack {
            units: vec![ship],
            target: enemy,
        };
        let attack = Command {
            player: PlayerId(0),
            seq: 0,
            kind: CommandKind::Order(order),
        };
        dogfight.step(std::slice::from_ref(&attack));
        normal.step(&[attack]);
        for _ in 0..240 {
            dogfight.step(&[]);
            normal.step(&[]);
        }
        assert_eq!(normal.unit(ship).unwrap().pos, initial);
        assert_ne!(dogfight.unit(ship).unwrap().pos.x, initial.x);
        assert!(dogfight.unit(ship).unwrap().speed > Fixed::ZERO);
        assert_eq!(dogfight.unit(enemy).unwrap().pos, Vec3::ZERO);

        let mut restored =
            Sim::from_snapshot(&dogfight.snapshot(), dogfight.content.clone()).unwrap();
        for _ in 0..240 {
            dogfight.step(&[]);
            restored.step(&[]);
        }
        assert_eq!(dogfight.digest(), restored.digest());
        assert!(dogfight.unit(enemy).unwrap().hull[0] < Fixed::from_int(10_000));
        restored.step(&[Command {
            player: PlayerId(0),
            seq: 1,
            kind: CommandKind::Order(Order::Stop { units: vec![ship] }),
        }]);
        assert!(restored.weapons.attack_of(ship).is_none());
        assert!(restored.unit(ship).unwrap().target.is_none());
    }

    #[test]
    fn old_a1_saves_and_replay_digests_ignore_only_the_new_rules() {
        let setup = setup(Profile::Sta1Classic, false);
        let sim = Sim::from_setup(&setup);
        let entries = setup.content.digest().entries();
        assert!(entries.iter().any(|e| e.key == "a1-dogfight:ship"));
        let old_hash = sta_content::ContentDigest::from_entries(
            entries
                .iter()
                .filter(|e| e.key != "a1-dogfight:ship")
                .cloned()
                .collect(),
        )
        .hash64();
        assert_ne!(old_hash, setup.content.hash());

        let mut old_save = sim.snapshot();
        // The Director's Cut flag (one byte), the move-mode, beams, dying and warp ramp records
        // (two bytes each: no rows) are the records appended after format 50 (SIM-76 appended
        // the second, #1068 the third, #1100 the fourth, #1102 the fifth).
        old_save.truncate(old_save.len() - 9);
        old_save[4..6].copy_from_slice(&crate::snapshot::PRE_DIRECTORS_CUT_FORMAT.to_le_bytes());
        old_save[10..18].copy_from_slice(&old_hash.to_le_bytes());
        let restored = Sim::from_snapshot(&old_save, Some(setup.content.clone())).unwrap();
        assert_eq!(restored.digest(), sim.digest());

        let mut old_replay = Recorder::with_interval(&setup, 1).finish();
        old_replay.content_hash = old_hash;
        old_replay.digests = vec![(0, sim.digest_with_content_hash(old_hash))];
        assert_eq!(
            old_replay.play(setup.content.clone()).unwrap().digest(),
            sim.digest()
        );
        old_replay.options.directors_cut = true;
        assert!(matches!(
            old_replay.play(setup.content),
            Err(crate::replay::ReplayError::ContentMismatch { .. })
        ));
    }

    #[test]
    fn directors_cut_is_a1_only_and_is_recorded_in_replays() {
        for profile in [
            Profile::Sta2Classic,
            Profile::Fo4,
            Profile::Fo3,
            Profile::Native,
        ] {
            assert_eq!(
                Sim::from_setup(&setup(profile, false)).digest(),
                Sim::from_setup(&setup(profile, true)).digest(),
            );
        }
        let setup = setup(Profile::Sta1Classic, true);
        let rec = Recorder::with_interval(&setup, 1);
        let digest = rec.sim().digest();
        let replay = Replay::decode(&rec.finish().encode()).unwrap();
        assert!(replay.options.directors_cut);
        assert_eq!(replay.play(setup.content).unwrap().digest(), digest);
    }
}
