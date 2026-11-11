//! Armada / Fleet Operations `.bzn` maps.
//!
//! A BZN starts with two text lines, `version [1] =` and `BinaryMode [1] =`,
//! and then has one of two bodies:
//!
//! - **Text**: `name [n] =` followed by the value lines, nested fields indented
//!   (`  x [1] =`). Strings are hex after a length field (`PrjIdLen [1] = 6`,
//!   `PrjID = 4d62616b7500`) or plain (`label = Baku`). `[GameObject]` starts
//!   each object.
//! - **Binary**: the same fields in the same order without names, each one a
//!   record `u32 type, u32 size, data`. Types: 0 raw string, 1 bool, 2 text or
//!   fixed buffer, 3 i16, 4 i32, 5 f32, 8 id, 9 vector3, 11 struct (a transform
//!   is right, up, front, position: 12 floats).
//!
//! The binary header differs between map versions, so binary maps are read by
//! version-independent anchors rather than strictly by position; text maps by
//! name. Both give the same [`Map`].
//!
//! Positions and transforms are Direct3D left-handed, like SOD.
//!
//! A1 front matter can be inspected through [`a1::inspect_header`]. Known binary
//! map bodies share [`Map`]; text A1 and saved-game state remain unsupported.

use std::collections::HashMap;

