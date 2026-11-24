//! Read-only import of the optional A2FOCraftIdentity Craft prefix.
//!
//! Provenance: `docs/native-object-editor-properties.md`, `SavedState` in
//! `modules/A2FOCraftIdentity/object_editor_state.hpp`, and
//! `rust/a2fo_core/src/lead.rs::editor_valid/editor_decode` in A2FOHookExtensions.
//! These describe a native format; the fixtures below are synthetic. This
//! module does not establish native game save/load compatibility.

use std::collections::{BTreeMap, BTreeSet};

use super::{Rec, binary_start, records};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CraftOverrides {
    pub captain: Option<String>,
    pub registry: Option<String>,
    /// Forward, aft, port, starboard: [current, maximum].
    pub shields: Option<[[f32; 2]; 4]>,
}

const SIZE: usize = 1076;

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn payload(bytes: &[u8]) -> Option<CraftOverrides> {
    if bytes.len() != SIZE
        || &bytes[..8] != b"A2FOEDIT"
        || word(bytes, 8) != 1
        || word(bytes, 12) != SIZE as u32
    {
        return None;
    }
    let flags = word(bytes, 16);
    if flags & !7 != 0 {
        return None;
    }
    let text = |start: usize| {
        let field = &bytes[start..start + 512];
        let end = field.iter().position(|&c| c == 0)?;
        // The native strings use the game's ANSI encoding. Match the existing
        // BZN reader's lossy byte-to-String convention without claiming an
        // exact code-page conversion.
        Some(String::from_utf8_lossy(&field[..end]).into_owned())
    };
    let captain = text(52)?;
    let registry = text(564)?;
    let shields = if flags & 4 != 0 {
        let mut pairs = [[0.0; 2]; 4];
        let mut current_total = 0.0_f32;
        let mut maximum_total = 0.0_f32;
        let mut active = false;
        for (i, pair) in pairs.iter_mut().enumerate() {
            let current = f32::from_bits(word(bytes, 20 + i * 4));
            let maximum = f32::from_bits(word(bytes, 36 + i * 4));
            if !current.is_finite()
                || !maximum.is_finite()
                || current < 0.0
                || maximum < 0.0
                || current > maximum
            {
                return None;
            }
            active |= maximum > 0.0;
            current_total += current;
            maximum_total += maximum;
            *pair = [current, maximum];
        }
        if !active || !current_total.is_finite() || !maximum_total.is_finite() {
            return None;
        }
        Some(pairs)
    } else {
        None
    };
    Some(CraftOverrides {
        captain: (flags & 1 != 0).then_some(captain),
        registry: (flags & 2 != 0).then_some(registry),
        shields,
    })
}

fn hex_payload(line: &[u8]) -> Option<CraftOverrides> {
    let mut pieces = line.splitn(2, |&c| c == b'=');
    let (key, value) = (pieces.next()?, pieces.next()?);
    if key.trim_ascii() != b"a2fo_objectEditor" {
        return None;
    }
    let value = value.trim_ascii();
    if value.len() != SIZE * 2 {
        return None;
    }
    let mut bytes = [0u8; SIZE];
    for (dst, pair) in bytes.iter_mut().zip(value.chunks_exact(2)) {
        let high = (pair[0] as char).to_digit(16)? as u8;
        let low = (pair[1] as char).to_digit(16)? as u8;
        *dst = high * 16 + low;
    }
    payload(&bytes)
}

fn text_records(data: &[u8]) -> Vec<(i32, Option<CraftOverrides>)> {
    let mut result = Vec::new();
    let mut in_object = false;
    let mut id = None;
    let mut id_count = 0;
    let mut expecting_id = false;
    let mut candidates = Vec::new();
    let finish =
        |id: Option<i32>, candidates: &mut Vec<Option<CraftOverrides>>, result: &mut Vec<_>| {
            if let Some(id) = id {
                result.push((
                    id,
                    if candidates.len() == 1 {
                        candidates.pop().unwrap()
                    } else {
                        None
                    },
                ));
            }
            candidates.clear();
        };
    for line in data.split(|&c| c == b'\n') {
        let line = line.trim_ascii();
        if line == b"[GameObject]" {
            finish(
                (id_count == 1).then_some(id).flatten(),
                &mut candidates,
                &mut result,
            );
            in_object = true;
            id = None;
            id_count = 0;
            expecting_id = false;
            continue;
        }
        if !in_object {
            continue;
        }
        if expecting_id {
            id = std::str::from_utf8(line).ok().and_then(|s| s.parse().ok());
            expecting_id = false;
        } else if line == b"EntityID [1] =" {
            expecting_id = true;
            id_count += 1;
        } else if line
            .splitn(2, |&c| c == b'=')
            .next()
            .is_some_and(|key| key.trim_ascii() == b"a2fo_objectEditor")
        {
            candidates.push(hex_payload(line));
        }
    }
    finish(
        (id_count == 1).then_some(id).flatten(),
        &mut candidates,
        &mut result,
    );
    result
}

