//! The `AiPaths` list of Armada II and Fleet Operations maps (CON-47).
//!
//! Missions refer to named map paths (`Goto(unit, "path_6")`, `BuildObject(odf,
//! team, "build_path_1", 3)`); the map holds them at its very end, after the
//! teams, the mission names and the areas of interest.
//!
//! # Layout (read from the installed maps; notes kept outside the repository)
//!
//! Text: `[AiPaths]`, `count [1] =` N, then N times `[AiPath]` with `old_ptr`
//! (a serialized pointer, never used), `size [1] =` label length, `m_pLabel`,
//! `pointCount [1] =`, `points [n] =` with `x`, `y`, `z` per point, `pathType`
//! (hex of a little-endian u32), `asteroid_belt_speed`, `m_nebula_type`.
//!
//! Binary: the same fields, one record each, `u32 type, u32 size, data`: a
//! count (type 4); per path a raw word (type 0, size 4, the pointer), the label
//! length (type 4), the label (type 0 or 2, that many bytes, no terminator; no
//! record when the length is 0), the point count (type 4), all points in one
//! record (type 9, 12 bytes each; no record for zero points), the path type
//! (type 0, size 4), the belt speed (type 5) and the nebula type (type 4).
//! The list ends the file, so the reader takes the first record that is a
//! count N after which N complete paths end exactly at the last record. This is
//! the Armada I `AiPath` record plus the trailing nebula type.
//!
//! Evidence: read from data (every binary map of the stock Armada II, Dominion
//! War, Fleet Operations 3.2.7 and Fleet Operations Roots installs parses this
//! way to its last record; the text maps carry the same names).

use crate::{Field, Rec};

/// One named path of a map.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapPath {
    pub label: String,
    pub points: Vec<[f32; 3]>,
    /// `pathType`: 0 plain, 2 (loop or patrol: not interpreted here).
    pub path_type: u32,
    pub belt_speed: f32,
    pub nebula_type: i32,
}

fn int(r: &Rec) -> Option<i32> {
    (r.kind == 4).then(|| r.i32()).flatten()
}

/// One path at record `k`: the path and the next record.
fn binary_path(recs: &[Rec], k: usize) -> Option<(MapPath, usize)> {
    let word = |r: &Rec| (r.kind == 0 && r.data.len() == 4).then(|| u32::from_le_bytes(r.data.try_into().unwrap()));
    word(recs.get(k)?)?;
    let n = usize::try_from(int(recs.get(k + 1)?)?).ok()?;
    let mut k = k + 2;
    let label = if n == 0 {
        String::new()
    } else {
        let r = recs.get(k).filter(|r| r.is_string() && r.data.len() == n)?;
        k += 1;
        r.text()
    };
    let count = usize::try_from(int(recs.get(k)?)?).ok()?;
    k += 1;
    let mut points = Vec::new();
    if count > 0 {
        let r = recs.get(k).filter(|r| r.kind == 9 && r.data.len() == count.saturating_mul(12))?;
        k += 1;
        let f = r.floats();
        if !f.iter().all(|v| v.is_finite()) {
            return None;
        }
        points = f.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    }
    let path_type = word(recs.get(k)?)?;
    let speed = recs.get(k + 1).filter(|r| r.kind == 5 && r.data.len() == 4)?.floats()[0];
    let nebula_type = int(recs.get(k + 2)?)?;
    Some((MapPath { label, points, path_type, belt_speed: speed, nebula_type }, k + 3))
}

/// The path list of a binary map; empty when the map has none (no complete
/// list ends the file).
pub(crate) fn from_records(recs: &[Rec]) -> Vec<MapPath> {
    for (k, r) in recs.iter().enumerate() {
        let Some(n) = int(r).and_then(|n| usize::try_from(n).ok()) else { continue };
        if n > recs.len() {
            continue;
        }
        let mut at = k + 1;
        let mut out = Vec::new();
        while out.len() < n {
            let Some((p, next)) = binary_path(recs, at) else { break };
            out.push(p);
            at = next;
        }
        if out.len() == n && at == recs.len() {
            return out;
        }
    }
    Vec::new()
}

/// `pathType` of a text map: hex of a little-endian u32, or (when the field
/// reader already decoded the hex as text) its first byte.
fn text_type(v: &str) -> u32 {
    if v.len() == 8 && v.bytes().all(|c| c.is_ascii_hexdigit()) {
        let b: Vec<u8> = (0..4).map(|i| u8::from_str_radix(&v[2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        v.bytes().next().map_or(0, u32::from)
    }
}

pub(crate) fn from_fields(fs: &[Field]) -> Vec<MapPath> {
    let Some(start) = fs.iter().position(|f| f.name == "[AiPaths]") else { return Vec::new() };
    let mut out: Vec<MapPath> = Vec::new();
    let mut axis: Vec<f32> = Vec::new();
    for f in &fs[start + 1..] {
        let v = f.values.first().map(String::as_str).unwrap_or("");
        match f.name.as_str() {
            "[AiPath]" => out.push(MapPath::default()),
            name => {
                let Some(p) = out.last_mut() else { continue };
                match name {
                    "m_pLabel" => p.label = v.to_owned(),
                    "x" | "y" | "z" => {
                        axis.push(v.parse().unwrap_or(0.0));
                        if axis.len() == 3 {
                            p.points.push([axis[0], axis[1], axis[2]]);
                            axis.clear();
                        }
                    }
                    "pathType" => p.path_type = text_type(v),
                    "asteroid_belt_speed" => p.belt_speed = v.parse().unwrap_or(0.0),
                    "m_nebula_type" => p.nebula_type = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
    }
    out
}

/// The path named `name`: an exact label first, else the one that matches
/// ignoring ASCII case.
pub fn find<'a>(paths: &'a [MapPath], name: &str) -> Option<&'a MapPath> {
    paths.iter().find(|p| p.label == name).or_else(|| paths.iter().find(|p| p.label.eq_ignore_ascii_case(name)))
}
