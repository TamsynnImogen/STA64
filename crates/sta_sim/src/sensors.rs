//! Sensors and per-player visibility (SIM-12), behind
//! `WorldView::visible_to` (S3).
//!
//! Visibility is a pure function of the entities, the content (for sensor
//! ranges) and the seats, recomputed at the end of every tick. Nothing here is
//! stored in `Sim`, so it is not in the digest and snapshots need no change.
//!
//! Rules:
//!
//! - A player sees every entity of its own team (allies share vision).
//! - A player also sees every other entity that is within the sensor range of
//!   any non-projectile entity of its team, measured in the XZ plane. Exactly
//!   at the range is not seen (the grid's overlap test is strict).
//! - Cloaked entities are seen only by their own team. There is no detection
//!   model yet, so a sensor never reveals a cloaked entity.
//! - Neutral entities (owner not seated) see nothing and are seen like any
//!   other non-allied entity.
//!
//! Sensor range is `UnitDef::sensor` (zero means blind). Entities not made
//! from content (the legacy `Sim::spawn`), or whose definition is gone, use
//! [`DEFAULT_SENSOR`].

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::spatial::{Body, Grid};
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, EntityKind};

/// Sensor range, in world units, of entities with no definition.
pub const DEFAULT_SENSOR: i32 = 800;

/// Grid cells are 2^9 = 512 units wide, about half a typical sensor range.
const CELL_LOG2: u32 = 9;

/// Sensor range of `e`.
pub fn range(e: &Entity, content: Option<&ContentDb>) -> Fixed {
    if e.kind == EntityKind::Projectile {
        return Fixed::ZERO;
    }
    match content.and_then(|c| c.unit(e.def)) {
        Some(def) => def.sensor,
        None => Fixed::from_int(DEFAULT_SENSOR),
    }
}

