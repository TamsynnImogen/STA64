//! Star Trek: Legacy `.sol` maps (CON-20): BinaryMode SOL, format version 3021.
//!
//! A port of the parser of A2FOExtensions' `A2FOSOLMaps` (`sol_format.cpp` over
//! `a2fo_core/src/sol.rs`), validated there on `mpbajor_all.sol`: 8 start
//! locations and 19 placed objects.
//!
//! A SOL starts with the BZN text header (`version [1] =` / `3021` /
//! `BinaryMode [1] =` / `true`), then a stream of tagged records, `u32 tag,
//! u32 size, data`, little endian. Tags: 2 string or fixed buffer, 3 `u16`,
//! 4 `u32`, 8 opaque id, 9 vector3, 11 `Matrix34` (12 floats: right, up,
//! front, position, as in a SOD). The reader keeps to what the original
//! does, by record index and shape:
//!
//! | records | content |
//! |---|---|
//! | 4..13 | five (length, string) pairs: game type, GUID, title, source path, file name |
//! | 15, 16 | minimum extent, size (vector3) |
//! | 19, 20 | mission script name (`.drl` / `.dsl`), optional |
//! | 27 | start-location count (at most 64) |
//! | 28 + 12 i | start `i`: position, type length, type string, 9 other records |
//! | after | object count, then 8 records per object (see [`SolObject`]), found by scanning |
//!
//! Only objects are read; Legacy cameras, paths, points and mission state
//! stay with the script. The mission script name is kept (in
//! [`SolMap::mission_script`] and as `Map::scripts`).
//!
//! [`SolMap::to_map`] gives the shared [`Map`], so `MapDef::from_bzn` and
//! everything else that reads BZNs reads SOLs, and [`crate::parse`] hands
//! version-3021 files here.

use crate::{Map, MapObject, StartLoc};

/// The format version this reader is written and validated for.
pub const VERSION: u32 = 3021;

const PREFIX: &[u8] = b"version [1] =\r\n";
const MARKER: &[u8] = b"\r\nBinaryMode [1] =\r\ntrue\r\n";
const EMPTY_MISSION: &[u8] = b"EmptyMission\0";
/// The original rejects larger files.
const MAX_FILE: usize = 256 * 1024 * 1024;
const MAX_STARTS: u32 = 64;
const MAX_OBJECTS: u32 = 100_000;

/// Why a SOL could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SolError {
    #[error("not a supported BinaryMode Legacy SOL file")]
    NotSol,
    #[error("SOL file size is invalid or unsupported")]
    Size,
    #[error("truncated SOL tagged-record header at byte {0}")]
    TruncatedHeader(usize),
    #[error("SOL record at byte {0} extends past end of file")]
    TruncatedRecord(usize),
    #[error("unsupported or corrupt Legacy SOL MapDetails prefix")]
    Prefix,
    #[error("invalid SOL start-location count")]
    StartCount,
    #[error("truncated SOL start-location table")]
    StartTable,
    #[error("invalid SOL start-location record")]
    StartRecord,
    #[error("SOL object table was not found")]
    NoObjectTable,
    #[error(
        "SOL declared {declared} map objects but only {found} common object records were decoded"
    )]
    ObjectCount { declared: u32, found: u32 },
}

/// A player start location.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SolStart {
    pub pos: [f32; 3],
    /// The location's type string (as BZN `SLType`).
    pub kind: String,
}

/// A placed object: the eight common records `u16 length`, ODF name, `u32 id`,
/// position, `u32 team`, a 40-byte instance name, an opaque id and `Matrix34`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SolObject {
    /// ODF name, as written (no extension).
    pub odf: String,
    pub id: u32,
    pub pos: [f32; 3],
    pub team: i32,
    pub instance_name: String,
    pub opaque_id: u32,
    /// Right, up, front, position.
    pub transform: [f32; 12],
}