pub mod a1;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BznError {
    #[error("not a BZN file")]
    NotBzn,
    #[error("truncated record at byte {0}")]
    Truncated(usize),
    #[error(transparent)]
    Armada1Header(#[from] a1::HeaderError),
    #[error("Armada 1 text BZN import is not implemented (version {version})")]
    UnsupportedArmada1Import { version: u32 },
    #[error("unsupported Armada 1 BZN feature: {0}")]
    UnsupportedArmada1Feature(&'static str),
    #[error("invalid Armada 1 BZN record {record}: expected {expected}")]
    InvalidArmada1Body { record: usize, expected: &'static str },
    #[error("Armada 1 BZN declares {declared} objects, found {found} placement records")]
    Armada1ObjectCount { declared: usize, found: usize },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapObject {
    /// The ODF name (`PrjID`), without extension.
    pub odf: String,
    pub entity_id: i32,
    pub team: i32,
    pub label: String,
    /// Prefix `pos`, separate from the final transform; absent when unavailable.
    /// Uses the returned map's coordinates, including A1 resize conversion.
    pub initial_position: Option<[f32; 3]>,
    /// Right, up, front, position: the same layout as a SOD `Matrix34`.
    pub transform: [f32; 12],
    /// Light objects (`dlight`): `lightColorRed/Green/Blue`, text maps only for now.
    pub light_colour: Option<[f32; 3]>,
}

/// A player start location (`SLPos`, `SLType`, `SLLkTeam`, `SLLkRace` in text
/// maps; a fixed run of records in binary ones).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StartLoc {
    pub pos: [f32; 3],
    /// `player`, `ai-easy`, `ai-medium`, `ai-hard`, `empty` or empty.
    pub kind: String,
    /// Locked team (`SLLkTeam`), -1 when free.
    pub team: i32,
    /// Locked race (`SLLkRace`): `race-unspecified`, a race name, or empty.
    pub race: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Map {
    pub version: u32,
    pub binary: bool,
    pub mission_type: String,
    pub description: String,
    pub file_name: String,
    pub tech_tree: String,
    /// Mission script or rules named by the map (`.dll`, `.drl`, `.dsl`, `.lua`).
    pub scripts: Vec<String>,
    pub min_extent: [f32; 3],
    pub size: [f32; 3],
    pub ambient: [f32; 3],
    /// Skybox SOD (`BackgroundArt`).
    pub background: String,
    pub starfield_odf: String,
    pub tech_tree_file: String,
    pub race: String,
    /// `MPDStLoc`: how many start locations a multiplayer map offers.
    pub start_count: i32,
    /// Every start location record, in file order. Entry 0 is the neutral
    /// slot (team 0) and entry `n` belongs to team `n`; unused entries sit at
    /// the entry-0 position (the map's centre).
    pub start_locs: Vec<StartLoc>,
    pub objects: Vec<MapObject>,
    /// Retained A1-specific source data; absent for existing A2 binary/text maps.
    pub a1: Option<a1::MapData>,
}

pub fn parse(data: &[u8]) -> Result<Map, BznError> {
    if !data.starts_with(b"version") {
        return Err(BznError::NotBzn);
    }
    if a1::has_header_marker(data) {
        let header = a1::inspect_header(data)?;
        return a1::parse_map(data, header);
    }
    let header_end = |key: &[u8]| data.windows(key.len()).position(|w| w == key);
    let version = text_value(data, b"version [1] =").and_then(|v| v.parse().ok()).unwrap_or(0);
    let binary = text_value(data, b"BinaryMode [1] =").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let mut map = if binary {
        let start = header_end(b"true").map(|i| i + 4).ok_or(BznError::NotBzn)?;
        let start = start + data[start..].iter().take_while(|&&c| c == b'\r' || c == b'\n').count();
        parse_binary(&records(data, start)?)
    } else {
        parse_text(&String::from_utf8_lossy(data))
    };
    map.version = version;
    map.binary = binary;
    Ok(map)
}

/// The value line after a text header line.
fn text_value(data: &[u8], key: &[u8]) -> Option<String> {
    let i = data.windows(key.len()).position(|w| w == key)? + key.len();
    let rest = &data[i..];
    let rest = &rest[rest.iter().take_while(|&&c| c == b'\r' || c == b'\n').count()..];
    let end = rest.iter().position(|&c| c == b'\r' || c == b'\n').unwrap_or(rest.len());
    Some(String::from_utf8_lossy(&rest[..end]).trim().to_owned())
}

// ---------------------------------------------------------------- binary

#[derive(Debug, Clone, Copy)]
struct Rec<'a> {
    kind: u32,
    data: &'a [u8],
}

impl Rec<'_> {
    fn i32(&self) -> Option<i32> {
        (self.data.len() == 4).then(|| i32::from_le_bytes(self.data.try_into().unwrap()))
    }
    fn i16(&self) -> Option<i16> {
        (self.data.len() == 2).then(|| i16::from_le_bytes(self.data.try_into().unwrap()))
    }
    fn floats(&self) -> Vec<f32> {
        self.data.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
    }
    fn text(&self) -> String {
        let end = self.data.iter().position(|&c| c == 0).unwrap_or(self.data.len());
        String::from_utf8_lossy(&self.data[..end]).into_owned()
    }
    fn is_string(&self) -> bool {
        self.kind == 0 || self.kind == 2
    }
}

fn records(data: &[u8], mut at: usize) -> Result<Vec<Rec<'_>>, BznError> {
    let mut out = Vec::new();
    while at + 8 <= data.len() {
        let kind = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        let size = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = data.get(at + 8..at + 8 + size).ok_or(BznError::Truncated(at))?;
        out.push(Rec { kind, data: body });
        at += 8 + size;
    }
    if at != data.len() {
        return Err(BznError::Truncated(at));
    }
    Ok(out)
}

fn has_ext(s: &str, exts: &[&str]) -> bool {
    let l = s.to_ascii_lowercase();
    exts.iter().any(|e| l.ends_with(e))
}

fn vec3(r: &Rec) -> [f32; 3] {
    let f = r.floats();
    [f[0], f[1], f[2]]
}

fn parse_binary(recs: &[Rec]) -> Map {
    let mut m = Map::default();

    // Length-prefixed strings in the header: (i32 length, string).
    let mut strings = Vec::new();
    let mut vectors = Vec::new();
    for (i, r) in recs.iter().enumerate().take(400) {
        if r.is_string() && i > 0 && recs[i - 1].i32() == Some(r.data.len() as i32) {
            strings.push(r.text());
        }
        if r.kind == 9 && r.data.len() == 12 {
            vectors.push(vec3(r));
        }
    }
    let s = |i: usize| strings.get(i).cloned().unwrap_or_default();
    m.mission_type = s(0);
    m.description = s(2);
    m.file_name = s(4);
    m.tech_tree = strings.iter().find(|x| has_ext(x, &[".tt"])).cloned().unwrap_or_default();
    m.scripts = strings.iter().filter(|x| has_ext(x, &[".dll", ".drl", ".dsl", ".lua"])).cloned().collect();
    if vectors.len() >= 2 {
        m.min_extent = vectors[0];
        m.size = vectors[1];
    }

    // Background block: four fixed text buffers (100, 100, 128, 128) right after
    // the three ambient floats; then the object count.
    let fixed = |r: &Rec, n: usize| r.kind == 2 && r.data.len() == n;
    let block = (3..recs.len().saturating_sub(4)).find(|&i| {
        fixed(&recs[i], 100) && fixed(&recs[i + 1], 100) && fixed(&recs[i + 2], 128) && fixed(&recs[i + 3], 128)
    });
    let mut objects_from = 0;
    if let Some(i) = block {
        m.background = recs[i].text();
        m.starfield_odf = recs[i + 1].text();
        m.tech_tree_file = recs[i + 2].text();
        m.race = recs[i + 3].text();
        if (i >= 3) && recs[i - 3..i].iter().all(|r| r.kind == 5 && r.data.len() == 4) {
            m.ambient = [0, 1, 2].map(|k| recs[i - 3 + k].floats()[0]);
        }
        objects_from = i + 4;
    }

    // Start locations follow the extent vectors by a few records.
    if let Some(&vs) = recs.iter().take(60).enumerate().filter(|(_, r)| r.kind == 9 && r.data.len() == 12).map(|(i, _)| i).collect::<Vec<_>>().get(1) {
        let first = (vs + 1..recs.len().min(vs + 60)).find(|&k| start_loc(recs, k).is_some());
        if let Some(mut k) = first {
            m.start_count = recs[k - 1].i32().unwrap_or(0);
            while let Some((loc, next)) = start_loc(recs, k) {
                m.start_locs.push(loc);
                k = next;
            }
        }
    }

    // Objects: PrjIdLen (i16), PrjID (text of that length), EntityID (i32),
    // pos (vector3), team (i32), label (text), id, transform (48-byte struct).
    let mut i = objects_from;
    while i + 3 < recs.len() {
        let (len, name, id, pos) = (&recs[i], &recs[i + 1], &recs[i + 2], &recs[i + 3]);
        let starts = len.kind == 3
            && name.is_string()
            && len.i16().is_some_and(|n| n as usize == name.data.len() && n > 1)
            && id.kind == 4
            && pos.kind == 9
            && pos.data.len() == 12;
        if !starts {
            i += 1;
            continue;
        }
        let mut o = MapObject {
            odf: strip_odf(&name.text()),
            entity_id: id.i32().unwrap_or(0),
            ..Default::default()
        };
        let p = vec3(pos);
        o.initial_position = Some(p);
        o.transform = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, p[0], p[1], p[2]];
        // The fields up to the transform, within a short window.
        let mut j = i + 4;
        if let Some(t) = recs.get(j).and_then(Rec::i32) {
            o.team = t;
            j += 1;
        }
        if let Some(l) = recs.get(j).filter(|r| r.is_string()) {
            o.label = l.text();
        }
        if let Some(t) = recs[j..(j + 6).min(recs.len())].iter().find(|r| r.kind == 11 && r.data.len() == 48) {
            let f = t.floats();
            o.transform.copy_from_slice(&f[..12]);
        }
        m.objects.push(o);
        i += 4;
    }
    m
}

