//! Where the placement ghost stands and whether the site is legal (SIM-62).
//!
//! One pure entry for the building ghost and the click that confirms it: the
//! general build-site rule ([`sta_sim::production::site::check_site`]) for an
//! ordinary structure, and for an orbital station (class `p_mining`) the orbit
//! slot of the planet under the cursor ([`sta_sim::production::orbit`]), which
//! is also the position the sim gives the station. The sim applies the same
//! functions to the order, so the ghost and the sim cannot disagree.

use sta_sim::production::orbit::{self, OrbitError};
use sta_sim::production::site::{self, SiteError};
use sta_sim::{ContentKey, EntityId, Fixed, Vec3, WorldView};
use sta_content::ContentDb;

/// What the ghost shows for a cursor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GhostSite {
    /// Where the ghost stands: the cursor point, or the orbit slot it snaps to.
    pub at: Vec3,
    /// Unit vector in XZ, `(x, z)`: the way the building faces (+z when the
    /// rule gives it no direction; an orbital station faces away from its planet).
    pub heading: (Fixed, Fixed),
    /// `Ok` when the site is legal. A refused orbit reads as
    /// [`SiteError::Blocked`] so existing red tint and message apply; `orbit`
    /// has the reason.
    pub status: Result<(), SiteError>,
    /// For an orbital station: its slot or why there is none; `None` otherwise.
    pub orbit: Option<Result<orbit::Slot, OrbitError>>,
}

