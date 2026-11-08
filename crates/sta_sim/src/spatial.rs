//! Space queries and collision (SIM-03): a uniform grid over the XZ plane,
//! unit separation, and push-out from planets and solid regions.
//!
//! Everything here is a pure function of its inputs and [`Fixed`] arithmetic:
//!
//! - [`Grid`] is a sorted `Vec` of `(cell, body index)`, no hash maps. Queries
//!   return ids in ascending order whatever the cell size.
//! - [`separate`] works from a snapshot, so the result does not depend on the
//!   order bodies are visited; displacements come back ascending by id.
//! - [`Solid`]s are spheres and boxes (map planets, region volumes). Bodies
//!   are pushed out along the shortest way.
//! - [`resolve`] applies both to [`Entities`] for a few passes. Immovable
//!   bodies (stations, moons) never move; they only push others.
//!
//! Collision is in the XZ plane: height is ignored, as ships fly level in
//! this game. Radii are not in the entity model yet, so callers supply them
//! (a [`Table`] of radii, set from the definition at spawn).

use sta_core::{EntityId, Fixed, Vec3};

use crate::world::{Entities, Entity, Table};

/// One thing that takes up room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub id: EntityId,
    pub pos: Vec3,
    pub radius: Fixed,
    /// False for bodies that only push others (stations, moons).
    pub movable: bool,
}

/// A fixed obstacle bodies cannot enter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Solid {
    Sphere { centre: Vec3, radius: Fixed },
    Box { min: Vec3, max: Vec3 },
}

const NO_SHIFT: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ZERO);

/// Cells are `2^cell_log2` world units wide on both axes.
fn cell_of(v: Fixed, cell_bits: u32) -> i64 {
    v.to_bits() >> cell_bits
}

fn overlaps(a: &Body, b: &Body) -> bool {
    let (dx, dz) = (b.pos.x - a.pos.x, b.pos.z - a.pos.z);
    let r = a.radius + b.radius;
    dx * dx + dz * dz < r * r
}

/// A uniform grid of bodies, built fresh whenever it is needed.
#[derive(Debug, Clone)]
pub struct Grid {
    cell_bits: u32,
    /// Ascending id.
    bodies: Vec<Body>,
    /// `(cell x, cell z, index into bodies)`, sorted.
    cells: Vec<(i64, i64, u32)>,
    max_radius: Fixed,
}

impl Grid {
    /// Cell width `2^cell_log2` world units (so `5` is 32 units). Pick about
    /// the size of the common unit's diameter or a little more.
    pub fn build(bodies: &[Body], cell_log2: u32) -> Grid {
        let cell_bits = 32 + cell_log2.min(24);
        let mut bodies = bodies.to_vec();
        bodies.sort_by_key(|b| b.id);
        let mut cells: Vec<(i64, i64, u32)> = bodies
            .iter()
            .enumerate()
            .map(|(i, b)| {
                (
                    cell_of(b.pos.x, cell_bits),
                    cell_of(b.pos.z, cell_bits),
                    i as u32,
                )
            })
            .collect();
        cells.sort_unstable();
        let max_radius = bodies.iter().map(|b| b.radius).max().unwrap_or(Fixed::ZERO);
        Grid {
            cell_bits,
            bodies,
            cells,
            max_radius,
        }
    }

    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    /// Indices (into the id-sorted bodies) of everything whose centre lies
    /// within `reach` of `pos` on both axes, ascending.
    fn near(&self, pos: Vec3, reach: Fixed) -> Vec<u32> {
        let bits = self.cell_bits;
        let (x0, x1) = (cell_of(pos.x - reach, bits), cell_of(pos.x + reach, bits));
        let (z0, z1) = (cell_of(pos.z - reach, bits), cell_of(pos.z + reach, bits));
        let mut out = Vec::new();
        for cx in x0..=x1 {
            // Cells are sorted by (x, z): one binary-searched run per column.
            let lo = self.cells.partition_point(|c| (c.0, c.1) < (cx, z0));
            for c in &self.cells[lo..] {
                if c.0 != cx || c.1 > z1 {
                    break;
                }
                out.push(c.2);
            }
        }
        out.sort_unstable();
        out
    }