/// A length-prefixed string at `k`: `(text, next record)`. A zero length has
/// no string record.
fn lp_string(recs: &[Rec], k: usize) -> Option<(String, usize)> {
    let n = recs.get(k)?.i32().filter(|&n| n >= 0)? as usize;
    if n == 0 {
        return Some((String::new(), k + 1));
    }
    let s = recs.get(k + 1).filter(|r| r.is_string() && r.data.len() == n)?;
    Some((s.text(), k + 2))
}

/// One start location record: SLPos (vector3), SLType (string), SLLkTeam
/// (i32), SLLkRace (string), SLLat, SLMetal, SLCrew, SLDil (f32), SLTName and
/// SLColor (strings). Returns the entry and the next record.
fn start_loc(recs: &[Rec], k: usize) -> Option<(StartLoc, usize)> {
    let pos = recs.get(k).filter(|r| r.kind == 9 && r.data.len() == 12)?;
    let (kind, k) = lp_string(recs, k + 1)?;
    let team = recs.get(k).filter(|r| r.kind == 4)?.i32()?;
    let (race, k) = lp_string(recs, k + 1)?;
    if !(0..4).all(|i| recs.get(k + i).is_some_and(|r| r.kind == 5 && r.data.len() == 4)) {
        return None;
    }
    let (_, k) = lp_string(recs, k + 4)?;
    let (_, k) = lp_string(recs, k)?;
    Some((StartLoc { pos: vec3(pos), kind, team, race }, k))
}

