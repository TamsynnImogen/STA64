//! Map writer (CON-32): a [`Map`] back to BZN bytes that [`crate::parse`]
//! reads as the same [`Map`].
//!
//! Two bodies, chosen by [`Map::binary`], each in the layout the reader's
//! anchors expect (see the crate docs):
//!
//! - **binary**: the header text, then records `u32 type, u32 size, data`:
//!   five length-prefixed header strings (mission type, a spare, description,
//!   a spare, file name), the tech tree and script names, the extent and size
//!   vectors, the start-location count and table, the three ambient floats, the
//!   four fixed background buffers (100, 100, 128, 128 bytes), the object count
//!   and the objects (`PrjIdLen`, `PrjID`, id, position, team, label buffer,
//!   opaque id, transform);
//! - **text**: `name [1] =` fields, strings as a length then a hex value,
//!   vectors as nested `x`, `y`, `z`.
//!
//! The round trip is exact for what [`Map`] holds, and that is all of what the
//! reader takes from a file. It is not a byte copy of the original: unread
//! parts of a file (records the reader skips, formatting, the spare strings)
//! are written in a canonical form, so [`write`] of a map read from an install
//! is not byte-equal to its file. Armada 1 bodies ([`Map::a1`]) are written by
//! [`crate::a1`] (CON-38, light colours CON-41). Light colours cannot be written
//! in A2/FO binary maps (the reader takes none from them) and text maps hold
//! two scripts.
//!
//! The native save format for editor-made maps is `MapDef` as JSON5
//! (`sta_content::json5::save_map`).

use crate::{Map, MapObject, StartLoc};

/// Why a map could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("the Armada 1 map does not reproduce through its own record layout (a count, a bound or a resized position that float rounding cannot map back)")]
    A1NotExact,
    #[error("the Armada 1 body written for this map does not read back: {0}")]
    A1Reparse(String),
    #[error(
        "Star Trek: Legacy SOL maps are not written; the BZN writer would produce a file that reads back as a broken SOL"
    )]
    Sol,
    #[error("{0} start locations are announced but none are given")]
    StartCount(i32),
    #[error("{field} is {len} bytes; the format holds {max}")]
    TooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    #[error("object {index} has no ODF name")]
    EmptyOdf { index: usize },
    #[error("{0} is not a finite number")]
    NotFinite(&'static str),
    #[error("a text map holds at most two scripts, this one has {0}")]
    TooManyScripts(usize),
}

/// Fixed buffer sizes of the binary background block.
const BACKGROUND: usize = 100;
const STARFIELD: usize = 100;
const TECH_TREE_FILE: usize = 128;
const RACE: usize = 128;
/// An object's label buffer, at least.
const LABEL: usize = 40;

/// Write `map` in its own format ([`Map::binary`]).
pub fn write(map: &Map) -> Result<Vec<u8>, WriteError> {
    if let Some(data) = &map.a1 {
        return crate::a1::write_map(map, data);
    }
    if map.version == crate::sol::VERSION {
        return Err(WriteError::Sol);
    }
    check(map)?;
    if map.binary && map.start_locs.is_empty() && map.start_count != 0 {
        return Err(WriteError::StartCount(map.start_count));
    }
    if map.binary {
        binary(map)
    } else {
        text(map).map(String::into_bytes)
    }
}

fn check(map: &Map) -> Result<(), WriteError> {
    let vectors = [
        ("min_extent", &map.min_extent),
        ("size", &map.size),
        ("ambient", &map.ambient),
    ];
    for (name, v) in vectors {
        if !v.iter().all(|f| f.is_finite()) {
            return Err(WriteError::NotFinite(name));
        }
    }
    for s in &map.start_locs {
        if !s.pos.iter().all(|f| f.is_finite()) {
            return Err(WriteError::NotFinite("start location"));
        }
    }
    for (index, o) in map.objects.iter().enumerate() {
        if o.odf.len() >= i16::MAX as usize {
            return Err(WriteError::TooLong {
                field: "odf name",
                len: o.odf.len(),
                max: i16::MAX as usize - 1,
            });
        }
        if o.odf.is_empty() {
            return Err(WriteError::EmptyOdf { index });
        }
        if !o
            .transform
            .iter()
            .chain(o.initial_position.iter().flatten())
            .all(|f| f.is_finite())
        {
            return Err(WriteError::NotFinite("object transform"));
        }
    }
    Ok(())
}

fn header(map: &Map, binary: bool) -> Vec<u8> {
    format!(
        "version [1] =\r\n{}\r\nBinaryMode [1] =\r\n{binary}\r\n",
        map.version
    )
    .into_bytes()
}

// ---------------------------------------------------------------- binary

struct Records(Vec<u8>);

