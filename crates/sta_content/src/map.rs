//! Maps as the sim and the renderer see them (S6). The types are the seam;
//! [`MapDef::from_bzn`] (CON-07) fills them from a parsed BZN. JSON5 maps
//! (native format) deserialize into the same types.
//!
//! The sim spawns from `objects`, `starts` and `regions` only. `scenery` is
//! render-only, in floats, and never enters the sim or the digest.
//!
//! # BZN conversion
//!
//! Space is the BZN's own (left-handed, +Y up), so nothing is mirrored.
//!
//! - **Seats.** BZN team `t` (1..=8) is seat `t - 1`: an object of team `t`
//!   gets `owner: Some(t - 1)` and the start location at index `t` gets
//!   `slot: t - 1`. Team 0 is neutral (`owner: None`).
//! - **Objects.** An object whose ODF the content database defines as a unit
//!   becomes a [`MapObject`], except `asteroidfield` class objects (the
//!   blocking fields and rock belts), which become [`RegionKind::AsteroidField`]
//!   regions (a sphere of the ODF's `fieldRadius`).
//! - **ODFs the database does not define** need an ODF lookup
//!   ([`MapDef::from_bzn_with`]): `nebula_obj` class becomes a
//!   [`RegionKind::Nebula`] region (sphere of `effectRadius`), `background_obj`
//!   named `map_planet*` a [`SceneryPlanet`] (its `radius` is the ODF's
//!   `ScaleSOD`, a factor the renderer multiplies by the model's own radius),
//!   a hidden `background_obj` named `*light*` a [`SceneryLight`]
//!   (directional: `pos` is 30000 units back along the marker's front axis).
//!   Other `background_obj` props (ice dust, wrecks), wormholes and unknown
//!   ODFs are counted in [`Counts`] and named in the warnings, not carried:
//!   [`Scenery`] has no place for props yet.
//! - **Starts.** A multiplayer map (`mission_type` starting `mp`) has its
//!   first `MPDStLoc` start locations; any other map (a mission) has every
//!   location not left at the default (the neutral entry's position, the
//!   map's centre). The BZN has no heading, so a start faces the map's centre.
//! - **Bounds** are `MPDMinExtent` and `MPDMinExtent + MPDSize`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_core::{Fixed, Vec3};

use crate::{ContentDb, Warning};

/// A map: what is in it at tick 0, and what it looks like.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapDef {
    /// File stem (`a2_borg01`), or `empty` for the built-in map.
    pub name: String,
    /// Display name, when the map has one.
    pub label: Option<String>,
    /// Mission script, resolved as docs/MISSIONS.md describes.
    pub mission: Option<String>,
    /// Playable volume: minimum and maximum corners.
    pub bounds: (Vec3, Vec3),
    /// Start positions, one per player slot.
    pub starts: Vec<StartPos>,
    pub objects: Vec<MapObject>,
    pub regions: Vec<Region>,
    pub scenery: Scenery,
}

/// Where a player slot starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartPos {
    pub slot: u8,
    pub pos: Vec3,
    /// Unit vector in XZ, `(x, z)`.
    pub heading: (Fixed, Fixed),
}

impl StartPos {
    /// `n` places for a seat's starting units: a square grid `spacing` apart,
    /// centred on the start position and turned to its heading, each place
    /// facing the heading; the place nearest the centre comes first, so one
    /// unit stands on the start itself. Fixed-point only, so every peer gets
    /// the same places.
    pub fn spawn_points(&self, n: usize, spacing: Fixed) -> Vec<(Vec3, (Fixed, Fixed))> {
        let width = (1..).find(|w: &i32| (w * w) as usize >= n.max(1)).unwrap_or(1);
        let (hx, hz) = self.heading;
        // Doubled coordinates keep the grid centred for even widths.
        let mut cells: Vec<(i32, i32)> = (0..width * width).map(|i| (i % width * 2 - (width - 1), i / width * 2 - (width - 1))).collect();
        cells.sort_by_key(|&(a, b)| (a * a + b * b, b, a));
        let half = spacing / Fixed::from_int(2);
        cells
            .into_iter()
            .take(n)
            .map(|(a, b)| {
                // `a` runs to the right of the heading, `b` along it.
                let (right, along) = (Fixed::from_int(a) * half, Fixed::from_int(b) * half);
                let pos = Vec3 { x: self.pos.x + hz * right + hx * along, y: self.pos.y, z: self.pos.z - hx * right + hz * along };
                (pos, self.heading)
            })
            .collect()
    }
}

