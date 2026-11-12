//! Armada 1 binary map bodies back to bytes (CON-38), in the layout
//! [`super::parse_map`] reads (docs/A1_BZN_IMPORT.md).
//!
//! Canonical, not a byte copy of the original file:
//!
//! - the upper 24 bits of every record tag are zero (the native readers compare
//!   only the type byte; the original's upper bits are not a stable field-name
//!   hash);
//! - the native class payload between placements (CON-40) is not written, except
//!   a light's colour record shape (CON-41) so the colour survives;
//! - the header is the original's form: CRLF lines, the 128-byte description as
//!   256 lower-case hex digits.
//!
//! Everything the reader keeps is written, and [`write_map`] reads its own output
//! back and refuses to return bytes that do not reproduce the [`Map`]. A resized
//! map (`Map::a1`'s `resized`) is written with the source bounds and positions
//! mapped back from the final ones; where float rounding makes that inexact the
//! write fails with [`WriteError::A1NotExact`] rather than drift.

use super::MapData;
use crate::write::WriteError;
use crate::{Map, MapObject};

struct Out(Vec<u8>);

impl Out {
    fn rec(&mut self, kind: u8, data: &[u8]) {
        self.0.extend(u32::from(kind).to_le_bytes());
        self.0.extend((data.len() as u32).to_le_bytes());
        self.0.extend(data);
    }
    fn int(&mut self, v: i32) {
        self.rec(4, &v.to_le_bytes());
    }
    fn word(&mut self, kind: u8, v: u32) {
        self.rec(kind, &v.to_le_bytes());
    }
    fn float(&mut self, v: f32) {
        self.rec(5, &v.to_le_bytes());
    }
    fn boolean(&mut self, v: bool) {
        self.rec(1, &[u8::from(v)]);
    }
    fn vec3(&mut self, v: [f32; 3]) {
        let b: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.rec(9, &b);
    }
    /// A zero-padded text of `size` bytes: `kind` 2 keeps a NUL (every stock buffer
    /// has one), `kind` 7 (a PrjID) fills its eight bytes.
    fn text(
        &mut self,
        kind: u8,
        field: &'static str,
        s: &str,
        size: usize,
    ) -> Result<(), WriteError> {
        let max = if kind == 7 { size } else { size - 1 };
        if s.len() > max {
            return Err(WriteError::TooLong {
                field,
                len: s.len(),
                max,
            });
        }
        let mut b = s.as_bytes().to_vec();
        b.resize(size, 0);
        self.rec(kind, &b);
        Ok(())
    }
}

fn finite(field: &'static str, v: &[f32]) -> Result<(), WriteError> {
    if v.iter().all(|f| f.is_finite()) {
        Ok(())
    } else {
        Err(WriteError::NotFinite(field))
    }
}

/// `(min, max)` as the file stores them: six floats, `minX maxX minY maxY minZ maxZ`.
fn bounds(o: &mut Out, min: [f32; 3], max: [f32; 3]) {
    for axis in 0..3 {
        o.float(min[axis]);
        o.float(max[axis]);
    }
}

/// The forward resize of one horizontal value, exactly as the reader does it.
fn resize(v: f32, from: (f32, f32), to: (f32, f32)) -> f32 {
    let (lo, hi) = (f64::from(from.0), f64::from(from.1));
    let (new_lo, new_hi) = (f64::from(to.0), f64::from(to.1));
    (new_lo + (f64::from(v) - lo) / (hi - lo) * (new_hi - new_lo)) as f32
}

fn neighbours(v: f32) -> impl Iterator<Item = f32> {
    let bits = v.to_bits();
    // Bits move monotonically with magnitude within a sign; a step across zero or
    // past the largest finite value gives NaN or inf, which the callers skip.
    let step = move |d: i32| f32::from_bits((bits as i64 + i64::from(d)) as u32);
    [0, 1, -1, 2, -2, 3, -3].into_iter().map(step)
}

/// A source value that the reader's forward resize maps to exactly `target`.
fn unresize(target: f32, from: (f32, f32), to: (f32, f32)) -> Result<f32, WriteError> {
    let (lo, hi) = (f64::from(from.0), f64::from(from.1));
    let (new_lo, new_hi) = (f64::from(to.0), f64::from(to.1));
    let guess = (lo + (f64::from(target) - new_lo) / (new_hi - new_lo) * (hi - lo)) as f32;
    neighbours(guess)
        .find(|c| c.is_finite() && resize(*c, from, to) == target)
        .ok_or(WriteError::A1NotExact)
}

