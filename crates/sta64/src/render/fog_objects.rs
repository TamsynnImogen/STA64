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
//!   update ([`FogGrid::explored_within`]). For an object that exists from the start and does not
//!   move this is the same as "the object has been in sight"; that equivalence is STA64's inference
//!   (grade: inferred), not the research's per-object record. It does not hold for an object that is
//!   created, destroyed or moved later (a station, a ship): those need a per-entity record next to this
//!   API (the station-copy and minimap steps add it; deferred, not dropped).
//! - Never (`Sight::Never`): neither.
//!
//! With the fog lifted (match setting, cheat) the grid [`FogState`] carries is already the lifted
//! copy: explored ground counts as in sight, and with the shroud lifted too every cell does. That
//! is the original's rule for fog off and shroud on, and STA64's own stated rule for the cheats.
//! Two limits of `Remembered`, both not established for the original and open: with the fog on and the
//! shroud lifted, `lifted` counts every cell as explored, so an object never seen reads `Remembered`,
//! not `Never`; and a script that clears the shroud resets the explored memory, which turns
//! remembered objects back into never-seen ones.
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
use sta_sim::PlayerId;

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
    /// Footprint radius: a sphere's own; for a box, the circle through its corners in XZ (interim,
    /// STA64's own, not established: it over-covers a long thin box).
    pub radius: f32,
    /// False for a shape this build does not know: such a region is never hidden.
    pub shape_known: bool,
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
                    let (centre, radius, shape_known) = match r.shape {
                        RegionShape::Sphere { centre, radius } => (Vec2::new(centre.x.to_f32(), centre.z.to_f32()), radius.to_f32(), true),
                        RegionShape::Box { min, max } => {
                            let (lo, hi) = (Vec2::new(min.x.to_f32(), min.z.to_f32()), Vec2::new(max.x.to_f32(), max.z.to_f32()));
                            ((lo + hi) * 0.5, (hi - lo).length() * 0.5, true)
                        }
                        // A shape this build does not know is never hidden by accident.
                        _ => (Vec2::ZERO, 0.0, false),
                    };
                    FogRegion { id, kind: r.kind, odf: r.odf.clone(), centre, radius, shape_known }
                })
                .collect(),
        )
    }

    /// The id of the region of `kind` at `(x, z)` (sim space), or, when the map declares none there, a new entry
    /// for an object of that kind at that place with `radius` (so an object with no region of the map is decided
    /// by its own footprint like the rest, and not shown early; interim: the research does not say how such an
    /// object is decided).
    pub fn region_or_add(&mut self, kind: RegionKind, x: f32, z: f32, radius: f32) -> usize {
        if let Some(r) = self.find(kind, x, z, 2.0) {
            return r.id;
        }
        let id = self.0.len();
        self.0.push(FogRegion { id, kind, odf: String::new(), centre: Vec2::new(x, z), radius, shape_known: true });
        id
    }

    /// The region of `kind` whose centre is within `tolerance` of `(x, z)` (sim space), for a
    /// renderer that knows an object only by where the map placed it.
    pub fn find(&self, kind: RegionKind, x: f32, z: f32, tolerance: f32) -> Option<&FogRegion> {
        self.0.iter().filter(|r| r.kind == kind).find(|r| r.centre.distance(Vec2::new(x, z)) <= tolerance)
    }
}

/// One [`Sight`] per entry of [`FogRegions`], kept current by [`update_region_sights`].
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct RegionSights {
    sights: Vec<Sight>,
    /// How many times the states were recomputed (for tests and marks).
    pub updates: u32,
}

impl RegionSights {
    /// The state of region `id`; a region the client does not know counts as in sight.
    pub fn get(&self, id: usize) -> Sight {
        self.sights.get(id).copied().unwrap_or_default()
    }
}

/// What [`update_region_sights`] last computed from.
#[derive(Default)]
pub struct SightMemo {
    cutscene: bool,
    had_fog: bool,
    pending: bool,
}

