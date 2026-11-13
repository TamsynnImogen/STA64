//! Pathfinding round obstacles and hazards (SIM-23).
//!
//! # What blocks, what costs
//!
//! The planner is the games' own cost grid, read from their data:
//!
//! - **Solids** are [`avoid`](crate::avoid)'s (SIM-21): planets, stations,
//!   `avoidMe` props and asteroid fields, which include Fleet Operations'
//!   hidden `trigger_blocking` map barriers. A player's own stations are not
//!   solid to its craft. A cell is blocked when its centre is closer than the
//!   solid's radius plus the craft's body radius, SIM-21's own contact
//!   distance (the body rounded up to whole units, so craft of one size share
//!   a grid).
//! - **Nebulae** ([`RegionKind::Nebula`], from `nebula_obj` map objects) cost
//!   their ODF's `pathing_cost` per cell
//!   ([`NebulaPathing`](sta_content::NebulaPathing), hashed with the
//!   content), on the scale where open space is `DEFAULT_TERRAIN_VALUE`
//!   (100; stock Armada II: Mutara 400, Metreon and radioactive 2000,
//!   Cerulean 800, fluidic 10000, Metaphasic 2, which is *cheaper* than open
//!   space; Fleet Operations' Tetrion 90000). A nebula ODF without the key,
//!   or one the content does not know, costs `NEBULAE_PATHING_COST` (300 in
//!   Armada II and Fleet Operations, 198 in Armada). A cost of
//!   `IMPASSABLE_TERRAIN` (-1) blocks. `addToPathPlanner = 0` (the generated
//!   nebula of Armada II's nebula-generation special) leaves it out. The
//!   radius planned round is the region's times
//!   `NEBULAE_PATHING_SAFETY_COEFFICIENT` (1.8).
//! - **Map borders**: a unit that SIM-22 confines treats every cell outside
//!   its volume as blocked; any other unit pays `OFF_MAP_PATHING_COST`
//!   (90000) outside the map's bounds.
//!
//! Every rule constant above is a [`PathingRules`](sta_content::PathingRules)
//! field read from `RTS_CFG.h` (the stock values when absent);
//! `USE_PATH_PLANNING = 0` turns the planner off. No ODF of Armada, Armada II
//! or Fleet Operations has a per-ship key that ignores nebula costs, so no
//! ship is exempt. The grid's cell is `HIGHEST_QUADTREE_GRID_RESOLUTION`
//! (50), doubled only while a side would exceed [`MAX_SIDE`] cells.
//!
//! # How a unit follows a path
//!
//! Nothing is stored. Each tick, before SIM-21's steering, every moving craft
//! whose straight leg to its goal passes a solid within its contact distance
//! (other than one the target lies inside, whose edge SIM-21 flies to), or
//! touches a nebula itself dearer than open space and than the nebula the
//! goal lies in, is given an aim point: the farthest
//! cell along its path it can fly to in a straight line costing no more
//! than the path to it. The leg test is exact geometry, so a craft whose way
//! is clear never builds a grid. The path is the descent of a cost-to-go
//! field computed from the goal cell (Dijkstra over 8-connected cells,
//! integer costs, ties to the lower index), so it is a pure function of the
//! unit's position, the target and the world, and a snapshot restored
//! mid-route continues on the same route. The real target is put back after
//! kinematics, like SIM-21's detours, so orders, queues and arrival never
//! see the aim point; SIM-21 still steers round whatever lies on the leg to
//! it. A station built across the route changes the grid, and the next tick
//! plans round it.
//!
//! When the goal cannot be reached (walled in, or across a blocked map
//! border), the unit heads for the reachable cell nearest the goal and its
//! order ends once within its deploy reach of that cell's centre. A craft in
//! a pocket of fewer than [`POCKET`] cells (wedged between solids, its grid
//! cell blocked) is left to SIM-21 rather than stopped.
//!
//! Grids, fields and nearest-cell answers are cached by the full set of
//! inputs that build them ([`grid::Spec`] equality and exact keys), never by
//! a hash, so the cache cannot change a result; it is outside the digest and
//! snapshots.
//!
//! The tuning constants below ([`MAX_SIDE`], [`LOOK`], [`POCKET`], the
//! two-cell tolerance for a goal at a solid's edge, the cache sizes) are
//! STA64's implementation limits; no game file carries them.