fn strip_odf(s: &str) -> String {
    let s = s.trim();
    if s.to_ascii_lowercase().ends_with(".odf") { s[..s.len() - 4].to_owned() } else { s.to_owned() }
}

// ---------------------------------------------------------------- text

/// One text field: name (without the `[n]` part), indentation, and its value lines.
#[derive(Debug)]
struct Field {
    name: String,
    values: Vec<String>,
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.len().is_multiple_of(2) && s.bytes().all(|c| c.is_ascii_hexdigit())
}

fn decode_hex(s: &str) -> String {
    let bytes: Vec<u8> = (0..s.len()).step_by(2).filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect();
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn fields(text: &str) -> Vec<Field> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut out: Vec<Field> = Vec::new();
    let mut i = 0;
    let mut last_len: Option<usize> = None;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        i += 1;
        if t.starts_with('[') && t.ends_with(']') {
            out.push(Field {
                name: t.to_owned(),
                values: Vec::new(),
            });
            last_len = None;
            continue;
        }
        let Some(eq) = t.find('=') else { continue };
        let lhs = t[..eq].trim();
        let rhs = t[eq + 1..].trim();
        if let Some(open) = lhs.find(" [") {
            // `name [n] =`: n value lines follow, unless the next line is a nested field.
            let name = lhs[..open].trim().to_owned();
            let n: usize = lhs[open + 2..].trim_end_matches(']').parse().unwrap_or(1);
            let mut values = Vec::new();
            for _ in 0..n {
                match lines.get(i) {
                    Some(v) if !v.contains('=') && !v.trim().starts_with('[') => {
                        values.push(v.trim().to_owned());
                        i += 1;
                    }
                    _ => break,
                }
            }
            last_len = values.first().and_then(|v| v.parse().ok());
            out.push(Field { name, values });
        } else {
            // `name = value`: hex when a length field just announced it.
            let value = match last_len {
                Some(n) if n > 0 && is_hex(rhs) && rhs.len() / 2 == n => decode_hex(rhs),
                _ => rhs.to_owned(),
            };
            last_len = None;
            out.push(Field {
                name: lhs.to_owned(),
                values: vec![value],
            });
        }
    }
    out
}

/// The vector whose field is on line `i`: its next three value lines, with
/// the nested `x [1] =` headers between them skipped.
fn vec3_at(lines: &[&str], i: usize) -> Option<[f32; 3]> {
    let mut v = lines.get(i + 1..)?.iter().take(8).filter(|l| !l.contains('=')).filter_map(|l| l.parse::<f32>().ok());
    Some([v.next()?, v.next()?, v.next()?])
}