/// Whether a fog grid is on its way: the resources `update_fog` runs on exist (its run conditions: a sim link, the
/// map's bounds and the layers' material; and the images the art is loaded into) but it has not built the grid yet. Until it does, nothing counts as seen, so a never-seen
/// object is not drawn for the frames before the first grid (#1146, review of the hide step). Where nothing will
/// build a grid (a set-up without the fog material, a failed network link), this is false and everything is drawn.
#[derive(bevy::ecs::system::SystemParam)]
pub struct GridPending<'w> {
    bounds: Option<Res<'w, crate::render::fog::FogBounds>>,
    link: Option<Res<'w, crate::sim_link::SimLink>>,
    materials: Option<Res<'w, Assets<crate::render::fog::FogMaterial>>>,
    images: Option<Res<'w, Assets<Image>>>,
}

impl GridPending<'_> {
    pub fn expected(&self) -> bool {
        self.bounds.is_some() && self.link.is_some() && self.materials.is_some() && self.images.is_some()
    }
}

/// Marks a [`SightGate`] whose object the local player's side has seen before and does not have in sight now: it
/// is drawn as a still, dimmed copy (the renderers read this marker).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OutOfSight;

/// The wall clock at the frame a [`SightGate`] went out of sight: the sprite nodes under it stand at this time of
/// their animations ([`SpriteStill`](crate::render::sprites::SpriteStill)). It goes with [`OutOfSight`].
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct StillSince(pub f32);

/// The factor on the light of the hull and rock models under an entity that is out of sight (a tag on each mesh,
/// read by the hull and rock shaders); it goes with [`OutOfSight`].
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct GhostLight(pub f32);

/// The look of an object out of sight (rule: read from the executable, static; the amounts are per class below).
/// A copy does not animate. Its light is reduced: grades per class.
pub mod ghost {
    /// A planet's brightness out of sight, relative to in sight. Grade: measured at one window size (1280x720) by a
    /// first-party bench of the original, as the copy's brightness against the seen planet's. That is a ratio of
    /// framebuffer values; STA64 draws in linear light into an sRGB target, so the shader's factor is
    /// [`planet_linear`] (display value ratio 0.20; interim: the original's blend is not read).
    pub const PLANET: f32 = 0.20;

    /// The factor on a planet's linear output that gives [`PLANET`] in the displayed values.
    pub fn planet_linear() -> f32 {
        PLANET.powf(2.2)
    }

    /// A nebula's light out of sight. Rule and grade: half; inferred from the executable (static), not measured on a
    /// nebula; interim. It scales a puff's colour in displayed (sRGB) values, the quantity the original scales
    /// (the sprites' vertex colour). A gas's cover is a density, not a brightness: scaling it by this is STA64's own
    /// stand-in (interim, not measured).
    pub const NEBULA: f32 = 0.5;

    /// The factor on the linear light of a model out of sight that has no measured value (asteroid rocks): the old
    /// fog layer's own, one minus its opacity of 0.7. Grade: interim, reproduces the previous on-screen dimming
    /// (the layer's colour term is not reproduced); the research gives no value for these.
    pub const INTERIM_LINEAR: f32 = 0.3;
}