mod grid;

use std::sync::Arc;

use sta_content::{PathingRules, RegionKind, RegionShape};
use sta_core::{EntityId, Fixed, Vec3};

use crate::Sim;
use crate::spatial::Solid;
use crate::world::EntityKind;
use grid::{Area, BLOCKED, Field, Grid, Hazard, Spec, ceil_units, floor_units};

/// The most cells a side of the grid has; the cell doubles from the
/// rules' resolution until the map fits.
const MAX_SIDE: i64 = 512;
/// Room round everything the grid must hold, units, and the alignment its
/// corners snap to (so a moving unit rarely changes its grid).
const PAD: i64 = 512;
const SNAP: i64 = 1024;
/// World coordinates the grid may span (keeps `cell << 32` in an `i64`).
const LIMIT: i64 = 1 << 30;
/// How many cells ahead along the path a unit looks for its aim point.
const LOOK: usize = 48;
/// A passable region smaller than this many cells is a pocket.
const POCKET: u32 = 16;
/// Cache sizes: grids, field cells in all (about 64 MB), nearest answers.
const GRIDS: usize = 24;
const FIELD_CELLS: usize = 8 << 20;
const NEAREST: usize = 256;
/// The highest cell cost (a modded `pathing_cost` of 1e9 stays ordered).
const MAX_COST: i64 = 1 << 30;

/// `(grid id, component, target x, target z, from cell)`.
type NearestKey = (u64, u32, i64, i64, u32);

/// Grids and fields from earlier ticks; a memo, outside the digest.
#[derive(Clone, Default)]
pub(crate) struct Cache {
    /// `(grid id, grid)`, most recent first.
    grids: Vec<(u64, Arc<Grid>)>,
    /// `(grid id, field)`, most recent first.
    fields: Vec<(u64, Arc<Field>)>,
    /// `(grid id, component, target x, target z, from cell)` -> cell.
    nearest: Vec<(NearestKey, Option<u32>)>,
    next: u64,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "nav::Cache({} grids, {} fields)",
            self.grids.len(),
            self.fields.len()
        )
    }
}

impl Cache {
    fn grid(&mut self, spec: Spec) -> (u64, Arc<Grid>) {
        if let Some(i) = self.grids.iter().position(|(_, g)| g.spec == spec) {
            let e = self.grids.remove(i);
            self.grids.insert(0, e.clone());
            return e;
        }
        let id = self.next;
        self.next += 1;
        let g = Arc::new(Grid::build(spec));
        self.grids.insert(0, (id, g.clone()));
        if self.grids.len() > GRIDS {
            let gone: Vec<u64> = self.grids.drain(GRIDS..).map(|(id, _)| id).collect();
            self.fields.retain(|(id, _)| !gone.contains(id));
            self.nearest.retain(|(k, _)| !gone.contains(&k.0));
        }
        (id, g)
    }

    fn field(&mut self, id: u64, grid: &Grid, goal: u32) -> Arc<Field> {
        if let Some(i) = self
            .fields
            .iter()
            .position(|(g, f)| *g == id && f.goal == goal)
        {
            let e = self.fields.remove(i);
            self.fields.insert(0, e.clone());
            return e.1;
        }
        let f = Arc::new(grid.field(goal));
        self.fields.insert(0, (id, f.clone()));
        // Keep the newest within the cell budget (the newest always stays).
        let mut cells = 0;
        let keep = self
            .fields
            .iter()
            .position(|(_, f)| {
                cells += f.len();
                cells > FIELD_CELLS
            })
            .unwrap_or(self.fields.len())
            .max(1);
        self.fields.truncate(keep);
        f
    }

