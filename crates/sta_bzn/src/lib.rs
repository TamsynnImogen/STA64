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

pub use paths::MapPath;

pub mod a1;
pub mod paths;
pub mod sol;
pub mod write;

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
    #[error(transparent)]
    Sol(#[from] sol::SolError),
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
    /// The map's display name for the object (Armada II binary maps: a separate 128-byte
    /// buffer after the label, `instance_name`; empty when the map gives none). Not the
    /// script label. Text-format maps are not read for it.
    pub instance_name: String,
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
    /// Named paths (`AiPaths`) of Armada II / Fleet Operations maps, in file order (CON-47).
    pub paths: Vec<MapPath>,
    /// Retained A1-specific source data; absent for existing A2 binary/text maps.
    pub a1: Option<a1::MapData>,
}

pub fn parse(data: &[u8]) -> Result<Map, BznError> {
    if !data.starts_with(b"version") {
        return Err(BznError::NotBzn);
    }
    // Star Trek: Legacy maps share the header; their version tells them apart.
    if sol::is_sol(data) {
        return Ok(sol::parse(data)?.to_map());
    }
    if a1::has_header_marker(data) {
        let header = a1::inspect_header(data)?;
        return a1::parse_map(data, header);
    }
    let version = text_value(data, b"version [1] =").and_then(|v| v.parse().ok()).unwrap_or(0);
    let binary = text_value(data, b"BinaryMode [1] =").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let mut map = if binary {
        let start = binary_start(data).ok_or(BznError::NotBzn)?;
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

/// Where the records of a binary map start: after the first `true` of the header and the line break.
fn binary_start(data: &[u8]) -> Option<usize> {
    let start = data.windows(4).position(|w| w == b"true").map(|i| i + 4)?;
    Some(start + data[start..].iter().take_while(|&&c| c == b'\r' || c == b'\n').count())
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
        o.instance_name = instance_name(recs, j + 1);
        if let Some(t) = recs[j..(j + 6).min(recs.len())].iter().find(|r| r.kind == 11 && r.data.len() == 48) {
            let f = t.floats();
            o.transform.copy_from_slice(&f[..12]);
        }
        m.objects.push(o);
        i += 4;
    }
    m.paths = paths::from_records(&recs[objects_from.min(recs.len())..]);
    m
}

/// Whether an object record starts at `k` (`PrjIdLen`, `PrjID`, id, position).
fn object_starts(recs: &[Rec], k: usize) -> bool {
    let (Some(len), Some(name), Some(id), Some(pos)) = (recs.get(k), recs.get(k + 1), recs.get(k + 2), recs.get(k + 3)) else { return false };
    len.kind == 3
        && name.is_string()
        && len.i16().is_some_and(|n| n as usize == name.data.len() && n > 1)
        && id.kind == 4
        && pos.kind == 9
        && pos.data.len() == 12
}

/// The object's display name: the first 128-byte string record from `from` on, before the next
/// object record and within 40 records (read from data on the installed Armada II maps: at
/// most one such buffer per object; the distance from the label differs by object class). Cut at the first NUL and trimmed; a blank buffer or none gives an empty string.
fn instance_name(recs: &[Rec], from: usize) -> String {
    for k in from..recs.len().min(from.saturating_add(40)) {
        if object_starts(recs, k) {
            break;
        }
        if recs[k].kind == 2 && recs[k].data.len() == 128 {
            return recs[k].text().trim().to_owned();
        }
    }
    String::new()
}

/// [`instance_name`] over `(kind, data)` pairs, for tests of other modules.
#[cfg(test)]
pub(crate) fn instance_name_of(recs: &[(u32, &[u8])], from: usize) -> String {
    let recs: Vec<Rec> = recs.iter().map(|&(kind, data)| Rec { kind, data }).collect();
    instance_name(&recs, from)
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

/// What one team of an Armada II / Fleet Operations binary map starts with (the map's team table).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TeamStart {
    pub dilithium: f32,
    pub latinum: f32,
    pub metal: f32,
    /// The biomatter ("supply" in Fleet Operations) stock.
    pub biomatter: f32,
    /// The team's crew stock.
    pub crew: f32,
    /// The team's officer limit.
    pub officers: i32,
}

/// The team table of a binary map: eight blocks after the objects, block `n` for team `n + 1` (Armada team
/// numbers; block 0 is the human seat of a mission). Two block shapes are recognised from the record
/// structure alone (not from a profile name): the older one is two i32 (not read) and eleven records; the
/// newer one has one extra one-byte bool record after the two i32, and when it is true an i16 and a string
/// record (a team name) follow it, so blocks of the newer shape vary in length. The eleven records are
/// dilithium, latinum and metal as f32 each followed by a zero pad, then biomatter ("supply") as f32, the
/// officer limit as i32, the crew capacity, crew and one more f32; dilithium, latinum, metal, biomatter,
/// the officer limit and crew are read. The two shapes cannot be mistaken for each other (the third
/// record is an f32 or a bool).
///
/// Evidence: read from data (every binary `a2_*` and `mp*` map of the Armada II install and the Fleet
/// Operations Roots maps carry exactly one run of eight such blocks, in either shape; unused teams hold
/// the `RTS_CFG.h` starting defaults; the first block of `a2_fed01` matches the stock game's start bar
/// for dilithium, latinum, metal and crew, measured; the officer limit of the player blocks equals
/// `SINGLE_PLAYER_OFFICER_LIMIT`, not measured). The biomatter slot is read from the Roots executable,
/// static (the team loader stores it into the biomatter field); applied to Armada II maps by inference.
/// Values are taken as written: real maps carry very large values and negative crew on some seats, which
/// a consumer clamps. Not read: text maps, Armada I maps, Star Trek: Legacy maps, and any map where a run
/// of exactly eight blocks (no block-shaped record right before or after it) is not found exactly once,
/// for which the answer is empty.
pub fn team_starts(data: &[u8]) -> Vec<TeamStart> {
    match team_table(data) {
        TeamTable::Found(t) => t,
        _ => Vec::new(),
    }
}

/// What [`team_table`] found.
#[derive(Debug, Clone, PartialEq)]
pub enum TeamTable {
    /// Not a binary Armada II / Fleet Operations map, or no block-shaped records at all.
    Absent,
    /// Block-shaped records exist but are not exactly one run of eight: the lengths of the runs found.
    /// Nothing is read from such a map; a host reports it as a gap.
    Refused(Vec<usize>),
    Found(Vec<TeamStart>),
}

/// [`team_starts`] saying why it read nothing.
pub fn team_table(data: &[u8]) -> TeamTable {
    if !data.starts_with(b"version") || sol::is_sol(data) || a1::has_header_marker(data) {
        return TeamTable::Absent;
    }
    if !text_value(data, b"BinaryMode [1] =").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
        return TeamTable::Absent;
    }
    let Some(start) = binary_start(data) else { return TeamTable::Absent };
    let Ok(recs) = records(data, start) else { return TeamTable::Absent };
    // A block is two i32 (not read), an optional extra bool record (the newer shape; a true bool is followed by
    // an i16 and a string record, a team name), then eleven records: dilithium, pad, latinum, pad, metal, pad,
    // biomatter ("supply"), officer limit (i32), crew capacity, crew, one more float. Returns the index of
    // those eleven records and the index after the block. The two shapes cannot be confused: the third record
    // is an f32 in one and a one-byte bool in the other.
    const TAIL: [u32; 11] = [5, 5, 5, 5, 5, 5, 5, 4, 5, 5, 5];
    let block_at = |i: usize| -> Option<(usize, usize)> {
        let is = |k: usize, kind: u32, len: usize| recs.get(k).is_some_and(|r| r.kind == kind && r.data.len() == len);
        if !(is(i, 4, 4) && is(i + 1, 4, 4)) {
            return None;
        }
        let mut t = i + 2;
        if is(t, 1, 1) {
            let named = recs[t].data[0];
            t += 1;
            match named {
                0 => {}
                1 if recs.get(t).is_some_and(|r| r.kind == 3 && r.data.len() == 2) && recs.get(t + 1).is_some_and(|r| r.kind == 2) => t += 2,
                _ => return None,
            }
        }
        let tail_ok = t + 11 <= recs.len()
            && TAIL.iter().enumerate().all(|(j, &kind)| recs[t + j].kind == kind && recs[t + j].data.len() == 4)
            && [1, 3, 5].iter().all(|&j| recs[t + j].data == [0u8; 4]);
        tail_ok.then_some((t, t + 11))
    };
    // Maximal runs of consecutive blocks: the table is the one run of exactly eight. A ninth block-shaped
    // record before or after (a run of 9 or more) would shift every seat, so such a map reads as empty.
    let mut runs = Vec::new();
    let mut i = 0;
    while i < recs.len() {
        let mut tails = Vec::new();
        let mut at = i;
        while let Some((t, end)) = block_at(at) {
            tails.push(t);
            at = end;
        }
        if tails.is_empty() {
            i += 1;
        } else {
            i = at;
            runs.push(tails);
        }
    }
    if runs.is_empty() {
        return TeamTable::Absent;
    }
    let [tails] = &runs[..] else { return TeamTable::Refused(runs.iter().map(Vec::len).collect()) };
    if tails.len() != 8 {
        return TeamTable::Refused(runs.iter().map(Vec::len).collect());
    }
    let f = |i: usize| f32::from_le_bytes(recs[i].data.try_into().unwrap());
    let n = |i: usize| i32::from_le_bytes(recs[i].data.try_into().unwrap());
    TeamTable::Found(
        tails
            .iter()
            .map(|&t| TeamStart { dilithium: f(t), latinum: f(t + 2), metal: f(t + 4), biomatter: f(t + 6), officers: n(t + 7), crew: f(t + 9) })
            .collect(),
    )
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
                Some(n) if n > 0 && is_hex(rhs) && rhs.len() / 2 == n && !matches!(lhs, "pathType" | "old_ptr") => decode_hex(rhs),
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
            instance_name: String::new(),
            initial_position,
            transform: t,
            light_colour: match (get("lightColorRed"), get("lightColorGreen"), get("lightColorBlue")) {
                (Some(r), Some(g), Some(b)) => Some([r, g, b].map(|v| v.parse().unwrap_or(1.0))),
                _ => None,
            },
        });
    }
    m.paths = paths::from_fields(&fs);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_stops_at_the_next_object_and_takes_the_first_buffer() {
        fn rec(kind: u32, data: &[u8]) -> Rec<'_> {
            Rec { kind, data }
        }
        let mut name = b"Quux Prime".to_vec();
        name.resize(128, 0);
        let label = [0u8; 40];
        let pos = [0u8; 12];
        let id = 7i32.to_le_bytes();
        let next = [rec(3, &[3, 0]), rec(2, b"abc"), rec(4, &id), rec(9, &pos)];
        // The distance from the label varies by class: a few scalars in between.
        let named = [rec(2, &label), rec(1, &[0]), rec(5, &[0; 4]), rec(2, &name)];
        assert_eq!(instance_name(&named, 1), "Quux Prime");
        // A buffer after the next object's start belongs to that object.
        let mut crossing = vec![rec(2, &label), rec(5, &[0; 4])];
        crossing.extend(next);
        crossing.push(rec(2, &name));
        assert_eq!(instance_name(&crossing, 1), "");
        assert_eq!(instance_name(&named, 99), "");
    }


    const TEXT: &str = "version [1] =\r\n2177\r\nBinaryMode [1] =\r\nfalse\r\nMPDType [1] =\r\n7\r\nMPDType = 6d697373696f6e\r\n\
MPDMinExtent [1] =\r\n  x [1] =\r\n0\r\n  y [1] =\r\n-1250\r\n  z [1] =\r\n0\r\nMPDScriptFile [1] =\r\n14\r\n\
MPDScriptFile = 6d6d6d5f6c6f616465722e646c6c\r\nAmbientRed [1] =\r\n0.25\r\nAmbientGreen [1] =\r\n0.5\r\nAmbientBlue [1] =\r\n1\r\n\
BackgroundArt = MbgBaku.SOD\r\nStarfieldOdf = ststars.odf\r\nsize [1] =\r\n1\r\n[GameObject]\r\nPrjIdLen [1] =\r\n6\r\n\
PrjID = 4d62616b7500\r\nEntityID [1] =\r\n7\r\nteam [1] =\r\n2\r\nlabel = Baku\r\ntransform [1] =\r\n  right_x [1] =\r\n0\r\n\
  right_y [1] =\r\n0\r\n  right_z [1] =\r\n-1\r\n  position.x [1] =\r\n15.5\r\n  position.y [1] =\r\n-4\r\n  position.z [1] =\r\n33\r\n";

    /// A binary map header and the given team blocks (dilithium, latinum, metal), own-authored.
    fn team_map(teams: &[[f32; 3]]) -> Vec<u8> {
        let rec = |out: &mut Vec<u8>, kind: u32, data: &[u8]| {
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        };
        let mut out = b"version [1] =\r\n2177\r\nBinaryMode [1] =\r\ntrue\r\n".to_vec();
        rec(&mut out, 4, &7i32.to_le_bytes());
        for t in teams {
            rec(&mut out, 4, &0i32.to_le_bytes());
            rec(&mut out, 4, &0i32.to_le_bytes());
            for v in t {
                rec(&mut out, 5, &v.to_le_bytes());
                rec(&mut out, 5, &0f32.to_le_bytes());
            }
            rec(&mut out, 5, &8888f32.to_le_bytes());
            rec(&mut out, 4, &321i32.to_le_bytes());
            for v in [0f32, 1717.0, 0.0] {
                rec(&mut out, 5, &v.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn team_starts_reads_the_eight_block_table() {
        let mut teams = vec![[4321.0, 65.0, 987.0]; 8];
        teams[0] = [1234.0, 56.0, 789.0];
        let got = team_starts(&team_map(&teams));
        assert_eq!(got.len(), 8);
        assert_eq!(got[0], TeamStart { dilithium: 1234.0, latinum: 56.0, metal: 789.0, biomatter: 8888.0, crew: 1717.0, officers: 321 });
        assert_eq!(got[7], TeamStart { dilithium: 4321.0, latinum: 65.0, metal: 987.0, biomatter: 8888.0, crew: 1717.0, officers: 321 });
    }

    /// The newer block shape, own-authored: one extra bool record after the two i32, and when it is true an
    /// i16 and a string record (a team name) before the floats. `named[n]` says whether block `n` has a name.
    fn team_map_v2(teams: &[[f32; 3]], named: &[bool]) -> Vec<u8> {
        let rec = |out: &mut Vec<u8>, kind: u32, data: &[u8]| {
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        };
        let mut out = b"version [1] =\r\n3013\r\nBinaryMode [1] =\r\ntrue\r\n".to_vec();
        rec(&mut out, 4, &7i32.to_le_bytes());
        for (n, t) in teams.iter().enumerate() {
            rec(&mut out, 4, &0i32.to_le_bytes());
            rec(&mut out, 4, &0i32.to_le_bytes());
            rec(&mut out, 1, &[u8::from(named[n])]);
            if named[n] {
                rec(&mut out, 3, &6i16.to_le_bytes());
                rec(&mut out, 2, b"team_x\0");
            }
            for v in t {
                rec(&mut out, 5, &v.to_le_bytes());
                rec(&mut out, 5, &0f32.to_le_bytes());
            }
            rec(&mut out, 5, &(8888.0 + n as f32).to_le_bytes());
            rec(&mut out, 4, &321i32.to_le_bytes());
            for v in [5.0f32, 1717.0, 0.0] {
                rec(&mut out, 5, &v.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn team_starts_reads_the_newer_block_shape() {
        let mut teams = vec![[4321.0, 65.0, 987.0]; 8];
        teams[0] = [1234.0, 56.0, 789.0];
        for named in [[false; 8], [true, false, false, false, false, false, false, false], [true; 8]] {
            let got = team_starts(&team_map_v2(&teams, &named));
            assert_eq!(got.len(), 8, "{named:?}");
            assert_eq!(got[0], TeamStart { dilithium: 1234.0, latinum: 56.0, metal: 789.0, biomatter: 8888.0, crew: 1717.0, officers: 321 });
            assert_eq!(got[7].biomatter, 8895.0);
            assert_eq!((got[7].dilithium, got[7].crew), (4321.0, 1717.0));
        }
        assert!(team_starts(&team_map_v2(&teams[..7], &[false; 7])).is_empty(), "seven newer blocks");
        assert_eq!(team_table(&team_map_v2(&[[1.0, 2.0, 3.0]; 9], &[false; 9])), TeamTable::Refused(vec![9]));
        assert_eq!(team_table(&team_map_v2(&[], &[])), TeamTable::Absent);
    }

    #[test]
    fn team_starts_is_empty_unless_the_table_is_exactly_eight_blocks() {
        assert!(team_starts(&team_map(&[[1.0, 2.0, 3.0]; 7])).is_empty(), "seven blocks");
        assert!(team_starts(&team_map(&[[1.0, 2.0, 3.0]; 9])).is_empty(), "nine blocks: a ninth would shift every seat");
        assert!(team_starts(&team_map(&[[1.0, 2.0, 3.0]; 16])).is_empty(), "two tables");
        assert_eq!(team_starts(&team_map(&[[1.0, 2.0, 3.0]; 8])).len(), 8, "the exact case");
        assert_eq!(team_table(&team_map(&[[1.0, 2.0, 3.0]; 9])), TeamTable::Refused(vec![9]), "a refused table is reported, not just empty");
        assert_eq!(team_table(&team_map(&[])), TeamTable::Absent);
        assert!(team_starts(TEXT.as_bytes()).is_empty(), "text map");
        assert!(team_starts(b"not a map").is_empty());
    }

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