/// Recompute the regions' states from the fog grid, only when the grid, the regions, the local player
/// or the cutscene flag changed (the grid changes once per sim tick, not per frame). With no local
/// player or in a cutscene every region counts as in sight; with a local player and no grid yet
/// none has been seen. A region of an unknown shape is always in sight.
pub fn update_region_sights(
    regions: Res<FogRegions>,
    fog: Option<Res<FogState>>,
    local: Res<LocalPlayer>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    mut sights: ResMut<RegionSights>,
    pending: GridPending,
    mut memo: Local<SightMemo>,
) {
    let cutscene = cine.is_some_and(|c| c.on);
    let had_fog = fog.is_some();
    let pending = pending.expected();
    let changed = regions.is_changed() || local.is_changed() || fog.as_ref().is_some_and(|f| f.is_changed()) || memo.cutscene != cutscene || memo.had_fog != had_fog || memo.pending != pending;
    if !changed {
        return;
    }
    *memo = SightMemo { cutscene, had_fog, pending };
    let sights = &mut *sights;
    sights.updates += 1;
    sights.sights.resize(regions.0.len(), Sight::Visible);
    for (slot, r) in sights.sights.iter_mut().zip(&regions.0) {
        let next = if r.shape_known { sight_now(fog.as_deref(), local.0, cutscene, pending, r.centre.x, r.centre.y, r.radius) } else { Sight::Visible };
        if *slot != next {
            *slot = next;
        }
    }
}

/// The states of a map that is gone.
fn clear_region_sights(mut sights: ResMut<RegionSights>) {
    *sights = RegionSights::default();
}

/// The state of an object of `radius` at `(x, z)` (sim space) for the local player.
/// With no local player or in a cutscene every object counts as in sight, and so does every object
/// while there is no fog grid (the match is still starting, or nothing builds one): fail-safe, one
/// rule for the gates, the gas and `sync_units`. The one exception is `pending` (a grid is on its way, see
/// [`GridPending`]): until it exists nothing has been seen.
pub fn sight_now(fog: Option<&FogState>, local: Option<PlayerId>, cutscene: bool, pending: bool, x: f32, z: f32, radius: f32) -> Sight {
    if local.is_none() || cutscene {
        return Sight::Visible;
    }
    fog.map_or(if pending { Sight::Never } else { Sight::Visible }, |f| Sight::of(&f.grid, x, z, radius))
}

/// An intermediate entity between the map root and an object (nebula billboards, an asteroid field, a
/// planet): its own [`Visibility`] is [`Hidden`](Visibility::Hidden) while the object has never been
/// seen (#1146). The renderers keep their own writes to the object's visibility, which this does not
/// touch. `region` is the object's entry in [`FogRegions`], when it has one; else the footprint
/// `radius` at `pos` (sim space) decides.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct SightGate {
    pub pos: Vec2,
    pub radius: f32,
    pub region: Option<usize>,
    /// The factor on the linear light of the hull and rock models under the gate while it is out of sight (1: none).
    pub ghost: f32,
}

/// Show or hide each [`SightGate`], when the states, the grid, the local player or the cutscene flag changed
/// or a gate is new.
/// A gate whose object is out of sight also carries [`OutOfSight`].
#[allow(clippy::too_many_arguments)]
pub fn apply_gates(
    mut commands: Commands,
    fog: Option<Res<FogState>>,
    local: Res<LocalPlayer>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    sights: Res<RegionSights>,
    new: Query<(), Added<SightGate>>,
    mut gates: Query<(Entity, &SightGate, &mut Visibility, Has<OutOfSight>)>,
    pending: GridPending,
    clock: Option<Res<crate::clock::AnimClock>>,
    mut memo: Local<SightMemo>,
) {
    let cutscene = cine.is_some_and(|c| c.on);
    let had_fog = fog.is_some();
    let pending = pending.expected();
    let changed = sights.is_changed() || local.is_changed() || fog.as_ref().is_some_and(|f| f.is_changed()) || memo.cutscene != cutscene || memo.had_fog != had_fog || memo.pending != pending || !new.is_empty();
    if !changed {
        return;
    }
    *memo = SightMemo { cutscene, had_fog, pending };
    for (e, g, mut v, out) in &mut gates {
        let sight = match g.region {
            Some(id) if local.0.is_some() && !cutscene => sights.get(id),
            _ => sight_now(fog.as_deref(), local.0, cutscene, pending, g.pos.x, g.pos.y, g.radius),
        };
        let want = if sight == Sight::Never { Visibility::Hidden } else { Visibility::Inherited };
        if *v != want {
            *v = want;
        }
        match (sight == Sight::Remembered, out) {
            (true, false) => {
                commands.entity(e).insert((OutOfSight, GhostLight(g.ghost), StillSince(clock.as_deref().map_or(0.0, |c| c.wall() as f32))));
            }
            (false, true) => {
                commands.entity(e).remove::<(OutOfSight, GhostLight, StillSince)>();
            }
            _ => {}
        }
    }
}

