//! What the random map generator places (CON-50): a table of resource types
//! and, per set, the ODFs each may be.
//!
//! The table is data: `random_map.json5` at the root of a mod or install,
//! where the highest layer's file is the whole table ([`Table::load`]).
//! Without one the built-in [`Table::stock`] stands, which names the stock
//! Armada II objects. `docs/RANDOM_MAP.md` describes the file.
//!
//! This format is STA64's own; the original has no counterpart.

use serde::Deserialize;

use crate::Warning;

/// The file a mod or install puts its table in.
pub const FILE: &str = "random_map.json5";

/// Why a table could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}: {message}")]
pub struct TableError {
    pub file: String,
    pub message: String,
}

/// A set: one look or theme of the map's resources (`normal`, `frozen`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Set {
    pub name: String,
    /// Its share when the set is drawn.
    pub weight: u32,
}

/// One ODF a resource may be, and its share among the set's names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// Lower case, without extension.
    pub odf: String,
    pub weight: u32,
}

/// Where a resource sits beside a start, per axis `[x, z]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartPlace {
    /// Distance towards the map's centre on an axis the start is at the edge of.
    pub inward: [i32; 2],
    /// Offset on an axis the start is in the middle of (an edge start).
    pub centred: [i32; 2],
    /// The names used beside a start, by set; a set not listed here uses the
    /// resource's own names.
    pub sets: Vec<(String, Vec<Variant>)>,
}

/// A resource type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub name: String,
    /// Share of the resource phase; zero keeps it out of the phase.
    pub weight: u32,
    /// Clear distance to other objects, units.
    pub clearance: i32,
    /// Clear distance in multiples of the object's own radius (the larger of
    /// the two applies).
    pub clearance_radii: i32,
    /// On every map: placed once more when the map would have none.
    pub required: bool,
    pub start: Option<StartPlace>,
    /// The names to pick from, by set, in the file's order.
    pub sets: Vec<(String, Vec<Variant>)>,
}

/// The generator's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// In the file's order; the first is the default.
    pub sets: Vec<Set>,
    /// How many resources the resource phase places, inclusive.
    pub count: (u32, u32),
    /// In the file's order, which is the order shares are laid out in.
    pub resources: Vec<Resource>,
}

fn find<'a>(sets: &'a [(String, Vec<Variant>)], name: &str) -> Option<&'a [Variant]> {
    sets.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice())
}

impl Resource {
    /// The names for `set`; a resource without that set uses its first.
    pub fn variants(&self, set: &str) -> &[Variant] {
        find(&self.sets, set).unwrap_or(&self.sets[0].1)
    }

    /// The names used beside a start for `set`.
    pub fn start_variants(&self, set: &str) -> &[Variant] {
        self.start.as_ref().and_then(|s| find(&s.sets, set)).unwrap_or_else(|| self.variants(set))
    }
}

fn names(odfs: &[&str]) -> Vec<Variant> {
    odfs.iter().map(|o| Variant { odf: (*o).to_owned(), weight: 1 }).collect()
}

impl Table {
    /// The stock Armada II table: the rules of CON-49 (see
    /// [`crate::random_map`]), draw for draw.
    pub fn stock() -> Table {
        use super::{LATINUM, MOON, MOON_INFINITE, PLANETS, START_PLANET};
        let normal = |v: Vec<Variant>| vec![("normal".to_owned(), v)];
        let start = |inward, centred, odf: Option<&str>| {
            Some(StartPlace { inward, centred, sets: odf.map(|o| normal(names(&[o]))).unwrap_or_default() })
        };
        Table {
            sets: vec![Set { name: "normal".into(), weight: 1 }],
            count: super::RESOURCES,
            resources: vec![
                Resource {
                    name: "planet".into(),
                    weight: 35,
                    clearance: 0,
                    clearance_radii: 5,
                    required: true,
                    start: start([1400, 1400], [0, 0], Some(START_PLANET)),
                    sets: normal(names(&PLANETS)),
                },
                Resource {
                    name: "latinum".into(),
                    weight: 30,
                    clearance: 1000,
                    clearance_radii: 0,
                    required: true,
                    start: start([1000, 500], [500, 500], None),
                    sets: normal(names(&[LATINUM])),
                },
                Resource {
                    name: "dilithium".into(),
                    weight: 35,
                    clearance: 1000,
                    clearance_radii: 0,
                    required: true,
                    start: start([1000, 1000], [-500, -500], Some(MOON)),
                    sets: normal(vec![Variant { odf: MOON_INFINITE.into(), weight: 1 }, Variant { odf: MOON.into(), weight: 4 }]),
                },
            ],
        }
    }

