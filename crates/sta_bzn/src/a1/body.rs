//! The known A1 binary map layout. Native per-class object payload is not a save-game import.

use super::Header;
use crate::{BznError, Map, MapObject, Rec, records, strip_odf};

/// Original A1 source information not yet consumed by mission/AI adapters.
#[derive(Debug, Clone, PartialEq)]
pub struct MapData {
    pub header: Header,
    pub terrain_name: String,
    /// Minimum and maximum before an optional editor resize.
    pub source_bounds: ([f32; 3], [f32; 3]),
    pub resized: bool,
    pub declared_object_count: usize,
    /// Serialized reference tokens in the same order as `Map::objects`.
    /// These are data, never addresses to dereference in this process.
    pub object_refs: Vec<u32>,
    /// Resized prefix positions used by the native factory before the final matrix.
    pub initial_positions: Vec<[f32; 3]>,
    pub mission_title: [u8; 64],
    pub paths: Vec<AiPath>,
    pub areas: Vec<AreaOfInterest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiPath {
    /// Serialized `old_ptr` token, never a host pointer.
    pub reference: u32,
    pub label: String,
    pub points: Vec<[f32; 3]>,
    pub path_type: i32,
    pub asteroid_belt_speed: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaOfInterest {
    pub object_ref: u32,
    pub team: i32,
    pub interesting: bool,
    pub inside: bool,
    pub value: i32,
    pub force: i32,
}

fn invalid(record: usize, expected: &'static str) -> BznError {
    BznError::InvalidArmada1Body { record, expected }
}

fn is(rec: &Rec<'_>, kind: u32, size: usize) -> bool {
    rec.kind & 255 == kind && rec.data.len() == size
}

fn text(bytes: &[u8]) -> Result<String, BznError> {
    let bytes = &bytes[..bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len())];
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| BznError::UnsupportedArmada1Feature("non-UTF-8 text before NUL"))
}

struct Reader<'r, 'd> {
    recs: &'r [Rec<'d>],
    at: usize,
}

