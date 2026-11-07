//! Bounds-checked LZO1X decompressor.
//!
//! Written from the public LZO1X stream grammar. Every read and back-reference is
//! checked, so malformed archives return an error instead of reading out of bounds.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LzoError {
    #[error("input overrun")]
    InputOverrun,
    #[error("output overrun")]
    OutputOverrun,
    #[error("back-reference before start of output")]
    LookbehindOverrun,
    #[error("no end-of-stream marker")]
    NoEof,
    #[error("decoded {got} bytes, expected {expected}")]
    SizeMismatch { got: usize, expected: usize },
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<usize, LzoError> {
        let b = *self.buf.get(self.pos).ok_or(LzoError::InputOverrun)?;
        self.pos += 1;
        Ok(b as usize)
    }
    fn le16(&mut self) -> Result<usize, LzoError> {
        Ok(self.byte()? | (self.byte()? << 8))
    }
    /// Run-length extension: each zero byte adds 255, then a final non-zero byte.
    fn extended(&mut self, base: usize) -> Result<usize, LzoError> {
        let mut n = base;
        loop {
            match self.byte()? {
                0 => n += 255,
                b => return Ok(n + b),
            }
        }
    }
    fn take(&mut self, n: usize) -> Result<&[u8], LzoError> {
        let end = self.pos.checked_add(n).ok_or(LzoError::InputOverrun)?;
        let s = self.buf.get(self.pos..end).ok_or(LzoError::InputOverrun)?;
        self.pos = end;
        Ok(s)
    }
}

/// Decompress `src`, requiring exactly `expected` output bytes.
pub fn decompress(src: &[u8], expected: usize) -> Result<Vec<u8>, LzoError> {
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut r = Reader { buf: src, pos: 0 };

    let literals = |out: &mut Vec<u8>, r: &mut Reader, n: usize| -> Result<(), LzoError> {
        if out.len() + n > expected {
            return Err(LzoError::OutputOverrun);
        }
        out.extend_from_slice(r.take(n)?);
        Ok(())
    };
    let copy_match = |out: &mut Vec<u8>, dist: usize, len: usize| -> Result<(), LzoError> {
        if dist == 0 || dist > out.len() {
            return Err(LzoError::LookbehindOverrun);
        }
        if out.len() + len > expected {
            return Err(LzoError::OutputOverrun);
        }
        let start = out.len() - dist;
        for i in 0..len {
            let b = out[start + i];
            out.push(b);
        }
        Ok(())
    };

    // `state` is the number of literals copied after the previous instruction
    // (0..=3), or 4 after a literal run of 4+ bytes. It changes how short
    // (t < 16) instructions are read.
    let mut state: usize;
    let first = *src.first().ok_or(LzoError::InputOverrun)? as usize;
    if first > 17 {
        r.pos = 1;
        let n = first - 17;
        literals(&mut out, &mut r, n)?;
        state = if n < 4 { n } else { 4 };
    } else {
        state = 0;
    }

    loop {
        let t = r.byte()?;
        let (dist, len, next);
        if t < 16 {
            if state == 0 {
                let n = if t == 0 { r.extended(15)? } else { t } + 3;
                literals(&mut out, &mut r, n)?;
                state = 4;
                continue;
            } else if state != 4 {
                next = t & 3;
                dist = 1 + (t >> 2) + (r.byte()? << 2);
                len = 2;
            } else {
                next = t & 3;
                dist = 1 + 0x800 + (t >> 2) + (r.byte()? << 2);
                len = 3;
            }
        } else if t >= 64 {
            next = t & 3;
            dist = 1 + ((t >> 2) & 7) + (r.byte()? << 3);
            len = (t >> 5) + 1;
        } else if t >= 32 {
            let l = t & 31;
            len = if l == 0 { r.extended(31)? } else { l } + 2;
            let d = r.le16()?;
            dist = 1 + (d >> 2);
            next = d & 3;
        } else {
            // 16..=31: far match or end of stream.
            let l = t & 7;
            len = if l == 0 { r.extended(7)? } else { l } + 2;
            let d = r.le16()?;
            let far = ((t & 8) << 11) + (d >> 2);
            if far == 0 {
                return if out.len() == expected {
                    Ok(out)
                } else {
                    Err(LzoError::SizeMismatch { got: out.len(), expected })
                };
            }
            dist = far + 0x4000;
            next = d & 3;
        }
        copy_match(&mut out, dist, len)?;
        literals(&mut out, &mut r, next)?;
        state = next;
        if r.pos >= src.len() {
            return Err(LzoError::NoEof);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_only_stream() {
        // 3 literals via the >17 first-byte form, then EOF marker.
        let s = [17 + 3, b'a', b'b', b'c', 0x11, 0, 0];
        assert_eq!(decompress(&s, 3).unwrap(), b"abc");
    }

    #[test]
    fn match_copies_overlapping() {
        // literals "ab" (first byte 17+2, state=2), then M2 len 3..8:
        // t >= 64: len = (t>>5)+1, dist = 1 + ((t>>2)&7) + (b<<3).
        // want len 6, dist 2: t>>5 = 5 -> 0xA0, (t>>2)&7 = 1 -> +4, next 0.
        let s = [17 + 2, b'a', b'b', 0xA4, 0x00, 0x11, 0, 0];
        assert_eq!(decompress(&s, 8).unwrap(), b"abababab");
    }

    #[test]
    fn rejects_bad_lookbehind_and_truncation() {
        let s = [17 + 1, b'a', 0xA4, 0x10, 0x11, 0, 0];
        assert_eq!(decompress(&s, 9), Err(LzoError::LookbehindOverrun));
        assert_eq!(decompress(&[17 + 3, b'a'], 3), Err(LzoError::InputOverrun));
    }
}
