//! The byte layout of a save file (ADR 0010 V2).
//!
//! All integers little-endian.
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 4 | magic `STAG` |
//! | 4 | 2 | container version |
//! | 6 | 2 | snapshot format the file holds |
//! | 8 | 2 | replay format the journal was written for |
//! | 10 | 4 | seam revision |
//! | 14 | 4 | header length `h` (at most [`MAX_HEADER`]) |
//! | 18 | h | the [`Header`] (postcard) |
//! | 18+h | 8 | checksum of bytes `0..18+h`: a load list reads this much only |
//! | 26+h | | five sections, each `id u8, version u32, length u32, bytes`, ids 1 to 5 in order: snapshot, journal, controllers, commands in flight, client |
//! | end-8 | 8 | checksum of every byte before it |
//!
//! **What decides loadability sits at fixed offsets, as plain integers, and
//! is checked first and by exact match** (container version, snapshot format,
//! replay format, seam revision: nothing before them is decoded, no previous
//! format is accepted, there is no migration, ADR 0010 V1). A file made by
//! another build is therefore refused with the number it has and the number
//! this build reads, never as "damaged", whatever changed inside the header
//! or the sections since.
//!
//! **When `CONTAINER_VERSION` must rise.** The header and the sections are
//! postcard, which does not describe itself. Raise the version whenever the
//! serialised form of anything in the header or the section records changes:
//! the header's own types ([`Header`], `MapId`, `MissionIdentity`,
//! `GameKind`), the types of other modules it embeds (`replay::SeatRec`,
//! `replay::ControlRec`, `replay::StartRec`, `setup::GameOptions` with
//! `sta_content::StartingResources`, `PlayerId`, `TeamId`), the section
//! records (`SeatController`, the journal's field order and `Logged`
//! digests, the commands-in-flight record: next sequence number then the
//! `Command` list) and the section framing above.
//! `Command` and `SimEvent` themselves move with the seam revision, the
//! snapshot with its format and the journal's layout with the replay format;
//! those numbers are in the prefix. The golden tests in `save/tests.rs`
//! (`golden_*`) fail when the encoding of a sample of each of these changes:
//! when one fails, raise `CONTAINER_VERSION` and regenerate the golden.
//!
//! The checksum is 64-bit FNV-1a: it detects accidental damage and nothing
//! else. It is not a signature and offers no protection against a file made
//! or altered on purpose (anyone can recompute it). A save from another
//! machine is untrusted input: every length is checked against the bytes
//! present, every count against a bound before anything is allocated from
//! it, and the setup in the header is cross-checked against the snapshot's;
//! what the snapshot decoder does with hostile content is that decoder's.

use super::SaveError;
use super::header::{CONTAINER_VERSION, Header, MAGIC, MAX_HEADER};

/// Bytes before the header: magic, the four numbers, the header length.
pub(super) const PREFIX: usize = 18;
const SUM: usize = 8;
const SECTION_HEAD: usize = 1 + 4 + 4;

pub(super) const SNAPSHOT_SECTION: u32 = 1;
pub(super) const JOURNAL_SECTION: u32 = 1;
pub(super) const CONTROLLERS_SECTION: u32 = 1;
pub(super) const IN_FLIGHT_SECTION: u32 = 1;

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

/// The fixed-offset part of a file, as read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Prefix {
    pub container: u16,
    pub snapshot: u16,
    pub replay: u16,
    pub seam: u32,
    pub header_len: u32,
}

/// Reads the prefix from its bytes: structure only (magic and length), no
/// judgement of the numbers.
pub(super) fn parse_prefix(bytes: &[u8]) -> Result<Prefix, SaveError> {
    if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
        return Err(SaveError::NotASave);
    }
    if bytes.len() < PREFIX {
        return Err(damaged("the file ends inside its first bytes"));
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Ok(Prefix { container: u16_at(4), snapshot: u16_at(6), replay: u16_at(8), seam: u32_at(10), header_len: u32_at(14) })
}

