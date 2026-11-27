//! The byte layout of a save file (ADR 0010 V2).
//!
//! All integers little-endian.
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 4 | magic `STAG` |
//! | 4 | 2 | container version |
//! | 6 | 4 | header length `h` (at most [`MAX_HEADER`]) |
//! | 10 | h | the [`Header`] (postcard) |
//! | 10+h | 8 | checksum of bytes `0..10+h`: a load list reads this much only |
//! | 18+h | | four sections, each `id u8, version u32, length u32, bytes`, ids 1 to 4 in order: snapshot, journal, controllers, client |
//! | end-8 | 8 | checksum of every byte before it |
//!
//! The checksum is 64-bit FNV-1a: it reports damage, it is not a signature.
//! Every length is checked against the bytes that are there before a slice
//! or an allocation is made from it.

use super::SaveError;
use super::header::{CONTAINER_VERSION, Header, MAGIC, MAX_HEADER};

/// Bytes before the header: magic, container version, header length.
pub(super) const PREFIX: usize = 10;
const SUM: usize = 8;
const SECTION_HEAD: usize = 1 + 4 + 4;

pub(super) const SNAPSHOT_SECTION: u32 = 1;
pub(super) const JOURNAL_SECTION: u32 = 1;
pub(super) const CONTROLLERS_SECTION: u32 = 1;

/// 64-bit FNV-1a.
pub(super) fn checksum(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn damaged(why: &str) -> SaveError {
    SaveError::Damaged(why.to_string())
}

/// The sections to write.
pub(super) struct Parts {
    pub snapshot: Vec<u8>,
    pub journal: Vec<u8>,
    pub controllers: Vec<u8>,
    pub client_version: u32,
    pub client: Vec<u8>,
}

/// The sections as read: slices of the file.
pub(super) struct Sections<'a> {
    pub snapshot: &'a [u8],
    pub journal: &'a [u8],
    pub controllers: &'a [u8],
    pub client_version: u32,
    pub client: &'a [u8],
}

fn put_section(out: &mut Vec<u8>, id: u8, version: u32, data: &[u8]) -> Result<(), SaveError> {
    let len = u32::try_from(data.len()).map_err(|_| SaveError::Invalid("a section is larger than 4 GiB".into()))?;
    out.push(id);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    Ok(())
}