/// The header lines, in the original's form.
fn header(data: &MapData, version: u32) -> Vec<u8> {
    let hex: String = data
        .header
        .save_game_desc
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let crlf = format!(
        "version [1] =\r\n{version}\r\nsaveGameDesc = {hex}\r\nbinarySave [1] =\r\ntrue\r\n"
    );
    // The reader keeps the header's length; a file with LF endings (five lines, five
    // bytes shorter) is written with LF again.
    if data.header.metadata_offset + 5 == crlf.len() {
        return crlf.replace("\r\n", "\n").into_bytes();
    }
    crlf.into_bytes()
}

/// A light's payload in the shape [`super::body`] recognises: the colour, `pos`,
/// `velMag`/`velMagInv`, `vel`/`omega`/`accel` and the 32-byte `name`.
fn light(o: &mut Out, colour: [f32; 3], pos: [f32; 3]) {
    for c in colour {
        o.float(c);
    }
    o.vec3(pos);
    // `velMag` is 0 and `velMagInv` about 1e30 in every stock light (0x7149F2CA); `vel`,
    // `omega` and `accel` are zero. `pos` is the placement's: the stock files keep a
    // slightly different value there that is not read (CON-40).
    o.float(0.0);
    o.float(f32::from_bits(0x7149_F2CA));
    for _ in 0..3 {
        o.vec3([0.0; 3]);
    }
    let mut name = b"Directional light".to_vec();
    name.resize(32, 0);
    o.rec(2, &name);
}

pub(crate) fn write_map(map: &Map, data: &MapData) -> Result<Vec<u8>, WriteError> {
    let n = map.objects.len();
    if data.object_refs.len() != n || data.initial_positions.len() != n {
        return Err(WriteError::A1NotExact);
    }
    let (src_min, src_max) = data.source_bounds;
    let target_min = map.min_extent;
    let size = map.size;
    finite("bounds", &[src_min, src_max, target_min, size].concat())?;
    finite("ambient", &map.ambient)?;

    let mut o = Out(header(data, map.version));
    o.text(2, "msn_filename", &map.file_name, 16)?;
    o.boolean(true); // missionSave: saved-game state is not a map
    o.text(2, "terrain name", &data.terrain_name, 100)?;
    bounds(&mut o, src_min, src_max);
    o.boolean(data.resized);

    // The target bounds the reader reports are `min` and `min + size` computed in
    // f32 as `max - min`; find the stored `max` that gives exactly `size`.
    let target_max = if data.resized {
        let mut max = [0.0; 3];
        for i in 0..3 {
            max[i] = neighbours(target_min[i] + size[i])
                .find(|m| m.is_finite() && *m - target_min[i] == size[i])
                .ok_or(WriteError::A1NotExact)?;
        }
        bounds(&mut o, target_min, max);
        max
    } else {
        // Unresized: the target is the source, exactly as retained.
        src_max
    };
    let axes = |axis: usize| {
        (
            (src_min[axis], src_max[axis]),
            (target_min[axis], target_max[axis]),
        )
    };
    // Positions as the file stores them (before the reader's resize).
    let source_point = |p: [f32; 3]| -> Result<[f32; 3], WriteError> {
        let mut q = p;
        if data.resized {
            for axis in [0, 2] {
                let (from, to) = axes(axis);
                q[axis] = unresize(p[axis], from, to)?;
            }
        }
        Ok(q)
    };

    for c in map.ambient {
        o.float(c);
    }
    o.text(2, "background", &map.background, 100)?;
    o.text(2, "tech tree file", &map.tech_tree_file, 128)?;
    o.text(2, "race", &map.race, 128)?;
    o.int(n as i32);

    for (i, obj) in map.objects.iter().enumerate() {
        write_object(
            &mut o,
            i,
            obj,
            data.object_refs[i],
            data.initial_positions[i],
            &source_point,
        )?;
    }

    o.text(2, "mission class", &map.mission_type, 40)?;
    o.word(8, data.mission_reference);
    // `scripts` keeps the non-empty names in order: script first, then rule.
    o.text(
        2,
        "script name",
        map.scripts.first().map_or("", String::as_str),
        64,
    )?;
    o.text(
        2,
        "rule name",
        map.scripts.get(1).map_or("", String::as_str),
        64,
    )?;
    o.rec(0, &data.mission_title);

    o.int(data.areas.len() as i32);
    for a in &data.areas {
        o.word(8, a.object_ref);
        o.int(a.team);
        o.boolean(a.interesting);
        o.boolean(a.inside);
        o.int(a.value);
        o.int(a.force);
    }
    o.int(data.paths.len() as i32);
    for p in &data.paths {
        o.word(0, p.reference);
        o.int(p.label.len() as i32);
        if !p.label.is_empty() {
            o.rec(2, p.label.as_bytes());
        }
        o.int(p.points.len() as i32);
        let mut bytes = Vec::with_capacity(p.points.len() * 12);
        for pt in &p.points {
            finite("path point", pt)?;
            for v in source_point(*pt)? {
                bytes.extend(v.to_le_bytes());
            }
        }
        o.rec(9, &bytes);
        o.word(0, p.path_type as u32);
        finite("asteroid belt speed", &[p.asteroid_belt_speed])?;
        o.float(p.asteroid_belt_speed);
    }

    // Never return bytes that do not read back to this map.
    let back = crate::parse(&o.0).map_err(|e| WriteError::A1Reparse(e.to_string()))?;
    if back != *map {
        return Err(WriteError::A1NotExact);
    }
    Ok(o.0)
}

