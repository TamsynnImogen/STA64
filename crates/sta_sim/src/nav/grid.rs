//! The navigation grid and the searches over it (SIM-23).
//!
//! All arithmetic is integer: positions are [`Fixed`] bits (32.32), squared
//! distances are compared in `i128`, path costs are `u64` (stored as `u32`,
//! saturating, in a field). Searches visit
//! cells by `(cost, index)`, so ties always break toward the lower index and
//! every target computes the same field.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use sta_core::{Fixed, Vec3};

use crate::spatial::Solid;

/// A cell cost meaning "cannot enter".
pub(crate) const BLOCKED: u32 = 0;
/// A cell that is in no component (blocked).
const NO_COMPONENT: u32 = u32::MAX;
/// Unreached in a field.
pub(crate) const FAR: u32 = u32::MAX;
/// Step weights: orthogonal and diagonal (10 and 14 for 1 and about sqrt 2).
const ORTHOGONAL: u64 = 10;
const DIAGONAL: u64 = 14;
/// The eight moves, in the fixed order every search uses.
const MOVES: [(i64, i64, bool); 8] = [
    (1, 0, false),
    (-1, 0, false),
    (0, 1, false),
    (0, -1, false),
    (1, 1, true),
    (1, -1, true),
    (-1, 1, true),
    (-1, -1, true),
];

const FRAC: u32 = 32;

/// Whole world units below `v` (floor).
pub(crate) fn floor_units(v: Fixed) -> i64 {
    v.to_bits() >> FRAC
}

/// Whole world units at or above `v` (ceiling).
pub(crate) fn ceil_units(v: Fixed) -> i64 {
    -((-v.to_bits()) >> FRAC)
}

/// A region with a path cost: a nebula (SIM-23).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hazard {
    pub shape: Solid,
    /// The cost of a cell inside, [`BLOCKED`] if impassable.
    pub cost: u32,
}

/// Where a unit may be: a rectangle (XZ) and what lies outside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Area {
    pub min: (Fixed, Fixed),
    pub max: (Fixed, Fixed),
    /// The cost outside, [`BLOCKED`] for a confined unit.
    pub outside: u32,
}

/// Everything a grid is built from. Two equal specs build equal grids, which
/// is what lets the cache reuse one without changing any result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Spec {
    /// World units of the lower corner (X, Z).
    pub origin: (i64, i64),
    /// Cell width, world units.
    pub cell: i64,
    pub width: u32,
    pub height: u32,
    /// Open space.
    pub default_cost: u32,
    /// The craft's body radius.
    pub body: Fixed,
    /// `COLLISION_SAFETY_MARGIN`: solids and bodies are this much wider.
    pub margin: Fixed,
    pub solids: Vec<Solid>,
    pub hazards: Vec<Hazard>,
    pub area: Option<Area>,
}

#[derive(Debug)]
pub(crate) struct Grid {
    pub spec: Spec,
    /// Per cell, row-major (`z * width + x`).
    cost: Vec<u32>,
    component: Vec<u32>,
}

fn sq(v: i128) -> i128 {
    v * v
}

/// Squared distance (in squared Fixed bits) from `(x, z)` to `shape`; zero
/// inside.
fn distance2(x: i64, z: i64, shape: &Solid) -> i128 {
    match *shape {
        Solid::Sphere { centre, .. } => {
            sq(x as i128 - centre.x.to_bits() as i128) + sq(z as i128 - centre.z.to_bits() as i128)
        }
        Solid::Box { min, max } => {
            let axis = |v: i64, lo: Fixed, hi: Fixed| -> i128 {
                if v < lo.to_bits() {
                    lo.to_bits() as i128 - v as i128
                } else if v > hi.to_bits() {
                    v as i128 - hi.to_bits() as i128
                } else {
                    0
                }
            };
            sq(axis(x, min.x, max.x)) + sq(axis(z, min.z, max.z))
        }
    }
}