    /// Ids of bodies overlapping the disc at `pos` with `radius`, ascending.
    pub fn query(&self, pos: Vec3, radius: Fixed) -> Vec<EntityId> {
        let probe = Body {
            id: EntityId::NONE,
            pos,
            radius,
            movable: false,
        };
        self.near(pos, radius + self.max_radius)
            .into_iter()
            .map(|i| &self.bodies[i as usize])
            .filter(|b| overlaps(&probe, b))
            .map(|b| b.id)
            .collect()
    }

    /// Every overlapping pair `(a, b)` with `a < b`, ascending. Touching
    /// (distance equal to the radii's sum) is not overlap.
    pub fn pairs(&self) -> Vec<(EntityId, EntityId)> {
        self.pair_indices()
            .into_iter()
            .map(|(i, j)| (self.bodies[i].id, self.bodies[j].id))
            .collect()
    }

    fn pair_indices(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (i, a) in self.bodies.iter().enumerate() {
            for j in self.near(a.pos, a.radius + self.max_radius) {
                let j = j as usize;
                if j > i && overlaps(a, &self.bodies[j]) {
                    out.push((i, j));
                }
            }
        }
        out
    }
}

/// How far a disc of `radius` at `p` must move to clear `solid`, in XZ; zero
/// when it does not touch.
pub fn push_from_solid(p: Vec3, radius: Fixed, solid: &Solid) -> (Fixed, Fixed) {
    match *solid {
        Solid::Sphere { centre, radius: sr } => {
            let (dx, dz) = (p.x - centre.x, p.z - centre.z);
            let r = sr + radius;
            let d2 = dx * dx + dz * dz;
            if d2 >= r * r {
                return NO_SHIFT;
            }
            let d = d2.sqrt();
            if d == Fixed::ZERO {
                // Dead centre: out along +X.
                return (r, Fixed::ZERO);
            }
            let gap = r - d;
            (dx / d * gap, dz / d * gap)
        }
        Solid::Box { min, max } => {
            let (lo_x, hi_x, lo_z, hi_z) = (
                min.x - radius,
                max.x + radius,
                min.z - radius,
                max.z + radius,
            );
            if p.x <= lo_x || p.x >= hi_x || p.z <= lo_z || p.z >= hi_z {
                return NO_SHIFT;
            }
            // Shortest exit along an axis; ties prefer X, then the low side.
            let opts = [
                (lo_x - p.x, Fixed::ZERO),
                (hi_x - p.x, Fixed::ZERO),
                (Fixed::ZERO, lo_z - p.z),
                (Fixed::ZERO, hi_z - p.z),
            ];
            let mut best = opts[0];
            for o in &opts[1..] {
                if o.0.abs() + o.1.abs() < best.0.abs() + best.1.abs() {
                    best = *o;
                }
            }
            best
        }
    }
}

/// Displacements that separate overlapping bodies, from the positions as
/// they are (a snapshot). Two movable bodies each move half the overlap;
/// against an immovable one the movable body moves all of it. Bodies on the
/// same spot split along X by id. Only non-zero displacements of movable
/// bodies are returned, ascending by id.
pub fn separate(grid: &Grid) -> Vec<(EntityId, (Fixed, Fixed))> {
    let mut acc = vec![NO_SHIFT; grid.bodies.len()];
    let half = Fixed::from_bits(Fixed::ONE.to_bits() / 2);
    for (ai, bi) in grid.pair_indices() {
        let (a, b) = (grid.bodies[ai], grid.bodies[bi]);
        let (wa, wb) = match (a.movable, b.movable) {
            (true, true) => (half, half),
            (true, false) => (Fixed::ONE, Fixed::ZERO),
            (false, true) => (Fixed::ZERO, Fixed::ONE),
            (false, false) => continue,
        };
        let (dx, dz) = (b.pos.x - a.pos.x, b.pos.z - a.pos.z);
        let r = a.radius + b.radius;
        let d = (dx * dx + dz * dz).sqrt();
        // Unit normal from a to b, and the overlap.
        let (nx, nz, gap) = if d == Fixed::ZERO {
            (Fixed::ONE, Fixed::ZERO, r)
        } else {
            (dx / d, dz / d, r - d)
        };
        acc[ai] = (acc[ai].0 - nx * gap * wa, acc[ai].1 - nz * gap * wa);
        acc[bi] = (acc[bi].0 + nx * gap * wb, acc[bi].1 + nz * gap * wb);
    }
    grid.bodies
        .iter()
        .zip(acc)
        .filter(|(b, d)| b.movable && *d != NO_SHIFT)
        .map(|(b, d)| (b.id, d))
        .collect()
}