/// The tag a hull or rock mesh carries while a model it belongs to is out of sight: the factor's bits, which the
/// hull and rock shaders multiply their light by (a tag of 0 means none). Set on the meshes under an entity that
/// gets [`GhostLight`] and taken off when it goes; sprite nodes keep their own tags.
#[allow(clippy::type_complexity)]
pub fn tag_ghost_meshes(
    mut commands: Commands,
    ghosts: Query<(Entity, &GhostLight, Ref<GhostLight>)>,
    mut removed: RemovedComponents<GhostLight>,
    children: Query<&Children>,
    parts: Query<Has<bevy::mesh::MeshTag>, (Or<(With<MeshMaterial3d<crate::render::hulls::HullMaterial>>, With<MeshMaterial3d<crate::render::rock::RockMaterial>>)>, Without<crate::render::sprites::SpriteFx>)>,
    mut frame: Local<u32>,
) {
    // A model's parts appear (and change material) after its entity gets the marker, so the walk repeats a few
    // times a second; a new marker walks at once. Few entities carry one.
    *frame = frame.wrapping_add(1);
    let sweep = frame.is_multiple_of(15);
    for (gate, g, fresh) in &ghosts {
        if !(sweep || fresh.is_added()) {
            continue;
        }
        for d in children.iter_descendants(gate) {
            if parts.get(d).is_ok_and(|tagged| !tagged) {
                commands.entity(d).try_insert(bevy::mesh::MeshTag::new(g.0.to_bits()));
            }
        }
    }
    for gate in removed.read() {
        for d in children.iter_descendants(gate) {
            if parts.contains(d) {
                commands.entity(d).try_remove::<bevy::mesh::MeshTag>();
            }
        }
    }
}

/// The sprite nodes under a gate that is out of sight stand still: each carries [`SpriteStill`] with the gate's
/// [`StillSince`] while it is, and loses it when the gate is in sight again. A model's parts appear after the
/// marker, so the walk repeats a few times a second; a new marker walks at once. Writes only to nodes that lack
/// the component, so nothing is touched in a frame where nothing changed.
pub fn still_sprites(
    mut commands: Commands,
    gates: Query<(Entity, &StillSince, Ref<StillSince>)>,
    mut removed: RemovedComponents<StillSince>,
    children: Query<&Children>,
    nodes: Query<Has<crate::render::sprites::SpriteStill>, With<crate::render::sprites::SpriteFx>>,
    stills: Query<(), With<crate::render::sprites::SpriteStill>>,
    mut frame: Local<u32>,
) {
    *frame = frame.wrapping_add(1);
    let sweep = frame.is_multiple_of(15);
    for (gate, since, fresh) in &gates {
        if !(sweep || fresh.is_added()) {
            continue;
        }
        for d in children.iter_descendants(gate) {
            if nodes.get(d).is_ok_and(|has| !has) {
                commands.entity(d).try_insert(crate::render::sprites::SpriteStill(since.0));
            }
        }
    }
    for gate in removed.read() {
        for d in children.iter_descendants(gate) {
            if stills.contains(d) {
                commands.entity(d).try_remove::<crate::render::sprites::SpriteStill>();
            }
        }
    }
}

pub struct FogObjectsPlugin;