/// The XZ extent of `shape` grown by `grow`, in Fixed bits.
fn extent(shape: &Solid, grow: Fixed) -> (i64, i64, i64, i64) {
    match *shape {
        Solid::Sphere { centre, .. } => {
            let g = grow.to_bits();
            (
                centre.x.to_bits().saturating_sub(g),
                centre.z.to_bits().saturating_sub(g),
                centre.x.to_bits().saturating_add(g),
                centre.z.to_bits().saturating_add(g),
            )
        }
        Solid::Box { min, max } => {
            let g = grow.to_bits();
            (
                min.x.to_bits().saturating_sub(g),
                min.z.to_bits().saturating_sub(g),
                max.x.to_bits().saturating_add(g),
                max.z.to_bits().saturating_add(g),
            )
        }
    }
}

impl Spec {
    /// How far round `solid` a cell centre is blocked: for a sphere the
    /// distance from its centre, for a box from its faces.
    pub(crate) fn reach(&self, solid: &Solid) -> Fixed {
        match *solid {
            Solid::Sphere { radius, .. } => (radius + self.body) * self.margin,
            Solid::Box { .. } => self.body * self.margin,
        }
    }

    /// The hazard's shape's reach: its radius (spheres; the multiplier is
    /// already in it) or zero (boxes).
    fn hazard_reach(h: &Hazard) -> Fixed {
        match h.shape {
            Solid::Sphere { radius, .. } => radius,
            Solid::Box { .. } => Fixed::ZERO,
        }
    }

    /// Fixed bits of the centre of cell `(i, j)`.
    pub(crate) fn centre_bits(&self, i: i64, j: i64) -> (i64, i64) {
        let half = |o: i64, k: i64| ((o + k * self.cell) << FRAC) + (self.cell << (FRAC - 1));
        (half(self.origin.0, i), half(self.origin.1, j))
    }

    /// The cell holding Fixed bits `(x, z)`, clamped into the grid.
    pub(crate) fn cell_of_bits(&self, x: i64, z: i64) -> (i64, i64) {
        let one = |v: i64, o: i64, n: u32| {
            ((v >> FRAC) - o)
                .div_euclid(self.cell)
                .clamp(0, n as i64 - 1)
        };
        (
            one(x, self.origin.0, self.width),
            one(z, self.origin.1, self.height),
        )
    }

    /// Cell range (inclusive) covering Fixed-bit extents.
    fn range(&self, lo: i64, hi: i64, o: i64, n: u32) -> (i64, i64) {
        let a = ((lo >> FRAC) - o).div_euclid(self.cell).max(0);
        let b = ((hi >> FRAC) - o + 1)
            .div_euclid(self.cell)
            .min(n as i64 - 1);
        (a, b)
    }
}

