//! Armada II's random map generator (CON-49): a seed and a player count give
//! the map the stock game makes when "Random map" is chosen.
//!
//! [`generate`] returns a [`sta_bzn::Map`], the same shape a map file parses
//! to, so one conversion ([`MapDef::convert_bzn`]) serves both and the map can
//! be written out with `sta_bzn::write`. [`generate_def`] does both steps.
//!
//! # Rules
//!
//! Read from the Armada II executable (static); not measured on the original.
//!
//! - **Extent.** A square, 10,000 to 15,000 units across, from the origin.
//! - **Starts.** Eight candidates: the four corners and the four edge
//!   midpoints, 200 in from the edge. The map keeps `players` to eight of
//!   them (a random number), removing the rest at random. Each start has a
//!   `Camera_N` path 100 further in, a class K planet 1,400 towards the
//!   centre on each axis, a dilithium moon and a latinum nebula beside it.
//! - **Resources.** 10 to 30, each one of: a planet of a random class (35%),
//!   a latinum nebula (30%), otherwise a moon (one in five inexhaustible).
//! - **Anomalies.** 1 to 5, each a black hole (10%) or a wormhole pair (the
//!   two at least 500 apart).
//! - **Asteroid belts.** 0 to 8, each 6 to 10 steps of up to 300 units along
//!   one axis towards the middle of the map, wandering sideways, with a field
//!   at every point and a belt path through them.
//! - **Nebulae.** 1 to 15, each a diamond 500 to 799 units in radius of one
//!   of the five large nebula kinds, filled on a grid (500 apart for a nebula
//!   whose `ScaleSOD` is above 1, else 160) with a third of a cell of jitter.
//! - **Placement.** A place is drawn inside the extent, 1,200 from the edge
//!   and within 100 of the plane, up to 4,999 times. It is taken when no
//!   object's bounding sphere is within the clearance plus 300, and no start
//!   is within 2,000 (or the clearance, when larger) of the object's own
//!   sphere. When every draw fails the thing is left out.
//!
//! # What differs from the original
//!
//! - Arithmetic is in whole units (determinism rule 1), where the original
//!   uses floats. A seed gives the same map on every peer and platform, but
//!   not the map Armada II itself makes for that seed.
//! - The sideways wander of a belt follows a normal distribution. The
//!   original reads it from a table; here it is the sum of four draws.
//! - The original also asks the object's class whether a place suits it. What
//!   the classes answer is not read; here every class accepts every place.
//! - The original retries the second wormhole of a pair without limit. Here
//!   it gives up after [`PAIR_TRIES`] placements.
//! - Sky, starfield, ambient light, the two lights and the map height are the
//!   stock multiplayer maps' (read from data), as the generator sets none.

use sta_bzn::{Map, MapObject, MapPath, StartLoc};

use crate::map::{Conversion, OdfLookup};
use crate::{ContentDb, MapDef};

/// The planet classes a resource planet is drawn from.
pub const PLANETS: [&str; 6] = ["pb_clssd", "pb_clssh", "pb_clssj", "pb_clssk", "pb_clssl", "pb_clssm"];
/// The planet beside every start.
pub const START_PLANET: &str = "pb_clssk";
pub const MOON: &str = "mdmoon";
pub const MOON_INFINITE: &str = "mmooninf";
pub const LATINUM: &str = "latinum";
pub const WORMHOLE: &str = "mwrmhole";
pub const BLACK_HOLE: &str = "mblkhole";
pub const ASTEROID_FIELD: &str = "mafield2";
/// The nebula kinds, `mnebula6` to `mnebula10`.
pub const NEBULA_KINDS: std::ops::Range<u32> = 6..11;

/// Count ranges of the phases, inclusive.
pub const RESOURCES: (u32, u32) = (10, 30);
pub const ANOMALIES: (u32, u32) = (1, 5);
pub const BELTS: (u32, u32) = (0, 8);
pub const NEBULAE: (u32, u32) = (1, 15);

/// How often the second wormhole of a pair is placed before the pair is dropped.
pub const PAIR_TRIES: u32 = 64;

const START_INSET: i32 = 200;
const CAMERA_INSET: i32 = 100;
const PLACE_MARGIN: i32 = 1200;
const START_CLEARANCE: i64 = 2000;
const OBJECT_GAP: i64 = 300;
const PLACE_TRIES: u32 = 5000;