/// Exact match of every number that decides loadability against this
/// build's: the first difference is the refusal.
pub(super) fn check_prefix(p: &Prefix) -> Result<(), SaveError> {
    if p.container != CONTAINER_VERSION {
        return Err(SaveError::ContainerVersion { found: p.container, expected: CONTAINER_VERSION });
    }
    if p.snapshot != crate::snapshot::FORMAT_VERSION {
        return Err(SaveError::SnapshotFormat { found: p.snapshot, expected: crate::snapshot::FORMAT_VERSION });
    }
    if p.replay != crate::replay::FORMAT_VERSION {
        return Err(SaveError::ReplayFormat { found: p.replay, expected: crate::replay::FORMAT_VERSION });
    }
    if p.seam != sta_core::SEAM_REV {
        return Err(SaveError::SeamRev { found: p.seam, expected: sta_core::SEAM_REV });
    }
    Ok(())
}

/// The sections to write.
pub(super) struct Parts {
    pub snapshot: Vec<u8>,
    pub journal: Vec<u8>,
    pub controllers: Vec<u8>,
    pub in_flight: Vec<u8>,
    pub client_version: u32,
    pub client: Vec<u8>,
}

/// The sections as read: slices of the file.
pub(super) struct Sections<'a> {
    pub snapshot: &'a [u8],
    pub journal: &'a [u8],
    pub controllers: &'a [u8],
    pub in_flight: &'a [u8],
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