/// Separation plus solid push-out over `entities`, up to `passes` times (each
/// pass moves bodies, then looks again; it stops early once nothing moves).
/// Entities without a radius in `radii` are ignored; `movable` says which
/// bodies yield. Positions change only in X and Z. Returns the ids that
/// moved, ascending.
pub fn resolve(
    entities: &mut Entities,
    radii: &Table<Fixed>,
    movable: impl Fn(&Entity) -> bool,
    solids: &[Solid],
    cell_log2: u32,
    passes: u32,
) -> Vec<EntityId> {
    let mut moved: Vec<EntityId> = Vec::new();
    for _ in 0..passes {
        let bodies: Vec<Body> = entities
            .iter()
            .filter_map(|e| {
                radii.get(e.id).map(|&radius| Body {
                    id: e.id,
                    pos: e.pos,
                    radius,
                    movable: movable(e),
                })
            })
            .collect();
        let grid = Grid::build(&bodies, cell_log2);
        let mut shifts = separate(&grid);
        for s in solids {
            for b in grid.bodies.iter().filter(|b| b.movable) {
                // Judge against the position after what this pass has moved.
                let at = shifts.binary_search_by_key(&b.id, |x| x.0);
                let (sx, sz) = at.map_or(NO_SHIFT, |i| shifts[i].1);
                let p = Vec3 {
                    x: b.pos.x + sx,
                    y: b.pos.y,
                    z: b.pos.z + sz,
                };
                let (px, pz) = push_from_solid(p, b.radius, s);
                if (px, pz) == NO_SHIFT {
                    continue;
                }
                match at {
                    Ok(i) => shifts[i].1 = (sx + px, sz + pz),
                    Err(i) => shifts.insert(i, (b.id, (px, pz))),
                }
            }
        }
        if shifts.is_empty() {
            break;
        }
        for (id, (dx, dz)) in shifts {
            if let Some(e) = entities.get_mut(id) {
                e.pos.x = e.pos.x + dx;
                e.pos.z = e.pos.z + dz;
                if let Err(i) = moved.binary_search(&id) {
                    moved.insert(i, id);
                }
            }
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rng;
    use crate::world::{EntityKind, Fnv};
    use sta_content::{ContentKey, MoveStats};
    use sta_core::PlayerId;

    fn f(v: i32) -> Fixed {
        Fixed::from_int(v)
    }
    fn at(x: i32, z: i32) -> Vec3 {
        Vec3 {
            x: f(x),
            y: Fixed::ZERO,
            z: f(z),
        }
    }
    /// A random integer in `-range..range`.
    fn rnd(rng: &mut Rng, range: i32) -> i32 {
        (rng.next_u64() % (2 * range as u64)) as i32 - range
    }
    fn scatter(seed: u64, n: u32, range: i32) -> Vec<Body> {
        let mut rng = Rng::new(seed);
        (1..=n)
            .map(|i| Body {
                id: EntityId(i),
                pos: at(rnd(&mut rng, range), rnd(&mut rng, range)),
                radius: f(1 + (rng.next_u64() % 4) as i32),
                movable: !rng.next_u64().is_multiple_of(5),
            })
            .collect()
    }
    /// Brute force pairs, the thing the grid must equal.
    fn brute(bodies: &[Body]) -> Vec<(EntityId, EntityId)> {
        let mut out = Vec::new();
        for a in bodies {
            for b in bodies.iter().filter(|b| b.id > a.id) {
                if overlaps(a, b) {
                    out.push((a.id, b.id));
                }
            }
        }
        out
    }

    #[test]
    fn grid_pairs_match_brute_force() {
        for seed in 1..40 {
            let bodies = scatter(seed, 120, 60);
            for cell in [1, 3, 6] {
                let mut want = brute(&bodies);
                want.sort();
                assert_eq!(
                    Grid::build(&bodies, cell).pairs(),
                    want,
                    "seed {seed} cell {cell}"
                );
            }
        }
    }

    #[test]
    fn grid_ignores_input_order() {
        for seed in 1..20 {
            let mut bodies = scatter(seed, 80, 40);
            let forward = Grid::build(&bodies, 3);
            bodies.reverse();
            let backward = Grid::build(&bodies, 3);
            assert_eq!(forward.pairs(), backward.pairs());
            assert_eq!(separate(&forward), separate(&backward));
        }
    }

    #[test]
    fn query_matches_brute_force_with_negative_coordinates() {
        let bodies = scatter(7, 150, 80);
        let grid = Grid::build(&bodies, 4);
        let mut rng = Rng::new(99);
        for _ in 0..200 {
            let probe = Body {
                id: EntityId::NONE,
                pos: at(rnd(&mut rng, 90), rnd(&mut rng, 90)),
                radius: f(1 + (rng.next_u64() % 12) as i32),
                movable: false,
            };
            let want: Vec<_> = bodies
                .iter()
                .filter(|b| overlaps(&probe, b))
                .map(|b| b.id)
                .collect();
            assert_eq!(grid.query(probe.pos, probe.radius), want);
        }
    }

    #[test]
    fn touching_is_not_overlap() {
        let bodies = [
            Body {
                id: EntityId(1),
                pos: at(0, 0),
                radius: f(2),
                movable: true,
            },
            Body {
                id: EntityId(2),
                pos: at(5, 0),
                radius: f(3),
                movable: true,
            },
        ];
        assert!(Grid::build(&bodies, 3).pairs().is_empty());
    }

    #[test]
    fn separation_of_a_pair_is_symmetric() {
        let bodies = [
            Body {
                id: EntityId(1),
                pos: at(0, 0),
                radius: f(3),
                movable: true,
            },
            Body {
                id: EntityId(2),
                pos: at(4, 0),
                radius: f(3),
                movable: true,
            },
        ];
        let s = separate(&Grid::build(&bodies, 3));
        assert_eq!(
            s,
            vec![
                (EntityId(1), (f(-1), Fixed::ZERO)),
                (EntityId(2), (f(1), Fixed::ZERO))
            ]
        );
    }

    #[test]
    fn coincident_bodies_split_by_id() {
        let bodies = [
            Body {
                id: EntityId(3),
                pos: at(5, 5),
                radius: f(1),
                movable: true,
            },
            Body {
                id: EntityId(9),
                pos: at(5, 5),
                radius: f(1),
                movable: true,
            },
        ];
        let s = separate(&Grid::build(&bodies, 3));
        assert!(s[0].1.0 < Fixed::ZERO && s[1].1.0 > Fixed::ZERO);
    }

    #[test]
    fn immovable_bodies_stay_and_movable_take_it_all() {
        let bodies = [
            Body {
                id: EntityId(1),
                pos: at(0, 0),
                radius: f(3),
                movable: false,
            },
            Body {
                id: EntityId(2),
                pos: at(4, 0),
                radius: f(3),
                movable: true,
            },
        ];
        let s = separate(&Grid::build(&bodies, 3));
        assert_eq!(s, vec![(EntityId(2), (f(2), Fixed::ZERO))]);
    }

    #[test]
    fn solids_push_out_to_the_surface() {
        let sphere = Solid::Sphere {
            centre: at(0, 0),
            radius: f(10),
        };
        assert_eq!(
            push_from_solid(at(5, 0), f(1), &sphere),
            (f(6), Fixed::ZERO)
        );
        assert_eq!(push_from_solid(at(20, 0), f(1), &sphere), NO_SHIFT);
        let b = Solid::Box {
            min: at(-10, -10),
            max: at(10, 10),
        };
        assert_eq!(push_from_solid(at(9, 0), f(1), &b), (f(2), Fixed::ZERO));
        assert_eq!(push_from_solid(at(0, -9), f(1), &b), (Fixed::ZERO, f(-2)));
        assert_eq!(push_from_solid(at(12, 0), f(1), &b), NO_SHIFT);
    }

    fn world(seed: u64, n: u32, range: i32) -> (Entities, Table<Fixed>) {
        let mut ents = Entities::new();
        let mut radii = Table::new();
        for b in scatter(seed, n, range) {
            let kind = if b.movable {
                EntityKind::Craft
            } else {
                EntityKind::Station
            };
            let heading = (Fixed::ZERO, Fixed::ONE);
            let id = ents.spawn(Entity::new(
                kind,
                ContentKey::NONE,
                PlayerId(0),
                b.pos,
                heading,
                MoveStats::from_f32(1.0, 1.0, 1.0, 1.0, 1.0),
            ));
            radii.insert(id, b.radius);
        }
        (ents, radii)
    }
    fn movable(e: &Entity) -> bool {
        e.kind == EntityKind::Craft
    }
    fn digest(e: &Entities) -> u64 {
        let mut h = Fnv::new();
        e.hash_into(&mut h);
        h.0
    }
    fn overlap_count(e: &Entities, radii: &Table<Fixed>) -> usize {
        let bodies: Vec<Body> = e
            .iter()
            .map(|u| Body {
                id: u.id,
                pos: u.pos,
                radius: *radii.get(u.id).unwrap(),
                movable: movable(u),
            })
            .collect();
        Grid::build(&bodies, 3).pairs().len()
    }

    #[test]
    fn resolve_clears_a_planet_and_never_moves_immovables() {
        for seed in 1..15 {
            let (mut ents, radii) = world(seed, 60, 50);
            let fixed_before: Vec<_> = ents.iter().filter(|e| !movable(e)).map(|e| e.pos).collect();
            let planet = Solid::Sphere {
                centre: at(10, 10),
                radius: f(12),
            };
            resolve(&mut ents, &radii, movable, &[planet], 3, 8);
            let fixed_after: Vec<_> = ents.iter().filter(|e| !movable(e)).map(|e| e.pos).collect();
            assert_eq!(fixed_before, fixed_after);
            for e in ents.iter().filter(|e| movable(e)) {
                // Neighbours can shove a unit back in between passes; allow a sliver.
                let (px, pz) = push_from_solid(e.pos, *radii.get(e.id).unwrap(), &planet);
                assert!(
                    px.abs() + pz.abs() <= Fixed::from_f64(0.5),
                    "seed {seed} unit {:?}",
                    e.id
                );
            }
        }
    }

    #[test]
    fn resolve_leaves_units_outside_a_box() {
        let (mut ents, radii) = world(4, 40, 30);
        let region = Solid::Box {
            min: at(-10, -10),
            max: at(10, 10),
        };
        resolve(&mut ents, &radii, movable, &[region], 3, 1);
        for e in ents.iter().filter(|e| movable(e)) {
            let ((x, z), r) = ((e.pos.x, e.pos.z), *radii.get(e.id).unwrap());
            let inside = x > f(-10) - r && x < f(10) + r && z > f(-10) - r && z < f(10) + r;
            // A pass may push a unit out one side into a neighbour, but never
            // leaves it strictly inside the box proper.
            assert!(!(x > f(-10) && x < f(10) && z > f(-10) && z < f(10)) || !inside);
        }
    }

    #[test]
    fn resolve_does_not_add_overlaps() {
        for seed in 1..15 {
            let (mut ents, radii) = world(seed, 80, 200);
            let before = overlap_count(&ents, &radii);
            resolve(&mut ents, &radii, movable, &[], 3, 16);
            assert!(overlap_count(&ents, &radii) <= before, "seed {seed}");
        }
    }

    fn run_500() -> Vec<u64> {
        let (mut ents, radii) = world(500, 500, 120);
        let planet = Solid::Sphere {
            centre: at(0, 0),
            radius: f(15),
        };
        (0..10)
            .map(|_| {
                resolve(&mut ents, &radii, movable, &[planet], 3, 4);
                digest(&ents)
            })
            .collect()
    }

    #[test]
    fn digest_is_stable_with_500_units() {
        let a = run_500();
        assert_eq!(a, run_500());
        assert_ne!(a[0], a[9], "separation should still be changing things");
        // Pinned: any change to arithmetic or order shows here.
        assert_eq!(a[9], PINNED_500, "got {:#x}", a[9]);
    }

    /// Digest of the 500-unit run after ten resolves.
    const PINNED_500: u64 = 0xddb1_45e8_3fa4_fccb;
}