    /// Read a table from JSON5 text. `file` names it in the error.
    pub fn parse(file: &str, text: &str) -> Result<Table, TableError> {
        let err = |message: String| TableError { file: file.to_owned(), message };
        let doc: Doc = ::json5::from_str(text).map_err(|e| err(e.to_string()))?;
        let mut sets: Vec<Set> = Vec::new();
        for s in doc.sets {
            let (name, weight) = match s {
                SetDoc::Name(name) => (name, 1),
                SetDoc::Full(SetFull { name, weight }) => (name, weight),
            };
            if name.is_empty() || sets.iter().any(|x| x.name == name) {
                return Err(err(format!("set {name:?} is empty or listed twice")));
            }
            sets.push(Set { name, weight });
        }
        if sets.is_empty() || sets.iter().all(|s| s.weight == 0) {
            return Err(err("no set with a weight above zero".into()));
        }
        let variants = |owner: &str, map: Vec<(String, Vec<VariantDoc>)>| -> Result<Vec<(String, Vec<Variant>)>, TableError> {
            let mut out: Vec<(String, Vec<Variant>)> = Vec::new();
            for (set, list) in map {
                if !sets.iter().any(|s| s.name == set) {
                    return Err(err(format!("{owner}: set {set:?} is not in `sets`")));
                }
                if out.iter().any(|(n, _)| *n == set) {
                    return Err(err(format!("{owner}: set {set:?} is given twice")));
                }
                let list: Vec<Variant> = list
                    .into_iter()
                    .map(|v| match v {
                        VariantDoc::Name(odf) => (odf, 1),
                        VariantDoc::Full(VariantFull { odf, weight }) => (odf, weight),
                    })
                    .map(|(odf, weight)| Variant { odf: crate::odf::stem(&odf), weight })
                    .collect();
                if list.iter().any(|v| v.odf.is_empty()) || list.iter().all(|v| v.weight == 0) {
                    return Err(err(format!("{owner}: set {set:?} needs a name with a weight above zero")));
                }
                out.push((set, list));
            }
            Ok(out)
        };
        let mut resources: Vec<Resource> = Vec::new();
        for r in doc.resources {
            if r.name.is_empty() || resources.iter().any(|x| x.name == r.name) {
                return Err(err(format!("resource {:?} is empty or listed twice", r.name)));
            }
            let own = variants(&r.name, r.sets.0)?;
            if own.is_empty() {
                return Err(err(format!("{}: no set of names", r.name)));
            }
            if r.clearance < 0 || r.clearance_radii < 0 {
                return Err(err(format!("{}: a clearance below zero", r.name)));
            }
            let start = match r.start {
                None => None,
                Some(s) => Some(StartPlace { inward: s.inward, centred: s.centred, sets: variants(&format!("{} start", r.name), s.sets.0)? }),
            };
            resources.push(Resource { name: r.name, weight: r.weight, clearance: r.clearance, clearance_radii: r.clearance_radii, required: r.required, start, sets: own });
        }
        let [lo, hi] = doc.count;
        if lo > hi {
            return Err(err(format!("count {lo} to {hi} runs backwards")));
        }
        if hi > 0 && resources.iter().all(|r| r.weight == 0) {
            return Err(err("no resource with a weight above zero".into()));
        }
        Ok(Table { sets, count: (lo, hi), resources })
    }

