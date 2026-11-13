//! Checked A1 start data, without filesystem lookup or native fallback policy.

use super::MapData;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StartError {
    #[error("missing canonical MDF StartLocations assignment")]
    MissingCount,
    #[error("duplicate MDF StartLocations assignment")]
    DuplicateCount,
    #[error("MDF StartLocations is not an unsigned decimal integer")]
    InvalidCount,
    #[error("unsupported A1 start count {0}; expected 2..=8")]
    UnsupportedCount(u32),
    #[error("missing A1 Camera_{0} path")]
    MissingCamera(u8),
    #[error("A1 Camera_{0} path has no first point")]
    EmptyCamera(u8),
    #[error("A1 Camera_{0} first point is not finite")]
    NonfiniteCamera(u8),
}

fn supported_count(count: u32) -> Result<u8, StartError> {
    if (2..=8).contains(&count) {
        Ok(count as u8)
    } else {
        Err(StartError::UnsupportedCount(count))
    }
}

/// Read the canonical `StartLocations = <unsigned decimal>` MDF assignment.
///
/// Supports LF/CRLF/CR lines and ASCII whitespace around the key/value.
/// Unrelated lines (including preview coordinates) are ignored. Duplicate,
/// missing, malformed and out-of-range counts are explicit errors. This is a
/// checked subset, not the native config reader's scanf/default-eight policy.
/// No whole-file text decoding or mutation is performed.
pub fn mdf_start_count(data: &[u8]) -> Result<u8, StartError> {
    let mut count = None;
    for line in data.split(|&c| c == b'\n' || c == b'\r') {
        let Some((key, value)) = line
            .iter()
            .position(|&c| c == b'=')
            .map(|i| (&line[..i], &line[i + 1..]))
        else {
            continue;
        };
        if key.trim_ascii() != b"StartLocations" {
            continue;
        }
        if count.is_some() {
            return Err(StartError::DuplicateCount);
        }
        let value = value.trim_ascii();
        if value.is_empty() {
            return Err(StartError::InvalidCount);
        }
        let number = value
            .iter()
            .try_fold(0_u32, |n, &c| {
                c.is_ascii_digit()
                    .then_some(c)
                    .and_then(|c| n.checked_mul(10)?.checked_add(u32::from(c - b'0')))
            })
            .ok_or(StartError::InvalidCount)?;
        count = Some(supported_count(number)?);
    }
    count.ok_or(StartError::MissingCount)
}

/// Resolve native teams 1..=count to their precise first `Camera_i` points.
///
/// Exact case-sensitive lookup uses the first path with each label, preserving
/// all XYZ coordinates. Element `i` belongs to native team `i + 1`; there is no
/// neutral entry. An invalid requested path rejects the complete result. No
/// preview-grid inversion, clamping, deduplication or native fallback is used.
pub fn player_start_positions(map: &MapData, count: u8) -> Result<Vec<[f32; 3]>, StartError> {
    player_start_positions_with_fallback(map, count, None)
}

/// [`player_start_positions`] with the native missing-path rule: a team whose
/// `Camera_i` path does not exist starts at `fallback` (map centre XZ, mean of
/// `SHIP_COMBAT_FLOOR` and `SHIP_COMBAT_CEILING` as Y; the caller supplies it
/// from the map bounds and `RTS_CFG.h`). With `None` a missing path rejects the
/// whole result, as before. A path that exists but is empty or not finite still
/// rejects the result (native behaviour there is not measured).
pub fn player_start_positions_with_fallback(
    map: &MapData,
    count: u8,
    fallback: Option<[f32; 3]>,
) -> Result<Vec<[f32; 3]>, StartError> {
    let count = supported_count(u32::from(count))?;
    (1..=count)
        .map(|slot| {
            let label = format!("Camera_{slot}");
            let Some(path) = map.paths.iter().find(|path| path.label == label) else {
                return fallback.ok_or(StartError::MissingCamera(slot));
            };
            let point = *path.points.first().ok_or(StartError::EmptyCamera(slot))?;
            if !point.iter().all(|v| v.is_finite()) {
                return Err(StartError::NonfiniteCamera(slot));
            }
            Ok(point)
        })
        .collect()
}

/// Native start count of the original A1 MDF reader, which never fails: an
/// absent file (`None`), a missing key, an unparsable value or a count outside
/// 2..=8 all give eight. Evidence grade: read from the original executable
/// (static), not executed.
///
/// Rules followed: the file is read as a C string (the first NUL ends it);
/// lines end at LF, except that an LF followed by a space or tab continues the
/// line; the first line that begins with `StartLocations` (compared
/// case-insensitively, as a prefix, so `StartLocationsX` also matches) and
/// contains an `=` supplies the value, read as `= %d` from that first `=`
/// (whitespace, optional sign, decimal digits). Lines with the prefix but no
/// `=` are skipped. A failed `%d` leaves an uninitialised native field, which
/// is modelled as the default eight. `#include` lines are not followed.
pub fn native_mdf_start_count(data: Option<&[u8]>) -> u8 {
    const KEY: &[u8] = b"StartLocations";
    const DEFAULT: u8 = 8;
    let Some(data) = data else { return DEFAULT };
    let data = data.split(|&c| c == 0).next().unwrap_or(&[]);
    let mut lines: Vec<Vec<u8>> = vec![Vec::new()];
    for (i, &c) in data.iter().enumerate() {
        if c == b'\n' {
            if matches!(data.get(i + 1), Some(b' ' | b'\t')) {
                lines.last_mut().unwrap().push(b' ');
            } else {
                lines.push(Vec::new());
            }
        } else {
            lines.last_mut().unwrap().push(c);
        }
    }
    for line in &lines {
        if line.len() < KEY.len() || !line[..KEY.len()].eq_ignore_ascii_case(KEY) {
            continue;
        }
        let Some(eq) = line.iter().position(|&c| c == b'=') else { continue };
        let rest = &line[eq + 1..];
        let rest = &rest[rest.iter().take_while(|c| matches!(c, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)).count()..];
        let (negative, digits) = match rest.first() {
            Some(b'-') => (true, &rest[1..]),
            Some(b'+') => (false, &rest[1..]),
            _ => (false, rest),
        };
        let len = digits.iter().take_while(|c| c.is_ascii_digit()).count();
        if len == 0 {
            return DEFAULT;
        }
        let value = digits[..len]
            .iter()
            .try_fold(0_i64, |n, &c| n.checked_mul(10)?.checked_add(i64::from(c - b'0')))
            .unwrap_or(i64::MAX);
        let value = if negative { -value } else { value };
        return if (2..=8).contains(&value) { value as u8 } else { DEFAULT };
    }
    DEFAULT
}