/// A thing the map places.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapObject {
    /// ODF name without extension; a key into the content database.
    pub odf: String,
    /// Owning player slot; `None` for neutral things (moons, scenery props).
    pub owner: Option<u8>,
    pub pos: Vec3,
    /// Unit vector in XZ, `(x, z)`.
    pub heading: (Fixed, Fixed),
    /// The map's label for it (missions find objects by label).
    pub label: Option<String>,
}

/// What a region does to things inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RegionKind {
    Nebula,
    AsteroidField,
}

/// The volume of a region.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RegionShape {
    Sphere { centre: Vec3, radius: Fixed },
    Box { min: Vec3, max: Vec3 },
}

/// A volume with rules: nebulae, asteroid fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub kind: RegionKind,
    /// The ODF that defines its effects (which nebula, which field).
    pub odf: String,
    pub shape: RegionShape,
}

/// How a map looks. Floats: render only.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scenery {
    /// Skybox model (`BackgroundArt`).
    pub sky: Option<String>,
    /// Starfield ODF.
    pub starfield: Option<String>,
    /// Ambient light colour, linear 0..1.
    pub ambient: [f32; 3],
    pub planets: Vec<SceneryPlanet>,
    pub lights: Vec<SceneryLight>,
}

/// A planet or moon drawn as scenery (background planets, `sta_surface`
/// recipes); a moon that can be mined is a [`MapObject`] instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneryPlanet {
    pub odf: String,
    pub pos: [f32; 3],
    pub radius: f32,
}

/// A map light.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneryLight {
    pub pos: [f32; 3],
    /// Linear RGB.
    pub colour: [f32; 3],
}

/// Resolves an ODF name to its resolved ODF, for the classes the content
/// database does not define (nebulae, background objects, wormholes).
pub type OdfLookup<'a> = dyn FnMut(&str) -> Option<Arc<sta_odf::Odf>> + 'a;

/// What the conversion did with each BZN object. Every object lands in
/// exactly one count, so [`Counts::total`] equals the BZN's object count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// Became a [`MapObject`].
    pub objects: usize,
    /// Became a [`Region`].
    pub regions: usize,
    /// Became a [`SceneryPlanet`].
    pub planets: usize,
    /// Became a [`SceneryLight`].
    pub lights: usize,
    /// Background props: not carried (see the module docs).
    pub props: usize,
    /// Not carried: ODF unknown, or a class with no place in [`MapDef`].
    pub skipped: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.objects + self.regions + self.planets + self.lights + self.props + self.skipped
    }
}

/// [`MapDef::convert_bzn`]'s result.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    pub map: MapDef,
    pub warnings: Vec<Warning>,
    pub counts: Counts,
}

fn fx(v: f32) -> Fixed {
    Fixed::from_f64(if v.is_finite() { v as f64 } else { 0.0 })
}

fn vec3(p: [f32; 3]) -> Vec3 {
    Vec3 { x: fx(p[0]), y: fx(p[1]), z: fx(p[2]) }
}

/// A unit vector in XZ from `(x, z)`; `(0, 1)` when there is no direction.
fn unit_xz(x: f32, z: f32) -> (Fixed, Fixed) {
    let (x, z) = (if x.is_finite() { x as f64 } else { 0.0 }, if z.is_finite() { z as f64 } else { 0.0 });
    let len = (x * x + z * z).sqrt();
    if len < 1e-6 { (Fixed::ZERO, Fixed::ONE) } else { (Fixed::from_f64(x / len), Fixed::from_f64(z / len)) }
}

