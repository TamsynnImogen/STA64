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
    ghost_site_with_context(db, bounds, view, what, builder, cursor, &[], &[])
}

/// Includes typed map regions and live pending reservations, which are not
/// serialized into WorldView. Geometry is shared with authoritative admission.
#[allow(clippy::too_many_arguments)]
pub fn ghost_site_with_context(
    db: &ContentDb,
    bounds: Option<&(Vec3, Vec3)>,
    view: &WorldView,
    what: ContentKey,
    builder: EntityId,
    cursor: Vec3,
    regions: &[sta_content::Region],
    production: &[sta_sim::production::view::ProductionView],
) -> GhostSite {
    let facing = (Fixed::ZERO, Fixed::ONE);
    let pending: Vec<_> = production
        .iter()
        .filter(|row| row.builder != builder)
        .filter_map(|row| {
            row.site.as_ref().filter(|site| {
                site.entity.is_none()
                    && row.jobs.first().is_some_and(|job| !job.started)
                    && db.unit(site.def).is_some_and(|def| {
                        sta_sim::world::EntityKind::of_def(def)
                            == sta_sim::world::EntityKind::Station
                    })
            })
        })
        .collect();
    let orbit = if db.unit(what).is_some_and(orbit::is_orbital) {
        let (planets, mut orbiters) = orbit::from_view(db, view);
        orbiters.extend(
            pending
                .iter()
                .filter(|site| db.unit(site.def).is_some_and(orbit::is_orbital))
                .map(|site| site.pos),
        );
        Some(orbit::snap(&planets, &orbiters, cursor))
    } else {
        None
    };
    let (at, heading, parent) = match orbit {
        Some(Ok(slot)) => (slot.at, slot.heading, Some(slot.planet)),
        Some(Err(e)) => {
            return GhostSite {
                at: cursor,
                heading: facing,
                status: Err(SiteError::Blocked),
                orbit: Some(Err(e)),
            };
        }
        None => (cursor, facing, None),
    };
    let others = view
        .units
        .iter()
        .filter(|unit| unit.id != builder && Some(unit.id) != parent)
        .filter(|unit| {
            db.unit(unit.def).is_some_and(|def| {
                sta_sim::world::EntityKind::of_def(def) != sta_sim::world::EntityKind::Projectile
            })
        })
        .map(|unit| site::unit_bounds(db, unit.def, unit.pos));
    let reserved = pending
        .iter()
        .map(|site| site::unit_bounds(db, site.def, site.pos));
    let limits = if parent.is_some() { None } else { bounds };
    let status = site::check_bounds(
        limits,
        site::unit_bounds(db, what, at),
        others
            .chain(reserved)
            .chain(regions.iter().filter_map(site::region_bounds)),
    );
    GhostSite {
        at,
        heading,
        status,
        orbit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::{ContentDb, MoveStats, Region, RegionKind, RegionShape, UnitDef};
    use sta_sim::command::{Command, CommandKind, Edit, Order};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{PlayerId, Sim, SimEvent, TeamId, WorldSource};
    use std::sync::Arc;

    fn v(x: i32, z: i32) -> Vec3 {
        Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
    }

    fn world() -> (Sim, Arc<ContentDb>, EntityId) {
        world_with(&[1000])
    }

    fn world_with(planets: &[i32]) -> (Sim, Arc<ContentDb>, EntityId) {
        world_with_regions(planets, Vec::new())
    }

    fn world_with_regions(planets: &[i32], regions: Vec<Region>) -> (Sim, Arc<ContentDb>, EntityId) {
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
        let mut map = sta_content::MapDef::empty();
        map.regions = regions;
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 1,
            map: Arc::new(map),
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

    fn spawn_at(sim: &mut Sim, db: &ContentDb, odf: &str, at: Vec3) -> EntityId {
        let id = sim.units.next_id();
        sim.step(&[Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key(odf).unwrap(), player: PlayerId(0), pos: at,
            heading: (Fixed::ZERO, Fixed::ONE),
        }) }]);
        assert!(sim.unit(id).is_some());
        id
    }

    fn fund(sim: &mut Sim) {
        sim.step(&[Command { player: PlayerId::BENCH, seq: 1,
            kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(9999))) }]);
    }

    fn build(sim: &mut Sim, rig: EntityId, what: ContentKey, at: Vec3) -> bool {
        let out = sim.step(&[Command { player: PlayerId(0), seq: 2,
            kind: CommandKind::Order(Order::Build { builder: rig, what, at: Some(at) }) }]);
        !out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
    }

    /// Steps until `n` stations of `what` exist and returns where they stand at that tick. A site is
    /// built where the ghost showed, then (once finished) circles its planet (SIM-65): the promise is
    /// about the site, so it is read the tick the site appears, still under construction.
    fn sites(sim: &mut Sim, what: ContentKey, n: usize) -> Vec<(Vec3, (Fixed, Fixed))> {
        for _ in 0..3000 {
            let found: Vec<_> = sim.view().units.iter().filter(|u| u.def == what).map(|u| (u.pos, u.heading, u.progress)).collect();
            if found.len() >= n {
                assert!(found.iter().any(|f| f.2.is_some()), "read while the new site is under construction");
                return found.into_iter().map(|f| (f.0, f.1)).collect();
            }
            sim.step(&[]);
        }
        panic!("no site appeared");
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
        let built = sites(&mut sim, what, 1);
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
        let feed = HudFeed { view: sim.view(), player: PlayerId(0), selected: vec![rig], content: Some(db.clone()), ..Default::default() };
        let armed = Armed { builder: rig, what, odf: "fx_orbital".into() };
        assert_eq!(site_status(&feed, None, &armed, cursor), (g.at, Ok(())));
        let (site_at, order) = build_order(&feed, None, &armed, cursor).unwrap();
        assert_eq!(site_at, g.at);
        assert_eq!(order, Order::Build { builder: rig, what, at: Some(cursor) });
        let e = Edit::Cargo(PlayerId(0), 0, Fixed::from_int(9999));
        sim.step(&[Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(e) }]);
        sim.step(&[Command { player: PlayerId(0), seq: 2, kind: CommandKind::Order(order) }]);
        let built: Vec<_> = sites(&mut sim, what, 1).into_iter().map(|b| b.0).collect();
        assert_eq!(built, [g.at]);
        // Open space is refused by the same call.
        assert_eq!(build_order(&feed, None, &armed, v(3000, 3000)), Err(SiteError::Blocked));
    }

    /// A second station's ghost agrees with the sim while the first one circles its planet: the first
    /// planet stays taken wherever its station is on the ring, the other planet's slot is built where
    /// the ghost showed.
    #[test]
    fn the_ghost_agrees_with_the_sim_while_a_neighbour_circles() {
        use sta_sim::command::Order;
        let (mut sim, db, rig) = world_with(&[1000, 2000]);
        let what = db.key("fx_orbital").unwrap();
        let e = Edit::Cargo(PlayerId(0), 0, Fixed::from_int(9999));
        sim.step(&[Command { player: PlayerId::BENCH, seq: 1, kind: CommandKind::Edit(e) }]);
        let first = v(900, 90);
        let order = |at, seq| Command { player: PlayerId(0), seq, kind: CommandKind::Order(Order::Build { builder: rig, what, at: Some(at) }) };
        sim.step(&[order(first, 2)]);
        let start = sites(&mut sim, what, 1)[0].0;
        // Until it is finished and has circled a good way round.
        for _ in 0..2400 {
            sim.step(&[]);
        }
        let now = sim.view().units.iter().find(|u| u.def == what).map(|u| (u.pos, u.progress)).unwrap();
        assert!(now.1.is_none() && now.0 != start, "finished and circling: {now:?}");
        let view = sim.view();
        // Its planet is taken at any cursor, the neighbour's is free and snaps where the sim builds.
        assert_eq!(ghost_site(&db, None, &view, what, rig, first).status, Err(SiteError::Blocked));
        assert_eq!(ghost_site(&db, None, &view, what, rig, now.0).status, Err(SiteError::Blocked));
        let cursor = v(2100, 60);
        let g = ghost_site(&db, None, &view, what, rig, cursor);
        assert_eq!(g.status, Ok(()));
        sim.step(&[order(cursor, 3)]);
        let built = sites(&mut sim, what, 2);
        assert!(built.iter().any(|b| b.0 == g.at), "{built:?} holds the ghost's {:?}", g.at);
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

    #[test]
    fn client_ghost_and_click_match_sim_for_diagonal_overlap_and_exact_touch() {
        use crate::ui::hud::{Armed, ghost::build_order, model::HudFeed};
        let (mut sim, db, rig) = world();
        let what = db.key("fx_station").unwrap();
        assert_eq!(site::footprint(&db, what), Fixed::from_f64(37.5));
        assert_eq!(site::obstacle_radius(&db, db.key("fx_world").unwrap()), Fixed::from_int(200));
        spawn_at(&mut sim, &db, "fx_station", v(3000, 3000));
        fund(&mut sim);
        let feed = HudFeed { view: sim.view(), player: PlayerId(0), selected: vec![rig], content: Some(db.clone()), ..Default::default() };
        let armed = Armed { builder: rig, what, odf: "fx_station".into() };
        let diagonal = v(3074, 3074);
        assert_eq!(ghost_site(&db, None, &feed.view, what, rig, diagonal).status, Err(SiteError::Blocked));
        assert_eq!(build_order(&feed, None, &armed, diagonal), Err(SiteError::Blocked));
        // Two 37.5-unit squares overlap by one unit on each axis, while their
        // centre distance exceeds the old circles' combined 75-unit radius.
        assert!(!build(&mut sim, rig, what, diagonal));
        let touching = v(3075, 3000);
        assert_eq!(ghost_site(&db, None, &feed.view, what, rig, touching).status, Ok(()));
        assert_eq!(build_order(&feed, None, &armed, touching).unwrap().1,
            Order::Build { builder: rig, what, at: Some(touching) });
        assert!(build(&mut sim, rig, what, touching), "exact edge contact is admitted by the sim");
    }

    #[test]
    fn client_context_uses_live_pending_reservations_but_not_the_armed_rigs_own() {
        let (mut sim, db, first) = world();
        let second = spawn_at(&mut sim, &db, "fx_rig", v(500, 0));
        let what = db.key("fx_station").unwrap();
        fund(&mut sim);
        let pending_at = v(3000, 3000);
        assert!(build(&mut sim, first, what, pending_at));
        let production = sim.production_view();
        let reserved = production.iter().find(|row| row.builder == first).unwrap();
        assert_eq!(reserved.site.as_ref().unwrap().entity, None);
        assert!(!reserved.jobs[0].started);
        let view = sim.view();
        assert_eq!(ghost_site_with_context(&db, None, &view, what, first, pending_at, &[], &production).status, Ok(()),
            "the armed rig's own reservation is omitted");
        let encroaching = v(3074, 3074);
        assert_eq!(ghost_site_with_context(&db, None, &view, what, second, encroaching, &[], &production).status,
            Err(SiteError::Blocked));
        assert!(!build(&mut sim, second, what, encroaching));
        let touching = v(3075, 3000);
        assert_eq!(ghost_site_with_context(&db, None, &view, what, second, touching, &[], &production).status, Ok(()));
        assert!(build(&mut sim, second, what, touching));
    }

    #[test]
    fn client_context_and_sim_both_reject_typed_sphere_and_box_regions() {
        let regions = vec![
            Region { kind: RegionKind::Nebula, odf: "synthetic_nebula".into(),
                shape: RegionShape::Sphere { centre: v(4000, 4000), radius: Fixed::from_int(100) } },
            Region { kind: RegionKind::AsteroidField, odf: "synthetic_field".into(),
                shape: RegionShape::Box { min: v(4900, 4950), max: v(5100, 5050) } },
        ];
        let (mut sim, db, rig) = world_with_regions(&[1000], regions.clone());
        let what = db.key("fx_station").unwrap();
        fund(&mut sim);
        for at in [v(4100, 4000), v(5110, 5000)] {
            assert_eq!(ghost_site_with_context(&db, None, &sim.view(), what, rig, at, &regions, &[]).status,
                Err(SiteError::Blocked));
            assert!(!build(&mut sim, rig, what, at));
        }
    }
}