    fn nearest(&mut self, id: u64, grid: &Grid, component: u32, t: Vec3, from: u32) -> Option<u32> {
        let key = (id, component, t.x.to_bits(), t.z.to_bits(), from);
        if let Some(&(_, k)) = self.nearest.iter().find(|(k, _)| *k == key) {
            return k;
        }
        let k = grid.nearest_in(component, t, from);
        self.nearest.insert(0, (key, k));
        self.nearest.truncate(NEAREST);
        k
    }
}

/// What a unit does this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Plan {
    /// Leave the target alone.
    Keep,
    /// Fly to this point instead, this tick.
    Aim(Vec3),
    /// The goal cannot be reached and the unit is as close as it gets: the
    /// order ends.
    Stop,
}

/// A body radius rounded up to whole units, so craft of one size share a
/// grid (never smaller than the body).
fn whole(body: Fixed) -> Fixed {
    Fixed::from_int(ceil_units(body).clamp(0, i32::MAX as i64) as i32)
}

fn sq(v: i128) -> i128 {
    v * v
}

/// Whether `a` and `b` are within `r` of each other (XZ), exactly.
fn within(a: Vec3, b: Vec3, r: Fixed) -> bool {
    let dx = a.x.to_bits() as i128 - b.x.to_bits() as i128;
    let dz = a.z.to_bits() as i128 - b.z.to_bits() as i128;
    sq(dx) + sq(dz) <= sq(r.to_bits() as i128)
}

/// Whether the segment `a`-`b` comes within `r` of `c` (XZ), in whole
/// units with `r` rounded up (conservative), exact in `i128`.
fn segment_near(a: Vec3, b: Vec3, c: Vec3, r: Fixed) -> bool {
    let u = |v: Fixed| floor_units(v) as i128;
    let (ax, az, bx, bz, cx, cz) = (u(a.x), u(a.z), u(b.x), u(b.z), u(c.x), u(c.z));
    let r = ceil_units(r) as i128 + 1;
    let (dx, dz, px, pz) = (bx - ax, bz - az, cx - ax, cz - az);
    let len2 = sq(dx) + sq(dz);
    let dot = px * dx + pz * dz;
    if len2 == 0 || dot <= 0 {
        sq(px) + sq(pz) <= sq(r)
    } else if dot >= len2 {
        sq(cx - bx) + sq(cz - bz) <= sq(r)
    } else {
        let cross = px * dz - pz * dx;
        sq(cross) <= sq(r) * len2
    }
}

/// A shape as a circle: a sphere's own, a box's bounding one.
fn circle(shape: &Solid) -> (Vec3, Fixed) {
    match *shape {
        Solid::Sphere { centre, radius } => (centre, radius),
        Solid::Box { min, max } => {
            let two = Fixed::from_int(2);
            let c = Vec3 {
                x: (min.x + max.x) / two,
                y: Fixed::ZERO,
                z: (min.z + max.z) / two,
            };
            let (hx, hz) = (
                ceil_units((max.x - min.x) / two) as i128,
                ceil_units((max.z - min.z) / two) as i128,
            );
            let r = ((sq(hx) + sq(hz)) as u128).isqrt() as i64 + 1;
            (c, Fixed::from_int(r.clamp(0, i32::MAX as i64) as i32))
        }
    }
}

/// Whether `p` lies in `shape` (XZ).
fn inside(p: Vec3, shape: &Solid) -> bool {
    match *shape {
        Solid::Sphere { centre, radius } => within(p, centre, radius),
        Solid::Box { min, max } => p.x >= min.x && p.x <= max.x && p.z >= min.z && p.z <= max.z,
    }
}

/// A cost from the data as a cell cost: impassable blocks, the rest at
/// least 1 (Metaphasic's 2 is cheaper than open space, never free).
fn cell_cost(cost: i64, rules: &PathingRules) -> u32 {
    if cost <= rules.impassable || cost < 0 {
        BLOCKED
    } else {
        cost.clamp(1, MAX_COST) as u32
    }
}

/// A nebula: as planned round, and its own extent (where "inside" begins).
struct Nebula {
    hazard: Hazard,
    core: Solid,
}