type Notes = std::collections::BTreeMap<(String, &'static str), usize>;

impl MapDef {
    /// Convert a parsed BZN with what the content database knows (CON-07).
    ///
    /// Without an ODF lookup only objects the database defines are placed;
    /// nebulae, planets and lights need [`MapDef::from_bzn_with`] or
    /// [`MapDef::from_bzn_in`].
    pub fn from_bzn(m: &sta_bzn::Map, db: &ContentDb) -> (MapDef, Vec<Warning>) {
        let c = MapDef::convert_bzn(m, db, &mut |_| None);
        (c.map, c.warnings)
    }

    /// [`MapDef::from_bzn`], looking up the ODFs the database lacks with `odfs`.
    pub fn from_bzn_with(m: &sta_bzn::Map, db: &ContentDb, odfs: &mut OdfLookup) -> (MapDef, Vec<Warning>) {
        let c = MapDef::convert_bzn(m, db, odfs);
        (c.map, c.warnings)
    }

    /// [`MapDef::from_bzn_with`] over an install: ODFs come from the index and
    /// the mounted files, layered as [`ContentDb::load`] reads them.
    pub fn from_bzn_in(m: &sta_bzn::Map, db: &ContentDb, vfs: &sta_vfs::Vfs, index: &crate::OdfIndex) -> (MapDef, Vec<Warning>) {
        let c = MapDef::convert_bzn_in(m, db, vfs, index);
        (c.map, c.warnings)
    }

    /// [`MapDef::from_bzn_in`] with its [`Counts`].
    pub fn convert_bzn_in(m: &sta_bzn::Map, db: &ContentDb, vfs: &sta_vfs::Vfs, index: &crate::OdfIndex) -> Conversion {
        Self::convert_bzn_in_context(m, None, db, vfs, index)
    }

    /// Convert the selected VFS map, using its stem relative to `bzn/` (without
    /// `.bzn`) for identity, diagnostics and A1's MDF companion. The internal
    /// BZN filename is never used for companion discovery. Existing unnamed
    /// conversion methods do not load companions.
    pub fn convert_bzn_named_in(m: &sta_bzn::Map, selected_stem: &str, db: &ContentDb, vfs: &sta_vfs::Vfs, index: &crate::OdfIndex) -> Conversion {
        Self::convert_bzn_in_context(m, Some(selected_stem), db, vfs, index)
    }

    fn convert_bzn_in_context(m: &sta_bzn::Map, selected_stem: Option<&str>, db: &ContentDb, vfs: &sta_vfs::Vfs, index: &crate::OdfIndex) -> Conversion {
        let mut cache: std::collections::HashMap<String, Option<Arc<sta_odf::Odf>>> = Default::default();
        let mut lookup = |name: &str| {
            let k = name.to_ascii_lowercase();
            cache.entry(k.clone()).or_insert_with(|| sta_odf::resolve_layered(&k, &mut |n, v| index.load(vfs, n, v)).ok().map(Arc::new)).clone()
        };
        let mut converted = Self::convert_bzn_context(m, selected_stem, db, &mut lookup);
        if let (Some(a1), Some(name)) = (&m.a1, selected_stem) {
            // Camera paths alone do not make a campaign map a multiplayer map.
            converted.map.starts.clear();
            let companion = format!("bzn/{name}.mdf");
            let Some(bytes) = vfs.read(&companion) else { return converted };
            let bytes = match bytes {
                Ok(bytes) => bytes,
                Err(error) => {
                    converted.warnings.push(Warning::new(&companion, format!("A1 starts unavailable: {error}")));
                    return converted;
                }
            };
            // CON-42: the original reader never fails (bad count -> eight) and a
            // team without a `Camera_i` path starts at the map centre XZ, mid
            // `SHIP_COMBAT_FLOOR`/`CEILING` Y (RTS_CFG.h; no fallback without both keys).
            let (lo, size) = (m.min_extent, m.size);
            let centre = [(lo[0] + (lo[0] + size[0])) * 0.5, (lo[2] + (lo[2] + size[2])) * 0.5];
            let combat_y = crate::layered::cfg_h(vfs, "RTS_CFG.h")
                .and_then(|c| Some((c.f32("SHIP_COMBAT_FLOOR")? + c.f32("SHIP_COMBAT_CEILING")?) * 0.5))
                .filter(|y| y.is_finite());
            let fallback = combat_y.filter(|_| centre.iter().all(|v| v.is_finite())).map(|y| [centre[0], y, centre[1]]);
            let count = sta_bzn::a1::native_mdf_start_count(Some(&bytes));
            match sta_bzn::a1::player_start_positions_with_fallback(a1, count, fallback) {
                Ok(points) => {
                    converted.map.starts = points.into_iter().enumerate().map(|(i, pos)| StartPos {
                        slot: i as u8,
                        pos: vec3(pos),
                        heading: unit_xz(centre[0] - pos[0], centre[1] - pos[2]),
                    }).collect();
                }
                Err(error) => converted.warnings.push(Warning::new(&companion, format!("A1 starts unavailable: {error}"))),
            }
        }
        converted
    }

    /// The conversion with its [`Counts`].
    pub fn convert_bzn(m: &sta_bzn::Map, db: &ContentDb, odfs: &mut OdfLookup) -> Conversion {
        Self::convert_bzn_context(m, None, db, odfs)
    }

    fn convert_bzn_context(m: &sta_bzn::Map, selected_stem: Option<&str>, db: &ContentDb, odfs: &mut OdfLookup) -> Conversion {
        let mut warnings = Vec::new();
        let mut counts = Counts::default();
        let mut map = MapDef::empty();
        let file = m.file_name.trim();
        map.name = selected_stem.map(str::to_owned).unwrap_or_else(|| match file.len().checked_sub(4).filter(|&i| file.is_char_boundary(i) && file[i..].eq_ignore_ascii_case(".bzn")) {
            Some(i) => file[..i].to_owned(),
            None => file.to_owned(),
        });
        let text = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
        map.label = text(&m.description);
        map.mission = m.scripts.iter().find_map(|s| text(s));

        // Bounds: the extent vectors.
        let lo = m.min_extent;
        let hi = [lo[0] + m.size[0], lo[1] + m.size[1], lo[2] + m.size[2]];
        if m.size.iter().all(|v| v.is_finite() && *v > 0.0) && lo.iter().all(|v| v.is_finite()) {
            map.bounds = (vec3(lo), vec3(hi));
        } else {
            warnings.push(Warning::new(&map.name, "no usable map extent; the empty map's bounds stand"));
        }

        // Scenery that is plain data.
        map.scenery.sky = text(&m.background);
        map.scenery.starfield = text(&m.starfield_odf).map(|s| s.strip_suffix(".odf").or_else(|| s.strip_suffix(".ODF")).unwrap_or(&s).to_owned());
        map.scenery.ambient = m.ambient.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });

        // Objects. Warnings are grouped by (ODF, reason) and counted.
        let mut notes = Notes::new();
        for o in &m.objects {
            let t = &o.transform;
            let pos = vec3([t[9], t[10], t[11]]);
            let key = db.key(&o.odf).filter(|&k| db.unit(k).is_some());
            // A resource marker is placed as the moon the game swaps it for.
            let moon = key.and_then(|k| db.unit(k)).and_then(|d| {
                let variants = moon_variants(db, d, odfs);
                (!variants.is_empty()).then(|| variants[(o.entity_id as u64 % variants.len() as u64) as usize])
            });
            let (key, placed_odf) = match moon.and_then(|k| db.unit(k)) {
                Some(m) => (moon, m.odf.clone()),
                None => (key, o.odf.clone()),
            };
            let (class, odf) = match key.and_then(|k| db.unit(k)) {
                Some(def) => (def.class.0.clone(), Some(Arc::clone(&def.raw))),
                None => {
                    let odf = odfs(&o.odf);
                    (odf.as_ref().and_then(|d| d.class_label()).unwrap_or("").trim().to_ascii_lowercase(), odf)
                }
            };
            let mut skip = |why: &'static str, n: &mut usize| {
                *n += 1;
                *notes.entry((o.odf.to_ascii_lowercase(), why)).or_default() += 1;
            };
            let Some(odf) = odf else {
                skip("no such ODF; not placed", &mut counts.skipped);
                continue;
            };
            match class.as_str() {
                "asteroidfield" => {
                    let radius = odf.f32("fieldRadius").filter(|r| *r > 0.0).unwrap_or(200.0);
                    map.regions.push(Region { kind: RegionKind::AsteroidField, odf: o.odf.clone(), shape: RegionShape::Sphere { centre: pos, radius: fx(radius) } });
                    counts.regions += 1;
                }
                "nebula_obj" => {
                    let radius = odf.f32("effectRadius").filter(|r| *r > 10.0 && *r < 5000.0).unwrap_or(300.0);
                    map.regions.push(Region { kind: RegionKind::Nebula, odf: o.odf.clone(), shape: RegionShape::Sphere { centre: pos, radius: fx(radius) } });
                    counts.regions += 1;
                }
                "background_obj" => {
                    let name = o.odf.to_ascii_lowercase();
                    if name.starts_with("map_planet") {
                        let radius = odf.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0);
                        map.scenery.planets.push(SceneryPlanet { odf: o.odf.clone(), pos: [t[9], t[10], t[11]], radius });
                        counts.planets += 1;
                    } else if name.contains("light") && odf.i64("hidden").unwrap_or(0) != 0 {
                        let back = |v: f32| if v.is_finite() { -v * 30000.0 } else { 0.0 };
                        map.scenery.lights.push(SceneryLight { pos: [back(t[6]), back(t[7]), back(t[8])], colour: o.light_colour.unwrap_or([1.0; 3]) });
                        counts.lights += 1;
                    } else {
                        skip("background prop; not carried (Scenery has no props)", &mut counts.props);
                    }
                }
                "wormhole_obj" => skip("wormhole; not carried (no map type for it yet)", &mut counts.skipped),
                _ if key.is_some() => {
                    map.objects.push(MapObject {
                        odf: placed_odf,
                        owner: if o.team > 0 { u8::try_from(o.team - 1).ok() } else { None },
                        pos,
                        heading: unit_xz(t[6], t[8]),
                        label: text(&o.label),
                    });
                    counts.objects += 1;
                }
                _ => skip("not a unit definition; not placed", &mut counts.skipped),
            }
        }
        for ((odf, why), n) in notes {
            warnings.push(Warning::new(&odf, format!("{}: {n} object(s): {why}", map.name)));
        }

        // Start locations: entry 0 is the neutral slot, entry n is seat n - 1.
        let multiplayer = m.mission_type.trim().to_ascii_lowercase().starts_with("mp");
        let default = m.start_locs.first().map(|l| l.pos);
        let limit = if multiplayer { m.start_count.max(0) as usize } else { usize::MAX };
        let centre = [(lo[0] + hi[0]) * 0.5, (lo[2] + hi[2]) * 0.5];
        for (i, loc) in m.start_locs.iter().enumerate().skip(1).take(limit.min(255)) {
            let at_default = default.is_some_and(|d| d[0] == loc.pos[0] && d[2] == loc.pos[2]);
            if (!multiplayer && at_default) || !loc.pos.iter().all(|v| v.is_finite()) {
                continue;
            }
            map.starts.push(StartPos { slot: (i - 1) as u8, pos: vec3(loc.pos), heading: unit_xz(centre[0] - loc.pos[0], centre[1] - loc.pos[2]) });
        }
        if multiplayer && map.starts.len() < m.start_count.max(0) as usize {
            warnings.push(Warning::new(&map.name, format!("{} start location(s) announced, {} found", m.start_count, map.starts.len())));
        }
        Conversion { map, warnings, counts }
    }

    /// The start position of seat `slot`, if the map has one.
    pub fn start_for(&self, slot: u8) -> Option<&StartPos> {
        self.starts.iter().find(|s| s.slot == slot)
    }

    /// The objects the map gives seat `slot` (a mission's starting forces).
    pub fn owned_by(&self, slot: u8) -> impl Iterator<Item = &MapObject> {
        self.objects.iter().filter(move |o| o.owner == Some(slot))
    }

    /// The bench's built-in empty map: nothing in it, a plain black sky.
    /// Bounds are 16384 across and 4096 high, about a stock 4-player map.
    pub fn empty() -> MapDef {
        let half = Vec3 { x: Fixed::from_int(8192), y: Fixed::from_int(2048), z: Fixed::from_int(8192) };
        MapDef {
            name: "empty".into(),
            label: None,
            mission: None,
            bounds: (Vec3 { x: -half.x, y: -half.y, z: -half.z }, half),
            starts: Vec::new(),
            objects: Vec::new(),
            regions: Vec::new(),
            scenery: Scenery::default(),
        }
    }
}