/// The game's integer random sequence: a 32-bit linear step (the product
/// wraps before the modulus is taken) and a result reduced to the bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence(u32);

impl Sequence {
    /// A zero seed stands for one.
    pub fn new(seed: u32) -> Self {
        Sequence(if seed == 0 { 1 } else { seed })
    }

    /// A number below `bound` (`bound` of zero gives zero).
    pub fn below(&mut self, bound: u32) -> u32 {
        self.0 = self.0.wrapping_mul(2273).wrapping_add(31) % 0x1998_1595;
        if bound == 0 { 0 } else { self.0 % bound }
    }

    /// A whole number from zero to `scale`: the game's scaled fraction, which
    /// has 100,001 steps.
    pub fn scaled(&mut self, scale: i32) -> i32 {
        (i64::from(self.below(100_001)) * i64::from(scale) / 100_000) as i32
    }

    /// A number in `lo..=hi`.
    fn between(&mut self, (lo, hi): (u32, u32)) -> u32 {
        lo + self.below(hi - lo + 1)
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u32) -> bool {
        self.below(100_001) < percent * 1000
    }

    /// A normal deviate of the given standard deviation: four draws summed
    /// (see the module docs).
    fn normal(&mut self, deviation: i32) -> i32 {
        let sum: i32 = (0..4).map(|_| self.below(2001) as i32).sum();
        // Four uniform draws over 0..=2000 have a standard deviation of 1,155.
        (sum - 4000) * deviation / 1155
    }
}

/// What the generator needs to know about an ODF; names are lower case,
/// without extension.
pub trait Classes {
    /// Bounding radius of the object in units, zero when unknown.
    fn radius(&mut self, odf: &str) -> i32;
    /// Whether a nebula is a large one (`ScaleSOD` above 1): its fill is 500
    /// apart, a small one's 160.
    fn large_nebula(&mut self, odf: &str) -> bool;
}

/// Sizes of the stock objects for callers without content (tests, tools):
/// the planets' radii are the order of the stock models', the rest are points.
#[derive(Debug, Clone, Copy, Default)]
pub struct StockSizes;

impl Classes for StockSizes {
    fn radius(&mut self, odf: &str) -> i32 {
        if odf.starts_with("pb_") { 150 } else { 0 }
    }
    fn large_nebula(&mut self, _: &str) -> bool {
        true
    }
}

/// Sizes from an install: [`crate::UnitDef::radius`] for what the database
/// defines, `ScaleSOD` through the ODF lookup.
pub struct ContentClasses<'a, 'b> {
    pub db: &'a ContentDb,
    pub odfs: &'a mut OdfLookup<'b>,
}

impl Classes for ContentClasses<'_, '_> {
    fn radius(&mut self, odf: &str) -> i32 {
        self.db.key(odf).and_then(|k| self.db.unit(k)).map_or(0, |u| u.radius.to_f32().max(0.0) as i32)
    }
    fn large_nebula(&mut self, odf: &str) -> bool {
        (self.odfs)(odf).and_then(|o| o.f32("ScaleSOD")).is_none_or(|s| s > 1.0)
    }
}

type P = [i32; 3];

struct Placed {
    pos: P,
    radius: i32,
}

struct Generator<'c> {
    rng: Sequence,
    classes: &'c mut dyn Classes,
    extent: i32,
    starts: Vec<P>,
    placed: Vec<Placed>,
    objects: Vec<MapObject>,
    paths: Vec<MapPath>,
}

fn square(v: i64) -> i64 {
    v * v
}

fn distance2(a: P, b: P) -> i64 {
    square(i64::from(a[0] - b[0])) + square(i64::from(a[1] - b[1])) + square(i64::from(a[2] - b[2]))
}

fn point(p: P) -> [f32; 3] {
    [p[0] as f32, p[1] as f32, p[2] as f32]
}