fn parse_text(text: &str) -> Map {
    let fs = fields(text);
    let mut m = Map::default();
    let first = |name: &str| fs.iter().find(|f| f.name.eq_ignore_ascii_case(name)).and_then(|f| f.values.first()).cloned();
    let float = |v: Option<String>| v.and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    let after = |name: &str| -> [f32; 3] {
        // A vector field: its x, y, z follow as nested fields.
        let Some(i) = fs.iter().position(|f| f.name.eq_ignore_ascii_case(name)) else { return [0.0; 3] };
        let mut out = [0.0; 3];
        for (k, axis) in ["x", "y", "z"].iter().enumerate() {
            if let Some(f) = fs[i + 1..].iter().take(6).find(|f| f.name == *axis) {
                out[k] = f.values.first().and_then(|v| v.parse().ok()).unwrap_or(0.0);
            }
        }
        out
    };
    // Strings carried as `NAME [1] = len` then `NAME = hex`: take the second.
    let string = |name: &str| {
        fs.iter()
            .filter(|f| f.name.eq_ignore_ascii_case(name))
            .nth(1)
            .and_then(|f| f.values.first())
            .cloned()
            .unwrap_or_default()
    };
    m.mission_type = string("MPDType");
    m.description = string("MPDDescription");
    m.file_name = string("MPDFilename");
    m.tech_tree = string("MPDTechTree");
    m.scripts = [string("MPDScriptFile"), string("MPDRuleFile")].into_iter().filter(|s| !s.is_empty()).collect();
    m.min_extent = after("MPDMinExtent");
    m.size = after("MPDSize");
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    m.start_count = lines
        .iter()
        .position(|l| *l == "MPDStLoc [1] =")
        .and_then(|i| lines.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let sl: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| **l == "SLPos [1] =").map(|(i, _)| i).collect();
    for (n, &i) in sl.iter().enumerate() {
        let Some(pos) = vec3_at(&lines, i) else { continue };
        let end = sl.get(n + 1).copied().unwrap_or(lines.len()).min(i + 60);
        let seg = &lines[i..end];
        let hex = |key: &str| seg.iter().find_map(|l| l.strip_prefix(key)).map(|v| if is_hex(v) { decode_hex(v) } else { v.to_owned() }).unwrap_or_default();
        let team = seg.iter().position(|l| *l == "SLLkTeam [1] =").and_then(|j| seg.get(j + 1)).and_then(|v| v.parse().ok()).unwrap_or(-1);
        m.start_locs.push(StartLoc { pos, kind: hex("SLType = "), team, race: hex("SLLkRace = ") });
    }
    m.ambient = [float(first("AmbientRed")), float(first("AmbientGreen")), float(first("AmbientBlue"))];
    m.background = first("BackgroundArt").unwrap_or_default();
    m.starfield_odf = first("StarfieldOdf").unwrap_or_default();
    m.tech_tree_file = first("techTreeFilename").unwrap_or_default();
    m.race = first("raceName").unwrap_or_default();

    // Objects: from each `[GameObject]` to the next.
    let starts: Vec<usize> = fs.iter().enumerate().filter(|(_, f)| f.name == "[GameObject]").map(|(i, _)| i).collect();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(fs.len());
        let block = &fs[s + 1..end];
        let get = |name: &str| block.iter().find(|f| f.name == name).and_then(|f| f.values.first()).cloned();
        // Preserve a complete prefix independently of transform precedence.
        // Leave the older partial-coordinate fallback below unchanged.
        let initial_position = block.iter().position(|f| f.name == "pos").and_then(|p| {
            let component = |axis: &str| {
                block[p + 1..].iter().take(4).find(|f| f.name == axis)
                    .and_then(|f| f.values.first()).and_then(|v| v.parse().ok())
            };
            Some([component("x")?, component("y")?, component("z")?])
        });
        let num: HashMap<&str, f32> = [
            "right_x", "right_y", "right_z", "up_x", "up_y", "up_z", "front_x", "front_y", "front_z", "position.x", "position.y",
            "position.z",
        ]
        .into_iter()
        .filter_map(|n| get(n).and_then(|v| v.parse().ok()).map(|v| (n, v)))
        .collect();
        let mut t = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        for (idx, n) in [
            "right_x", "right_y", "right_z", "up_x", "up_y", "up_z", "front_x", "front_y", "front_z", "position.x", "position.y",
            "position.z",
        ]
        .iter()
        .enumerate()
        {
            if let Some(v) = num.get(n) {
                t[idx] = *v;
            }
        }
        if !num.contains_key("position.x") {
            // Older maps: no position in the transform; use `pos`.
            if let Some(p) = block.iter().position(|f| f.name == "pos") {
                for (k, axis) in ["x", "y", "z"].iter().enumerate() {
                    if let Some(f) = block[p + 1..].iter().take(4).find(|f| f.name == *axis) {
                        t[9 + k] = f.values.first().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    }
                }
            }
        }
        let odf = get("PrjID").unwrap_or_default();
        if odf.is_empty() {
            continue;
        }
        m.objects.push(MapObject {
            odf: strip_odf(&odf),
            entity_id: get("EntityID").and_then(|v| v.parse().ok()).unwrap_or(0),
            team: get("team").and_then(|v| v.parse().ok()).unwrap_or(0),
            label: get("label").unwrap_or_default(),
            initial_position,
            transform: t,
            light_colour: match (get("lightColorRed"), get("lightColorGreen"), get("lightColorBlue")) {
                (Some(r), Some(g), Some(b)) => Some([r, g, b].map(|v| v.parse().unwrap_or(1.0))),
                _ => None,
            },
        });
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "version [1] =\r\n2177\r\nBinaryMode [1] =\r\nfalse\r\nMPDType [1] =\r\n7\r\nMPDType = 6d697373696f6e\r\n\
MPDMinExtent [1] =\r\n  x [1] =\r\n0\r\n  y [1] =\r\n-1250\r\n  z [1] =\r\n0\r\nMPDScriptFile [1] =\r\n14\r\n\
MPDScriptFile = 6d6d6d5f6c6f616465722e646c6c\r\nAmbientRed [1] =\r\n0.25\r\nAmbientGreen [1] =\r\n0.5\r\nAmbientBlue [1] =\r\n1\r\n\
BackgroundArt = MbgBaku.SOD\r\nStarfieldOdf = ststars.odf\r\nsize [1] =\r\n1\r\n[GameObject]\r\nPrjIdLen [1] =\r\n6\r\n\
PrjID = 4d62616b7500\r\nEntityID [1] =\r\n7\r\nteam [1] =\r\n2\r\nlabel = Baku\r\ntransform [1] =\r\n  right_x [1] =\r\n0\r\n\
  right_y [1] =\r\n0\r\n  right_z [1] =\r\n-1\r\n  position.x [1] =\r\n15.5\r\n  position.y [1] =\r\n-4\r\n  position.z [1] =\r\n33\r\n";

    #[test]
    fn text_map() {
        let m = parse(TEXT.as_bytes()).unwrap();
        assert!(!m.binary);
        assert_eq!(m.version, 2177);
        assert_eq!(m.mission_type, "mission");
        assert_eq!(m.scripts, ["mmm_loader.dll"]);
        assert_eq!(m.min_extent, [0.0, -1250.0, 0.0]);
        assert_eq!(m.ambient, [0.25, 0.5, 1.0]);
        assert_eq!(m.background, "MbgBaku.SOD");
        assert_eq!(m.objects.len(), 1);
        let o = &m.objects[0];
        assert_eq!((o.odf.as_str(), o.entity_id, o.team, o.label.as_str()), ("Mbaku", 7, 2, "Baku"));
        assert_eq!(&o.transform[..3], &[0.0, 0.0, -1.0]);
        assert_eq!(&o.transform[9..], &[15.5, -4.0, 33.0]);
    }

    fn rec(kind: u32, data: &[u8]) -> Vec<u8> {
        let mut v = kind.to_le_bytes().to_vec();
        v.extend((data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn binary_map() {
        let mut b = b"version [1] =\r\n2172\r\nBinaryMode [1] =\r\ntrue\r\n".to_vec();
        b.extend(rec(4, &7i32.to_le_bytes()));
        b.extend(rec(0, b"mission"));
        for v in [0.1f32, 0.2, 0.3] {
            b.extend(rec(5, &v.to_le_bytes()));
        }
        let buf = |s: &str, n: usize| {
            let mut v = s.as_bytes().to_vec();
            v.resize(n, 0);
            v
        };
        b.extend(rec(2, &buf("MbgBorg.SOD", 100)));
        b.extend(rec(2, &buf("ststars.odf", 100)));
        b.extend(rec(2, &buf("borg.tt", 128)));
        b.extend(rec(2, &buf("borg", 128)));
        b.extend(rec(4, &1i32.to_le_bytes()));
        b.extend(rec(3, &5i16.to_le_bytes()));
        b.extend(rec(2, b"bcube"));
        b.extend(rec(4, &(-9i32).to_le_bytes()));
        let pos: Vec<u8> = [1.0f32, 2.0, 3.0].iter().flat_map(|f| f.to_le_bytes()).collect();
        b.extend(rec(9, &pos));
        b.extend(rec(4, &3i32.to_le_bytes()));
        b.extend(rec(2, &buf("Cube", 40)));
        b.extend(rec(8, &0u32.to_le_bytes()));
        let t: Vec<u8> = [0.0f32, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0, 2.0, 3.0].iter().flat_map(|f| f.to_le_bytes()).collect();
        b.extend(rec(11, &t));
        let m = parse(&b).unwrap();
        assert!(m.binary);
        assert_eq!(m.mission_type, "mission");
        assert_eq!(m.background, "MbgBorg.SOD");
        assert_eq!(m.ambient, [0.1, 0.2, 0.3]);
        assert_eq!(m.objects.len(), 1);
        let o = &m.objects[0];
        assert_eq!((o.odf.as_str(), o.entity_id, o.team, o.label.as_str()), ("bcube", -9, 3, "Cube"));
        assert_eq!(o.transform[2], 1.0);
        assert_eq!(&o.transform[9..], &[1.0, 2.0, 3.0]);
    }

    fn start_text(pos: [f32; 3], kind: &str, team: i32) -> String {
        let hex: String = kind.bytes().map(|b| format!("{b:02x}")).collect();
        format!(
            "SLPos [1] =\r\n  x [1] =\r\n{}\r\n  y [1] =\r\n{}\r\n  z [1] =\r\n{}\r\nSLType [1] =\r\n{}\r\nSLType = {hex}\r\n\
SLLkTeam [1] =\r\n{team}\r\nSLLkRace [1] =\r\n0\r\nSLLat [1] =\r\n-1\r\n",
            pos[0],
            pos[1],
            pos[2],
            kind.len()
        )
    }

    #[test]
    fn text_start_locations() {
        let text = format!(
            "version [1] =\r\n3013\r\nBinaryMode [1] =\r\nfalse\r\nMPDSize [1] =\r\n  x [1] =\r\n12000\r\n  y [1] =\r\n500\r\n  z [1] =\r\n12000\r\n\
MPDStLoc [1] =\r\n2\r\n{}{}{}",
            start_text([6000.0, 0.0, 6000.0], "empty", -1),
            start_text([3734.12, 0.0, 10682.7], "player", 3),
            start_text([6000.0, 0.0, 6000.0], "empty", -1)
        );
        let m = parse(text.as_bytes()).unwrap();
        assert_eq!(m.size, [12000.0, 500.0, 12000.0]);
        assert_eq!(m.start_count, 2);
        assert_eq!(m.start_locs.len(), 3);
        assert_eq!(m.start_locs[1], StartLoc { pos: [3734.12, 0.0, 10682.7], kind: "player".into(), team: 3, race: String::new() });
        assert_eq!(m.start_locs[0].kind, "empty");
    }

    #[test]
    fn binary_start_locations() {
        let mut b = b"version [1] =\r\n3013\r\nBinaryMode [1] =\r\ntrue\r\n".to_vec();
        let vec = |v: [f32; 3]| rec(9, &v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>());
        let string = |s: &str| {
            let mut v = rec(4, &(s.len() as i32).to_le_bytes());
            if !s.is_empty() {
                v.extend(rec(0, s.as_bytes()));
            }
            v
        };
        b.extend(vec([0.0, -500.0, 0.0]));
        b.extend(vec([9500.0, 1000.0, 9500.0]));
        b.extend(rec(4, &0i32.to_le_bytes()));
        b.extend(rec(4, &2i32.to_le_bytes()));
        for (pos, kind, team, race) in [([4750.0, 0.0, 4750.0], "empty", -1i32, ""), ([1602.0, 0.0, 7902.0], "player", -1, "race-unspecified")] {
            b.extend(vec(pos));
            b.extend(string(kind));
            b.extend(rec(4, &team.to_le_bytes()));
            b.extend(string(race));
            for _ in 0..4 {
                b.extend(rec(5, &(-1.0f32).to_le_bytes()));
            }
            b.extend(string(""));
            b.extend(string("empty"));
        }
        let m = parse(&b).unwrap();
        assert_eq!(m.start_count, 2);
        assert_eq!(m.start_locs.len(), 2);
        assert_eq!(m.start_locs[1], StartLoc { pos: [1602.0, 0.0, 7902.0], kind: "player".into(), team: -1, race: "race-unspecified".into() });
    }

    /// With STA64_TEST_BZN_DIRS (colon-separated dirs): parse every map.
    #[test]
    fn real_maps_if_available() {
        let Ok(dirs) = std::env::var("STA64_TEST_BZN_DIRS") else { return };
        let (mut ok, mut bin, mut objects, mut no_bg) = (0, 0, 0, 0);
        let mut failed = Vec::new();
        for dir in dirs.split(':') {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bzn")) {
                    continue;
                }
                match parse(&std::fs::read(&p).unwrap()) {
                    Ok(m) => {
                        ok += 1;
                        bin += m.binary as usize;
                        objects += m.objects.len();
                        if m.background.is_empty() {
                            no_bg += 1;
                        }
                        assert!(m.objects.iter().all(|o| o.transform.iter().all(|v| v.is_finite())), "{}", p.display());
                    }
                    Err(e) => failed.push(format!("{}: {e}", p.display())),
                }
            }
        }
        eprintln!("BZN: {ok} parsed ({bin} binary), {objects} objects, {no_bg} without background, {} failed", failed.len());
        for f in failed.iter().take(10) {
            eprintln!("  {f}");
        }
        assert!(failed.is_empty());
    }
}

#[cfg(test)]
mod count_check {
    /// With STA64_TEST_BZN_DIRS: for binary maps, the object count stored after
    /// the background block must equal the objects found.
    #[test]
    fn binary_object_counts_match() {
        let Ok(dirs) = std::env::var("STA64_TEST_BZN_DIRS") else { return };
        let (mut checked, mut bad) = (0, Vec::new());
        for dir in dirs.split(':') {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bzn")) {
                    continue;
                }
                let data = std::fs::read(&p).unwrap();
                let Ok(m) = super::parse(&data) else { continue };
                if !m.binary {
                    continue;
                }
                let start = data.windows(4).position(|w| w == b"true").unwrap() + 4;
                let start = start + data[start..].iter().take_while(|&&c| c == b'\r' || c == b'\n').count();
                let recs = super::records(&data, start).unwrap();
                let fixed = |r: &super::Rec, n: usize| r.kind == 2 && r.data.len() == n;
                let Some(i) = (0..recs.len() - 4).find(|&i| fixed(&recs[i], 100) && fixed(&recs[i + 1], 100) && fixed(&recs[i + 2], 128) && fixed(&recs[i + 3], 128)) else { continue };
                let declared = recs[i + 4].i32().unwrap_or(-1);
                checked += 1;
                if declared != m.objects.len() as i32 {
                    bad.push(format!("{}: declared {declared}, found {}", p.display(), m.objects.len()));
                }
            }
        }
        eprintln!("object counts: {checked} binary maps checked, {} mismatched", bad.len());
        for b in bad.iter().take(10) {
            eprintln!("  {b}");
        }
        assert!(bad.is_empty());
    }
}