fn write_object(
    o: &mut Out,
    index: usize,
    obj: &MapObject,
    reference: u32,
    initial: [f32; 3],
    source_point: &dyn Fn([f32; 3]) -> Result<[f32; 3], WriteError>,
) -> Result<(), WriteError> {
    finite("object transform", &obj.transform)?;
    finite("object position", &initial)?;
    if obj.odf.is_empty() {
        return Err(WriteError::EmptyOdf { index });
    }
    o.text(7, "PrjID", &obj.odf, 8)?;
    o.int(obj.entity_id);
    o.vec3(source_point(initial)?);
    o.int(obj.team);
    o.text(2, "label", &obj.label, 40)?;
    o.word(8, reference);
    let mut t = obj.transform;
    let at = source_point([t[9], t[10], t[11]])?;
    t[9..12].copy_from_slice(&at);
    let bytes: Vec<u8> = t.iter().flat_map(|f| f.to_le_bytes()).collect();
    o.rec(11, &bytes);
    if let Some(colour) = obj.light_colour {
        finite("light colour", &colour)?;
        light(o, colour, source_point(initial)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a1::{AiPath, AreaOfInterest};

    /// A small, fully populated A1 map built in memory (never from game bytes).
    fn sample(resized: bool) -> Map {
        let mut desc = [0u8; 128];
        desc[..4].copy_from_slice(b"Test");
        let header = crate::a1::Header {
            version: 2051,
            save_game_desc: desc,
            binary_save: true,
            metadata_offset: 318,
        };
        let (src_min, src_max) = ([0.0, -50.0, 0.0], [6400.0, 150.0, 6400.0]);
        let (min, max) = if resized {
            ([0.0, -50.0, 0.0], [3200.0, 150.0, 3200.0])
        } else {
            (src_min, src_max)
        };
        let object =
            |odf: &str, id: i32, p: [f32; 3], team: i32, label: &str, light: Option<[f32; 3]>| {
                MapObject {
                    odf: odf.into(),
                    entity_id: id,
                    team,
                    label: label.into(),
                    initial_position: Some(p),
                    transform: [
                        1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, p[0], p[1], p[2],
                    ],
                    light_colour: light,
                }
            };
        let objects = vec![
            object("bcruiser", -7, [1000.0, 10.0, 2000.5], 1, "Flagship", None),
            object(
                "dlight",
                9,
                [1500.0, 38.5, 1500.0],
                0,
                "",
                Some([1.0, 0.5, 0.25]),
            ),
            object("mbaku", 12, [2500.0, 0.0, 3000.0], 2, "Baku", None),
        ];
        let paths = vec![
            AiPath {
                reference: 11,
                label: "Camera_1".into(),
                points: vec![[1000.0, 38.5, 1000.0], [2000.0, 38.5, 1500.0]],
                path_type: 1,
                asteroid_belt_speed: 0.5,
            },
            AiPath {
                reference: 12,
                label: String::new(),
                points: vec![],
                path_type: 0,
                asteroid_belt_speed: 0.0,
            },
        ];
        let mut mission_title = [0u8; 64];
        mission_title[..5].copy_from_slice(b"Title");
        Map {
            version: 2051,
            binary: true,
            mission_type: "Inst4XMission".into(),
            description: "Title".into(),
            file_name: "test.bzn".into(),
            tech_tree: "fed.tt".into(),
            tech_tree_file: "fed.tt".into(),
            race: "fed".into(),
            scripts: vec!["a.dll".into(), "rules.drl".into()],
            min_extent: min,
            size: [max[0] - min[0], max[1] - min[1], max[2] - min[2]],
            ambient: [0.25, 0.5, 0.75],
            background: "bg.sod".into(),
            starfield_odf: String::new(),
            objects,
            a1: Some(MapData {
                header,
                terrain_name: "terrain".into(),
                source_bounds: (src_min, src_max),
                resized,
                declared_object_count: 3,
                object_refs: vec![0x1111, 0x2222, 0x3333],
                initial_positions: vec![
                    [1000.0, 10.0, 2000.5],
                    [1500.0, 38.5, 1500.0],
                    [2500.0, 0.0, 3000.0],
                ],
                mission_title,
                mission_reference: 0xABCD,
                paths,
                areas: vec![AreaOfInterest {
                    object_ref: 5,
                    team: -1,
                    interesting: true,
                    inside: false,
                    value: 3,
                    force: -4,
                }],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn round_trip_keeps_everything_the_reader_keeps() {
        let m = sample(false);
        let bytes = crate::write::write(&m).unwrap();
        assert_eq!(crate::parse(&bytes).unwrap(), m);
        // Deterministic, and a fixed point of read then write.
        assert_eq!(
            crate::write::write(&crate::parse(&bytes).unwrap()).unwrap(),
            bytes
        );
        assert!(bytes.starts_with(b"version [1] =\r\n2051\r\nsaveGameDesc = 54657374"));
    }

    #[test]
    fn light_colours_survive() {
        let m = sample(false);
        let back = crate::parse(&crate::write::write(&m).unwrap()).unwrap();
        assert_eq!(back.objects[1].light_colour, Some([1.0, 0.5, 0.25]));
        assert_eq!(back.objects[0].light_colour, None);
        assert_eq!(back.objects[2].light_colour, None);
    }

    #[test]
    fn resized_maps_write_source_coordinates() {
        let m = sample(true);
        // `sample` gives final positions inside the target bounds; the file holds the
        // source-space values, which read back (through the resize) to the same map.
        let bytes = crate::write::write(&m).unwrap();
        assert_eq!(crate::parse(&bytes).unwrap(), m);
        // Half-size target: the file holds twice the final X and Z, Y unchanged.
        let (_, recs) = records(&bytes);
        let pos = recs
            .iter()
            .find(|r| {
                r.0 == 9
                    && r.1.len() == 12
                    && f32::from_le_bytes(r.1[0..4].try_into().unwrap()) == 2000.0
            })
            .expect("object 0 stored at twice its final X");
        assert_eq!(f32::from_le_bytes(pos.1[4..8].try_into().unwrap()), 10.0);
        assert_eq!(f32::from_le_bytes(pos.1[8..12].try_into().unwrap()), 4001.0);
    }

    #[test]
    fn too_long_or_missing_fields_are_refused() {
        let mut m = sample(false);
        m.objects[0].odf = "nine_char".into();
        assert!(matches!(
            crate::write::write(&m),
            Err(WriteError::TooLong { field: "PrjID", .. })
        ));
        let mut m = sample(false);
        m.race = "x".repeat(129);
        assert!(matches!(
            crate::write::write(&m),
            Err(WriteError::TooLong { field: "race", .. })
        ));
        let mut m = sample(false);
        m.ambient[1] = f32::NAN;
        assert_eq!(
            crate::write::write(&m),
            Err(WriteError::NotFinite("ambient"))
        );
        let mut m = sample(false);
        m.objects.pop();
        assert_eq!(crate::write::write(&m), Err(WriteError::A1NotExact));
    }

    #[test]
    fn written_files_are_valid_a1_records() {
        // Every record tag has zero upper bytes and the stream walks to the end.
        let bytes = crate::write::write(&sample(false)).unwrap();
        let start = bytes.windows(6).position(|w| w == b"true\r\n").unwrap() + 6;
        let mut at = start;
        let mut count = 0;
        while at < bytes.len() {
            let tag = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            assert!(tag < 256, "tag {tag:#x}");
            at += 8 + size;
            count += 1;
        }
        assert_eq!(at, bytes.len());
        assert!(count > 40);
    }

    /// The records of a file as `(type byte, data)`, from the end of the header.
    fn records(bytes: &[u8]) -> (Vec<u8>, Vec<(u8, Vec<u8>)>) {
        let end = bytes.windows(6).position(|w| w == b"true\r\n").unwrap() + 6;
        let mut at = end;
        let mut out = Vec::new();
        while at + 8 <= bytes.len() {
            let tag = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            out.push((tag as u8, bytes[at + 8..at + 8 + size].to_vec()));
            at += 8 + size;
        }
        (bytes[..end].to_vec(), out)
    }

    type Rec = (u8, Vec<u8>);

    fn is_placement(r: &[Rec]) -> bool {
        r.len() >= 7 && r[0].0 == 7 && r[0].1.len() == 8 && r[1].0 == 4 && r[2].0 == 9
    }

    fn is_mission(r: &[Rec]) -> bool {
        r.len() >= 5
            && r[0].0 == 2
            && r[0].1.len() == 40
            && (r[0].1.starts_with(b"EmptyMission") || r[0].1.starts_with(b"Inst4XMission"))
            && r[1].0 == 8
    }

    /// What the writer and the original can be compared on: after each placement
    /// prefix only a light's three colour records are kept (the native class payload
    /// is not written, CON-40).
    fn comparable(recs: &[Rec]) -> Vec<Rec> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < recs.len() {
            if is_placement(&recs[i..]) {
                out.extend_from_slice(&recs[i..i + 7]);
                let mut j = i + 7;
                let colour = recs.len() >= j + 4
                    && recs[j..j + 3].iter().all(|r| r.0 == 5 && r.1.len() == 4)
                    && recs[j + 3].0 == 9;
                if colour {
                    out.extend_from_slice(&recs[j..j + 3]);
                    j += 3;
                }
                while j < recs.len() && !is_placement(&recs[j..]) && !is_mission(&recs[j..]) {
                    j += 1;
                }
                i = j;
            } else {
                out.push(recs[i].clone());
                i += 1;
            }
        }
        // Fixed text buffers keep whatever bytes followed the NUL in the writer's memory
        // (`2wide.bzn\0n\0`); only the text up to it is data.
        for (kind, data) in &mut out {
            if matches!(*kind, 2 | 7) {
                let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                data.truncate(end);
            }
        }
        out
    }

    /// Install-gated: `STA64_TEST_A1_BZN_DIR` names the stock Armada I `bzn` directory.
    /// Every map reads, writes and reads back to an equal `Map` (all 49 of the stock
    /// install); the header bytes are compared with the original's, and the records of
    /// each map with the original's on everything but the native class payload.
    #[test]
    fn every_stock_a1_map_round_trips() {
        let Some(dir) = std::env::var_os("STA64_TEST_A1_BZN_DIR") else {
            eprintln!("SKIP: set STA64_TEST_A1_BZN_DIR to the stock Armada I bzn directory");
            return;
        };
        let (mut maps, mut lights, mut equal_records, mut header_equal) = (0, 0, 0, 0);
        let mut failed = Vec::new();
        let mut first_diffs = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bzn")) {
                continue;
            }
            let original = std::fs::read(&p).unwrap();
            let Ok(map) = crate::parse(&original) else {
                failed.push(format!("{}: does not read", p.display()));
                continue;
            };
            if map.a1.is_none() {
                failed.push(format!("{}: not an A1 body", p.display()));
                continue;
            }
            maps += 1;
            lights += map
                .objects
                .iter()
                .filter(|o| o.light_colour.is_some())
                .count();
            let bytes = match crate::write::write(&map) {
                Ok(b) => b,
                Err(e) => {
                    failed.push(format!("{}: write: {e}", p.display()));
                    continue;
                }
            };
            match crate::parse(&bytes) {
                Ok(back) if back == map => {}
                _ => {
                    failed.push(format!("{}: read back differs", p.display()));
                    continue;
                }
            }
            let ((oh, orecs), (nh, nrecs)) = (records(&original), records(&bytes));
            header_equal += (oh == nh) as usize;
            let (a, b) = (comparable(&orecs), comparable(&nrecs));
            if a == b {
                equal_records += 1;
            } else if first_diffs.len() < 5 {
                let i = a
                    .iter()
                    .zip(&b)
                    .position(|(x, y)| x != y)
                    .unwrap_or(a.len().min(b.len()));
                let show = |r: Option<&Rec>| {
                    r.map(|r| {
                        (
                            r.0,
                            r.1.len(),
                            r.1.iter().take(12).copied().collect::<Vec<_>>(),
                        )
                    })
                };
                first_diffs.push(format!(
                    "{}: record {i} of {}/{}: original {:?} vs ours {:?}",
                    p.display(),
                    a.len(),
                    b.len(),
                    show(a.get(i)),
                    show(b.get(i))
                ));
            }
        }
        eprintln!(
            "A1 round trip: {maps} maps equal, {lights} lights, {header_equal} headers byte-equal, {equal_records} maps record-equal to the original apart from the class payload and tag upper bytes; {} failed",
            failed.len()
        );
        for d in &first_diffs {
            eprintln!("  {d}");
        }
        for f in failed.iter().take(10) {
            eprintln!("  {f}");
        }
        assert!(
            failed.is_empty() && maps == 49,
            "{} failed, {maps} maps",
            failed.len()
        );
        // The stock install: 106 lights (CON-41), the original header and every record.
        assert_eq!((lights, header_equal, equal_records), (106, 49, 49));
    }

    #[test]
    fn lf_headers_are_written_with_lf() {
        let mut m = sample(false);
        let a1 = m.a1.as_mut().unwrap();
        a1.header.metadata_offset = 313;
        let bytes = crate::write::write(&m).unwrap();
        assert!(
            !bytes.windows(2).any(|w| w == b"\r\n"),
            "CRLF in an LF file"
        );
        assert_eq!(crate::parse(&bytes).unwrap(), m);
    }

    #[test]
    fn resize_maps_back_for_awkward_factors() {
        // Not a power of two, with an offset target: a value the reader's resize produced
        // has a stored source value that gives it back exactly.
        let (from, to) = ((0.0_f32, 6400.0_f32), (500.0_f32, 4500.0_f32));
        let mut exact = 0;
        for i in 0..2000 {
            let source = i as f32 * 3.1 + 0.37;
            let target = resize(source, from, to);
            if let Ok(back) = unresize(target, from, to) {
                assert_eq!(resize(back, from, to), target);
                exact += 1;
            }
        }
        assert!(exact > 1900, "only {exact} of 2000 values map back");
    }

    #[test]
    fn a_full_text_buffer_is_refused_so_every_text_keeps_its_nul() {
        let mut m = sample(false);
        m.file_name = "x".repeat(16);
        assert!(matches!(
            crate::write::write(&m),
            Err(WriteError::TooLong {
                field: "msn_filename",
                max: 15,
                ..
            })
        ));
        let mut m = sample(false);
        m.file_name = "x".repeat(15);
        assert!(crate::write::write(&m).is_ok());
        let mut m = sample(false);
        m.objects[2].odf = "eightchr".into(); // a PrjID fills its eight bytes
        assert!(crate::write::write(&m).is_ok());
    }

    #[test]
    fn header_variants_round_trip_and_keep_the_description_bytes() {
        for version in [2050, 2051, 2052, 2053] {
            let mut m = sample(false);
            m.version = version;
            let a1 = m.a1.as_mut().unwrap();
            a1.header.version = version;
            // Not UTF-8, and a NUL inside: the 128 bytes are kept as they are.
            a1.header.save_game_desc = [0xE9; 128];
            a1.header.save_game_desc[5] = 0;
            let bytes = crate::write::write(&m).unwrap();
            let back = crate::parse(&bytes).unwrap();
            assert_eq!(back, m, "version {version}");
            assert_eq!(
                back.a1.unwrap().header.save_game_desc[..6],
                [0xE9, 0xE9, 0xE9, 0xE9, 0xE9, 0]
            );
        }
    }

    #[test]
    fn a_light_colour_is_read_only_from_the_whole_light_shape() {
        // Three floats after a placement are not a light unless the rest of the
        // class payload follows in the DLight order.
        let mut broken = crate::write::write(&sample(false)).unwrap();
        // Corrupt the light's 32-byte name record into a 31-byte one: no longer the shape.
        let name = broken
            .windows(17)
            .position(|w| w == b"Directional light")
            .unwrap();
        broken[name - 4..name].copy_from_slice(&31u32.to_le_bytes());
        broken.drain(name + 31..name + 32);
        let back = crate::parse(&broken).unwrap();
        assert_eq!(
            back.objects[1].light_colour, None,
            "not a light without the 32-byte name"
        );
    }

    #[test]
    fn a1_bodies_that_do_not_parse_back_are_errors() {
        // A body the reader rejects (here: a header with a version it refuses).
        let mut m = sample(false);
        m.version = 9999;
        assert!(matches!(
            crate::write::write(&m),
            Err(WriteError::A1Reparse(_))
        ));
    }
}