fn object_start(recs: &[Rec<'_>], i: usize) -> Option<i32> {
    let [len, name, id, pos] = recs.get(i..i + 4)? else {
        return None;
    };
    if len.kind == 3
        && name.is_string()
        && len
            .i16()
            .is_some_and(|n| n > 1 && n as usize == name.data.len())
        && id.kind == 4
        && pos.kind == 9
        && pos.data.len() == 12
    {
        id.i32()
    } else {
        None
    }
}

fn binary_records(data: &[u8]) -> Vec<(i32, Option<CraftOverrides>)> {
    let Some(start) = binary_start(data) else {
        return Vec::new();
    };
    let Ok(recs) = records(data, start) else {
        return Vec::new();
    };
    let starts: Vec<_> = (0..recs.len())
        .filter_map(|i| object_start(&recs, i).map(|id| (i, id)))
        .collect();
    let mut result = Vec::new();
    for (n, &(start, id)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map_or(recs.len(), |&(i, _)| i);
        let candidates: Vec<_> = recs[start + 4..end]
            .iter()
            .filter(|r| r.kind & 255 == 0 && r.data.starts_with(b"A2FOEDIT"))
            .map(|r| payload(r.data))
            .collect();
        result.push((
            id,
            if candidates.len() == 1 {
                candidates.into_iter().next().unwrap()
            } else {
                None
            },
        ));
    }
    result
}

/// Return unambiguous, valid overrides keyed by authored BZN entity ID.
/// Native untagged binary has no reliable object framing here and is ignored.
pub fn read(data: &[u8]) -> BTreeMap<i32, CraftOverrides> {
    if !data.starts_with(b"version") {
        return BTreeMap::new();
    }
    let binary = super::text_value(data, b"BinaryMode [1] =")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let found = if binary {
        binary_records(data)
    } else {
        text_records(data)
    };
    let mut seen = BTreeSet::new();
    let mut duplicate = BTreeSet::new();
    for &(id, _) in &found {
        if !seen.insert(id) {
            duplicate.insert(id);
        }
    }
    found
        .into_iter()
        .filter_map(|(id, value)| (!duplicate.contains(&id)).then_some((id, value?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic bytes made from the documented SavedState offsets, not a
    // captured game map or a copied native fixture.
    fn state() -> Vec<u8> {
        let mut bytes = vec![0; SIZE];
        bytes[..8].copy_from_slice(b"A2FOEDIT");
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&(SIZE as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&7u32.to_le_bytes());
        for i in 0..4 {
            bytes[20 + i * 4..24 + i * 4].copy_from_slice(&(i as f32).to_le_bytes());
            bytes[36 + i * 4..40 + i * 4].copy_from_slice(&5f32.to_le_bytes());
        }
        bytes[52..55].copy_from_slice(b"Jan");
        bytes[564..570].copy_from_slice(b"NX-742");
        bytes
    }

    fn line(bytes: &[u8]) -> String {
        let mut out = String::from("a2fo_objectEditor = ");
        for byte in bytes {
            use std::fmt::Write;
            write!(out, "{byte:02X}").unwrap();
        }
        out
    }

    fn binary_record(kind: u32, bytes: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }

    fn binary_object(id: i32, bytes: &[u8], out: &mut Vec<u8>) {
        binary_record(3, &5i16.to_le_bytes(), out);
        binary_record(2, b"ship\0", out);
        binary_record(4, &id.to_le_bytes(), out);
        binary_record(9, &[0; 12], out);
        binary_record(0, bytes, out);
    }

    #[test]
    fn text_and_tagged_binary_associate_records_with_entity_ids() {
        let bytes = state();
        let text = format!(
            "version [1] =\n2\nBinaryMode [1] =\nfalse\n[GameObject]\nEntityID [1] =\n42\n{}\n",
            line(&bytes)
        );
        let value = read(text.as_bytes())[&42].clone();
        assert_eq!(value.captain.as_deref(), Some("Jan"));
        assert_eq!(value.registry.as_deref(), Some("NX-742"));
        assert_eq!(value.shields.unwrap()[3], [3.0, 5.0]);

        let mut binary = b"version [1] =\n2\nBinaryMode [1] =\ntrue\n".to_vec();
        binary_object(42, &bytes, &mut binary);
        assert_eq!(read(&binary)[&42], value);
    }

    #[test]
    fn rejects_bad_values_and_ambiguous_ids() {
        let mut bytes = state();
        bytes[36..40].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(payload(&bytes).is_none());
        bytes = state();
        bytes[16..20].copy_from_slice(&8u32.to_le_bytes());
        assert!(payload(&bytes).is_none());
        bytes = state();
        bytes[52..564].fill(b'x');
        assert!(payload(&bytes).is_none());
        bytes = state();
        let text = format!(
            "version [1] =\n2\nBinaryMode [1] =\nfalse\n[GameObject]\nEntityID [1] =\n42\n{}\n[GameObject]\nEntityID [1] =\n42\n",
            line(&bytes)
        );
        assert!(read(text.as_bytes()).is_empty());
        let mut binary = b"version [1] =\n2\nBinaryMode [1] =\ntrue\n".to_vec();
        binary_object(42, &bytes, &mut binary);
        binary_object(42, &bytes, &mut binary);
        assert!(read(&binary).is_empty());
    }
}