/// The whole file for `header` and `parts`.
pub(super) fn seal(header: &Header, parts: &Parts) -> Result<Vec<u8>, SaveError> {
    let hb = postcard::to_allocvec(header).map_err(|e| SaveError::Invalid(e.to_string()))?;
    if hb.len() > MAX_HEADER {
        return Err(SaveError::Invalid(format!("the header is {} bytes, at most {MAX_HEADER} are allowed (a shorter description?)", hb.len())));
    }
    let mut out = Vec::with_capacity(PREFIX + hb.len() + SUM + parts.snapshot.len() + parts.journal.len() + 64);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
    out.extend_from_slice(&(hb.len() as u32).to_le_bytes());
    out.extend_from_slice(&hb);
    let sum = checksum(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    put_section(&mut out, 1, SNAPSHOT_SECTION, &parts.snapshot)?;
    put_section(&mut out, 2, JOURNAL_SECTION, &parts.journal)?;
    put_section(&mut out, 3, CONTROLLERS_SECTION, &parts.controllers)?;
    put_section(&mut out, 4, parts.client_version, &parts.client)?;
    let sum = checksum(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    Ok(out)
}

/// Reads and checks the header from the start of a file (a prefix of the
/// file is enough). Returns it and the offset where the sections begin.
pub(super) fn parse_header(bytes: &[u8]) -> Result<(Header, usize), SaveError> {
    if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
        return Err(SaveError::NotASave);
    }
    if bytes.len() < PREFIX {
        return Err(damaged("the file ends inside its first bytes"));
    }
    let found = u16::from_le_bytes([bytes[4], bytes[5]]);
    if found != CONTAINER_VERSION {
        return Err(SaveError::ContainerVersion { found });
    }
    let hl = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    if hl > MAX_HEADER {
        return Err(damaged("the header length is impossible"));
    }
    let end = PREFIX + hl;
    if bytes.len() < end + SUM {
        return Err(damaged("the file ends inside its header"));
    }
    let stored = u64::from_le_bytes(bytes[end..end + SUM].try_into().expect("eight bytes"));
    if stored != checksum(&bytes[..end]) {
        return Err(damaged("the header checksum differs"));
    }
    let (header, rest): (Header, &[u8]) =
        postcard::take_from_bytes(&bytes[PREFIX..end]).map_err(|e| SaveError::Damaged(format!("the header does not parse: {e}")))?;
    if !rest.is_empty() {
        return Err(damaged("the header has trailing bytes"));
    }
    Ok((header, end + SUM))
}

/// Checks the checksum over the whole file.
pub(super) fn verify_whole(bytes: &[u8], body_at: usize) -> Result<(), SaveError> {
    if bytes.len() < body_at + SUM {
        return Err(damaged("the file ends before its checksum"));
    }
    let split = bytes.len() - SUM;
    let stored = u64::from_le_bytes(bytes[split..].try_into().expect("eight bytes"));
    if stored != checksum(&bytes[..split]) {
        return Err(damaged("the file checksum differs (damaged or cut short)"));
    }
    Ok(())
}

fn take_section<'a>(rest: &mut &'a [u8], id: u8) -> Result<(u32, &'a [u8]), SaveError> {
    let bytes: &'a [u8] = rest;
    if bytes.len() < SECTION_HEAD {
        return Err(damaged("the file ends inside a section header"));
    }
    if bytes[0] != id {
        return Err(damaged("the sections are not in order"));
    }
    let version = u32::from_le_bytes(bytes[1..5].try_into().expect("four bytes"));
    let len = u32::from_le_bytes(bytes[5..9].try_into().expect("four bytes")) as usize;
    let after = &bytes[SECTION_HEAD..];
    if len > after.len() {
        return Err(damaged("a section is longer than the file"));
    }
    let (data, tail) = after.split_at(len);
    *rest = tail;
    Ok((version, data))
}

/// Splits the part between the header and the final checksum into sections.
pub(super) fn parse_sections(body: &[u8]) -> Result<Sections<'_>, SaveError> {
    let mut rest = body;
    let (sv, snapshot) = take_section(&mut rest, 1)?;
    let (jv, journal) = take_section(&mut rest, 2)?;
    let (cv, controllers) = take_section(&mut rest, 3)?;
    let (client_version, client) = take_section(&mut rest, 4)?;
    if !rest.is_empty() {
        return Err(damaged("bytes follow the last section"));
    }
    if sv != SNAPSHOT_SECTION || jv != JOURNAL_SECTION || cv != CONTROLLERS_SECTION {
        return Err(damaged("a section has a record version this build does not read"));
    }
    Ok(Sections { snapshot, journal, controllers, client_version, client })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_fnv1a() {
        assert_eq!(checksum(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(checksum(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn sections_split_and_refuse_overlong_lengths() {
        let mut body = Vec::new();
        put_section(&mut body, 1, 1, b"snap").unwrap();
        put_section(&mut body, 2, 1, b"jr").unwrap();
        put_section(&mut body, 3, 1, b"").unwrap();
        put_section(&mut body, 4, 9, b"client").unwrap();
        let s = parse_sections(&body).unwrap();
        assert_eq!((s.snapshot, s.journal, s.controllers, s.client_version, s.client), (&b"snap"[..], &b"jr"[..], &b""[..], 9, &b"client"[..]));
        // A length that points past the end is refused without allocating.
        let mut bad = body.clone();
        bad[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(parse_sections(&bad), Err(SaveError::Damaged(_))));
        // Every cut is refused.
        for n in 0..body.len() {
            assert!(parse_sections(&body[..n]).is_err(), "cut at {n}");
        }
        body.push(0);
        assert!(parse_sections(&body).is_err());
    }
}
