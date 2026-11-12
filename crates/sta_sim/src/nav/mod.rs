//! Pathfinding round obstacles and hazards (SIM-23).
//!
//! # What blocks, what costs
//!
//! The planner is the games' own cost grid, read from their data:
//!
//! - **Solids** are [`avoid`](crate::avoid)'s (SIM-21): planets, stations,
//!   `avoidMe` props and asteroid fields, which include Fleet Operations'
//!   hidden `trigger_blocking` map barriers. A player's own stations are not
//!   solid to its craft. A cell is blocked when its centre is within the
//!   solid's radius plus the craft's body radius (rounded up to half the
//!   planner's resolution so similar craft share a grid), times `RTS_CFG.h`'s
//!   `COLLISION_SAFETY_MARGIN` ("how much wider than reality to pretend
//!   objects are").
//! - **Nebulae** ([`RegionKind::Nebula`], from `nebula_obj` map objects) cost
//!   their ODF's `pathing_cost` per cell, on the scale where open space is
//!   `DEFAULT_TERRAIN_VALUE` (100; stock Armada II: Mutara 400, Metreon and
//!   radioactive 2000, Cerulean 800, fluidic 10000, Metaphasic 2, which is
//!   *cheaper* than open space). A nebula ODF without the key, or one the
//!   content does not know, costs `NEBULAE_PATHING_COST` (300). A cost of
//!   `IMPASSABLE_TERRAIN` (-1) blocks. `addToPathPlanner = 0` (the
//!   generated nebula of Armada II's nebula-generation special) leaves it
//!   out. The radius planned round is the region's times
//!   `NEBULAE_PATHING_SAFETY_COEFFICIENT` (1.8).
//! - **Map borders**: a unit that SIM-22 confines treats every cell outside
//!   its volume as blocked; any other unit pays `OFF_MAP_PATHING_COST`
//!   (90000) outside the map's bounds.
//!
//! Every constant above is a [`PathingRules`](sta_content::PathingRules)
//! field read from `RTS_CFG.h` (the stock values when absent);
//! `USE_PATH_PLANNING = 0` turns the planner off. No ODF of Armada, Armada II
//! or Fleet Operations has a per-ship key that ignores nebula costs, so no
//! ship is exempt.
//!
//! # How a unit follows a path
//!
//! Nothing is stored. Each tick, before SIM-21's steering, every moving craft
//! whose straight leg to its target crosses a blocked cell or one dearer than
//! both open space and the target's own cell (so an order into a nebula
//! flies straight in, but one out of or across it plans) is given an aim point
//! instead: the farthest cell along its path it can fly to in a straight
//! line that costs no more than the path to it. The
//! path is the descent of a cost-to-go field computed from the goal cell
//! (Dijkstra over 8-connected cells, integer costs, ties to the lower index),
//! so it is a pure function of the unit's position, the target and the
//! world, and a snapshot restored mid-route continues on the same route. The
//! real target is put back after kinematics, like SIM-21's detours, so
//! orders, queues and arrival never see the aim point; SIM-21 still steers
//! round whatever lies on the leg to it. A station built across the route
//! changes the grid, and the next tick plans round it.
//!
//! When the goal cannot be reached (walled in, or across a blocked map
//! border), the unit heads for the reachable cell nearest the goal and its
//! order ends once within its deploy reach of that cell's centre.
//!
//! Units whose leg is clear are not touched, so scenes without obstacles or
//! hazards on the way keep their digests. Grids and fields are cached by
//! the full set of inputs that build them ([`grid::Spec`] equality), never by
//! a hash, so the cache cannot change a result; it is outside the digest and
//! snapshots.

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
const MAX_SIDE: i64 = 256;
/// Room round everything the grid must hold, units, and the alignment its
/// corners snap to (so a moving unit rarely changes its grid).
const PAD: i64 = 512;
const SNAP: i64 = 1024;
/// World coordinates the grid may span (keeps `cell << 32` in an `i64`).
const LIMIT: i64 = 1 << 30;
/// How many cells ahead along the path a unit looks for its aim point.
const LOOK: usize = 48;
/// Cache sizes.
const GRIDS: usize = 8;
/// Fields kept per grid: as many as fit about two million cells, at least 16.
const FIELD_CELLS: usize = 2 << 20;