impl Records {
    fn rec(&mut self, kind: u32, data: &[u8]) {
        self.0.extend(kind.to_le_bytes());
        self.0.extend((data.len() as u32).to_le_bytes());
        self.0.extend(data);
    }
    fn i32(&mut self, v: i32) {
        self.rec(4, &v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.rec(5, &v.to_le_bytes());
    }
    fn vec3(&mut self, v: [f32; 3]) {
        let b: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.rec(9, &b);
    }
    /// Length (with the NUL), then the string, even when empty: the header
    /// strings are found by their position.
    fn string(&mut self, s: &str) {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        self.i32(b.len() as i32);
        self.rec(0, &b);
    }
    /// As [`Records::string`], but an empty string is only its zero length.
    fn lp_string(&mut self, s: &str) {
        if s.is_empty() {
            self.i32(0);
        } else {
            self.string(s);
        }
    }
    /// A zero-padded text buffer of `n` bytes.
    fn buffer(&mut self, field: &'static str, s: &str, n: usize) -> Result<(), WriteError> {
        if s.len() >= n {
            return Err(WriteError::TooLong {
                field,
                len: s.len(),
                max: n - 1,
            });
        }
        let mut b = s.as_bytes().to_vec();
        b.resize(n, 0);
        self.rec(2, &b);
        Ok(())
    }
}

fn binary(map: &Map) -> Result<Vec<u8>, WriteError> {
    let mut r = Records(header(map, true));
    // Header strings, by position: mission type, spare, description, spare,
    // file name; then the tech tree and scripts, which are found by extension.
    for s in [&map.mission_type, "", &map.description, "", &map.file_name] {
        r.string(s);
    }
    if !map.tech_tree.is_empty() {
        r.string(&map.tech_tree);
    }
    for s in &map.scripts {
        r.string(s);
    }
    r.vec3(map.min_extent);
    r.vec3(map.size);

    if !map.start_locs.is_empty() {
        r.i32(map.start_count);
        for s in &map.start_locs {
            r.vec3(s.pos);
            r.lp_string(&s.kind);
            r.i32(s.team);
            r.lp_string(&s.race);
            for _ in 0..4 {
                r.f32(-1.0);
            }
            r.lp_string("");
            r.lp_string("empty");
        }
    }

    for v in map.ambient {
        r.f32(v);
    }
    r.buffer("background", &map.background, BACKGROUND)?;
    r.buffer("starfield", &map.starfield_odf, STARFIELD)?;
    r.buffer("tech tree file", &map.tech_tree_file, TECH_TREE_FILE)?;
    r.buffer("race", &map.race, RACE)?;

    r.i32(map.objects.len() as i32);
    for o in &map.objects {
        let mut name = o.odf.as_bytes().to_vec();
        name.push(0);
        r.rec(3, &(name.len() as i16).to_le_bytes());
        r.rec(2, &name);
        r.i32(o.entity_id);
        r.vec3(
            o.initial_position
                .unwrap_or([o.transform[9], o.transform[10], o.transform[11]]),
        );
        r.i32(o.team);
        r.buffer("label", &o.label, LABEL.max(o.label.len() + 1))?;
        r.rec(8, &0u32.to_le_bytes());
        let t: Vec<u8> = o.transform.iter().flat_map(|f| f.to_le_bytes()).collect();
        r.rec(11, &t);
    }
    Ok(r.0)
}

// ---------------------------------------------------------------- text

fn hex(s: &str) -> String {
    s.bytes()
        .chain(std::iter::once(0))
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `NAME [1] =` / length / `NAME = hex`.
fn text_string(out: &mut String, name: &str, s: &str) {
    if s.is_empty() {
        out.push_str(&format!("{name} [1] =\r\n0\r\n{name} = \r\n"));
    } else {
        out.push_str(&format!(
            "{name} [1] =\r\n{}\r\n{name} = {}\r\n",
            s.len() + 1,
            hex(s)
        ));
    }
}

fn text_vec3(out: &mut String, name: &str, v: [f32; 3]) {
    out.push_str(&format!(
        "{name} [1] =\r\n  x [1] =\r\n{}\r\n  y [1] =\r\n{}\r\n  z [1] =\r\n{}\r\n",
        v[0], v[1], v[2]
    ));
}

fn text_start(out: &mut String, s: &StartLoc) {
    text_vec3(out, "SLPos", s.pos);
    text_string(out, "SLType", &s.kind);
    out.push_str(&format!("SLLkTeam [1] =\r\n{}\r\n", s.team));
    text_string(out, "SLLkRace", &s.race);
    out.push_str("SLLat [1] =\r\n-1\r\n");
}

const TRANSFORM: [&str; 12] = [
    "right_x",
    "right_y",
    "right_z",
    "up_x",
    "up_y",
    "up_z",
    "front_x",
    "front_y",
    "front_z",
    "position.x",
    "position.y",
    "position.z",
];

fn text_object(out: &mut String, o: &MapObject) {
    out.push_str("[GameObject]\r\n");
    out.push_str(&format!(
        "PrjIdLen [1] =\r\n{}\r\nPrjID = {}\r\n",
        o.odf.len() + 1,
        hex(&o.odf)
    ));
    // After a `name = value` line, so the label is never read as hex.
    out.push_str(&format!("label = {}\r\n", o.label));
    out.push_str(&format!(
        "EntityID [1] =\r\n{}\r\nteam [1] =\r\n{}\r\n",
        o.entity_id, o.team
    ));
    if let Some(p) = o.initial_position {
        text_vec3(out, "pos", p);
    }
    if let Some(c) = o.light_colour {
        out.push_str(&format!("lightColorRed [1] =\r\n{}\r\nlightColorGreen [1] =\r\n{}\r\nlightColorBlue [1] =\r\n{}\r\n", c[0], c[1], c[2]));
    }
    out.push_str("transform [1] =\r\n");
    for (name, v) in TRANSFORM.iter().zip(o.transform) {
        out.push_str(&format!("  {name} [1] =\r\n{v}\r\n"));
    }
}

fn text(map: &Map) -> Result<String, WriteError> {
    if map.scripts.len() > 2 {
        return Err(WriteError::TooManyScripts(map.scripts.len()));
    }
    let mut out = String::from_utf8(header(map, false)).expect("ASCII");
    text_string(&mut out, "MPDType", &map.mission_type);
    text_string(&mut out, "MPDDescription", &map.description);
    text_string(&mut out, "MPDFilename", &map.file_name);
    text_string(&mut out, "MPDTechTree", &map.tech_tree);
    text_string(
        &mut out,
        "MPDScriptFile",
        map.scripts.first().map_or("", String::as_str),
    );
    if let Some(rule) = map.scripts.get(1) {
        text_string(&mut out, "MPDRuleFile", rule);
    }
    text_vec3(&mut out, "MPDMinExtent", map.min_extent);
    text_vec3(&mut out, "MPDSize", map.size);
    out.push_str(&format!("MPDStLoc [1] =\r\n{}\r\n", map.start_count));
    for s in &map.start_locs {
        text_start(&mut out, s);
    }
    for (name, v) in ["AmbientRed", "AmbientGreen", "AmbientBlue"]
        .iter()
        .zip(map.ambient)
    {
        out.push_str(&format!("{name} [1] =\r\n{v}\r\n"));
    }
    out.push_str(&format!("BackgroundArt = {}\r\n", map.background));
    out.push_str(&format!("StarfieldOdf = {}\r\n", map.starfield_odf));
    out.push_str(&format!("techTreeFilename = {}\r\n", map.tech_tree_file));
    out.push_str(&format!("raceName = {}\r\n", map.race));
    for o in &map.objects {
        text_object(&mut out, o);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(binary: bool) -> Map {
        let loc = |x: f32, kind: &str, team: i32, race: &str| StartLoc {
            pos: [x, 0.0, x * 2.0],
            kind: kind.into(),
            team,
            race: race.into(),
        };
        Map {
            version: 3013,
            binary,
            mission_type: "mp".into(),
            description: "A test map".into(),
            file_name: "test.bzn".into(),
            tech_tree: "borg.tt".into(),
            scripts: vec!["mmm_loader.dll".into(), "rules.drl".into()],
            min_extent: [-100.0, -5.5, -200.0],
            size: [2000.0, 100.0, 4000.0],
            ambient: [0.25, 0.5, 1.0],
            background: "MbgBaku.SOD".into(),
            starfield_odf: "ststars.odf".into(),
            tech_tree_file: "borg.tt".into(),
            race: "borg".into(),
            start_count: 2,
            start_locs: vec![
                loc(1000.0, "empty", -1, ""),
                loc(10.5, "player", 1, "race-unspecified"),
                loc(30.25, "ai-easy", -1, ""),
            ],
            objects: vec![
                MapObject {
                    odf: "mbaku".into(),
                    entity_id: 7,
                    team: 2,
                    label: "Baku".into(),
                    initial_position: Some([15.5, -4.0, 33.0]),
                    transform: [
                        0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 15.5, -4.0, 33.0,
                    ],
                    light_colour: None,
                },
                MapObject {
                    odf: "fcruiser".into(),
                    entity_id: -9,
                    team: 0,
                    label: String::new(),
                    initial_position: Some([1.0, 2.0, 3.0]),
                    transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0],
                    light_colour: None,
                },
            ],
            a1: None,
        }
    }

    #[test]
    fn binary_round_trip() {
        let m = sample(true);
        let bytes = write(&m).unwrap();
        assert_eq!(crate::parse(&bytes).unwrap(), m);
        // Writing is deterministic and a fixed point.
        assert_eq!(write(&crate::parse(&bytes).unwrap()).unwrap(), bytes);
    }

    #[test]
    fn text_round_trip() {
        let mut m = sample(false);
        m.objects[1].initial_position = None;
        m.objects[1].light_colour = Some([1.0, 0.5, 0.25]);
        let bytes = write(&m).unwrap();
        assert_eq!(crate::parse(&bytes).unwrap(), m);
        assert_eq!(write(&crate::parse(&bytes).unwrap()).unwrap(), bytes);
    }

    #[test]
    fn minimal_maps_round_trip() {
        for binary in [true, false] {
            let m = Map {
                version: 3012,
                binary,
                ..Default::default()
            };
            assert_eq!(
                crate::parse(&write(&m).unwrap()).unwrap(),
                m,
                "binary {binary}"
            );
        }
    }

    #[test]
    fn a_sol_map_is_refused() {
        let m = crate::parse(&crate::sol::tests::fixture()).unwrap();
        assert_eq!(write(&m), Err(WriteError::Sol));
    }

    #[test]
    fn what_cannot_be_written_is_refused() {
        let mut m = sample(true);
        m.background = "x".repeat(100);
        assert!(matches!(
            write(&m),
            Err(WriteError::TooLong {
                field: "background",
                ..
            })
        ));
        let mut m = sample(true);
        m.objects[0].odf.clear();
        assert_eq!(write(&m), Err(WriteError::EmptyOdf { index: 0 }));
        let mut m = sample(true);
        m.size[1] = f32::NAN;
        assert_eq!(write(&m), Err(WriteError::NotFinite("size")));
        let mut m = sample(false);
        m.scripts.push("third.lua".into());
        assert_eq!(write(&m), Err(WriteError::TooManyScripts(3)));
    }

    /// Install-gated, like the reader's own: with `STA64_TEST_BZN_DIRS`
    /// (colon-separated dirs), every A2/FO map reads, writes and reads again
    /// to an equal `Map`, Armada 1 maps included.
    #[test]
    fn installed_maps_round_trip() {
        let Ok(dirs) = std::env::var("STA64_TEST_BZN_DIRS") else {
            eprintln!("SKIP: set STA64_TEST_BZN_DIRS to colon-separated map directories");
            return;
        };
        let (mut ok, mut a1, mut sol, mut bin) = (0, 0, 0, 0);
        let mut failed = Vec::new();
        for dir in dirs.split(':') {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bzn")) {
                    continue;
                }
                let Ok(m) = crate::parse(&std::fs::read(&p).unwrap()) else {
                    continue;
                };
                a1 += m.a1.is_some() as usize;
                if m.version == crate::sol::VERSION {
                    sol += 1;
                }
                let back = write(&m)
                    .map_err(|e| e.to_string())
                    .and_then(|b| crate::parse(&b).map_err(|e| e.to_string()));
                match back {
                    Ok(back) if back == m => {
                        ok += 1;
                        bin += m.binary as usize;
                    }
                    Ok(back) => failed.push(format!(
                        "{}: differs: {}",
                        p.display(),
                        first_difference(&m, &back)
                    )),
                    Err(e) => failed.push(format!("{}: {e}", p.display())),
                }
            }
        }
        eprintln!(
            "round trip: {ok} equal ({bin} binary), {a1} of them Armada 1, {sol} SOL, {} failed",
            failed.len()
        );
        for f in failed.iter().take(15) {
            eprintln!("  {f}");
        }
        assert!(
            failed.is_empty(),
            "{} maps did not round-trip",
            failed.len()
        );
    }

    fn first_difference(a: &Map, b: &Map) -> String {
        macro_rules! field {
            ($($f:ident),*) => { $( if a.$f != b.$f { return format!("{}: {:?} -> {:?}", stringify!($f), a.$f, b.$f).chars().take(200).collect(); } )* };
        }
        field!(
            version,
            binary,
            mission_type,
            description,
            file_name,
            tech_tree,
            scripts,
            min_extent,
            size,
            ambient,
            background,
            starfield_odf,
            tech_tree_file,
            race,
            start_count,
            start_locs,
            a1
        );
        for (i, (x, y)) in a.objects.iter().zip(&b.objects).enumerate() {
            if x != y {
                return format!("object {i}: {x:?} -> {y:?}");
            }
        }
        format!("objects {} -> {}", a.objects.len(), b.objects.len())
    }
}