/// What the reader takes from a SOL.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SolMap {
    pub version: u32,
    pub game_type: String,
    pub guid: String,
    pub title: String,
    pub source_path: String,
    pub filename: String,
    /// Legacy mission script (`CoopWaveR.drl`); empty when none.
    pub mission_script: String,
    pub min_extent: [f32; 3],
    pub size: [f32; 3],
    pub starts: Vec<SolStart>,
    pub objects: Vec<SolObject>,
    /// Absolute file offset of the object-count record.
    pub object_count_offset: usize,
    /// Absolute file offset of the `EmptyMission` record, 0 when absent.
    pub mission_record_offset: usize,
}

/// Whether `data` is a BinaryMode SOL of [`VERSION`]. A2 and Fleet Operations
/// BZNs have the same header with versions up to 3013.
pub fn is_sol(data: &[u8]) -> bool {
    header(data).is_some_and(|(version, _)| version == VERSION)
}

/// `(version, offset of the first record)`.
fn header(bytes: &[u8]) -> Option<(u32, usize)> {
    if !bytes.starts_with(PREFIX) {
        return None;
    }
    let text = &bytes[..bytes.len().min(256)];
    let version_end = text[PREFIX.len()..].windows(2).position(|b| b == b"\r\n")? + PREFIX.len();
    let digits = std::str::from_utf8(&text[PREFIX.len()..version_end])
        .ok()?
        .trim();
    let version: u32 = digits.parse().ok()?;
    let marker = text[version_end..]
        .windows(MARKER.len())
        .position(|b| b == MARKER)?
        + version_end;
    Some((version, marker + MARKER.len()))
}

#[derive(Clone, Copy)]
struct Record<'a> {
    tag: u32,
    data: &'a [u8],
    offset: usize,
}

impl Record<'_> {
    fn u32(&self) -> Option<u32> {
        (self.tag == 4).then_some(())?;
        Some(u32::from_le_bytes(self.data.try_into().ok()?))
    }
    fn u16(&self) -> Option<u16> {
        (self.tag == 3).then_some(())?;
        Some(u16::from_le_bytes(self.data.try_into().ok()?))
    }
    fn words<const N: usize>(&self, tag: u32) -> Option<[u32; N]> {
        if self.tag != tag || self.data.len() != N * 4 {
            return None;
        }
        let mut out = [0; N];
        for (slot, c) in out.iter_mut().zip(self.data.chunks_exact(4)) {
            *slot = u32::from_le_bytes(c.try_into().ok()?);
        }
        Some(out)
    }
    fn floats<const N: usize>(&self, tag: u32) -> Option<[f32; N]> {
        self.words::<N>(tag).map(|w| w.map(f32::from_bits))
    }
    /// A tag-2 buffer up to its first NUL.
    fn text(&self) -> String {
        if self.tag != 2 {
            return String::new();
        }
        let end = self
            .data
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.data.len());
        String::from_utf8_lossy(&self.data[..end]).into_owned()
    }
}

/// A (length, string) pair at `i`: the length record must match the string's size.
fn pair(records: &[Record], i: usize) -> Option<String> {
    let length = records.get(i)?.u32()?;
    let text = records.get(i + 1)?;
    (text.tag == 2 && text.data.len() == length as usize).then(|| text.text())
}

fn tokenize(bytes: &[u8], mut offset: usize) -> Result<Vec<Record<'_>>, SolError> {
    let mut records = Vec::new();
    while offset < bytes.len() {
        if bytes.len() - offset < 8 {
            return Err(SolError::TruncatedHeader(offset));
        }
        let tag = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let payload = offset + 8;
        if size > bytes.len() - payload {
            return Err(SolError::TruncatedRecord(offset));
        }
        records.push(Record {
            tag,
            data: &bytes[payload..payload + size],
            offset,
        });
        offset = payload + size;
    }
    Ok(records)
}