impl Sim {
    fn pathing_rules(&self) -> PathingRules {
        self.content
            .as_ref()
            .map(|c| c.rules().pathing())
            .unwrap_or_default()
    }

    /// The map's nebulae that cost anything other than open space.
    fn nebulae(&self, rules: &PathingRules) -> Vec<Nebula> {
        let default = cell_cost(rules.default_cost, rules);
        let db = self.content.as_deref();
        let mut out = Vec::new();
        for r in self.regions.iter().filter(|r| r.kind == RegionKind::Nebula) {
            let data = db
                .and_then(|db| db.key(&r.odf).and_then(|k| db.unit(k)))
                .and_then(|d| d.nebula_pathing);
            if data.is_some_and(|n| !n.planned) {
                continue;
            }
            let cost = cell_cost(
                data.and_then(|n| n.cost).unwrap_or(rules.nebula_cost),
                rules,
            );
            if cost == default {
                continue;
            }
            let (shape, core) = match r.shape {
                RegionShape::Sphere { centre, radius } => (
                    Solid::Sphere {
                        centre,
                        radius: radius * rules.nebula_safety,
                    },
                    Solid::Sphere { centre, radius },
                ),
                RegionShape::Box { min, max } => (Solid::Box { min, max }, Solid::Box { min, max }),
                _ => continue,
            };
            out.push(Nebula {
                hazard: Hazard { shape, cost },
                core,
            });
        }
        out
    }

    /// Give every craft whose way is blocked an aim point along its path, or
    /// end an order whose goal cannot be reached. Returns the real targets
    /// to put back after kinematics.
    pub(crate) fn route_orders(&mut self) -> Vec<(EntityId, Vec3)> {
        let rules = self.pathing_rules();
        if !rules.enabled {
            return Vec::new();
        }
        let nebulae = self.nebulae(&rules);
        let obstacles = self.obstacles();
        if nebulae.is_empty() && obstacles.is_empty() {
            return Vec::new();
        }
        let hazards: Vec<Hazard> = nebulae.iter().map(|n| n.hazard).collect();
        let default_cost = cell_cost(rules.default_cost, &rules).max(1);
        let off_map = cell_cost(rules.off_map_cost, &rules);
        let resolution = ceil_units(rules.resolution).max(1);
        let flows = crate::kinematics::Model::of(
            self.content
                .as_ref()
                .map(|c| c.profile())
                .unwrap_or_default(),
        )
        .flows_through_waypoints();
        let movers: Vec<EntityId> = self
            .units
            .iter()
            .filter(|u| u.kind == EntityKind::Craft && u.mobile() && u.target.is_some())
            .map(|u| u.id)
            .collect();
        let mut held = Vec::new();
        for id in movers {
            let Some(u) = self.units.get(id) else {
                continue;
            };
            let Some(t) = u.target else { continue };
            let p = u.pos;
            let body = whole(self.body_radius(u.def));
            let solids: Vec<Solid> = obstacles
                .iter()
                .filter(|o| o.friend != Some(u.owner))
                .map(|o| o.solid)
                .collect();
            // The goal as SIM-21 will fly it: a target inside a solid ends
            // at its edge (SIM-21 adds its own margin on top).
            let mut goal = t;
            for s in &solids {
                let (c, r) = circle(s);
                let r = r + body;
                // Far solids are skipped before squaring (Fixed overflows
                // past about 46000 units).
                if (goal.x - c.x).abs() > r || (goal.z - c.z).abs() > r {
                    continue;
                }
                let (dx, dz) = crate::spatial::push_from_solid(goal, body, s);
                goal.x = goal.x + dx;
                goal.z = goal.z + dz;
            }
            // An order into a nebula may cross nebulae no dearer than it.
            let into = nebulae
                .iter()
                .filter(|n| inside(goal, &n.core))
                .map(|n| {
                    if n.hazard.cost == BLOCKED {
                        u32::MAX
                    } else {
                        n.hazard.cost
                    }
                })
                .fold(default_cost, u32::max);
            // The leg needs a plan when it passes a solid within contact
            // distance (except the solid the target itself lies in, whose
            // edge SIM-21 flies to), or touches a nebula itself (not its
            // safety halo) dearer than open space and than the goal's own.
            let blocked = solids.iter().any(|s| {
                let (c, r) = circle(s);
                let own = matches!(s, Solid::Sphere { .. }) && within(t, c, r + body);
                !own && segment_near(p, goal, c, r + body)
            }) || nebulae.iter().any(|n| {
                (n.hazard.cost == BLOCKED || n.hazard.cost > into) && {
                    let (c, r) = circle(&n.core);
                    segment_near(p, goal, c, r + body)
                }
            });
            if !blocked {
                continue;
            }
            let reach = crate::production::deploy_reach(&u.stats);
            let queued = flows
                && self
                    .movement
                    .course(id)
                    .is_some_and(|c| !c.waypoints.is_empty());
            let area = if let Some(volume) = self.confinement.active_volume(u) {
                let (lo, hi) = volume.comparison_limits();
                Some(Area {
                    min: (lo.x, lo.z),
                    max: (hi.x, hi.z),
                    outside: BLOCKED,
                })
            } else {
                self.bounds.map(|(lo, hi)| Area {
                    min: (lo.x, lo.z),
                    max: (hi.x, hi.z),
                    outside: off_map,
                })
            };
            // An aim point within the smooth model's arrival radius
            // (`tooCloseToTurn`) of a craft at rest would count as reached:
            // the craft halts, the real target is put back, and it halts
            // again every tick. Aim points are kept beyond it.
            let arrive = if flows {
                u.stats.too_close_to_turn.max(crate::kinematics::arrive_distance())
            } else {
                Fixed::ZERO
            };
            let lead = if queued {
                u.stats.path_lead.max(arrive)
            } else {
                arrive
            };
            let mut spec = Spec {
                origin: (0, 0),
                cell: resolution,
                width: 1,
                height: 1,
                default_cost,
                body,
                solids,
                hazards: hazards.clone(),
                area,
            };
            frame(&mut spec, p, goal);
            let plan = plan(&mut self.nav, spec, p, goal, reach, lead);
            let Some(u) = self.units.get_mut(id) else {
                continue;
            };
            match plan {
                Plan::Keep => {}
                Plan::Aim(a) => {
                    u.target = Some(a);
                    held.push((id, t));
                }
                Plan::Stop => u.target = None,
            }
        }
        held
    }
}