/// Grids and fields from earlier ticks; a memo, outside the digest.
#[derive(Clone, Default)]
pub(crate) struct Cache {
    entries: Vec<Entry>,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "nav::Cache({} grids)", self.entries.len())
    }
}

#[derive(Clone)]
struct Entry {
    grid: Arc<Grid>,
    fields: Vec<Arc<Field>>,
}

impl Cache {
    fn entry(&mut self, spec: Spec) -> &mut Entry {
        match self.entries.iter().position(|e| e.grid.spec == spec) {
            Some(i) => {
                let e = self.entries.remove(i);
                self.entries.insert(0, e);
            }
            None => {
                self.entries.insert(
                    0,
                    Entry {
                        grid: Arc::new(Grid::build(spec)),
                        fields: Vec::new(),
                    },
                );
                self.entries.truncate(GRIDS);
            }
        }
        &mut self.entries[0]
    }
}

impl Entry {
    fn field(&mut self, goal: u32) -> Arc<Field> {
        if let Some(i) = self.fields.iter().position(|f| f.goal == goal) {
            let f = self.fields.remove(i);
            self.fields.insert(0, f.clone());
            return f;
        }
        let f = Arc::new(self.grid.field(goal));
        self.fields.insert(0, f.clone());
        self.fields
            .truncate((FIELD_CELLS / self.grid.len().max(1)).max(16));
        f
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

/// A body radius rounded up to half the planner's resolution, so craft of
/// similar size share a grid (conservative: never smaller).
fn quantise(body: Fixed, resolution: i64) -> Fixed {
    let step = (resolution / 2).max(1);
    let units = ceil_units(body).max(0);
    Fixed::from_int((((units + step - 1) / step) * step).min(i32::MAX as i64) as i32)
}

/// Whether `a` and `b` are within `r` of each other (XZ), exactly.
fn within(a: Vec3, b: Vec3, r: Fixed) -> bool {
    let (dx, dz) = (
        (a.x.to_bits() - b.x.to_bits()) as i128,
        (a.z.to_bits() - b.z.to_bits()) as i128,
    );
    let r = r.to_bits() as i128;
    dx * dx + dz * dz <= r * r
}

/// A cost from the rules as a cell cost: impassable blocks, the rest at
/// least 1 (Metaphasic's 2 is cheaper than open space, never free).
fn cell_cost(cost: i64, rules: &PathingRules) -> u32 {
    if cost <= rules.impassable || cost < 0 {
        BLOCKED
    } else {
        cost.clamp(1, (u32::MAX / 64) as i64) as u32
    }
}

impl Sim {
    fn pathing_rules(&self) -> PathingRules {
        self.content
            .as_ref()
            .map(|c| c.rules().pathing())
            .unwrap_or_default()
    }

    /// The map's nebulae that cost anything other than open space.
    fn hazards(&self, rules: &PathingRules) -> Vec<Hazard> {
        let default = cell_cost(rules.default_cost, rules);
        let db = self.content.as_deref();
        let mut out = Vec::new();
        for r in self.regions.iter().filter(|r| r.kind == RegionKind::Nebula) {
            let raw = db
                .and_then(|db| db.key(&r.odf).and_then(|k| db.unit(k)))
                .map(|d| d.raw.clone());
            if raw.as_ref().and_then(|o| o.bool("addToPathPlanner")) == Some(false) {
                continue;
            }
            let cost = raw
                .as_ref()
                .and_then(|o| {
                    o.i64("pathing_cost").or_else(|| {
                        o.f32("pathing_cost")
                            .filter(|v| v.is_finite())
                            .map(|v| v.round() as i64)
                    })
                })
                .unwrap_or(rules.nebula_cost);
            let cost = cell_cost(cost, rules);
            if cost == default {
                continue;
            }
            let shape = match r.shape {
                RegionShape::Sphere { centre, radius } => Solid::Sphere {
                    centre,
                    radius: radius * rules.nebula_safety,
                },
                RegionShape::Box { min, max } => Solid::Box { min, max },
                _ => continue,
            };
            out.push(Hazard { shape, cost });
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
        let hazards = self.hazards(&rules);
        let obstacles = self.obstacles();
        if hazards.is_empty() && obstacles.is_empty() {
            return Vec::new();
        }
        let default_cost = cell_cost(rules.default_cost, &rules).max(1);
        let off_map = cell_cost(rules.off_map_cost, &rules);
        let resolution = ceil_units(rules.resolution).max(1);
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
            let reach = crate::production::deploy_reach(&u.stats);
            let solids: Vec<Solid> = obstacles
                .iter()
                .filter(|o| o.friend != Some(u.owner))
                .map(|o| o.solid)
                .collect();
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
            let mut spec = Spec {
                origin: (0, 0),
                cell: resolution,
                width: 1,
                height: 1,
                default_cost,
                body: quantise(self.body_radius(u.def), resolution),
                margin: rules.collision_margin,
                solids,
                hazards: hazards.clone(),
                area,
            };
            // The goal as SIM-21 will fly it: a target inside a solid ends
            // at its edge (SIM-21 adds its own margin on top).
            let mut goal = t;
            for s in &spec.solids {
                // Far solids are skipped before squaring (Fixed overflows
                // past about 46000 units).
                if let Solid::Sphere { centre, radius } = *s {
                    let r = radius + spec.body;
                    if (goal.x - centre.x).abs() > r || (goal.z - centre.z).abs() > r {
                        continue;
                    }
                }
                let (dx, dz) = crate::spatial::push_from_solid(goal, spec.body, s);
                goal.x = goal.x + dx;
                goal.z = goal.z + dz;
            }
            frame(&mut spec, p, goal);
            let entry = self.nav.entry(spec);
            let plan = plan(entry, p, goal, reach);
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

/// The plan for a unit at `p` with target `t` on `entry`'s grid.
fn plan(entry: &mut Entry, p: Vec3, t: Vec3, reach: Fixed) -> Plan {
    let grid = entry.grid.clone();
    // Nothing dearer than open space, or than the target's own cell (an
    // order into a nebula flies straight in), between the unit's cell and
    // the target's: no plan.
    let into = match grid.cost(grid.cell_of(t)) {
        BLOCKED => grid.spec.default_cost,
        c => c.max(grid.spec.default_cost),
    };
    if grid.clear(p, t, into) {
        return Plan::Keep;
    }
    let Some(start) = grid.entry(p) else {
        return Plan::Keep;
    };
    let Some(component) = grid.component(start) else {
        return Plan::Keep;
    };
    let cell = Fixed::from_int(grid.spec.cell as i32);
    let g = grid.cell_of(t);
    let (goal, exact) = if grid.component(g) == Some(component) {
        (g, true)
    } else {
        let Some(n) = grid.nearest_in(component, t, p) else {
            return Plan::Keep;
        };
        // A goal just inside a solid's inflated edge is reached by SIM-21's
        // final approach; only a goal two cells beyond is out of reach.
        (n, within(grid.centre(n, t.y), t, cell + cell))
    };
    if start == goal {
        if exact {
            return Plan::Keep;
        }
        let c = grid.centre(goal, t.y);
        return if within(p, c, reach) {
            Plan::Stop
        } else {
            Plan::Aim(c)
        };
    }
    let field = entry.field(goal);
    if !field.reaches(start) {
        return Plan::Keep;
    }
    // String pulling: the farthest cell along the path the unit can fly to
    // in a straight line costing no more than the path does to get there
    // (plus a cell of open space for the unit's offset in its own cell). A
    // path that must cross a nebula may cut across it; one that goes round
    // never cuts through it.
    let base = field.dist(start);
    let slack = 10 * grid.spec.default_cost as u64;
    let mut k = start;
    let mut first = None;
    let mut aim = None;
    for _ in 0..LOOK {
        let Some(n) = grid.descend(&field, k) else {
            break;
        };
        k = n;
        let c = grid.centre(k, t.y);
        first.get_or_insert(c);
        let path = base.saturating_sub(field.dist(k));
        if grid
            .line_cost(p, c)
            .is_some_and(|line| line <= path + slack)
        {
            aim = Some(c);
        } else if aim.is_some() {
            break;
        }
    }
    match aim.or(first) {
        Some(a) => Plan::Aim(a),
        None => Plan::Keep,
    }
}

#[cfg(test)]
mod tests;