/// What each seat sees: `(player, entity ids ascending)`, ascending player.
pub fn compute(
    units: &Entities,
    content: Option<&ContentDb>,
    players: &[(PlayerId, TeamId)],
) -> Vec<(PlayerId, Vec<EntityId>)> {
    let team_of = |p: PlayerId| players.iter().find(|s| s.0 == p).map(|s| s.1);
    let bodies: Vec<Body> = units
        .iter()
        .map(|e| Body {
            id: e.id,
            pos: e.pos,
            radius: Fixed::ZERO,
            movable: false,
        })
        .collect();
    let grid = Grid::build(&bodies, CELL_LOG2);

    let mut teams: Vec<TeamId> = players.iter().map(|s| s.1).collect();
    teams.sort();
    teams.dedup();
    let seen_by: Vec<(TeamId, Vec<EntityId>)> = teams
        .into_iter()
        .map(|team| {
            let mut ids: Vec<EntityId> = Vec::new();
            for e in units.iter().filter(|e| team_of(e.owner) == Some(team)) {
                ids.push(e.id);
                let r = range(e, content);
                if r > Fixed::ZERO {
                    ids.extend(
                        grid.query(
                            Vec3 {
                                x: e.pos.x,
                                y: e.pos.y,
                                z: e.pos.z,
                            },
                            r,
                        )
                        .into_iter()
                        .filter(|&id| {
                            units
                                .get(id)
                                .is_some_and(|t| !t.flags.contains(UnitFlags::CLOAKED))
                        }),
                    );
                }
            }
            ids.sort();
            ids.dedup();
            (team, ids)
        })
        .collect();

    players
        .iter()
        .map(|&(p, t)| {
            let ids = seen_by
                .iter()
                .find(|s| s.0 == t)
                .map(|s| s.1.clone())
                .unwrap_or_default();
            (p, ids)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldSource;
    use crate::command::{Command, CommandKind, Order};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use crate::{Sim, WorldView};
    use std::sync::Arc;

    fn at(x: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fixed::from_int(x),
            y: Fixed::ZERO,
            z: Fixed::from_int(z),
        }
    }

    fn seat(id: u8, team: u8) -> PlayerSetup {
        PlayerSetup {
            id: PlayerId(id),
            team: TeamId(team),
            race: "fx".into(),
            control: Control::Idle,
        }
    }

    /// Players 0 and 1 are allies (team 0), player 2 is the enemy (team 1).
    /// The map's own objects are far from the test area.
    fn sim() -> Sim {
        let setup = GameSetup {
            seed: 3,
            map: Arc::new(sta_content::fake::two_bases()),
            content: Arc::new(sta_content::fake::db()),
            players: vec![seat(0, 0), seat(1, 0), seat(2, 1)],
            options: GameOptions::default(),
        };
        Sim::from_setup(&setup)
    }

    fn sees(v: &WorldView, p: u8, id: EntityId) -> bool {
        v.visible_to(PlayerId(p)).iter().any(|u| u.id == id)
    }

    fn within(a: Vec3, b: Vec3, r: i32) -> bool {
        let (dx, dz) = (a.x - b.x, a.z - b.z);
        dx * dx + dz * dz < Fixed::from_int(r) * Fixed::from_int(r)
    }

    /// The visibility scenario: an enemy cruiser flies past a station, and
    /// the station's owner (and its ally) sees it exactly while it is inside
    /// the range.
    #[test]
    fn enemy_enters_and_leaves_sensor_range() {
        let mut sim = sim();
        let content = sim.content().unwrap().clone();
        let key = |n: &str| content.key(n).unwrap();
        // Far off the map's own objects.
        let home = at(90_000, 90_000);
        let station = sim
            .spawn_def(
                key("fx_station"),
                PlayerId(0),
                home,
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        let ally = sim
            .spawn_def(
                key("fx_miner"),
                PlayerId(1),
                at(90_000, 80_000),
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        let cruiser = sim
            .spawn_def(
                key("fx_cruiser"),
                PlayerId(2),
                at(88_000, 90_000),
                (Fixed::ONE, Fixed::ZERO),
            )
            .unwrap();
        assert_eq!(
            content.unit(key("fx_station")).unwrap().sensor,
            Fixed::from_int(1500)
        );
        assert_eq!(
            content.unit(key("fx_cruiser")).unwrap().sensor,
            Fixed::from_int(1000)
        );

        let go = Command {
            player: PlayerId(2),
            seq: 0,
            kind: CommandKind::Order(Order::Move {
                units: vec![cruiser],
                to: at(92_000, 90_000),
                queue: false,
            }),
        };
        let mut cmds = vec![go];
        let (mut entered, mut left) = (None, None);
        let mut digest = 0xcbf2_9ce4_8422_2325u64;
        for _ in 0..60 * 120 {
            let v = sim.step(&cmds).view;
            cmds.clear();
            let p = v.unit(cruiser).unwrap().pos;
            let inside = within(p, home, 1500);
            assert_eq!(sees(&v, 0, cruiser), inside, "tick {}", v.tick);
            assert_eq!(
                sees(&v, 1, cruiser),
                inside,
                "ally shares vision, tick {}",
                v.tick
            );
            // The cruiser's own 1000 reaches the station only when closer.
            assert_eq!(sees(&v, 2, station), within(p, home, 1000));
            assert!(sees(&v, 2, cruiser));
            // Allies always see each other; the enemy never sees the far miner.
            assert!(sees(&v, 0, ally) && sees(&v, 1, station));
            assert!(!sees(&v, 2, ally));
            if inside && entered.is_none() {
                entered = Some(v.tick);
            }
            if !inside && entered.is_some() && left.is_none() {
                left = Some(v.tick);
            }
            for id in &v.visibility.iter().find(|x| x.0 == PlayerId(0)).unwrap().1 {
                digest = (digest ^ id.0 as u64 ^ (v.tick << 32)).wrapping_mul(0x100_0000_01b3);
            }
        }
        println!("entered {entered:?} left {left:?} digest {digest:#x}");
        assert_eq!((entered, left), EXPECTED_TICKS);
        assert_eq!(digest, EXPECTED_DIGEST);
    }

    #[test]
    fn cloaked_is_hidden_from_enemies_only() {
        let mut sim = sim();
        let content = sim.content().unwrap().clone();
        let key = |n: &str| content.key(n).unwrap();
        let a = sim
            .spawn_def(
                key("fx_cruiser"),
                PlayerId(0),
                at(90_000, 90_000),
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        let c = sim
            .spawn_def(
                key("fx_cruiser"),
                PlayerId(2),
                at(90_100, 90_000),
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        assert!(sees(&sim.view(), 0, c));
        let e = sim.units.get_mut(c).unwrap();
        e.flags = e.flags | UnitFlags::CLOAKED;
        let v = sim.view();
        assert!(!sees(&v, 0, c) && !sees(&v, 1, c));
        assert!(sees(&v, 2, c) && sees(&v, 2, a));
    }

    #[test]
    fn views_without_visibility_data_are_omniscient() {
        let v = WorldView {
            units: sim().view().units.clone(),
            ..Default::default()
        };
        assert_eq!(v.visible_to(PlayerId(9)).len(), v.units.len());
    }

    #[test]
    fn sensors_are_deterministic() {
        assert_eq!(sim().view().visibility, sim().view().visibility);
    }

    const EXPECTED_TICKS: (Option<u64>, Option<u64>) = (Some(122), Some(765));
    const EXPECTED_DIGEST: u64 = 0xeb35_965c_18a5_09b9;
}
