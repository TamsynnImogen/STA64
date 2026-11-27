//! #1144: what the local player's side has had in sight, per map object.
//!
//! Rule (Armada II; read from the executable, static; not measured; interim where it rests on
//! STA64's own grid): every object the client draws is in one of three states for the local
//! player's side.
//!
//! - [`Sight::Visible`]: the fog grid shows the cell at the object's centre in sight or, for an
//!   object whose radius is more than two cells, any cell within its radius
//!   ([`FogGrid::in_sight_within`]). A large object is therefore in sight as a whole as soon as one
//!   cell of its footprint is, also where the rest of it lies over unexplored ground.
//! - [`Sight::Remembered`]: not in sight now, but its footprint has been in sight at some earlier
//!   update ([`FogGrid::explored_within`]). For an object that does not move, this is the same as
//!   "the object has been in sight".
//! - Never (`Sight::Never`): neither.
//!
//! With the fog lifted (match setting, cheat) the grid [`FogState`] carries is already the lifted
//! copy: explored ground counts as in sight, and with the shroud lifted too every cell does. That
//! is the original's rule for fog off and shroud on, and STA64's own stated rule for the cheats.
//! In a cutscene (the cinematic view) every object counts as in sight, no layer being drawn.
//!
//! The regions a map declares (nebulae, asteroid fields) are not sim entities, so the client keeps
//! them here ([`FogRegions`], from `MapDef::regions` at map build) and derives one state per region
//! ([`RegionSights`], indexed like the map's region list). Regions a script creates later are not
//! covered (#1156). Nothing here draws anything: later steps read the states.

use bevy::prelude::*;
use sta_content::map::{RegionKind, RegionShape};
use sta_content::MapDef;

use crate::app::GameSet;
use crate::render::fog::{FogGrid, FogState, update_fog};
use crate::render::units::LocalPlayer;

/// What the local player's side has had in sight of one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sight {
    /// Never in sight.
    Never,
    /// In sight at some earlier update, not now.
    Remembered,
    /// In sight now.
    #[default]
    Visible,
}

impl Sight {
    /// The state of an object of `radius` at the sim-space point `(x, z)` on `grid`.
    pub fn of(grid: &FogGrid, x: f32, z: f32, radius: f32) -> Sight {
        if grid.in_sight_within(x, z, radius) {
            Sight::Visible
        } else if grid.explored_within(x, z, radius) {
            Sight::Remembered
        } else {
            Sight::Never
        }
    }
}

/// One region of the map, in the sim's XZ plane. `id` is its index in `MapDef::regions`.
#[derive(Debug, Clone, PartialEq)]
pub struct FogRegion {
    pub id: usize,
    pub kind: RegionKind,
    /// The ODF that defines the region.
    pub odf: String,
    pub centre: Vec2,
    /// Footprint radius: a sphere's own; for a box, the circle through its corners in XZ.
    pub radius: f32,
}

/// The map's nebulae and asteroid fields, as the client keeps them (set at map build, removed when
/// the match ends).
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct FogRegions(pub Vec<FogRegion>);

impl FogRegions {
    pub fn of_map(map: &MapDef) -> FogRegions {
        FogRegions(
            map.regions
                .iter()
                .enumerate()
                .map(|(id, r)| {
                    let (centre, radius) = match r.shape {
                        RegionShape::Sphere { centre, radius } => (Vec2::new(centre.x.to_f32(), centre.z.to_f32()), radius.to_f32()),
                        RegionShape::Box { min, max } => {
                            let (lo, hi) = (Vec2::new(min.x.to_f32(), min.z.to_f32()), Vec2::new(max.x.to_f32(), max.z.to_f32()));
                            ((lo + hi) * 0.5, (hi - lo).length() * 0.5)
                        }
                        // A shape this build does not know: no footprint, so the centre cell decides.
                        _ => (Vec2::ZERO, 0.0),
                    };
                    FogRegion { id, kind: r.kind, odf: r.odf.clone(), centre, radius }
                })
                .collect(),
        )
    }

    /// The region of `kind` whose centre is within `tolerance` of `(x, z)` (sim space), for a
    /// renderer that knows an object only by where the map placed it.
    #[allow(dead_code)] // read by the step that draws by state (#1146)
    pub fn find(&self, kind: RegionKind, x: f32, z: f32, tolerance: f32) -> Option<&FogRegion> {
        self.0.iter().filter(|r| r.kind == kind).find(|r| r.centre.distance(Vec2::new(x, z)) <= tolerance)
    }
}

/// One [`Sight`] per entry of [`FogRegions`], kept current by [`update_region_sights`].
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct RegionSights(Vec<Sight>);

impl RegionSights {
    /// The state of region `id`; a region the client does not know counts as in sight.
    #[allow(dead_code)] // read by the step that draws by state (#1146)
    pub fn get(&self, id: usize) -> Sight {
        self.0.get(id).copied().unwrap_or_default()
    }
}

