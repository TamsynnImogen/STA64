//! Armada 1 front matter and known binary map-body data.
//!
//! Informed by the project's A2FOExtensions `a1_maps` reader and the earlier
//! OpenSourced-Space-RTS header inspector. No opaque post-header skip is used.

mod body;
mod starts;
mod write;
pub(crate) use body::parse_map;
pub(crate) use write::write_map;
pub use body::{AiPath, AreaOfInterest, MapData};
pub use starts::{StartError, mdf_start_count, native_mdf_start_count, player_start_positions, player_start_positions_with_fallback};

/// Lossless A1 front matter. The offset identifies metadata, not an object list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub save_game_desc: [u8; 128],
    pub binary_save: bool,
    pub metadata_offset: usize,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HeaderError {
    #[error("invalid Armada 1 BZN header at byte {0}")]
    Invalid(usize),
    #[error("unsupported Armada 1 BZN header version {0}")]
    UnsupportedVersion(u32),
}

/// Inspect known A1 front matter without interpreting or changing the body.
/// LF and CRLF are accepted; every header line must have a terminating newline.
pub fn inspect_header(data: &[u8]) -> Result<Header, HeaderError> {
    let mut cursor = Cursor { data, at: 0 };
    cursor.expect(b"version [1] =")?;
    let version_at = cursor.at;
    let version = cursor.line()?;
    let version = (!version.is_empty())
        .then(|| {
            version.iter().try_fold(0_u32, |n, &c| {
                if !c.is_ascii_digit() {
                    return None;
                }
                n.checked_mul(10)?.checked_add(u32::from(c - b'0'))
            })
        })
        .flatten()
        .ok_or(HeaderError::Invalid(version_at))?;
    if !(2050..=2053).contains(&version) {
        return Err(HeaderError::UnsupportedVersion(version));
    }

    let description_at = cursor.at;
    let hex = cursor
        .line()?
        .strip_prefix(b"saveGameDesc = ")
        .filter(|hex| hex.len() == 256 && hex.iter().all(u8::is_ascii_hexdigit))
        .ok_or(HeaderError::Invalid(description_at))?;
    let mut save_game_desc = [0; 128];
    for (byte, pair) in save_game_desc.iter_mut().zip(hex.chunks_exact(2)) {
        let nibble = |c: u8| {
            if c.is_ascii_digit() {
                c - b'0'
            } else {
                c.to_ascii_lowercase() - b'a' + 10
            }
        };
        *byte = nibble(pair[0]) * 16 + nibble(pair[1]);
    }

    cursor.expect(b"binarySave [1] =")?;
    let binary_at = cursor.at;
    let binary_save = match cursor.line()? {
        b"true" => true,
        b"false" => false,
        _ => return Err(HeaderError::Invalid(binary_at)),
    };
    Ok(Header {
        version,
        save_game_desc,
        binary_save,
        metadata_offset: cursor.at,
    })
}

// Recognize only front matter, never a matching string elsewhere in an A2 body.
pub(crate) fn has_header_marker(data: &[u8]) -> bool {
    data.split(|&c| c == b'\n').nth(2).is_some_and(|line| {
        let name = line
            .split(|&c| c.is_ascii_whitespace() || matches!(c, b'=' | b':' | b'['))
            .next()
            .unwrap_or_default();
        matches!(name, b"saveGameDesc" | b"binarySave")
    })
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn line(&mut self) -> Result<&'a [u8], HeaderError> {
        let start = self.at;
        let end = self.data[start..]
            .iter()
            .position(|&c| c == b'\n')
            .map(|n| start + n)
            .ok_or(HeaderError::Invalid(start))?;
        self.at = end + 1;
        let line = &self.data[start..end];
        Ok(line.strip_suffix(b"\r").unwrap_or(line))
    }

    fn expect(&mut self, expected: &[u8]) -> Result<(), HeaderError> {
        let at = self.at;
        if self.line()? == expected {
            Ok(())
        } else {
            Err(HeaderError::Invalid(at))
        }
    }
}