impl Generator<'_> {
    fn add(&mut self, odf: &str, pos: P) {
        let radius = self.classes.radius(odf);
        self.placed.push(Placed { pos, radius });
        let n = self.objects.len() as i32;
        let [x, y, z] = point(pos);
        self.objects.push(MapObject {
            odf: odf.to_owned(),
            entity_id: n + 1,
            team: 0,
            label: format!("{odf}{}", n + 1),
            initial_position: Some([x, y, z]),
            transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, y, z],
            light_colour: None,
        });
    }

    /// A free place for `odf`, or `None` when every draw fails.
    fn place(&mut self, odf: &str, clearance: i32) -> Option<P> {
        let radius = self.classes.radius(odf);
        let (lo, hi) = (PLACE_MARGIN, self.extent - PLACE_MARGIN);
        let clear = i64::from(clearance.max(radius));
        let from_start = START_CLEARANCE.max(i64::from(clearance)) + i64::from(radius);
        // The last of the draws is made and never tested.
        for attempt in 1..=PLACE_TRIES {
            let x = lo + self.rng.scaled(hi - lo);
            let z = lo + self.rng.scaled(hi - lo);
            let y = self.rng.scaled(200) - 100;
            let p = [x, y, z];
            if attempt == PLACE_TRIES {
                break;
            }
            let hit = self.placed.iter().any(|o| distance2(o.pos, p) < square(i64::from(o.radius) + clear + OBJECT_GAP));
            if !hit && !self.starts.iter().any(|&s| distance2(s, p) < square(from_start)) {
                return Some(p);
            }
        }
        None
    }

    fn spawn(&mut self, odf: &str, clearance: i32) {
        if let Some(p) = self.place(odf, clearance) {
            self.add(odf, p);
        }
    }

    fn starts(&mut self, players: u32) {
        let (lo, hi, mid) = (START_INSET, self.extent - START_INSET, self.extent / 2);
        // Corners first, then the edge midpoints.
        let all: [(i32, i32); 8] = [(lo, lo), (lo, hi), (hi, lo), (hi, hi), (mid, hi), (mid, lo), (hi, mid), (lo, mid)];
        let count = players + self.rng.below(9 - players);
        self.starts = all.iter().map(|&(x, z)| [x, 0, z]).collect();
        while self.starts.len() > count as usize {
            let i = self.rng.below(self.starts.len() as u32) as usize;
            self.starts.remove(i);
        }
        let inward = |v: i32, by: i32| by * (mid - v).signum();
        for (i, &[x, y, z]) in self.starts.iter().enumerate() {
            self.paths.push(MapPath {
                label: format!("Camera_{}", i + 1),
                points: vec![point([x + inward(x, CAMERA_INSET), y, z + inward(z, CAMERA_INSET)])],
                path_type: 0,
                belt_speed: 0.0,
                nebula_type: 1,
            });
        }
        // An edge start has its moon and latinum either side of it on the
        // axis it sits in the middle of.
        for [x, _, z] in self.starts.clone() {
            let side = |v: i32, by: i32, centred: i32| if v == mid { v + centred } else { v + inward(v, by) };
            self.add(MOON, [side(x, 1000, -500), 0, side(z, 1000, -500)]);
            self.add(LATINUM, [side(x, 1000, 500), 0, side(z, 500, 500)]);
            self.add(START_PLANET, [side(x, 1400, 0), 0, side(z, 1400, 0)]);
        }
    }

    fn moon(&mut self) {
        let odf = if self.rng.chance(20) { MOON_INFINITE } else { MOON };
        self.spawn(odf, 1000);
    }

    fn resources(&mut self) {
        for _ in 0..self.rng.between(RESOURCES) {
            let choice = self.rng.below(100_001);
            if choice < 35_000 {
                let odf = PLANETS[self.rng.below(6) as usize];
                let clearance = self.classes.radius(odf) * 5;
                self.spawn(odf, clearance);
            } else if choice < 65_000 {
                self.spawn(LATINUM, 1000);
            } else {
                self.moon();
            }
        }
    }

    fn anomalies(&mut self) {
        for _ in 0..self.rng.between(ANOMALIES) {
            if self.rng.chance(10) {
                self.spawn(BLACK_HOLE, 1000);
                continue;
            }
            let Some(first) = self.place(WORMHOLE, 300) else { continue };
            for _ in 0..PAIR_TRIES {
                let Some(second) = self.place(WORMHOLE, 300) else { break };
                if distance2(first, second) >= square(500) {
                    // Partners are consecutive objects.
                    self.add(WORMHOLE, first);
                    self.add(WORMHOLE, second);
                    break;
                }
            }
        }
    }

    fn belts(&mut self) {
        for i in 0..self.rng.between(BELTS) {
            let Some(mut p) = self.place(ASTEROID_FIELD, 900) else { continue };
            let along_z = self.rng.below(2) == 0;
            let steps = 6 + self.rng.below(5);
            let axis = if along_z { 2 } else { 0 };
            let toward = if self.extent / 2 > p[axis] { 1 } else { -1 };
            let mut fields = vec![p];
            let mut path = Vec::new();
            for _ in 0..steps {
                path.push(point(p));
                if along_z {
                    p[0] += self.rng.normal(300);
                    p[2] += toward * self.rng.below(300) as i32;
                } else {
                    p[0] += toward * self.rng.below(300) as i32;
                    p[2] += self.rng.normal(300);
                }
                fields.push(p);
            }
            self.paths.push(MapPath { label: format!("Asteroid {i}"), points: path, path_type: 3, belt_speed: 0.0, nebula_type: 1 });
            for f in fields {
                self.add(ASTEROID_FIELD, f);
            }
        }
    }

    fn nebulae(&mut self) {
        for i in 0..self.rng.between(NEBULAE) {
            let radius = 500 + self.rng.below(300) as i32;
            let kind = NEBULA_KINDS.start + self.rng.below(NEBULA_KINDS.end - NEBULA_KINDS.start);
            let odf = format!("mnebula{kind}");
            let Some(c) = self.place(&odf, radius) else { continue };
            let [x, y, z] = c;
            let diamond = [[x + radius, y, z], [x, y, z + radius], [x - radius, y, z], [x, y, z - radius]];
            self.paths.push(MapPath {
                label: format!("Nebula {i}"),
                points: diamond.iter().map(|&p| point(p)).collect(),
                path_type: 4,
                belt_speed: 0.0,
                nebula_type: kind as i32,
            });
            // Fill: a grid over the diamond's box, a cell kept when its centre
            // is inside the diamond, each piece moved up to a third of a cell.
            let spacing = if self.classes.large_nebula(&odf) { 500 } else { 160 };
            let jitter = spacing / 3;
            let cells = 2 * radius / spacing;
            for row in 0..cells {
                for column in 0..cells {
                    // After the first row the original starts a row on the
                    // box's edge, not half a cell in.
                    let gx = x - radius + column * spacing + if row == 0 { spacing / 2 } else { 0 };
                    let gz = z - radius + row * spacing + spacing / 2;
                    if (gx - x).abs() + (gz - z).abs() > radius {
                        continue;
                    }
                    let px = gx + self.rng.scaled(jitter) - jitter / 2;
                    let pz = gz + self.rng.scaled(jitter) - jitter / 2;
                    self.add(&odf, [px, 0, pz]);
                }
            }
            self.add(&odf, c);
        }
    }
}