    /// The table of a mounted install: its [`FILE`], or [`Table::stock`]
    /// when there is none or it cannot be read (then with a warning).
    pub fn load(vfs: &sta_vfs::Vfs) -> (Table, Vec<Warning>) {
        let text = match vfs.read(FILE) {
            None => return (Table::stock(), Vec::new()),
            Some(Err(e)) => return (Table::stock(), vec![Warning::new(FILE, format!("not read ({e}); the stock table stands"))]),
            Some(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        };
        match Table::parse(FILE, &text) {
            Ok(t) => (t, Vec::new()),
            Err(e) => (Table::stock(), vec![Warning::new(FILE, format!("{}; the stock table stands", e.message))]),
        }
    }

    /// Every ODF the table names, sorted, each once.
    pub fn odfs(&self) -> Vec<&str> {
        let lists = self.resources.iter().flat_map(|r| r.sets.iter().chain(r.start.iter().flat_map(|s| s.sets.iter())));
        let mut all: Vec<&str> = lists.flat_map(|(_, v)| v.iter().map(|v| v.odf.as_str())).collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// A warning for every named ODF that `exists` does not know, so a typo
    /// in a mod's table is reported when the mod loads, not as a missing moon.
    pub fn check(&self, exists: &mut dyn FnMut(&str) -> bool) -> Vec<Warning> {
        self.odfs().into_iter().filter(|o| !exists(o)).map(|o| Warning::new(FILE, format!("no {o}.odf; the generator's pick of it places nothing"))).collect()
    }

    /// The index of the set named `name`.
    pub fn set(&self, name: &str) -> Option<usize> {
        self.sets.iter().position(|s| s.name == name)
    }
}

/// A JSON5 object as its entries in the file's order (a map would sort them).
#[derive(Debug)]
struct Ordered<T>(Vec<(String, T)>);

impl<T> Default for Ordered<T> {
    fn default() -> Self {
        Ordered(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Ordered<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for V<T> {
            type Value = Ordered<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object of sets")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetFull {
    name: String,
    #[serde(default = "one")]
    weight: u32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SetDoc {
    Name(String),
    Full(SetFull),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VariantFull {
    odf: String,
    #[serde(default = "one")]
    weight: u32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum VariantDoc {
    Name(String),
    Full(VariantFull),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartDoc {
    inward: [i32; 2],
    #[serde(default)]
    centred: [i32; 2],
    #[serde(default)]
    sets: Ordered<Vec<VariantDoc>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceDoc {
    name: String,
    #[serde(default = "one")]
    weight: u32,
    #[serde(default)]
    clearance: i32,
    #[serde(default)]
    clearance_radii: i32,
    #[serde(default = "yes")]
    required: bool,
    #[serde(default)]
    start: Option<StartDoc>,
    sets: Ordered<Vec<VariantDoc>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Doc {
    sets: Vec<SetDoc>,
    #[serde(default = "stock_count")]
    count: [u32; 2],
    resources: Vec<ResourceDoc>,
}

fn one() -> u32 {
    1
}

fn yes() -> bool {
    true
}

fn stock_count() -> [u32; 2] {
    [super::RESOURCES.0, super::RESOURCES.1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/random_map").join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn the_stock_fixture_is_the_stock_table() {
        assert_eq!(Table::parse("stock.json5", &fixture("stock.json5")).unwrap(), Table::stock());
    }

    #[test]
    fn sets_fall_back_to_the_first() {
        let t = Table::parse("two_sets.json5", &fixture("two_sets.json5")).unwrap();
        assert_eq!(t.sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["normal", "frozen"]);
        let ore = t.resources.iter().find(|r| r.name == "ore").unwrap();
        assert_eq!(ore.variants("frozen").iter().map(|v| v.odf.as_str()).collect::<Vec<_>>(), ["fx_ore_ice_a", "fx_ore_ice_b"]);
        // Gas has no frozen names: its first set stands.
        let gas = t.resources.iter().find(|r| r.name == "gas").unwrap();
        assert_eq!(gas.variants("frozen"), gas.variants("normal"));
        // The start names are their own list where given, the set's otherwise.
        assert_eq!(ore.start_variants("normal").len(), 1);
        assert_eq!(ore.start_variants("frozen"), ore.variants("frozen"));
        assert!(t.odfs().contains(&"fx_bio_pool") && t.odfs().is_sorted());
        let missing = t.check(&mut |o| o != "fx_gas_b");
        assert_eq!(missing.len(), 1);
        assert!(missing[0].message.contains("fx_gas_b"));
    }

    #[test]
    fn bad_tables_are_errors_naming_the_file() {
        let base = "{ sets: ['a'], resources: [{ name: 'r', sets: { a: ['x'] } }] }";
        assert!(Table::parse("ok", base).is_ok());
        for (text, why) in [
            ("{ sets: ['a'], extra: 1, resources: [] }", "unknown"),
            ("{ sets: [], resources: [] }", "no set"),
            ("{ sets: ['a', 'a'], resources: [] }", "twice"),
            ("{ sets: [{ name: 'a', weight: 0 }], resources: [] }", "no set"),
            ("{ sets: ['a'], resources: [{ name: 'r', sets: { a: [] } }] }", "needs a name"),
            ("{ sets: ['a'], resources: [{ name: 'r', sets: { a: [{ odf: 'x', weight: 0 }] } }] }", "needs a name"),
            ("{ sets: ['a'], resources: [{ name: 'r', sets: { b: ['x'] } }] }", "not in `sets`"),
            ("{ sets: ['a'], resources: [{ name: 'r', sets: {} }] }", "no set of names"),
            ("{ sets: ['a'], resources: [{ name: 'r', sets: { a: ['x'] } }, { name: 'r', sets: { a: ['y'] } }] }", "twice"),
            ("{ sets: ['a'], resources: [{ name: 'r', weight: 0, sets: { a: ['x'] } }] }", "no resource"),
            ("{ sets: ['a'], count: [5, 2], resources: [{ name: 'r', sets: { a: ['x'] } }] }", "backwards"),
            ("{ sets: ['a'], resources: [{ name: 'r', clearance: -1, sets: { a: ['x'] } }] }", "below zero"),
        ] {
            let e = Table::parse("mod/random_map.json5", text).expect_err(text);
            assert_eq!(e.file, "mod/random_map.json5");
            assert!(e.to_string().contains(why), "{text}: {e}");
        }
    }

    #[test]
    fn names_lose_case_and_extension() {
        let t = Table::parse("t", "{ sets: ['a'], resources: [{ name: 'r', sets: { a: ['MdMoon.odf'] } }] }").unwrap();
        assert_eq!(t.resources[0].sets[0].1[0].odf, "mdmoon");
    }
}