/// The eight records of an object starting at `i`.
fn object(records: &[Record], i: usize, require_odf: bool) -> Option<SolObject> {
    if i + 7 >= records.len() {
        return None;
    }
    let length = records[i].u16()?;
    if length == 0 || records[i + 1].tag != 2 || records[i + 1].data.len() != length as usize {
        return None;
    }
    let id = records[i + 2].u32()?;
    let pos = records[i + 3].floats::<3>(9)?;
    let team = records[i + 4].u32()?;
    if records[i + 5].tag != 2 || records[i + 5].data.len() != 40 {
        return None;
    }
    let opaque = records[i + 6];
    if opaque.tag != 8 || opaque.data.len() != 4 {
        return None;
    }
    let transform = records[i + 7].floats::<12>(11)?;
    let odf = records[i + 1].text();
    if require_odf && odf.is_empty() {
        return None;
    }
    Some(SolObject {
        odf,
        id,
        pos,
        team: team as i32,
        instance_name: records[i + 5].text(),
        opaque_id: u32::from_le_bytes(opaque.data.try_into().ok()?),
        transform,
    })
}

/// Read a SOL.
pub fn parse(bytes: &[u8]) -> Result<SolMap, SolError> {
    if bytes.is_empty() || bytes.len() > MAX_FILE {
        return Err(SolError::Size);
    }
    let (version, offset) = header(bytes).ok_or(SolError::NotSol)?;
    let records = tokenize(bytes, offset)?;
    if records.len() < 40 {
        return Err(SolError::Prefix);
    }
    let mut map = SolMap {
        version,
        ..Default::default()
    };
    for (index, slot) in [
        (4, &mut map.game_type),
        (6, &mut map.guid),
        (8, &mut map.title),
        (10, &mut map.source_path),
        (12, &mut map.filename),
    ] {
        *slot = pair(&records, index).ok_or(SolError::Prefix)?;
    }
    map.min_extent = records[15].floats::<3>(9).ok_or(SolError::Prefix)?;
    map.size = records[16].floats::<3>(9).ok_or(SolError::Prefix)?;
    // Optional: a non-empty script name.
    if records[19].u32().is_some_and(|n| n != 0)
        && let Some(name) = pair(&records, 19)
    {
        map.mission_script = name;
    }

    let start_count = records[27].u32().ok_or(SolError::StartCount)?;
    if start_count > MAX_STARTS {
        return Err(SolError::StartCount);
    }
    for i in 0..start_count as usize {
        let base = 28 + i * 12;
        if base + 2 >= records.len() {
            return Err(SolError::StartTable);
        }
        let pos = records[base].floats::<3>(9).ok_or(SolError::StartRecord)?;
        let kind = pair(&records, base + 1).ok_or(SolError::StartRecord)?;
        map.starts.push(SolStart { pos, kind });
    }

    // The object count is the first plausible u32 record followed by a valid
    // object; the objects follow, with unknown records in between skipped.
    let first = 28 + start_count as usize * 12;
    let index = (first..records.len().saturating_sub(9))
        .find(|&i| {
            records[i].u32().is_some_and(|c| c != 0 && c <= MAX_OBJECTS)
                && object(&records, i + 1, false).is_some()
        })
        .ok_or(SolError::NoObjectTable)?;
    let declared = records[index].u32().unwrap_or(0);
    map.object_count_offset = records[index].offset;
    let mut i = index + 1;
    while i < records.len() && (map.objects.len() as u32) < declared {
        match object(&records, i, true) {
            Some(o) => {
                map.objects.push(o);
                i += 8;
            }
            None => i += 1,
        }
    }
    if (map.objects.len() as u32) != declared {
        return Err(SolError::ObjectCount {
            declared,
            found: map.objects.len() as u32,
        });
    }
    if let Some(r) = records
        .iter()
        .find(|r| r.tag == 2 && r.data.len() == 40 && r.data.starts_with(EMPTY_MISSION))
    {
        map.mission_record_offset = r.offset;
    }
    Ok(map)
}