/// Class label of the weapon that swaps a marker for another object.
const REPLACE_CLASS: &str = "replaceweapon";

/// The moon definitions a resource marker (`trigger_dilithium`, ...) is swapped for when
/// the map starts: for each of its weapons that is a `replaceweapon`, the
/// `replacement0class<N>` ODFs (N from 0 up, until one is missing) that the database defines
/// as resource nodes. Empty for anything that is not a marker. The sim places the moon, so
/// the node is a resource whatever the marker's own class says.
pub fn moon_variants(db: &ContentDb, def: &crate::UnitDef, odfs: &mut OdfLookup) -> Vec<crate::ContentKey> {
    let mut out = Vec::new();
    // The weapon is read from the unit's own ODF keys: the database does not define
    // `replaceweapon`s, so they are not among its hardpoints.
    for n in 1..=8 {
        let Some(wname) = def.raw.str(&format!("weapon{n}")) else { continue };
        let Some(weapon) = odfs(wname.trim()) else { continue };
        if !weapon.class_label().is_some_and(|c| c.trim().eq_ignore_ascii_case(REPLACE_CLASS)) {
            continue;
        }
        for n in 0.. {
            let Some(name) = weapon.str(&format!("replacement0class{n}")) else { break };
            if let Some(k) = db.key(name.trim())
                && db.unit(k).is_some_and(|u| u.resource.is_some())
                && !out.contains(&k)
            {
                out.push(k);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_map_is_empty() {
        let m = MapDef::empty();
        assert!(m.starts.is_empty() && m.objects.is_empty() && m.regions.is_empty());
        assert!(m.bounds.0.x < m.bounds.1.x && m.bounds.0.y < m.bounds.1.y && m.bounds.0.z < m.bounds.1.z);
    }

    fn obj(odf: &str, team: i32, label: &str, at: [f32; 3], front: [f32; 3]) -> sta_bzn::MapObject {
        let mut t = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, front[0], front[1], front[2], at[0], at[1], at[2]];
        t[6..9].copy_from_slice(&front);
        sta_bzn::MapObject { odf: odf.into(), team, label: label.into(), transform: t, ..Default::default() }
    }

    fn loc(x: f32, z: f32) -> sta_bzn::StartLoc {
        sta_bzn::StartLoc { pos: [x, 0.0, z], kind: "player".into(), team: -1, race: String::new() }
    }

    /// ODFs by name for the lookups, as text.
    fn lookup(odfs: &'static [(&'static str, &'static str)]) -> impl FnMut(&str) -> Option<Arc<sta_odf::Odf>> {
        move |name| {
            let load = &mut |n: &str| odfs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n.trim_end_matches(".odf"))).map(|(_, v)| (*v).to_owned());
            sta_odf::resolve(name, load).ok().map(Arc::new)
        }
    }

    const ODFS: &[(&str, &str)] = &[
        ("fx_nebula", "[GameObjectClass]\nclassLabel = \"nebula_obj\"\neffectRadius = 450\n"),
        ("fx_field", "[GameObjectClass]\nclassLabel = \"asteroidfield\"\nfieldRadius = 125\n"),
        ("map_planet_x", "[GameObjectClass]\nclassLabel = \"background_obj\"\nScaleSOD = 0.6\n"),
        ("dlight", "[GameObjectClass]\nclassLabel = \"background_obj\"\nhidden = 1\n"),
        ("map_dust", "[GameObjectClass]\nclassLabel = \"background_obj\"\n"),
        ("map_wormhole", "[GameObjectClass]\nclassLabel = \"wormhole_obj\"\n"),
    ];

    fn sample() -> sta_bzn::Map {
        sta_bzn::Map {
            file_name: "Sample.BZN".into(),
            description: "A Sample".into(),
            mission_type: "mp-regular".into(),
            scripts: vec!["sample.lua".into()],
            min_extent: [0.0, -500.0, 0.0],
            size: [10000.0, 1000.0, 10000.0],
            ambient: [0.25, 0.5, 2.0],
            background: "env_blue".into(),
            starfield_odf: "stars_default.odf".into(),
            start_count: 2,
            start_locs: vec![loc(5000.0, 5000.0), loc(1000.0, 5000.0), loc(9000.0, 5000.0), loc(5000.0, 5000.0)],
            objects: vec![
                obj("fx_moon", 0, "moon", [10.0, 0.0, 20.0], [0.0, 0.0, 1.0]),
                obj("fx_station", 2, "", [30.0, 5.0, 40.0], [3.0, 0.0, 4.0]),
                obj("fx_nebula", 0, "", [100.0, 0.0, 200.0], [0.0, 0.0, 1.0]),
                obj("fx_field", 0, "", [300.0, 0.0, 400.0], [0.0, 0.0, 1.0]),
                obj("map_planet_x", 0, "", [20000.0, -3000.0, 15000.0], [0.0, 0.0, 1.0]),
                obj("dlight", 0, "", [0.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
                obj("map_dust", 0, "", [1.0, 2.0, 3.0], [0.0, 0.0, 1.0]),
                obj("map_wormhole", 0, "", [1.0, 2.0, 3.0], [0.0, 0.0, 1.0]),
                obj("nonesuch", 0, "", [1.0, 2.0, 3.0], [0.0, 0.0, 1.0]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn from_bzn_sorts_objects_regions_and_scenery() {
        let db = crate::fake::db();
        let c = MapDef::convert_bzn(&sample(), &db, &mut lookup(ODFS));
        let m = &c.map;
        assert_eq!((m.name.as_str(), m.label.as_deref(), m.mission.as_deref()), ("Sample", Some("A Sample"), Some("sample.lua")));
        assert_eq!(m.bounds.1, Vec3 { x: Fixed::from_int(10000), y: Fixed::from_int(500), z: Fixed::from_int(10000) });
        assert_eq!(c.counts, Counts { objects: 2, regions: 2, planets: 1, lights: 1, props: 1, skipped: 2 });
        assert_eq!(c.counts.total(), 9);
        // Objects in map order; team 2 is seat 1; team 0 is neutral.
        assert_eq!(m.objects.iter().map(|o| (o.odf.as_str(), o.owner, o.label.as_deref())).collect::<Vec<_>>(), [("fx_moon", None, Some("moon")), ("fx_station", Some(1), None)]);
        assert_eq!(m.objects[1].heading, (Fixed::from_f64(0.6), Fixed::from_f64(0.8)));
        assert_eq!(m.objects[1].pos.y, Fixed::from_int(5));
        assert_eq!(m.regions[0].kind, RegionKind::Nebula);
        assert_eq!(m.regions[0].shape, RegionShape::Sphere { centre: Vec3 { x: Fixed::from_int(100), y: Fixed::ZERO, z: Fixed::from_int(200) }, radius: Fixed::from_int(450) });
        assert_eq!(m.regions[1].kind, RegionKind::AsteroidField);
        assert!(matches!(m.regions[1].shape, RegionShape::Sphere { radius, .. } if radius == Fixed::from_int(125)));
        // Scenery.
        assert_eq!(m.scenery.sky.as_deref(), Some("env_blue"));
        assert_eq!(m.scenery.starfield.as_deref(), Some("stars_default"));
        assert_eq!(m.scenery.ambient, [0.25, 0.5, 1.0]);
        assert_eq!(m.scenery.planets, [SceneryPlanet { odf: "map_planet_x".into(), pos: [20000.0, -3000.0, 15000.0], radius: 0.6 }]);
        assert_eq!(m.scenery.lights, [SceneryLight { pos: [0.0, 30000.0, 0.0], colour: [1.0; 3] }]);
        // The props, the wormhole and the unknown ODF are named.
        let notes: Vec<String> = c.warnings.iter().map(|w| w.to_string()).collect();
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes.iter().any(|n| n.starts_with("map_dust:") && n.contains("prop")));
        assert!(notes.iter().any(|n| n.starts_with("nonesuch:") && n.contains("no such ODF")));
    }

    /// A map places `trigger_dilithium` (a craft-class marker); the game swaps it for one of
    /// its moons (class `scrap`, with a `resource`), so the map places the moon, by the object's id.
    #[test]
    fn a_resource_marker_is_placed_as_its_moon() {
        use crate::UnitDef;
        let mut marker = UnitDef::new("trigger_dilithium", "craft");
        marker.raw = Arc::new(sta_odf::resolve("trigger_dilithium", &mut |_| Some("weapon1 = \"triggerW_dilithium\"\nclassLabel = \"craft\"\n".to_owned())).unwrap());
        let mut moons = [UnitDef::new("map_dilithium_moon1", "scrap"), UnitDef::new("map_dilithium_moon2", "scrap")];
        moons.iter_mut().for_each(|m| m.resource = Some(0));
        let [m1, m2] = moons;
        let db = ContentDb::builder().unit(marker).unit(m1).unit(m2).build().unwrap();
        const W: &[(&str, &str)] = &[(
            "triggerW_dilithium",
            "classLabel = \"replaceweapon\"\nreplacement0class0 = \"map_dilithium_moon1\"\nreplacement0class1 = \"map_dilithium_moon2\"\n",
        )];
        let mut o0 = obj("trigger_dilithium", 0, "", [10.0, 0.0, 20.0], [0.0, 0.0, 1.0]);
        let mut o1 = o0.clone();
        (o0.entity_id, o1.entity_id) = (4, 7);
        let map = sta_bzn::Map { file_name: "m.bzn".into(), objects: vec![o0, o1], ..Default::default() };
        let c = MapDef::convert_bzn(&map, &db, &mut lookup(W));
        let placed: Vec<_> = c.map.objects.iter().map(|o| o.odf.as_str()).collect();
        assert_eq!(placed, ["map_dilithium_moon1", "map_dilithium_moon2"]);
    }

    #[test]
    fn from_bzn_without_a_lookup_places_the_database_units_only() {
        let db = crate::fake::db();
        let (m, w) = MapDef::from_bzn(&sample(), &db);
        assert_eq!(m.objects.len(), 2);
        assert!(m.regions.is_empty() && m.scenery.planets.is_empty());
        assert_eq!(w.len(), 7, "{w:?}");
        assert_eq!(MapDef::from_bzn(&sta_bzn::Map { file_name: "x.bzn".into(), ..Default::default() }, &db).0.name, "x");
    }

    /// Entry 0 is neutral; entry n is seat n - 1. A multiplayer map takes the
    /// announced count, and each start faces the map's centre.
    #[test]
    fn multiplayer_starts_follow_the_count() {
        let db = crate::fake::db();
        let m = MapDef::from_bzn(&sample(), &db).0;
        assert_eq!(m.starts.len(), 2);
        assert_eq!((m.starts[0].slot, m.starts[1].slot), (0, 1));
        assert_eq!(m.starts[0].pos.x, Fixed::from_int(1000));
        assert_eq!(m.starts[0].heading, (Fixed::ONE, Fixed::ZERO));
        assert_eq!(m.starts[1].heading, (-Fixed::ONE, Fixed::ZERO));
        assert_eq!(m.start_for(1).map(|s| s.pos.x), Some(Fixed::from_int(9000)));
        assert!(m.start_for(2).is_none());
        // Announced more than there are: a warning.
        let mut b = sample();
        b.start_count = 5;
        let (m, w) = MapDef::from_bzn(&b, &db);
        assert_eq!(m.starts.len(), 3);
        assert!(w.iter().any(|w| w.message.contains("5 start location(s) announced, 3 found")));
    }

    /// A mission has no useful count: every entry off the default is a start.
    #[test]
    fn mission_starts_skip_the_default_position() {
        let db = crate::fake::db();
        let mut b = sample();
        b.mission_type = "mission".into();
        b.start_count = 1;
        let m = MapDef::from_bzn(&b, &db).0;
        assert_eq!(m.starts.iter().map(|s| s.slot).collect::<Vec<_>>(), [0, 1]);
        b.start_locs.clear();
        assert!(MapDef::from_bzn(&b, &db).0.starts.is_empty());
    }

    #[test]
    fn owned_objects_by_seat() {
        let db = crate::fake::db();
        let m = MapDef::from_bzn(&sample(), &db).0;
        assert_eq!(m.owned_by(1).map(|o| o.odf.as_str()).collect::<Vec<_>>(), ["fx_station"]);
        assert_eq!(m.owned_by(0).count(), 0);
    }

    #[test]
    fn spawn_points_form_a_grid_around_the_start() {
        let s = StartPos { slot: 0, pos: Vec3 { x: Fixed::from_int(100), y: Fixed::from_int(2), z: Fixed::from_int(50) }, heading: (Fixed::ZERO, Fixed::ONE) };
        let one = s.spawn_points(1, Fixed::from_int(40));
        assert_eq!(one, [(s.pos, s.heading)]);
        let four = s.spawn_points(4, Fixed::from_int(40));
        assert_eq!(four.len(), 4);
        // Facing +z: right is +x. Two across, two along, 40 apart.
        let xs: Vec<i32> = four.iter().map(|p| p.0.x.to_f32().round() as i32).collect();
        let zs: Vec<i32> = four.iter().map(|p| p.0.z.to_f32().round() as i32).collect();
        assert_eq!(xs.iter().min(), Some(&80));
        assert_eq!(xs.iter().max(), Some(&120));
        assert_eq!(zs.iter().min(), Some(&30));
        assert_eq!(zs.iter().max(), Some(&70));
        assert!(four.iter().all(|p| p.0.y == s.pos.y && p.1 == s.heading));
        // Nine: the first is the centre.
        let nine = s.spawn_points(9, Fixed::from_int(40));
        assert_eq!(nine[0].0, s.pos);
        assert_eq!(nine.len(), 9);
        assert!(s.spawn_points(0, Fixed::ONE).is_empty());
    }

    /// Native maps are JSON5 with plain numbers where the sim has `Fixed`.
    #[test]
    fn deserializes_from_json5() {
        let text = r#"{
            name: "tiny", label: null, mission: null,
            bounds: [{x: -100, y: -10, z: -100}, {x: 100, y: 10, z: 100}],
            starts: [{slot: 0, pos: {x: 50.5, y: 0, z: 0}, heading: [0, 1]}],
            objects: [], regions: [],
            scenery: {sky: null, starfield: null, ambient: [0, 0, 0], planets: [], lights: []},
        }"#;
        let m: MapDef = json5::from_str(text).unwrap();
        assert_eq!(m.starts[0].pos.x, Fixed::from_f64(50.5));
        assert_eq!(m.starts[0].heading, (Fixed::ZERO, Fixed::ONE));
    }
}