impl Grid {
    pub(crate) fn build(spec: Spec) -> Grid {
        let (w, h) = (spec.width as i64, spec.height as i64);
        let n = (w * h) as usize;
        let mut base = vec![spec.default_cost; n];
        if let Some(area) = spec.area {
            let (lx, lz, hx, hz) = (
                area.min.0.to_bits(),
                area.min.1.to_bits(),
                area.max.0.to_bits(),
                area.max.1.to_bits(),
            );
            for j in 0..h {
                for i in 0..w {
                    let (x, z) = spec.centre_bits(i, j);
                    if x < lx || x > hx || z < lz || z > hz {
                        base[(j * w + i) as usize] = area.outside;
                    }
                }
            }
        }
        // Hazards: the dearest one covering a cell sets its cost.
        let mut hazard = vec![None::<u32>; n];
        for hz in &spec.hazards {
            let r = Spec::hazard_reach(hz);
            let (x0, z0, x1, z1) = extent(&hz.shape, r);
            let (i0, i1) = spec.range(x0, x1, spec.origin.0, spec.width);
            let (j0, j1) = spec.range(z0, z1, spec.origin.1, spec.height);
            let r2 = sq(r.to_bits() as i128);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, z) = spec.centre_bits(i, j);
                    let d2 = distance2(x, z, &hz.shape);
                    let inside = match hz.shape {
                        Solid::Sphere { .. } => d2 < r2,
                        Solid::Box { .. } => d2 == 0,
                    };
                    if inside {
                        let k = (j * w + i) as usize;
                        hazard[k] = Some(match (hazard[k], hz.cost) {
                            (Some(BLOCKED), _) | (_, BLOCKED) => BLOCKED,
                            (Some(c), d) => c.max(d),
                            (None, d) => d,
                        });
                    }
                }
            }
        }
        let mut cost: Vec<u32> = base
            .iter()
            .zip(&hazard)
            .map(|(&b, &hz)| match hz {
                None => b,
                Some(BLOCKED) => BLOCKED,
                Some(_) if b == BLOCKED => BLOCKED,
                // Off the map and in a nebula: the dearer.
                Some(c) if b != spec.default_cost => b.max(c),
                Some(c) => c,
            })
            .collect();
        for s in &spec.solids {
            let r = spec.reach(s);
            let (x0, z0, x1, z1) = extent(s, r);
            let (i0, i1) = spec.range(x0, x1, spec.origin.0, spec.width);
            let (j0, j1) = spec.range(z0, z1, spec.origin.1, spec.height);
            let r2 = sq(r.to_bits() as i128);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, z) = spec.centre_bits(i, j);
                    let d2 = distance2(x, z, s);
                    let blocked = match s {
                        Solid::Sphere { .. } => d2 < r2,
                        Solid::Box { .. } => d2 <= r2,
                    };
                    if blocked {
                        cost[(j * w + i) as usize] = BLOCKED;
                    }
                }
            }
        }
        let mut grid = Grid {
            spec,
            cost,
            component: vec![NO_COMPONENT; n],
        };
        grid.label();
        grid
    }

    pub(crate) fn len(&self) -> usize {
        self.cost.len()
    }

    pub(crate) fn index(&self, (i, j): (i64, i64)) -> u32 {
        (j * self.spec.width as i64 + i) as u32
    }

    pub(crate) fn coords(&self, k: u32) -> (i64, i64) {
        let w = self.spec.width as i64;
        (k as i64 % w, k as i64 / w)
    }

    pub(crate) fn cost(&self, k: u32) -> u32 {
        self.cost[k as usize]
    }

    pub(crate) fn passable(&self, k: u32) -> bool {
        self.cost[k as usize] != BLOCKED
    }

    pub(crate) fn component(&self, k: u32) -> Option<u32> {
        Some(self.component[k as usize]).filter(|&c| c != NO_COMPONENT)
    }

    /// The cost of entering `k`: a blocked cell (only ever a search's own
    /// goal or start) costs open space.
    fn enter(&self, k: u32) -> u64 {
        match self.cost(k) {
            BLOCKED => self.spec.default_cost as u64,
            c => c as u64,
        }
    }

    /// The cell centre of `k` as a point at height `y`.
    pub(crate) fn centre(&self, k: u32, y: Fixed) -> Vec3 {
        let (i, j) = self.coords(k);
        let (x, z) = self.spec.centre_bits(i, j);
        Vec3 {
            x: Fixed::from_bits(x),
            y,
            z: Fixed::from_bits(z),
        }
    }

    pub(crate) fn cell_of(&self, p: Vec3) -> u32 {
        self.index(self.spec.cell_of_bits(p.x.to_bits(), p.z.to_bits()))
    }

    /// The moves out of `k` that land in the grid, with their weights. A
    /// diagonal move needs both cells beside it passable (no corner cutting).
    fn moves(&self, k: u32) -> impl Iterator<Item = (u32, u64)> + '_ {
        let (i, j) = self.coords(k);
        let (w, h) = (self.spec.width as i64, self.spec.height as i64);
        MOVES.iter().filter_map(move |&(di, dj, diagonal)| {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= w || b >= h {
                return None;
            }
            if diagonal && !(self.passable(self.index((a, j))) && self.passable(self.index((i, b))))
            {
                return None;
            }
            Some((
                self.index((a, b)),
                if diagonal { DIAGONAL } else { ORTHOGONAL },
            ))
        })
    }

    /// Connected components of passable cells, numbered in index order.
    fn label(&mut self) {
        let mut next = 0;
        let mut queue = VecDeque::new();
        for start in 0..self.len() as u32 {
            if !self.passable(start) || self.component[start as usize] != NO_COMPONENT {
                continue;
            }
            self.component[start as usize] = next;
            queue.push_back(start);
            while let Some(k) = queue.pop_front() {
                let mut found = [u32::MAX; 8];
                for (slot, (m, _)) in found.iter_mut().zip(self.moves(k)) {
                    *slot = m;
                }
                for m in found {
                    if m != u32::MAX
                        && self.cost[m as usize] != BLOCKED
                        && self.component[m as usize] == NO_COMPONENT
                    {
                        self.component[m as usize] = next;
                        queue.push_back(m);
                    }
                }
            }
            next += 1;
        }
    }

    /// The passable cell nearest `p`: its own, else the nearest within three
    /// cells (distance to `p`, then index). `None`: walled in.
    pub(crate) fn entry(&self, p: Vec3) -> Option<u32> {
        let (ci, cj) = self.spec.cell_of_bits(p.x.to_bits(), p.z.to_bits());
        let own = self.index((ci, cj));
        if self.passable(own) {
            return Some(own);
        }
        let (w, h) = (self.spec.width as i64, self.spec.height as i64);
        let mut best: Option<(i128, u32)> = None;
        for j in (cj - 3).max(0)..=(cj + 3).min(h - 1) {
            for i in (ci - 3).max(0)..=(ci + 3).min(w - 1) {
                let k = self.index((i, j));
                if !self.passable(k) {
                    continue;
                }
                let (x, z) = self.spec.centre_bits(i, j);
                let d2 =
                    sq(x as i128 - p.x.to_bits() as i128) + sq(z as i128 - p.z.to_bits() as i128);
                if best.is_none_or(|b| (d2, k) < b) {
                    best = Some((d2, k));
                }
            }
        }
        best.map(|(_, k)| k)
    }

    /// The cell of `component` nearest `t`: distance in half cells, then
    /// nearest `p` (so of equally near cells the unit takes the one on its
    /// own side), then index.
    pub(crate) fn nearest_in(&self, component: u32, t: Vec3, p: Vec3) -> Option<u32> {
        let half = (self.spec.cell / 2).max(1) as u128;
        let mut best: Option<(u128, i128, u32)> = None;
        for k in 0..self.len() as u32 {
            if self.component[k as usize] != component {
                continue;
            }
            let (i, j) = self.coords(k);
            let (x, z) = self.spec.centre_bits(i, j);
            let d2 = sq(x as i128 - t.x.to_bits() as i128) + sq(z as i128 - t.z.to_bits() as i128);
            let band = ((d2 >> (2 * FRAC)) as u128).isqrt() / half;
            let near =
                sq(x as i128 - p.x.to_bits() as i128) + sq(z as i128 - p.z.to_bits() as i128);
            if best.is_none_or(|b| (band, near, k) < b) {
                best = Some((band, near, k));
            }
        }
        best.map(|(_, _, k)| k)
    }

    /// Cost-to-go to `goal` from every cell (Dijkstra from the goal). The
    /// goal itself may be blocked; nothing else blocked is entered.
    pub(crate) fn field(&self, goal: u32) -> Field {
        let mut dist = vec![FAR; self.len()];
        let mut heap = BinaryHeap::new();
        dist[goal as usize] = 0;
        heap.push(Reverse((0u32, goal)));
        while let Some(Reverse((d, n))) = heap.pop() {
            if d > dist[n as usize] {
                continue;
            }
            let enter = self.enter(n);
            // A move m -> n is the reverse of n -> m here: the same cells
            // either way, so the corner rule reads the same.
            for (m, w) in self.moves(n).filter(|&(m, _)| self.passable(m)) {
                let nd = (d as u64 + w * enter).min(FAR as u64 - 1) as u32;
                if nd < dist[m as usize] {
                    dist[m as usize] = nd;
                    heap.push(Reverse((nd, m)));
                }
            }
        }
        Field { goal, dist }
    }

    /// The next cell from `k` toward the field's goal: the move with the
    /// least step plus remaining cost, first in move order on a tie.
    pub(crate) fn descend(&self, field: &Field, k: u32) -> Option<u32> {
        if k == field.goal || field.dist[k as usize] == FAR {
            return None;
        }
        let mut best: Option<(u64, u32)> = None;
        for (m, w) in self.moves(k) {
            let rest = field.dist[m as usize];
            if rest == FAR || (m != field.goal && !self.passable(m)) {
                continue;
            }
            let total = rest as u64 + w * self.enter(m);
            if best.is_none_or(|b| total < b.0) {
                best = Some((total, m));
            }
        }
        best.map(|(_, m)| m)
    }

    /// Whether the straight leg `a` -> `b` crosses no blocked cell and no
    /// cell dearer than `limit`, apart from the cells holding its ends.
    /// Sampled at a quarter cell along the longer axis; integer throughout.
    pub(crate) fn clear(&self, a: Vec3, b: Vec3, limit: u32) -> bool {
        let (ax, az, bx, bz) = (
            a.x.to_bits() as i128,
            a.z.to_bits() as i128,
            b.x.to_bits() as i128,
            b.z.to_bits() as i128,
        );
        let step = ((self.spec.cell as i128) << FRAC) / 4;
        let span = (bx - ax).abs().max((bz - az).abs());
        let n = (span / step.max(1) + 1).min(1 << 16);
        let (first, last) = (self.cell_of(a), self.cell_of(b));
        for s in 1..n {
            let x = ax + (bx - ax) * s / n;
            let z = az + (bz - az) * s / n;
            let k = self.index(self.spec.cell_of_bits(x as i64, z as i64));
            if k == first || k == last {
                continue;
            }
            let c = self.cost(k);
            if c == BLOCKED || c > limit {
                return false;
            }
        }
        true
    }

    /// The cost of flying the straight leg `a` -> `b`, in a field's units
    /// (`10 * cell cost` per cell width), or `None` when it crosses a
    /// blocked cell other than the one it starts in. Sampled like
    /// [`clear`](Grid::clear); integer throughout.
    pub(crate) fn line_cost(&self, a: Vec3, b: Vec3) -> Option<u64> {
        let (ax, az, bx, bz) = (
            a.x.to_bits() as i128,
            a.z.to_bits() as i128,
            b.x.to_bits() as i128,
            b.z.to_bits() as i128,
        );
        let step = ((self.spec.cell as i128) << FRAC) / 4;
        let span = (bx - ax).abs().max((bz - az).abs());
        let n = (span / step.max(1) + 1).min(1 << 16);
        let first = self.cell_of(a);
        let mut sum: i128 = 0;
        for s in 1..=n {
            let x = ax + (bx - ax) * s / n;
            let z = az + (bz - az) * s / n;
            let k = self.index(self.spec.cell_of_bits(x as i64, z as i64));
            sum += match self.cost(k) {
                BLOCKED if k == first => self.spec.default_cost as i128,
                BLOCKED => return None,
                c => c as i128,
            };
        }
        let len = (sq(bx - ax) + sq(bz - az)) as u128;
        let len = len.isqrt() as i128;
        let cost = ORTHOGONAL as i128 * len * sum / (n * ((self.spec.cell as i128) << FRAC));
        Some(cost.clamp(0, u64::MAX as i128) as u64)
    }
}

/// Cost-to-go to one goal cell.
#[derive(Debug)]
pub(crate) struct Field {
    pub goal: u32,
    dist: Vec<u32>,
}

impl Field {
    pub(crate) fn reaches(&self, k: u32) -> bool {
        self.dist[k as usize] != FAR
    }

    /// The cost to go from `k`.
    pub(crate) fn dist(&self, k: u32) -> u64 {
        self.dist[k as usize] as u64
    }
}