/// The whole file for `header` and `parts`. The numbers in the prefix are
/// the header's `container`-independent fields `snapshot_format`,
/// `replay_format` and `seam_rev` (they are not part of the postcard header).
pub(super) fn seal(header: &Header, parts: &Parts) -> Result<Vec<u8>, SaveError> {
    let hb = postcard::to_allocvec(header).map_err(|e| SaveError::Invalid(e.to_string()))?;
    if hb.len() > MAX_HEADER {
        return Err(SaveError::Invalid(format!("the header is {} bytes, at most {MAX_HEADER} are allowed (a shorter description?)", hb.len())));
    }
    let mut out = Vec::with_capacity(PREFIX + hb.len() + SUM + parts.snapshot.len() + parts.journal.len() + 64);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
    out.extend_from_slice(&header.snapshot_format.to_le_bytes());
    out.extend_from_slice(&header.replay_format.to_le_bytes());
    out.extend_from_slice(&header.seam_rev.to_le_bytes());
    out.extend_from_slice(&(hb.len() as u32).to_le_bytes());
    out.extend_from_slice(&hb);
    let sum = checksum(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    put_section(&mut out, 1, SNAPSHOT_SECTION, &parts.snapshot)?;
    put_section(&mut out, 2, JOURNAL_SECTION, &parts.journal)?;
    put_section(&mut out, 3, CONTROLLERS_SECTION, &parts.controllers)?;
    put_section(&mut out, 4, IN_FLIGHT_SECTION, &parts.in_flight)?;
    put_section(&mut out, 5, parts.client_version, &parts.client)?;
    let sum = checksum(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    Ok(out)
}

/// Reads and checks the header from the start of a file (a prefix of the
/// file is enough): magic, then the four numbers by exact match, then the
/// header's checksum, then the header. Returns it, with the numbers filled
/// in from the prefix, and the offset where the sections begin.
pub(super) fn parse_header(bytes: &[u8]) -> Result<(Header, usize), SaveError> {
    // The container version first, so that a file of another layout is
    // reported as that even if it is shorter than this one's prefix.
    if bytes.len() >= 6 && bytes[..4] == MAGIC {
        let found = u16::from_le_bytes([bytes[4], bytes[5]]);
        if found != CONTAINER_VERSION {
            return Err(SaveError::ContainerVersion { found, expected: CONTAINER_VERSION });
        }
    }
    let prefix = parse_prefix(bytes)?;
    check_prefix(&prefix)?;
    let hl = prefix.header_len as usize;
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
    let (mut header, rest): (Header, &[u8]) =
        postcard::take_from_bytes(&bytes[PREFIX..end]).map_err(|e| SaveError::Damaged(format!("the header does not parse: {e}")))?;
    if !rest.is_empty() {
        return Err(damaged("the header has trailing bytes"));
    }
    header.snapshot_format = prefix.snapshot;
    header.replay_format = prefix.replay;
    header.seam_rev = prefix.seam;
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
    let (ev, in_flight) = take_section(&mut rest, 4)?;
    let (client_version, client) = take_section(&mut rest, 5)?;
    if !rest.is_empty() {
        return Err(damaged("bytes follow the last section"));
    }
    if sv != SNAPSHOT_SECTION || jv != JOURNAL_SECTION || cv != CONTROLLERS_SECTION || ev != IN_FLIGHT_SECTION {
        return Err(damaged("a section has a record version this build does not read"));
    }
    Ok(Sections { snapshot, journal, controllers, in_flight, client_version, client })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_fnv1a() {
        assert_eq!(checksum(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(checksum(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    /// A complete prefix, built by hand: the offsets and the little-endian
    /// encoding of each number are part of the file format and must keep
    /// decoding to these values.
    #[test]
    fn golden_prefix_decodes_to_its_values() {
        let bytes: [u8; 18] = [
            b'S', b'T', b'A', b'G', // magic
            0x01, 0x00, // container version 1
            0x34, 0x12, // snapshot format 0x1234
            0x78, 0x56, // replay format 0x5678
            0xEF, 0xBE, 0xAD, 0xDE, // seam 0xDEADBEEF
            0x40, 0x03, 0x00, 0x00, // header length 0x340
        ];
        let p = parse_prefix(&bytes).unwrap();
        assert_eq!(p, Prefix { container: 1, snapshot: 0x1234, replay: 0x5678, seam: 0xDEAD_BEEF, header_len: 0x340 });
        assert_eq!(PREFIX, bytes.len());
        assert_eq!(parse_prefix(&bytes[..17]), Err(SaveError::Damaged("the file ends inside its first bytes".into())));
        assert_eq!(parse_prefix(b"STAG"), Err(SaveError::Damaged("the file ends inside its first bytes".into())));
        assert_eq!(parse_prefix(b"XTAG"), Err(SaveError::NotASave));
        assert_eq!(parse_prefix(b"STAS"), Err(SaveError::NotASave));
    }

    /// Each number alone, changed, gives exactly its own refusal, with the
    /// found and the expected value, and never `Damaged`.
    #[test]
    fn each_number_has_its_own_refusal() {
        let good = Prefix {
            container: CONTAINER_VERSION,
            snapshot: crate::snapshot::FORMAT_VERSION,
            replay: crate::replay::FORMAT_VERSION,
            seam: sta_core::SEAM_REV,
            header_len: 0,
        };
        assert_eq!(check_prefix(&good), Ok(()));
        assert_eq!(
            check_prefix(&Prefix { container: good.container + 1, ..good }),
            Err(SaveError::ContainerVersion { found: good.container + 1, expected: good.container })
        );
        // The previous formats are refused too: no migration.
        assert_eq!(
            check_prefix(&Prefix { snapshot: good.snapshot - 1, ..good }),
            Err(SaveError::SnapshotFormat { found: good.snapshot - 1, expected: good.snapshot })
        );
        assert_eq!(
            check_prefix(&Prefix { replay: good.replay - 1, ..good }),
            Err(SaveError::ReplayFormat { found: good.replay - 1, expected: good.replay })
        );
        assert_eq!(check_prefix(&Prefix { seam: good.seam + 1, ..good }), Err(SaveError::SeamRev { found: good.seam + 1, expected: good.seam }));
    }

    #[test]
    fn sections_split_and_refuse_overlong_lengths() {
        let mut body = Vec::new();
        put_section(&mut body, 1, 1, b"snap").unwrap();
        put_section(&mut body, 2, 1, b"jr").unwrap();
        put_section(&mut body, 3, 1, b"").unwrap();
        put_section(&mut body, 4, 1, b"ev").unwrap();
        put_section(&mut body, 5, 9, b"client").unwrap();
        let s = parse_sections(&body).unwrap();
        assert_eq!(
            (s.snapshot, s.journal, s.controllers, s.in_flight, s.client_version, s.client),
            (&b"snap"[..], &b"jr"[..], &b""[..], &b"ev"[..], 9, &b"client"[..])
        );
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