/// The ghost's pose and status for `what` with the cursor on the ground at
/// `cursor`, `builder` left out of the obstacles. `bounds` is the map's
/// playable volume (`None`: unbounded).
pub fn ghost_site(
    db: &ContentDb,
    bounds: Option<&(Vec3, Vec3)>,
    view: &WorldView,
    what: ContentKey,
    builder: EntityId,
    cursor: Vec3,
) -> GhostSite {
    let facing = (Fixed::ZERO, Fixed::ONE);
    match orbit::snap_view(db, view, what, cursor) {
        Some(Ok(slot)) => GhostSite { at: slot.at, heading: slot.heading, status: Ok(()), orbit: Some(Ok(slot)) },
        Some(Err(e)) => GhostSite { at: cursor, heading: facing, status: Err(SiteError::Blocked), orbit: Some(Err(e)) },
        None => GhostSite {
            at: cursor,
            heading: facing,
            status: site::check_site(db, bounds, what, cursor, site::view_obstacles(db, view, Some(builder))),
            orbit: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::{ContentDb, MoveStats, UnitDef};
    use sta_sim::command::{Command, CommandKind, Edit};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{PlayerId, Sim, TeamId, WorldSource};
    use std::sync::Arc;

    fn v(x: i32, z: i32) -> Vec3 {
        Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
    }

    fn world() -> (Sim, Arc<ContentDb>, EntityId) {
        world_with(&[1000])
    }

    fn world_with(planets: &[i32]) -> (Sim, Arc<ContentDb>, EntityId) {
        let fake = sta_content::fake::db();
        let mut rig = UnitDef::new("fx_rig", "constructionrig");
        rig.race = "fx".into();
        rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        let mut orbital = UnitDef::new("fx_orbital", "p_mining");
        orbital.orbit_time = Some(Fixed::from_int(120));
        let mut planet = UnitDef::new("fx_world", "planet");
        planet.radius = Fixed::from_int(200);
        planet.resource = Some(2);
        planet.can_have_orbital_station = true;
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone()).unit(rig).unit(orbital).unit(planet);
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            b = b.unit(u.clone());
        }
        let db = Arc::new(b.build().unwrap());
        let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 1,
            map: Arc::new(sta_content::MapDef::empty()),
            content: db.clone(),
            players: vec![seat(0), seat(1)],
            options: GameOptions::default(),
        });
        let mut spawn = |odf: &str, at| {
            let id = sim.units.next_id();
            let e = Edit::Spawn { odf: db.key(odf).unwrap(), player: PlayerId(0), pos: at, heading: (Fixed::ZERO, Fixed::ONE) };
            sim.step(&[Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(e) }]);
            id
        };
        for &x in planets {
            spawn("fx_world", v(x, 0));
        }
        let rig = spawn("fx_rig", v(0, 0));
        (sim, db, rig)
    }

    #[test]
    fn an_orbital_station_snaps_over_a_planet_and_is_refused_elsewhere() {
        let (sim, db, rig) = world();
        let (view, what) = (sim.view(), db.key("fx_orbital").unwrap());
        let g = ghost_site(&db, None, &view, what, rig, v(1100, 0));
        assert_eq!(g.status, Ok(()));
        assert_eq!(g.at, v(1275, 0), "the bearing of the cursor, radius 200 + 75 from the centre");
        assert_eq!(g.heading, (Fixed::ONE, Fixed::ZERO), "facing away from the planet");
        let open = ghost_site(&db, None, &view, what, rig, v(3000, 3000));
        assert_eq!((open.status, open.at, open.orbit), (Err(SiteError::Blocked), v(3000, 3000), Some(Err(OrbitError::NoPlanet))));
    }

    #[test]
    fn the_ghost_position_is_the_one_the_sim_builds_at() {
        use sta_sim::command::Order;
        let (mut sim, db, rig) = world();
        let what = db.key("fx_orbital").unwrap();
        let cursor = v(900, 90);
        let g = ghost_site(&db, None, &sim.view(), what, rig, cursor);
        let e = Edit::Cargo(PlayerId(0), 0, Fixed::from_int(9999));
        sim.step(&[Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(e) }]);
        let o = Order::Build { builder: rig, what, at: Some(cursor) };
        sim.step(&[Command { player: PlayerId(0), seq: 2, kind: CommandKind::Order(o) }]);
        for _ in 0..900 {
            sim.step(&[]);
        }
        let built: Vec<_> = sim.view().units.iter().filter(|u| u.def == what).map(|u| (u.pos, u.heading)).collect();
        // The position is exact; the heading is recomputed from the snapped point, so it may differ in the last bit.
        assert_eq!(built.len(), 1);
        assert_eq!(built[0].0, g.at);
        assert!((built[0].1.0 - g.heading.0).abs() < Fixed::from_f64(1e-6) && (built[0].1.1 - g.heading.1).abs() < Fixed::from_f64(1e-6));
        assert_eq!(ghost_site(&db, None, &sim.view(), what, rig, cursor).status, Err(SiteError::Blocked), "the planet is taken now");
    }

    /// The click's order carries the raw cursor point: with two planets in reach the sim builds where the
    /// ghost showed, not where a second snap of the snapped point lands (the click used to order the snapped one).
    #[test]
    fn the_click_order_builds_where_the_ghost_is_with_two_planets() {
        use crate::ui::hud::{Armed, ghost::{build_order, site_status}, model::HudFeed};
        use sta_sim::command::Order;
        let (mut sim, db, rig) = world_with(&[1000, 1520]);
        let what = db.key("fx_orbital").unwrap();
        let cursor = v(1230, 0);
        let g = ghost_site(&db, None, &sim.view(), what, rig, cursor);
        assert_eq!((g.status, g.at), (Ok(()), v(1275, 0)));
        assert_ne!(ghost_site(&db, None, &sim.view(), what, rig, g.at).at, g.at, "a second snap moves it: the scenario");
        let feed = HudFeed { view: sim.view(), player: PlayerId(0), selected: vec![rig], content: Some(db.clone()) };
        let armed = Armed { builder: rig, what, odf: "fx_orbital".into() };
        assert_eq!(site_status(&feed, None, &armed, cursor), (g.at, Ok(())));
        let (site_at, order) = build_order(&feed, None, &armed, cursor).unwrap();
        assert_eq!(site_at, g.at);
        assert_eq!(order, Order::Build { builder: rig, what, at: Some(cursor) });
        let e = Edit::Cargo(PlayerId(0), 0, Fixed::from_int(9999));
        sim.step(&[Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(e) }]);
        sim.step(&[Command { player: PlayerId(0), seq: 2, kind: CommandKind::Order(order) }]);
        for _ in 0..1500 {
            sim.step(&[]);
        }
        let built: Vec<_> = sim.view().units.iter().filter(|u| u.def == what).map(|u| u.pos).collect();
        assert_eq!(built, [g.at]);
        // Open space is refused by the same call.
        assert_eq!(build_order(&feed, None, &armed, v(3000, 3000)), Err(SiteError::Blocked));
    }

    #[test]
    fn an_ordinary_structure_keeps_the_site_rule() {
        let (sim, db, rig) = world();
        let what = db.key("fx_station").unwrap();
        let g = ghost_site(&db, None, &sim.view(), what, rig, v(3000, 3000));
        assert_eq!((g.status, g.at, g.orbit), (Ok(()), v(3000, 3000), None));
        // On the planet itself it is blocked, as before.
        assert_eq!(ghost_site(&db, None, &sim.view(), what, rig, v(1000, 10)).status, Err(SiteError::Blocked));
    }
}