/// Place the grid: everything it holds (the unit, its target, the area and
/// every obstacle), padded and snapped, in cells of the rules' resolution
/// doubled until no side has more than [`MAX_SIDE`].
fn frame(spec: &mut Spec, p: Vec3, t: Vec3) {
    let mut lo = (floor_units(p.x.min(t.x)), floor_units(p.z.min(t.z)));
    let mut hi = (ceil_units(p.x.max(t.x)), ceil_units(p.z.max(t.z)));
    let mut add = |x0: Fixed, z0: Fixed, x1: Fixed, z1: Fixed| {
        lo = (lo.0.min(floor_units(x0)), lo.1.min(floor_units(z0)));
        hi = (hi.0.max(ceil_units(x1)), hi.1.max(ceil_units(z1)));
    };
    if let Some(a) = spec.area {
        add(a.min.0, a.min.1, a.max.0, a.max.1);
    }
    for s in &spec.solids {
        let r = spec.reach(s);
        match *s {
            Solid::Sphere { centre, .. } => {
                add(centre.x - r, centre.z - r, centre.x + r, centre.z + r)
            }
            Solid::Box { min, max } => add(min.x - r, min.z - r, max.x + r, max.z + r),
        }
    }
    for h in &spec.hazards {
        match h.shape {
            Solid::Sphere { centre, radius } => add(
                centre.x - radius,
                centre.z - radius,
                centre.x + radius,
                centre.z + radius,
            ),
            Solid::Box { min, max } => add(min.x, min.z, max.x, max.z),
        }
    }
    let snap_lo = |v: i64| ((v - PAD).div_euclid(SNAP) * SNAP).clamp(-LIMIT, LIMIT);
    let snap_hi = |v: i64| ((v + PAD).div_euclid(SNAP) * SNAP + SNAP).clamp(-LIMIT, LIMIT);
    let (x0, z0, x1, z1) = (snap_lo(lo.0), snap_lo(lo.1), snap_hi(hi.0), snap_hi(hi.1));
    let mut cell = spec.cell;
    while (x1 - x0) / cell > MAX_SIDE || (z1 - z0) / cell > MAX_SIDE {
        cell *= 2;
    }
    spec.origin = (x0, z0);
    spec.cell = cell;
    spec.width = ((x1 - x0 + cell - 1) / cell).max(1) as u32;
    spec.height = ((z1 - z0 + cell - 1) / cell).max(1) as u32;
}

