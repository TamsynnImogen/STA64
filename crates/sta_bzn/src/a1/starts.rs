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
    let count = supported_count(u32::from(count))?;
    (1..=count)
        .map(|slot| {
            let label = format!("Camera_{slot}");
            let path = map
                .paths
                .iter()
                .find(|path| path.label == label)
                .ok_or(StartError::MissingCamera(slot))?;
            let point = *path.points.first().ok_or(StartError::EmptyCamera(slot))?;
            if !point.iter().all(|v| v.is_finite()) {
                return Err(StartError::NonfiniteCamera(slot));
            }
            Ok(point)
        })
        .collect()
}