impl<'r, 'd> Reader<'r, 'd> {
    fn take(&mut self, kind: u32, size: Option<usize>, expected: &'static str) -> Result<Rec<'d>, BznError> {
        let r = self
            .recs
            .get(self.at)
            .filter(|r| r.kind & 255 == kind && size.is_none_or(|n| r.data.len() == n))
            .copied()
            .ok_or_else(|| invalid(self.at, expected))?;
        self.at += 1;
        Ok(r)
    }

    fn bytes(&mut self, kind: u32, size: usize, expected: &'static str) -> Result<&'d [u8], BznError> {
        Ok(self.take(kind, Some(size), expected)?.data)
    }

    fn string(&mut self, kind: u32, size: Option<usize>, expected: &'static str) -> Result<String, BznError> {
        text(self.take(kind, size, expected)?.data)
    }

    fn word(&mut self, kind: u32, expected: &'static str) -> Result<u32, BznError> {
        Ok(u32::from_le_bytes(self.bytes(kind, 4, expected)?.try_into().unwrap()))
    }

    fn integer(&mut self, expected: &'static str) -> Result<i32, BznError> {
        Ok(self.word(4, expected)? as i32)
    }

    fn count(&mut self, expected: &'static str) -> Result<usize, BznError> {
        let at = self.at;
        let n = self.integer(expected)?;
        if n < 0 || n as usize > self.recs.len().saturating_sub(self.at) {
            return Err(invalid(at, expected));
        }
        Ok(n as usize)
    }

    fn boolean(&mut self, expected: &'static str) -> Result<bool, BznError> {
        Ok(self.bytes(1, 1, expected)?[0] != 0)
    }

    fn scalar(&mut self, expected: &'static str) -> Result<f32, BznError> {
        let at = self.at;
        let f = f32::from_le_bytes(self.bytes(5, 4, expected)?.try_into().unwrap());
        if !f.is_finite() {
            return Err(invalid(at, expected));
        }
        Ok(f)
    }

    fn bounds(&mut self) -> Result<Bounds, BznError> {
        let at = self.at;
        let mut f = [0.0; 6];
        for v in &mut f {
            *v = self.scalar("finite bound scalar")?;
        }
        let b = Bounds {
            min: [f[0], f[2], f[4]],
            max: [f[1], f[3], f[5]],
        };
        if (0..3).any(|i| b.max[i] <= b.min[i] || !(b.max[i] - b.min[i]).is_finite()) {
            return Err(invalid(at, "finite positive bounds"));
        }
        Ok(b)
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
}

struct Coordinates {
    source: Bounds,
    target: Bounds,
    resized: bool,
}

impl Coordinates {
    fn point(&self, mut p: [f32; 3], at: usize) -> Result<[f32; 3], BznError> {
        if !p.iter().all(|v| v.is_finite()) {
            return Err(invalid(at, "finite position"));
        }
        if self.resized {
            for axis in [0, 2] {
                let (lo, hi) = (f64::from(self.source.min[axis]), f64::from(self.source.max[axis]));
                let (new_lo, new_hi) = (f64::from(self.target.min[axis]), f64::from(self.target.max[axis]));
                p[axis] = (new_lo + (f64::from(p[axis]) - lo) / (hi - lo) * (new_hi - new_lo)) as f32;
            }
        }
        if !p.iter().all(|v| v.is_finite()) {
            return Err(invalid(at, "finite resized position"));
        }
        Ok(p)
    }
}

fn floats<const N: usize>(bytes: &[u8], at: usize) -> Result<[f32; N], BznError> {
    let mut out = [0.0; N];
    for (v, b) in out.iter_mut().zip(bytes.chunks_exact(4)) {
        *v = f32::from_le_bytes(b.try_into().unwrap());
    }
    if !out.iter().all(|v| v.is_finite()) {
        return Err(invalid(at, "finite vector or transform"));
    }
    Ok(out)
}

fn placement(recs: &[Rec<'_>]) -> bool {
    recs.len() >= 7
        && is(&recs[0], 7, 8)
        && is(&recs[1], 4, 4)
        && is(&recs[2], 9, 12)
        && is(&recs[3], 4, 4)
        && is(&recs[4], 2, 40)
        && is(&recs[5], 8, 4)
        && is(&recs[6], 11, 48)
}

fn mission(recs: &[Rec<'_>]) -> bool {
    is(&recs[0], 2, 40) && is(&recs[1], 8, 4) && is(&recs[2], 2, 64) && is(&recs[3], 2, 64) && is(&recs[4], 0, 64)
}

pub(crate) fn parse_map(data: &[u8], header: Header) -> Result<Map, BznError> {
    if !header.binary_save {
        return Err(BznError::UnsupportedArmada1Import {
            version: header.version,
        });
    }
    let recs = records(data, header.metadata_offset)?;
    let mut r = Reader { recs: &recs, at: 0 };
    let file_name = r.string(2, Some(16), "msn_filename text16")?;
    if !r.boolean("missionSave bool")? {
        return Err(BznError::UnsupportedArmada1Feature(
            "saved-game state (missionSave=false)",
        ));
    }
    let terrain_name = r.string(2, Some(100), "TerrainName text100")?;
    let source = r.bounds()?;
    let resized = r.boolean("resize bool")?;
    let target = if resized { r.bounds()? } else { source };
    let coords = Coordinates {
        source,
        target,
        resized,
    };
    let ambient = [
        r.scalar("ambient red")?,
        r.scalar("ambient green")?,
        r.scalar("ambient blue")?,
    ];
    let background = r.string(2, Some(100), "BackgroundArt text100")?;
    let tech_tree_file = r.string(2, Some(128), "techTreeFilename text128")?;
    let race = r.string(2, Some(128), "race text128")?;
    let declared_object_count = r.count("nonnegative object count bounded by remaining records")?;
    let objects_at = r.at;
    let mut markers = recs
        .windows(5)
        .enumerate()
        .skip(objects_at)
        .filter(|(_, window)| mission(window));
    let mission_at = markers
        .next()
        .map(|(at, _)| at)
        .ok_or_else(|| invalid(objects_at, "mission descriptor"))?;
    if let Some((at, _)) = markers.next() {
        return Err(invalid(at, "one unambiguous mission descriptor"));
    }

    let mut indices = Vec::new();
    for at in objects_at..mission_at {
        if at + 7 <= mission_at && placement(&recs[at..at + 7]) {
            indices.push(at);
        }
    }
    if declared_object_count != indices.len() {
        return Err(BznError::Armada1ObjectCount {
            declared: declared_object_count,
            found: indices.len(),
        });
    }
    if indices.first().copied().unwrap_or(mission_at) != objects_at {
        return Err(invalid(
            objects_at,
            "object list begins with placement or empty mission block",
        ));
    }
    let mut objects = Vec::with_capacity(indices.len());
    let mut object_refs = Vec::with_capacity(indices.len());
    let mut initial_positions = Vec::with_capacity(indices.len());
    for at in indices {
        let mut o = Reader { recs: &recs, at };
        let odf = strip_odf(&o.string(7, Some(8), "PrjID eight bytes")?);
        if odf.is_empty() {
            return Err(invalid(at, "nonempty PrjID"));
        }
        let entity_id = o.integer("EntityID")?;
        let pos_at = o.at;
        let position = floats::<3>(o.bytes(9, 12, "position vector3")?, pos_at)?;
        let position = coords.point(position, pos_at)?;
        initial_positions.push(position);
        let team = o.integer("team")?;
        let label = o.string(2, Some(40), "label text40")?;
        object_refs.push(o.word(8, "obj_addr reference")?);
        let matrix_at = o.at;
        let mut transform = floats::<12>(o.bytes(11, 48, "transform Matrix34")?, matrix_at)?;
        let p = coords.point([transform[9], transform[10], transform[11]], matrix_at)?;
        transform[9..12].copy_from_slice(&p);
        objects.push(MapObject {
            odf,
            entity_id,
            team,
            label,
            initial_position: Some(position),
            transform,
            light_colour: None,
        });
    }

    r.at = mission_at;
    let mission_type = r.string(2, Some(40), "mission class text40")?;
    if !matches!(mission_type.as_str(), "EmptyMission" | "Inst4XMission") {
        return Err(BznError::UnsupportedArmada1Feature(
            "mission class (expected EmptyMission or Inst4XMission)",
        ));
    }
    r.word(8, "mission reference")?;
    let scripts = [
        r.string(2, Some(64), "scriptName text64")?,
        r.string(2, Some(64), "ruleName text64")?,
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    let mut mission_title = [0; 64];
    mission_title.copy_from_slice(r.bytes(0, 64, "missionTitle raw64")?);
    let description = text(&mission_title)?;

    let area_count = r.count("nonnegative AOI count bounded by remaining records")?;
    if area_count > recs.len().saturating_sub(r.at) / 6 {
        return Err(invalid(r.at, "complete AOI list"));
    }
    let mut areas = Vec::with_capacity(area_count);
    for _ in 0..area_count {
        areas.push(AreaOfInterest {
            object_ref: r.word(8, "AOI object reference")?,
            team: r.integer("AOI team")?,
            interesting: r.boolean("AOI interesting")?,
            inside: r.boolean("AOI inside")?,
            value: r.integer("AOI value")?,
            force: r.integer("AOI force")?,
        });
    }
    let path_count = r.count("nonnegative AiPath count bounded by remaining records")?;
    if path_count > recs.len().saturating_sub(r.at) / 6 {
        return Err(invalid(r.at, "complete AiPath list"));
    }
    let mut paths = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        let reference = r.word(0, "AiPath old_ptr raw4")?;
        let label_at = r.at;
        let label_len = r.integer("AiPath label length")?;
        if label_len < 0 {
            return Err(invalid(label_at, "nonnegative AiPath label length"));
        }
        let label = if label_len == 0 {
            String::new()
        } else {
            r.string(2, Some(label_len as usize), "AiPath label")?
        };
        let point_at = r.at;
        let n = r.integer("AiPath point count")?;
        let len = usize::try_from(n)
            .ok()
            .and_then(|n| n.checked_mul(12))
            .ok_or_else(|| invalid(point_at, "nonnegative bounded AiPath point count"))?;
        let payload_at = r.at;
        let bytes = r.bytes(9, len, "AiPath point array")?;
        let mut points = Vec::with_capacity(bytes.len() / 12);
        for b in bytes.chunks_exact(12) {
            points.push(coords.point(floats::<3>(b, payload_at)?, payload_at)?);
        }
        let path_type = r.word(0, "AiPath pathType raw4")? as i32;
        let asteroid_belt_speed = r.scalar("AiPath asteroid_belt_speed")?;
        paths.push(AiPath {
            reference,
            label,
            points,
            path_type,
            asteroid_belt_speed,
        });
    }
    if r.at != recs.len() {
        return Err(invalid(r.at, "end of map after AiPaths"));
    }
    let min_extent = target.min;
    let size = [0, 1, 2].map(|i| target.max[i] - target.min[i]);
    Ok(Map {
        version: header.version,
        binary: true,
        mission_type,
        description,
        file_name,
        tech_tree: tech_tree_file.clone(),
        tech_tree_file,
        race,
        min_extent,
        size,
        ambient,
        background,
        objects,
        scripts,
        a1: Some(MapData {
            header,
            terrain_name,
            source_bounds: (source.min, source.max),
            resized,
            declared_object_count,
            object_refs,
            initial_positions,
            mission_title,
            paths,
            areas,
        }),
        ..Default::default()
    })
}