/// The plan for a unit at `p` with goal `t`, on the grid `spec` builds.
/// `lead`: how far ahead an aim point must be (the smooth model hands a
/// course over to its next waypoint within `pathLeadDistance` of a target).
fn plan(cache: &mut Cache, spec: Spec, p: Vec3, t: Vec3, reach: Fixed, lead: Fixed) -> Plan {
    let (id, grid) = cache.grid(spec);
    let Some(start) = grid.entry(p) else {
        return Plan::Keep;
    };
    let Some(component) = grid.component(start) else {
        return Plan::Keep;
    };
    // Wedged in a pocket between solids: SIM-21 gets it out, not an order end.
    if grid.size(component) < POCKET {
        return Plan::Keep;
    }
    let cell = Fixed::from_int(grid.spec.cell as i32);
    let g = grid.cell_of(t);
    let (goal, exact) = if grid.component(g) == Some(component) {
        (g, true)
    } else {
        let Some(n) = cache.nearest(id, &grid, component, t, start) else {
            return Plan::Keep;
        };
        // A goal just inside a solid's edge is reached by SIM-21's final
        // approach; only a goal two cells beyond is out of reach.
        (n, within(grid.centre(n, t.y), t, cell + cell))
    };
    if start == goal {
        if exact {
            return Plan::Keep;
        }
        let c = grid.centre(goal, t.y);
        return if within(p, c, reach) && grid.passable(grid.cell_of(p)) {
            Plan::Stop
        } else {
            Plan::Aim(c)
        };
    }
    let field = cache.field(id, &grid, goal);
    if !field.reaches(start) {
        return Plan::Keep;
    }
    // String pulling: the farthest cell along the path the unit can fly to
    // in a straight line costing no more than the path does to get there
    // (plus a cell of open space for the unit's offset in its own cell). A
    // path that must cross a nebula may cut across it; one that goes round
    // never cuts through it. Cells nearer than half a cell (or the lead)
    // are passed over, so kinematics never arrives on an aim point.
    let near = (cell / Fixed::from_int(2)).max(lead);
    let base = field.dist(start);
    let slack = grid::ORTHOGONAL * grid.spec.default_cost as u64;
    let mut k = start;
    let mut first = None;
    let mut beyond = None;
    let mut aim = None;
    for _ in 0..LOOK {
        let Some(n) = grid.descend(&field, k) else {
            break;
        };
        k = n;
        let c = grid.centre(k, t.y);
        first.get_or_insert(c);
        if !within(p, c, near) {
            beyond.get_or_insert(c);
        }
        let path = base.saturating_sub(field.dist(k));
        if grid
            .line_cost(p, c)
            .is_some_and(|line| line <= path.saturating_add(slack))
        {
            if !within(p, c, near) || k == goal {
                aim = Some(c);
            }
        } else if aim.is_some() {
            break;
        }
    }
    // No cell is both clear in a straight line and beyond the arrival
    // radius: fly to the first path cell that is beyond it (SIM-21 steers
    // round what grazes the leg), never to one a craft at rest would count
    // as reached.
    match aim.or(beyond).or(first) {
        Some(a) => Plan::Aim(a),
        None => Plan::Keep,
    }
}

#[cfg(test)]
mod tests;