/// The random map for `seed` with room for `players` (1 to 8; other values
/// are clamped).
pub fn generate(seed: u32, players: u8, classes: &mut dyn Classes) -> Map {
    let players = u32::from(players.clamp(1, 8));
    let mut rng = Sequence::new(seed);
    let extent = 10_000 + rng.scaled(5000);
    let mut g = Generator { rng, classes, extent, starts: Vec::new(), placed: Vec::new(), objects: Vec::new(), paths: Vec::new() };
    g.starts(players);
    g.resources();
    g.anomalies();
    g.belts();
    g.nebulae();

    let size = extent as f32;
    let centre = [size / 2.0, 0.0, size / 2.0];
    let loc = |pos: [f32; 3], kind: &str| StartLoc { pos, kind: kind.to_owned(), team: -1, race: String::new() };
    // Entry 0 is the neutral slot; a stock map has nine entries.
    let mut start_locs = vec![loc(centre, "empty")];
    start_locs.extend(g.starts.iter().map(|&s| loc(point(s), "player")));
    start_locs.resize(9, loc(centre, ""));
    // Two lights, as every stock multiplayer map has: one from above and
    // ahead, one from below and behind.
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let mut objects = g.objects;
    for (n, front) in [[0.0, -h, h], [0.0, h, -h]].into_iter().enumerate() {
        let up = [0.0, front[2], -front[1]];
        objects.push(MapObject {
            odf: "dlight".into(),
            entity_id: objects.len() as i32 + 1,
            team: 0,
            label: format!("dlight{}", n + 1),
            initial_position: Some(centre),
            transform: [1.0, 0.0, 0.0, up[0], up[1], up[2], front[0], front[1], front[2], centre[0], centre[1], centre[2]],
            light_colour: None,
        });
    }
    Map {
        version: 2167,
        binary: true,
        mission_type: "mp-regular".into(),
        description: format!("Random map {seed}"),
        file_name: "RandomMap.bzn".into(),
        tech_tree: "tech1.tt".into(),
        scripts: Vec::new(),
        min_extent: [0.0, -1250.0, 0.0],
        size: [size, 2500.0, size],
        ambient: [64.0 / 255.0; 3],
        background: "mbg02.sod".into(),
        starfield_odf: "ststars.odf".into(),
        tech_tree_file: "tech1.tt".into(),
        race: "federation".into(),
        start_count: g.starts.len() as i32,
        start_locs,
        objects,
        paths: g.paths,
        a1: None,
    }
}

