//! Maps as the sim and the renderer see them (S6). The types are the seam;
//! CON-07 writes [`MapDef::from_bzn`]. JSON5 maps (native format)
//! deserialize into the same types.
//!
//! The sim spawns from `objects`, `starts` and `regions` only. `scenery` is
//! render-only, in floats, and never enters the sim or the digest.

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

impl MapDef {
    /// Convert a parsed BZN (CON-07; not yet written: returns the empty map
    /// under the BZN's name and an unsupported warning).
    pub fn from_bzn(m: &sta_bzn::Map, db: &ContentDb) -> (MapDef, Vec<Warning>) {
        let _ = db;
        let mut map = MapDef::empty();
        map.name = m.file_name.trim_end_matches(".bzn").to_owned();
        (map, vec![Warning::new(&m.file_name, "unsupported: MapDef::from_bzn (CON-07)")])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_map_is_empty() {
        let m = MapDef::empty();
        assert!(m.starts.is_empty() && m.objects.is_empty() && m.regions.is_empty());
        assert!(m.bounds.0.x < m.bounds.1.x && m.bounds.0.y < m.bounds.1.y && m.bounds.0.z < m.bounds.1.z);
    }

    #[test]
    fn from_bzn_is_unsupported() {
        let bzn = sta_bzn::Map { file_name: "x.bzn".into(), ..Default::default() };
        let db = ContentDb::builder().build().unwrap();
        let (m, w) = MapDef::from_bzn(&bzn, &db);
        assert_eq!(m.name, "x");
        assert!(w[0].message.starts_with("unsupported"));
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