/// Recompute the regions' states from the fog grid. With no grid (no local player, no match yet) or
/// in a cutscene every region counts as in sight.
pub fn update_region_sights(
    regions: Res<FogRegions>,
    fog: Option<Res<FogState>>,
    local: Res<LocalPlayer>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    mut sights: ResMut<RegionSights>,
) {
    let cutscene = cine.is_some_and(|c| c.on);
    let grid = fog.as_deref().map(|f| &f.grid).filter(|_| local.0.is_some() && !cutscene);
    let next: Vec<Sight> = regions
        .0
        .iter()
        .map(|r| grid.map_or(Sight::Visible, |g| Sight::of(g, r.centre.x, r.centre.y, r.radius)))
        .collect();
    if sights.0 != next {
        sights.0 = next;
    }
}

pub struct FogObjectsPlugin;

impl Plugin for FogObjectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RegionSights>().add_systems(
            Update,
            update_region_sights.in_set(GameSet::Render).after(update_fog).run_if(resource_exists::<FogRegions>).run_if(resource_exists::<LocalPlayer>),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::fog::CELL;
    use sta_content::{Region, RegionShape};
    use sta_sim::{Fixed, Vec3 as SimVec3};

    fn grid() -> FogGrid {
        FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0])
    }

    /// A grid where a disc of `radius` around `(x, z)` was in sight earlier (explored) and
    /// `now` of it still is.
    fn grid_with(sight: &[(f32, f32, f32, bool)]) -> FogGrid {
        let mut g = grid();
        for &(x, z, r, now) in sight {
            let (lo, hi) = (((x - r - g.min[0]) / g.cell) as usize, ((x + r - g.min[0]) / g.cell) as usize);
            let (zlo, zhi) = (((z - r - g.min[1]) / g.cell) as usize, ((z + r - g.min[1]) / g.cell) as usize);
            for cz in zlo..=zhi {
                for cx in lo..=hi {
                    g.set_cell_for_test(cx, cz, now, true);
                }
            }
        }
        g
    }

    #[test]
    fn a_small_object_is_decided_by_the_cell_at_its_centre() {
        let (x, z) = (1000.0, 1000.0);
        let (cx, cz) = (((x - -4096.0) / CELL) as usize, ((z - -4096.0) / CELL) as usize);
        let mut g = grid();
        assert_eq!(Sight::of(&g, x, z, 50.0), Sight::Never);
        g.set_cell_for_test(cx, cz, true, true);
        assert_eq!(Sight::of(&g, x, z, 50.0), Sight::Visible);
        g.set_cell_for_test(cx, cz, false, true);
        assert_eq!(Sight::of(&g, x, z, 50.0), Sight::Remembered, "seen before, not now");
        g.set_cell_for_test(cx, cz, false, false);
        assert_eq!(Sight::of(&g, x, z, 50.0), Sight::Never);
        // A neighbouring cell does not count for an object of two cells' radius or less.
        g.set_cell_for_test(cx + 1, cz, true, true);
        assert_eq!(Sight::of(&g, x, z, 2.0 * CELL), Sight::Never);
    }

    #[test]
    fn a_large_object_is_in_sight_once_any_cell_within_its_radius_is() {
        let (x, z, r) = (0.0, 0.0, 600.0);
        let mut g = grid();
        // A cell at the edge of the footprint, far from the centre.
        let (cx, cz) = (((x + 500.0 - -4096.0) / CELL) as usize, ((z - -4096.0) / CELL) as usize);
        g.set_cell_for_test(cx, cz, true, true);
        assert_eq!(Sight::of(&g, x, z, r), Sight::Visible, "one cell of the footprint is enough");
        assert!(!g.visible_at(x, z), "the centre itself is not in sight");
        // The same cell out of the footprint does not count.
        assert_eq!(Sight::of(&g, x, z, 300.0), Sight::Never);
        // Gone out of sight: remembered.
        g.set_cell_for_test(cx, cz, false, true);
        assert_eq!(Sight::of(&g, x, z, r), Sight::Remembered);
    }

    #[test]
    fn the_footprint_rule_starts_above_two_cells() {
        let mut g = grid();
        let (cx, cz) = (((0.0 - -4096.0) / CELL) as usize + 1, ((0.0 - -4096.0) / CELL) as usize);
        g.set_cell_for_test(cx, cz, true, true);
        // The cell's centre is 192 units from the origin: a radius over two cells (256) reaches it,
        // one of exactly two cells does not use the footprint at all.
        assert_eq!(Sight::of(&g, 0.0, 0.0, 2.0 * CELL), Sight::Never);
        assert_eq!(Sight::of(&g, 0.0, 0.0, 2.0 * CELL + 1.0), Sight::Visible);
        assert_eq!(Sight::of(&g, 0.0, 0.0, 150.0), Sight::Never, "the cell's centre lies outside the radius");
    }

    #[test]
    fn a_footprint_over_the_edge_of_the_map_uses_the_cells_it_has() {
        let g = grid_with(&[(-4000.0, 0.0, 300.0, true)]);
        assert_eq!(Sight::of(&g, -4200.0, 0.0, 600.0), Sight::Visible, "centre off the grid, footprint on it");
        assert_eq!(Sight::of(&g, -9000.0, 0.0, 600.0), Sight::Never);
    }

    #[test]
    fn a_nebula_half_on_unexplored_ground_is_in_sight_as_a_whole() {
        // The east half of the nebula was explored and is in sight; the west half is not.
        let g = grid_with(&[(500.0, 0.0, 300.0, true)]);
        assert_eq!(Sight::of(&g, 0.0, 0.0, 600.0), Sight::Visible);
        assert!(!g.explored_at(-500.0, 0.0), "its far part lies over unexplored ground");
    }

    fn region_map() -> MapDef {
        let v = |x: i32, z: i32| SimVec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) };
        let mut map = MapDef::empty();
        map.regions.push(Region { kind: RegionKind::Nebula, odf: "n".into(), shape: RegionShape::Sphere { centre: v(1000, -2000), radius: Fixed::from_int(450) } });
        map.regions.push(Region { kind: RegionKind::AsteroidField, odf: "f".into(), shape: RegionShape::Box { min: v(-300, 100), max: v(300, 500) } });
        map
    }

    #[test]
    fn regions_are_taken_from_the_map_in_order() {
        let r = FogRegions::of_map(&region_map());
        assert_eq!(r.0.len(), 2);
        assert_eq!((r.0[0].id, r.0[0].kind, r.0[0].centre, r.0[0].radius), (0, RegionKind::Nebula, Vec2::new(1000.0, -2000.0), 450.0));
        assert_eq!((r.0[1].id, r.0[1].centre), (1, Vec2::new(0.0, 300.0)));
        assert!((r.0[1].radius - (600.0f32.hypot(400.0) * 0.5)).abs() < 1e-3);
        assert_eq!(r.find(RegionKind::Nebula, 1002.0, -1999.0, 5.0).map(|f| f.id), Some(0));
        assert_eq!(r.find(RegionKind::AsteroidField, 1000.0, -2000.0, 5.0), None, "kind and place both match");
    }

    fn app(map: &MapDef) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<RegionSights>()
            .insert_resource(FogRegions::of_map(map))
            .insert_resource(LocalPlayer(Some(sta_sim::PlayerId(1))))
            .add_systems(Update, update_region_sights);
        app
    }

    fn set_fog(app: &mut App, g: FogGrid) {
        app.insert_resource(FogState { grid: g, updates: 1 });
        app.update();
    }

    fn sights(app: &App) -> [Sight; 2] {
        let s = app.world().resource::<RegionSights>();
        [s.get(0), s.get(1)]
    }

    #[test]
    fn region_states_follow_the_grid_both_ways() {
        let map = region_map();
        let mut app = app(&map);
        app.update();
        assert_eq!(sights(&app), [Sight::Visible; 2], "without a grid nothing is hidden");
        set_fog(&mut app, grid());
        assert_eq!(sights(&app), [Sight::Never; 2]);
        // A cell inside the nebula (centre 1000,-2000; radius 450) is in sight.
        set_fog(&mut app, grid_with(&[(1000.0, -1700.0, 20.0, true)]));
        assert_eq!(sights(&app), [Sight::Visible, Sight::Never]);
        set_fog(&mut app, grid_with(&[(1000.0, -1700.0, 20.0, false)]));
        assert_eq!(sights(&app), [Sight::Remembered, Sight::Never]);
        set_fog(&mut app, grid_with(&[(1000.0, -1700.0, 20.0, true)]));
        assert_eq!(sights(&app), [Sight::Visible, Sight::Never], "and back into sight");
        assert_eq!(app.world().resource::<RegionSights>().get(99), Sight::Visible, "an unknown region is not hidden");
    }

    #[test]
    fn lifted_fog_counts_explored_ground_as_in_sight() {
        let map = region_map();
        let mut app = app(&map);
        let g = grid_with(&[(1000.0, -1700.0, 20.0, false)]);
        set_fog(&mut app, g.lifted(true, false));
        assert_eq!(sights(&app), [Sight::Visible, Sight::Never], "fog off, shroud on: explored is in sight");
        set_fog(&mut app, g.lifted(true, true));
        assert_eq!(sights(&app), [Sight::Visible; 2], "both lifted: everything");
    }

    #[test]
    fn a_cutscene_and_no_local_player_hide_nothing() {
        let map = region_map();
        let mut app = app(&map);
        app.init_resource::<crate::ui::presentation::CineractiveState>();
        set_fog(&mut app, grid());
        assert_eq!(sights(&app), [Sight::Never; 2]);
        app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = true;
        app.update();
        assert_eq!(sights(&app), [Sight::Visible; 2]);
        app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = false;
        app.update();
        assert_eq!(sights(&app), [Sight::Never; 2]);
        app.insert_resource(LocalPlayer(None));
        app.update();
        assert_eq!(sights(&app), [Sight::Visible; 2]);
    }
}