impl Plugin for FogObjectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RegionSights>().add_systems(
            Update,
            (
                update_region_sights.in_set(GameSet::Render).after(update_fog).run_if(resource_exists::<FogRegions>).run_if(resource_exists::<LocalPlayer>),
                clear_region_sights.run_if(resource_removed::<FogRegions>),
                apply_gates.in_set(GameSet::Render).after(update_region_sights).run_if(resource_exists::<LocalPlayer>),
                tag_ghost_meshes.after(apply_gates),
                still_sprites.after(apply_gates),
            ),
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
        // A neighbouring cell does not count for an object at or under the footprint threshold.
        g.set_cell_for_test(cx + 1, cz, true, true);
        assert_eq!(Sight::of(&g, x, z, g.footprint_threshold()), Sight::Never);
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
    fn the_footprint_rule_starts_above_two_cells_of_the_originals_grid() {
        let mut g = grid();
        // The map's longer side is 8192: two cells of 128 along it are 128 units.
        assert_eq!(g.footprint_threshold(), 128.0);
        // An object at (100, 64) stands in the cell with centre (64, 64); the cell with centre
        // (192, 64) is 92 units from it.
        let (cx, cz) = (((100.0 - -4096.0) / CELL) as usize + 1, ((64.0 - -4096.0) / CELL) as usize);
        g.set_cell_for_test(cx, cz, true, true);
        assert_eq!(Sight::of(&g, 100.0, 64.0, 128.0), Sight::Never, "at the threshold only the centre cell counts");
        assert_eq!(Sight::of(&g, 100.0, 64.0, 129.0), Sight::Visible, "over it the footprint counts");
        assert_eq!(Sight::of(&g, 100.0, 64.0, 80.0), Sight::Never, "under it the centre cell decides");
        // A bigger map has a bigger threshold.
        assert_eq!(FogGrid::new([0.0, 0.0], [32768.0, 100.0]).footprint_threshold(), 512.0);
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
            .add_systems(Update, update_region_sights.run_if(resource_exists::<FogRegions>));
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
        assert_eq!(sights(&app), [Sight::Visible; 2], "no grid yet: everything is drawn");
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

    #[test]
    fn a_frame_without_a_change_does_not_recompute_and_a_new_grid_does() {
        let map = region_map();
        let mut app = app(&map);
        set_fog(&mut app, grid());
        let n = app.world().resource::<RegionSights>().updates;
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<RegionSights>().updates, n, "nothing changed: nothing recomputed");
        set_fog(&mut app, grid());
        assert_eq!(app.world().resource::<RegionSights>().updates, n + 1, "a new grid recomputes once");
        app.update();
        assert_eq!(app.world().resource::<RegionSights>().updates, n + 1);
    }

    #[test]
    fn the_states_are_reset_when_the_map_goes() {
        let map = region_map();
        let mut app = app(&map);
        app.add_systems(Update, clear_region_sights.run_if(resource_removed::<FogRegions>));
        set_fog(&mut app, grid());
        assert_eq!(sights(&app), [Sight::Never; 2]);
        app.world_mut().remove_resource::<FogRegions>();
        app.update();
        assert_eq!(sights(&app), [Sight::Visible; 2], "no state left: the default is in sight");
        assert_eq!(app.world().resource::<RegionSights>().updates, 0);
    }

    #[test]
    fn a_region_of_an_unknown_shape_is_never_hidden() {
        let map = region_map();
        let mut regions = FogRegions::of_map(&map);
        assert!(regions.0.iter().all(|r| r.shape_known));
        regions.0[1].shape_known = false;
        let mut app = app(&map);
        app.insert_resource(regions);
        set_fog(&mut app, grid());
        assert_eq!(sights(&app), [Sight::Never, Sight::Visible], "the unknown one is drawn, the known one never seen");
    }

    #[test]
    fn a_grid_on_its_way_means_nothing_seen_and_no_grid_expected_means_draw() {
        let some = Some(PlayerId(1));
        assert_eq!(sight_now(None, some, false, true, 0.0, 0.0, 10.0), Sight::Never, "pending: not drawn yet");
        assert_eq!(sight_now(None, some, false, false, 0.0, 0.0, 10.0), Sight::Visible, "set-up without fog: drawn");
        assert_eq!(sight_now(None, None, false, true, 0.0, 0.0, 10.0), Sight::Visible, "no local player: drawn");
        assert_eq!(sight_now(None, some, true, true, 0.0, 0.0, 10.0), Sight::Visible, "cutscene: drawn");
        let fog = FogState { grid: grid(), updates: 1 };
        assert_eq!(sight_now(Some(&fog), some, false, true, 0.0, 0.0, 10.0), Sight::Never, "a grid decides, pending or not");
    }

    /// The hull and rock meshes under an entity out of sight get the tag the shaders read; a sprite node keeps its
    /// own tag; the tag goes when the marker does.
    #[test]
    fn ghost_light_tags_the_hull_and_rock_meshes_under_it_and_only_those() {
        use bevy::mesh::MeshTag;
        use crate::render::{hulls::HullMaterial, rock::RockMaterial, sprites::SpriteFx};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, tag_ghost_meshes);
        let root = app.world_mut().spawn(Transform::IDENTITY).id();
        let node = app.world_mut().spawn((Transform::IDENTITY, ChildOf(root))).id();
        let hull = app.world_mut().spawn((MeshMaterial3d::<HullMaterial>(Handle::default()), ChildOf(node))).id();
        let rock = app.world_mut().spawn((MeshMaterial3d::<RockMaterial>(Handle::default()), ChildOf(root))).id();
        let sprite = app
            .world_mut()
            .spawn((MeshMaterial3d::<HullMaterial>(Handle::default()), SpriteFx { node_anim: None, sprite: String::new(), colour: [1.0; 3], material: Handle::default() }, MeshTag::new(7), ChildOf(root)))
            .id();
        let plain = app.world_mut().spawn((Transform::IDENTITY, ChildOf(root))).id();
        app.update();
        assert!(app.world().get::<MeshTag>(hull).is_none(), "nothing before the marker");
        app.world_mut().entity_mut(root).insert((OutOfSight, GhostLight(0.25)));
        app.update();
        let tag = |app: &App, e: Entity| app.world().get::<MeshTag>(e).map(|t| t.value);
        assert_eq!((tag(&app, hull), tag(&app, rock)), (Some(0.25f32.to_bits()), Some(0.25f32.to_bits())));
        assert_eq!(tag(&app, sprite), Some(7), "a sprite node keeps its own tag");
        assert_eq!(tag(&app, plain), None);
        app.world_mut().entity_mut(root).remove::<(OutOfSight, GhostLight)>();
        app.update();
        assert_eq!((tag(&app, hull), tag(&app, rock), tag(&app, sprite)), (None, None, Some(7)));
        // A part that appears later (a model loaded after the marker) is tagged by the next sweep.
        app.world_mut().entity_mut(root).insert((OutOfSight, GhostLight(0.25)));
        app.update();
        let late = app.world_mut().spawn((MeshMaterial3d::<RockMaterial>(Handle::default()), ChildOf(root))).id();
        for _ in 0..16 {
            app.update();
        }
        assert_eq!(tag(&app, late), Some(0.25f32.to_bits()));
    }

    /// F6: a grid is expected exactly where `update_fog` can run (its resources), and not otherwise.
    #[test]
    fn a_grid_is_expected_only_where_update_fog_can_build_one() {
        #[derive(Resource, Default)]
        struct Probe(Vec<bool>);
        fn probe(p: GridPending, mut out: ResMut<Probe>) {
            out.0.push(p.expected());
        }
        let mut app = App::new();
        app.init_resource::<Probe>().add_systems(Update, probe);
        let run = |app: &mut App| {
            app.update();
            *app.world().resource::<Probe>().0.last().unwrap()
        };
        assert!(!run(&mut app), "nothing");
        app.insert_resource(crate::render::fog::FogBounds { min: Vec2::ZERO, max: Vec2::ONE });
        app.init_resource::<Assets<crate::render::fog::FogMaterial>>();
        app.init_resource::<Assets<Image>>();
        assert!(!run(&mut app), "no sim link: update_fog does not run");
        let link = crate::sim_link::SimLink::new(Box::new(sta_sim::fake::ScriptedSource::from_json5("{ units: [], events: [] }", &sta_content::fake::db()).unwrap()), Box::new(sta_sim::command::LocalRouter::new()), PlayerId(1));
        app.insert_resource(link);
        assert!(run(&mut app), "everything update_fog needs exists");
        app.world_mut().remove_resource::<Assets<Image>>();
        assert!(!run(&mut app), "no images: the art is never loaded");
        app.init_resource::<Assets<Image>>();
        app.world_mut().remove_resource::<Assets<crate::render::fog::FogMaterial>>();
        assert!(!run(&mut app), "no layer material");
        app.init_resource::<Assets<crate::render::fog::FogMaterial>>();
        app.world_mut().remove_resource::<crate::render::fog::FogBounds>();
        assert!(!run(&mut app), "no map bounds");
    }

    fn gate_app(map: &MapDef) -> App {
        let mut app = app(map);
        app.add_systems(Update, apply_gates.after(update_region_sights));
        app
    }

    fn vis(app: &App, e: Entity) -> Visibility {
        *app.world().get::<Visibility>(e).unwrap()
    }

    #[test]
    fn a_gate_hides_its_object_while_never_seen_and_shows_it_once_any_footprint_cell_is_in_sight() {
        let map = region_map();
        let mut app = gate_app(&map);
        // A region gate (the nebula, id 0), a footprint gate (a planet of radius 300 at 3000,3000)
        // and a small object whose centre cell alone decides.
        let nebula = app.world_mut().spawn((SightGate { pos: Vec2::new(1000.0, -2000.0), radius: 450.0, region: Some(0), ghost: 1.0 }, Visibility::Inherited)).id();
        let planet = app.world_mut().spawn((SightGate { pos: Vec2::new(3000.0, 3000.0), radius: 300.0, region: None, ghost: 0.5 }, Visibility::Inherited)).id();
        // The asteroid field (region 1, a box centred at 0,300): a region gate too.
        let field = app.world_mut().spawn((SightGate { pos: Vec2::new(0.0, 300.0), radius: 360.0, region: Some(1), ghost: 0.3 }, Visibility::Inherited)).id();
        // No grid yet: every gate shows (fail-safe).
        app.update();
        assert_eq!((vis(&app, nebula), vis(&app, planet), vis(&app, field)), (Visibility::Inherited, Visibility::Inherited, Visibility::Inherited));
        set_fog(&mut app, grid());
        assert_eq!((vis(&app, nebula), vis(&app, planet), vis(&app, field)), (Visibility::Hidden, Visibility::Hidden, Visibility::Hidden));
        // A cell inside the field's footprint in sight: the field is drawn whole.
        set_fog(&mut app, grid_with(&[(0.0, 600.0, 10.0, true)]));
        assert_eq!(vis(&app, field), Visibility::Inherited);
        set_fog(&mut app, grid());
        // One cell at the edge of the planet's footprint in sight: drawn whole.
        set_fog(&mut app, grid_with(&[(3250.0, 3000.0, 10.0, true)]));
        assert_eq!((vis(&app, nebula), vis(&app, planet)), (Visibility::Hidden, Visibility::Inherited));
        // Out of sight again: still drawn (the looks of a remembered object are a later step).
        set_fog(&mut app, grid_with(&[(3250.0, 3000.0, 10.0, false), (1000.0, -1700.0, 20.0, false)]));
        assert_eq!((vis(&app, nebula), vis(&app, planet)), (Visibility::Inherited, Visibility::Inherited));
        let out = |app: &App, e: Entity| app.world().get::<OutOfSight>(e).is_some();
        assert!(out(&app, nebula) && out(&app, planet), "seen before and not in sight: the out-of-sight marker");
        assert!(!out(&app, field), "the field never seen has none");
        // Back in sight: the marker goes.
        set_fog(&mut app, grid_with(&[(3250.0, 3000.0, 10.0, true), (1000.0, -1700.0, 20.0, true)]));
        assert!(!out(&app, nebula) && !out(&app, planet));
        set_fog(&mut app, grid_with(&[(3250.0, 3000.0, 10.0, false), (1000.0, -1700.0, 20.0, false)]));
        // Never seen again (a script cleared the memory): hidden.
        set_fog(&mut app, grid());
        assert_eq!((vis(&app, nebula), vis(&app, planet)), (Visibility::Hidden, Visibility::Hidden));
        // With the fog and shroud lifted every object is drawn; in a cutscene too.
        set_fog(&mut app, grid().lifted(true, true));
        assert_eq!((vis(&app, nebula), vis(&app, planet)), (Visibility::Inherited, Visibility::Inherited));
        app.init_resource::<crate::ui::presentation::CineractiveState>();
        set_fog(&mut app, grid());
        app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = true;
        app.update();
        assert_eq!((vis(&app, nebula), vis(&app, planet)), (Visibility::Inherited, Visibility::Inherited));
    }

    /// #1146: the sprite nodes under a gate out of sight stand at the clock's time of the frame it went out of sight
    /// (also a node that appears later), and take the clock's time again when it is in sight.
    #[test]
    fn the_sprite_nodes_under_a_gate_out_of_sight_stand_at_the_time_it_went_out() {
        use crate::render::sprites::{SpriteFx, SpriteStill};
        let map = region_map();
        let mut app = gate_app(&map);
        app.init_resource::<crate::clock::AnimClock>().add_systems(Update, still_sprites.after(apply_gates));
        let node = |app: &mut App, parent: Entity| {
            app.world_mut().spawn((SpriteFx { node_anim: None, sprite: String::new(), colour: [1.0; 3], material: Handle::default() }, ChildOf(parent))).id()
        };
        let gate = app.world_mut().spawn((SightGate { pos: Vec2::new(3000.0, 3000.0), radius: 300.0, region: None, ghost: 0.5 }, Visibility::Inherited)).id();
        let pivot = app.world_mut().spawn(ChildOf(gate)).id();
        let first = node(&mut app, pivot);
        let tick = |app: &mut App, dt: f64| {
            app.world_mut().resource_mut::<crate::clock::AnimClock>().advance_idle(dt);
            app.update();
        };
        tick(&mut app, 2.0);
        set_fog(&mut app, grid_with(&[(3250.0, 3000.0, 10.0, true)]));
        assert!(app.world().get::<SpriteStill>(first).is_none(), "in sight: animating");
        tick(&mut app, 3.0);
        // Out of sight at wall time 5 (the clock was advanced by tick before the fog update).
        set_fog(&mut app, grid_with(&[(3250.0, 3000.0, 10.0, false)]));
        let went = app.world().get::<StillSince>(gate).expect("the gate stands still").0;
        assert_eq!(went, 5.0);
        assert_eq!(app.world().get::<SpriteStill>(first), Some(&SpriteStill(went)));
        // Time passes; a part adopted later is caught by the sweep and stands at the same time.
        let late = node(&mut app, pivot);
        for _ in 0..20 {
            tick(&mut app, 1.0);
        }
        assert_eq!(app.world().get::<StillSince>(gate).unwrap().0, went, "the time is not renewed while out of sight");
        assert_eq!(app.world().get::<SpriteStill>(late), Some(&SpriteStill(went)));
        assert_eq!(app.world().get::<SpriteStill>(first), Some(&SpriteStill(went)));
        // In sight again: all nodes animate.
        set_fog(&mut app, grid_with(&[(3250.0, 3000.0, 10.0, true)]));
        assert!(app.world().get::<SpriteStill>(first).is_none() && app.world().get::<SpriteStill>(late).is_none());
    }
}