/// [`generate`] with the sizes of `db`'s content, converted for the sim.
pub fn generate_def(seed: u32, players: u8, db: &ContentDb, odfs: &mut OdfLookup) -> Conversion {
    let bzn = generate(seed, players, &mut ContentClasses { db, odfs: &mut *odfs });
    MapDef::convert_bzn(&bzn, db, odfs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(seed: u32, players: u8) -> Map {
        generate(seed, players, &mut StockSizes)
    }

    fn count(m: &Map, odf: impl Fn(&str) -> bool) -> usize {
        m.objects.iter().filter(|o| odf(&o.odf)).count()
    }

    #[test]
    fn sequence_wraps_before_the_modulus() {
        let mut s = Sequence::new(0);
        // From one: 2273 + 31.
        assert_eq!(s.below(u32::MAX), 2304);
        let mut s = Sequence::new(0x1998_1594);
        let wrapped = 0x1998_1594_u32.wrapping_mul(2273).wrapping_add(31) % 0x1998_1595;
        assert_eq!(s.below(u32::MAX), wrapped);
        assert_ne!(u64::from(wrapped), (0x1998_1594_u64 * 2273 + 31) % 0x1998_1595);
        assert_eq!(s.below(0), 0);
    }

    #[test]
    fn same_seed_same_map() {
        assert_eq!(make(1234, 4), make(1234, 4));
        assert_ne!(make(1234, 4), make(1235, 4));
        assert_ne!(make(1234, 4).objects, make(1234, 2).objects);
    }

    #[test]
    fn counts_stay_in_range() {
        for seed in 0..1000 {
            let players = (seed % 8 + 1) as u8;
            let m = make(seed, players);
            let extent = m.size[0];
            assert!((10_000.0..=15_000.0).contains(&extent) && m.size[2] == extent, "seed {seed}: extent {extent}");
            let starts = m.start_count as usize;
            assert!((players as usize..=8).contains(&starts), "seed {seed}: {starts} starts for {players} players");
            assert_eq!(m.start_locs.len(), 9);
            assert_eq!(m.start_locs.iter().filter(|l| l.kind == "player").count(), starts);
            assert_eq!(m.paths.iter().filter(|p| p.label.starts_with("Camera_")).count(), starts);
            // One planet, moon and latinum per start, then the resource phase.
            let resources = count(&m, |o| o.starts_with("pb_") || o == LATINUM || o == MOON || o == MOON_INFINITE) - 3 * starts;
            assert!(resources <= RESOURCES.1 as usize, "seed {seed}: {resources} resources");
            let holes = count(&m, |o| o == WORMHOLE);
            assert_eq!(holes % 2, 0, "seed {seed}: wormholes come in pairs");
            assert!(holes / 2 + count(&m, |o| o == BLACK_HOLE) <= ANOMALIES.1 as usize);
            let belts = m.paths.iter().filter(|p| p.path_type == 3).count();
            assert!(belts <= BELTS.1 as usize);
            for p in m.paths.iter().filter(|p| p.path_type == 3) {
                assert!((6..=10).contains(&p.points.len()), "seed {seed}: belt of {} points", p.points.len());
            }
            assert_eq!(count(&m, |o| o == ASTEROID_FIELD), m.paths.iter().filter(|p| p.path_type == 3).map(|p| p.points.len() + 1).sum::<usize>());
            let nebulae: Vec<_> = m.paths.iter().filter(|p| p.path_type == 4).collect();
            assert!(nebulae.len() <= NEBULAE.1 as usize);
            for n in nebulae {
                assert!(NEBULA_KINDS.contains(&(n.nebula_type as u32)) && n.points.len() == 4);
            }
            assert_eq!(count(&m, |o| o == "dlight"), 2);
        }
    }

    /// The phases with a lower bound do make something on an ordinary map.
    #[test]
    fn a_map_has_resources_and_nebulae() {
        let m = make(7, 2);
        assert!(count(&m, |o| o.starts_with("mnebula")) > 0);
        assert!(count(&m, |o| o == LATINUM || o == MOON) > 2 * m.start_count as usize);
    }

    #[test]
    fn placed_objects_keep_their_clearance() {
        for seed in 0..200 {
            let m = make(seed, 2);
            let starts: Vec<_> = m.start_locs.iter().filter(|l| l.kind == "player").map(|l| l.pos).collect();
            let per_start = 3 * starts.len();
            let d = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            let pos = |o: &MapObject| [o.transform[9], o.transform[10], o.transform[11]];
            // Placed things (not the start resources, belts' later fields or
            // nebula fill) stay 2,000 clear of every start and inside the margin.
            let free: Vec<_> = m.objects[per_start..].iter().filter(|o| o.odf.starts_with("pb_") || [LATINUM, MOON, MOON_INFINITE, WORMHOLE, BLACK_HOLE].contains(&o.odf.as_str())).collect();
            for o in &free {
                let p = pos(o);
                let radius = StockSizes.radius(&o.odf) as f32;
                for s in &starts {
                    assert!(d(p, *s) - radius >= 2000.0, "seed {seed}: {} at {p:?} near start {s:?}", o.odf);
                }
                assert!(p[0] >= 1200.0 && p[0] <= m.size[0] - 1200.0 && p[2] >= 1200.0 && p[2] <= m.size[2] - 1200.0 && p[1].abs() <= 100.0);
            }
            // And 300 clear of each other's spheres, except the two of a
            // wormhole pair, which only keep 500 between centres.
            for (i, a) in free.iter().enumerate() {
                for b in &free[..i] {
                    let gap = d(pos(a), pos(b)) - StockSizes.radius(&a.odf) as f32 - StockSizes.radius(&b.odf) as f32;
                    let pair = a.odf == WORMHOLE && b.odf == WORMHOLE;
                    assert!(gap >= if pair { 500.0 } else { 300.0 }, "seed {seed}: {} and {} are {gap} apart", a.odf, b.odf);
                }
            }
        }
    }

    #[test]
    fn starts_are_corners_and_edges() {
        let m = make(99, 8);
        assert_eq!(m.start_count, 8);
        let e = m.size[0];
        for l in &m.start_locs[1..] {
            for v in [l.pos[0], l.pos[2]] {
                assert!(v == 200.0 || v == e - 200.0 || v == (e as i32 / 2) as f32, "start at {:?} on a {e} map", l.pos);
            }
        }
        // The first start's planet is 1,400 in on both axes.
        let planet = m.objects.iter().find(|o| o.odf == START_PLANET).unwrap();
        assert_eq!([planet.transform[9], planet.transform[11]], [1600.0, 1600.0]);
        assert_eq!(m.paths[0].label, "Camera_1");
        assert_eq!(m.paths[0].points, [[300.0, 0.0, 300.0]]);
    }

    #[test]
    fn round_trips_through_the_map_writer() {
        for seed in [1, 42, 4000] {
            let m = make(seed, 3);
            let bytes = sta_bzn::write::write(&m).unwrap();
            assert_eq!(sta_bzn::parse(&bytes).unwrap(), m, "seed {seed}");
        }
    }

    /// The fake database defines none of the stock ODFs, so nothing is placed,
    /// but the starts, bounds and scenery arrive.
    #[test]
    fn converts_to_a_map_def() {
        let db = crate::fake::db();
        let c = generate_def(5, 4, &db, &mut |_| None);
        assert!(c.map.starts.len() >= 4);
        assert_eq!(c.map.starts[0].slot, 0);
        assert_eq!(c.map.bounds.0.x, crate::Fixed::ZERO);
        assert_eq!(c.map.scenery.starfield.as_deref(), Some("ststars"));
        assert_eq!(c.counts.objects, 0);
    }
}