impl SolMap {
    /// The shared [`Map`]. Start locations become entries 1.. (entry 0 is the
    /// neutral slot at the map's centre, as in BZNs), the game type is the
    /// mission type, the title the description, the Legacy script the map's
    /// script.
    pub fn to_map(&self) -> Map {
        let centre = [
            self.min_extent[0] + self.size[0] * 0.5,
            self.min_extent[1] + self.size[1] * 0.5,
            self.min_extent[2] + self.size[2] * 0.5,
        ];
        let neutral = StartLoc {
            pos: centre,
            kind: "empty".into(),
            team: 0,
            race: String::new(),
        };
        let mut start_locs = vec![neutral];
        start_locs.extend(self.starts.iter().enumerate().map(|(i, s)| StartLoc {
            pos: s.pos,
            kind: s.kind.clone(),
            team: i as i32 + 1,
            race: String::new(),
        }));
        Map {
            version: self.version,
            binary: true,
            mission_type: self.game_type.clone(),
            description: self.title.clone(),
            file_name: self.filename.clone(),
            scripts: Some(self.mission_script.clone())
                .filter(|s| !s.is_empty())
                .into_iter()
                .collect(),
            min_extent: self.min_extent,
            size: self.size,
            start_count: self.starts.len() as i32,
            start_locs,
            objects: self
                .objects
                .iter()
                .map(|o| MapObject {
                    odf: o.odf.clone(),
                    entity_id: o.id as i32,
                    team: o.team,
                    label: o.instance_name.clone(),
                    initial_position: Some(o.pos),
                    transform: o.transform,
                    light_colour: None,
                })
                .collect(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Synthetic SOL builder; the layout is the table in the module docs.
    pub(crate) struct Builder {
        out: Vec<u8>,
        pub(crate) records: usize,
    }

    impl Builder {
        pub(crate) fn new(version: u32) -> Self {
            let mut out =
                format!("version [1] =\r\n{version}\r\nBinaryMode [1] =\r\ntrue\r\n").into_bytes();
            out.shrink_to_fit();
            Builder { out, records: 0 }
        }
        pub(crate) fn rec(&mut self, tag: u32, data: &[u8]) -> &mut Self {
            self.out.extend(tag.to_le_bytes());
            self.out.extend((data.len() as u32).to_le_bytes());
            self.out.extend(data);
            self.records += 1;
            self
        }
        pub(crate) fn filler(&mut self, n: usize) -> &mut Self {
            for _ in 0..n {
                self.rec(1, &[1]);
            }
            self
        }
        fn u32(&mut self, v: u32) -> &mut Self {
            self.rec(4, &v.to_le_bytes())
        }
        fn vec3(&mut self, v: [f32; 3]) -> &mut Self {
            let b: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
            self.rec(9, &b)
        }
        fn string_pair(&mut self, s: &str) -> &mut Self {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            self.u32(b.len() as u32).rec(2, &b)
        }
        fn object(
            &mut self,
            odf: &str,
            id: u32,
            pos: [f32; 3],
            team: u32,
            name: &str,
        ) -> &mut Self {
            let mut o = odf.as_bytes().to_vec();
            o.push(0);
            self.rec(3, &(o.len() as u16).to_le_bytes())
                .rec(2, &o)
                .u32(id)
                .vec3(pos)
                .u32(team);
            let mut n = name.as_bytes().to_vec();
            n.resize(40, 0);
            self.rec(2, &n).rec(8, &(id + 1000).to_le_bytes());
            // right, up, front, position
            let t = [
                1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, pos[0], pos[1], pos[2],
            ];
            let b: Vec<u8> = t.iter().flat_map(|f| f.to_le_bytes()).collect();
            self.rec(11, &b)
        }
        pub(crate) fn bytes(&self) -> Vec<u8> {
            self.out.clone()
        }
    }

    /// A SOL with 3 start locations, a mission script and 2 objects.
    pub(crate) fn fixture() -> Vec<u8> {
        fixture_with("Test.drl")
    }

    /// An empty script name writes the absent form: length 0, empty string.
    fn fixture_with(script: &str) -> Vec<u8> {
        let mut b = Builder::new(3021);
        b.filler(4); // 0..3
        b.string_pair("mp")
            .string_pair("{GUID}")
            .string_pair("Test Map")
            .string_pair("sol\\test.sol")
            .string_pair("test.sol"); // 4..13
        b.filler(1); // 14
        b.vec3([-1000.0, 0.0, -2000.0])
            .vec3([2000.0, 100.0, 4000.0]); // 15, 16
        b.filler(2); // 17, 18
        if script.is_empty() {
            b.u32(0).rec(2, &[]); // 19, 20
        } else {
            b.string_pair(script);
        }
        b.filler(6); // 21..26
        b.u32(3); // 27
        for (i, p) in [
            [-500.0f32, 0.0, -500.0],
            [500.0, 0.0, 500.0],
            [0.0, 0.0, 900.0],
        ]
        .into_iter()
        .enumerate()
        {
            b.vec3(p)
                .string_pair(if i == 2 { "ai-easy" } else { "player" });
            b.filler(9); // 12 records per start: 1 + 2 + 9
        }
        b.filler(3); // unknown records before the table
        b.u32(2);
        b.object("fcruiser", 7, [10.0, 0.0, 20.0], 1, "flagship");
        b.filler(2); // unknown records between objects
        b.object("fyard", 8, [-30.0, 5.0, 40.0], 2, "");
        let mut mission = b"EmptyMission\0".to_vec();
        mission.resize(40, 0);
        b.rec(2, &mission);
        b.bytes()
    }

    #[test]
    fn parses_the_fixture() {
        let m = parse(&fixture()).unwrap();
        assert_eq!(m.version, 3021);
        assert_eq!(
            (m.game_type.as_str(), m.guid.as_str(), m.title.as_str()),
            ("mp", "{GUID}", "Test Map")
        );
        assert_eq!(
            (m.source_path.as_str(), m.filename.as_str()),
            ("sol\\test.sol", "test.sol")
        );
        assert_eq!(m.mission_script, "Test.drl");
        assert_eq!(
            (m.min_extent, m.size),
            ([-1000.0, 0.0, -2000.0], [2000.0, 100.0, 4000.0])
        );
        assert_eq!(m.starts.len(), 3);
        assert_eq!(
            m.starts[1],
            SolStart {
                pos: [500.0, 0.0, 500.0],
                kind: "player".into()
            }
        );
        assert_eq!(m.starts[2].kind, "ai-easy");
        assert_eq!(m.objects.len(), 2);
        let o = &m.objects[0];
        assert_eq!(
            (
                o.odf.as_str(),
                o.id,
                o.team,
                o.instance_name.as_str(),
                o.opaque_id
            ),
            ("fcruiser", 7, 1, "flagship", 1007)
        );
        assert_eq!(o.pos, [10.0, 0.0, 20.0]);
        assert_eq!(&o.transform[9..], [10.0, 0.0, 20.0]);
        assert_eq!(m.objects[1].odf, "fyard");
        assert_eq!(m.objects[1].instance_name, "");
        assert!(m.object_count_offset > 0 && m.mission_record_offset > m.object_count_offset);
    }

    #[test]
    fn converts_to_the_shared_map() {
        let sol = parse(&fixture()).unwrap();
        let m = sol.to_map();
        assert_eq!(
            (
                m.version,
                m.binary,
                m.mission_type.as_str(),
                m.description.as_str()
            ),
            (3021, true, "mp", "Test Map")
        );
        assert_eq!(m.scripts, ["Test.drl"]);
        assert_eq!(m.start_count, 3);
        assert_eq!(m.start_locs.len(), 4);
        assert_eq!(
            m.start_locs[0].pos,
            [0.0, 50.0, 0.0],
            "entry 0 is the map's centre"
        );
        assert_eq!(m.start_locs[1].pos, [-500.0, 0.0, -500.0]);
        assert_eq!(
            (m.start_locs[3].team, m.start_locs[3].kind.as_str()),
            (3, "ai-easy")
        );
        assert_eq!(m.objects.len(), 2);
        assert_eq!(m.objects[0].odf, "fcruiser");
        assert_eq!(
            (
                m.objects[0].entity_id,
                m.objects[0].team,
                m.objects[0].label.as_str()
            ),
            (7, 1, "flagship")
        );
    }

    #[test]
    fn detection_is_by_version() {
        assert!(is_sol(&fixture()));
        assert!(!is_sol(&Builder::new(3013).bytes()));
        assert!(!is_sol(
            b"version [1] =\r\n3021\r\nBinaryMode [1] =\r\nfalse\r\n"
        ));
        assert!(!is_sol(b"not a map"));
    }

    #[test]
    fn rejects_bad_files() {
        assert_eq!(parse(b""), Err(SolError::Size));
        assert_eq!(parse(b"hello"), Err(SolError::NotSol));
        assert_eq!(parse(&Builder::new(3021).bytes()), Err(SolError::Prefix));
        let good = fixture();
        // Cut inside a record.
        assert!(matches!(
            parse(&good[..good.len() - 5]),
            Err(SolError::TruncatedRecord(_))
        ));
        // Cut inside a record header.
        let mut cut = good.clone();
        cut.extend([1, 2, 3]);
        assert!(matches!(parse(&cut), Err(SolError::TruncatedHeader(_))));
    }

    #[test]
    fn too_many_starts_and_missing_objects() {
        let mut b = Builder::new(3021);
        b.filler(4);
        b.string_pair("mp")
            .string_pair("g")
            .string_pair("t")
            .string_pair("p")
            .string_pair("f");
        b.filler(1)
            .vec3([0.0; 3])
            .vec3([1.0; 3])
            .filler(2)
            .u32(0)
            .filler(7);
        b.u32(65);
        b.filler(20);
        assert_eq!(parse(&b.bytes()), Err(SolError::StartCount));

        // Declares 3 objects, has 2.
        let good = parse(&fixture()).unwrap();
        let mut bytes = fixture();
        let at = good.object_count_offset;
        bytes[at + 8..at + 12].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(
            parse(&bytes),
            Err(SolError::ObjectCount {
                declared: 3,
                found: 2
            })
        );
    }

    #[test]
    fn a_script_is_optional() {
        let m = parse(&fixture_with("")).unwrap();
        assert_eq!(m.mission_script, "");
        assert_eq!(m.objects.len(), 2);
        assert!(m.to_map().scripts.is_empty());
    }

    #[test]
    fn crate_parse_hands_sol_files_here() {
        let bytes = fixture();
        assert_eq!(
            crate::parse(&bytes).unwrap(),
            parse(&bytes).unwrap().to_map()
        );
        assert!(matches!(
            crate::parse(&fixture()[..60]),
            Err(crate::BznError::Sol(_))
        ));
    }

    /// The size of the map the original was validated on: 8 starts, 19 objects.
    #[test]
    fn eight_starts_and_nineteen_objects() {
        let mut b = Builder::new(3021);
        b.filler(4);
        b.string_pair("mp")
            .string_pair("g")
            .string_pair("Bajor")
            .string_pair("p")
            .string_pair("f");
        b.filler(1)
            .vec3([0.0; 3])
            .vec3([1.0; 3])
            .filler(2)
            .u32(0)
            .rec(2, &[])
            .filler(6);
        b.u32(8);
        for i in 0..8 {
            b.vec3([i as f32, 0.0, 0.0]).string_pair("player").filler(9);
        }
        b.u32(19);
        for i in 0..19 {
            b.object("ambientlight", i, [i as f32, 0.0, 0.0], 0, "");
        }
        let m = parse(&b.bytes()).unwrap();
        assert_eq!((m.starts.len(), m.objects.len()), (8, 19));
        assert_eq!(m.objects[18].id, 18);
    }

    /// Install-gated: `STA64_TEST_SOL` names a real `.sol` (Star Trek: Legacy
    /// `mpbajor_all.sol` gives 8 start locations and 19 objects).
    #[test]
    fn a_real_sol() {
        let Some(path) = std::env::var_os("STA64_TEST_SOL") else {
            eprintln!("SKIP: set STA64_TEST_SOL to a Star Trek: Legacy .sol");
            return;
        };
        let m = parse(&std::fs::read(path).unwrap()).unwrap();
        eprintln!(
            "{}: {} starts, {} objects, script {:?}",
            m.title,
            m.starts.len(),
            m.objects.len(),
            m.mission_script
        );
        assert!(!m.starts.is_empty() && !m.objects.is_empty());
    }
}
