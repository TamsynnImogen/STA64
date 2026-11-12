//! Bink bit reader: bits are taken least-significant first from
//! little-endian 32-bit words, i.e. bit `p` of the stream is bit `p % 8` of
//! byte `p / 8`. Reads past the end yield zero bits and set `overrun`.

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    len_bits: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            len_bits: data.len() * 8,
        }
    }

    /// A read went past the end of the data.
    pub fn overrun(&self) -> bool {
        self.pos > self.len_bits
    }

    /// Bytes up to and including the last 32-bit word any consumed bit came
    /// from (where the next word-aligned unit starts).
    pub fn words_used_bytes(&self) -> usize {
        self.pos.div_ceil(32) * 4
    }

    #[inline]
    fn window(&self) -> u64 {
        let byte = self.pos >> 3;
        let mut buf = [0u8; 8];
        if byte + 8 <= self.data.len() {
            buf.copy_from_slice(&self.data[byte..byte + 8]);
        } else if byte < self.data.len() {
            let n = self.data.len() - byte;
            buf[..n].copy_from_slice(&self.data[byte..]);
        }
        u64::from_le_bytes(buf) >> (self.pos & 7)
    }

    /// The next `n` bits (0..=32) without consuming them.
    #[inline]
    pub fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        (self.window() & ((1u64 << n) - 1)) as u32
    }

    #[inline]
    pub fn skip(&mut self, n: u32) {
        self.pos += n as usize;
    }

    /// Read `n` bits (0..=32).
    #[inline]
    pub fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    #[inline]
    pub fn bit(&mut self) -> bool {
        self.read(1) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsb_first_words() {
        let data = [0b1010_0110u8, 0xff, 0x01, 0x80, 0x12];
        let mut b = Bits::new(&data);
        assert_eq!(b.read(1), 0);
        assert_eq!(b.read(2), 0b11);
        assert_eq!(b.read(5), 0b10100);
        assert_eq!(b.read(16), 0x01ff);
        assert_eq!(b.peek(8), 0x80);
        assert_eq!(b.read(12), 0x280);
        assert_eq!(b.words_used_bytes(), 8);
        assert!(!b.overrun());
        b.read(8);
        assert!(b.overrun());
    }
}
