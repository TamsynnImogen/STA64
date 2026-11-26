//! What the lifters read from a DLL: sections, the mapping between virtual addresses and file
//! bytes, strings and numbers at an address (the `Pe` methods of `tools/drl_to_lua.py`).
//!
//! [`Image`] is a trait so the lifter tests can use a fake image (the Python tests' `FakePe`);
//! [`crate::Pe`] is the real one, [`FlatImage`] the fake.

use crate::{DecodeError, Result};

/// `IMAGE_SCN_CNT_CODE`: the section holds code.
pub const SCN_CNT_CODE: u32 = 0x20;
/// `IMAGE_SCN_MEM_EXECUTE`: the section can be executed.
pub const SCN_MEM_EXECUTE: u32 = 0x2000_0000;

/// One section header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Trailing NULs stripped, bytes read as Latin-1 (like the Python tool).
    pub name: String,
    /// Offset from the image base.
    pub va: u32,
    pub vsize: u32,
    pub raw_off: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

impl Section {
    /// The section's flags mark it as code. The decoder step (LUA-15b) settles which flags
    /// the linear sweep needs to match the reference listing (not yet measured).
    pub fn is_code(&self) -> bool {
        self.characteristics & (SCN_CNT_CODE | SCN_MEM_EXECUTE) != 0
    }
}

/// A loaded-looking view of a file: address to bytes.
pub trait Image {
    /// Preferred load address.
    fn base(&self) -> u32;
    /// The whole file (or fake image) as bytes.
    fn data(&self) -> &[u8];
    fn sections(&self) -> &[Section];

    /// File offset of an RVA, or `None` outside every section's raw data (the first section
    /// that contains it wins).
    fn off(&self, rva: u32) -> Option<usize> {
        for s in self.sections() {
            let end = s.va as u64 + s.vsize.max(s.raw_size) as u64;
            if s.va <= rva && (rva as u64) < end && rva - s.va < s.raw_size {
                return Some(s.raw_off as usize + (rva - s.va) as usize);
            }
        }
        None
    }

    /// File offset of a virtual address (`None` below the base or outside the raw data).
    fn off_of(&self, addr: u32) -> Option<usize> {
        self.off(addr.checked_sub(self.base())?)
    }

    /// Name of the section whose virtual extent contains the address (the first one).
    fn section_of(&self, addr: u32) -> Option<&str> {
        let base = self.base() as u64;
        self.sections()
            .iter()
            .find(|s| {
                base + s.va as u64 <= addr as u64
                    && (addr as u64) < base + s.va as u64 + s.vsize as u64
            })
            .map(|s| s.name.as_str())
    }

    /// The NUL-terminated string at a virtual address, bytes read as Latin-1.
    fn cstr(&self, addr: u32) -> Result<String> {
        let o = self
            .off_of(addr)
            .ok_or_else(|| DecodeError::new(format!("string at {addr:#x} outside the file")))?;
        let rest = self.data().get(o..).unwrap_or(&[]);
        let end = rest
            .iter()
            .position(|&c| c == 0)
            .ok_or_else(|| DecodeError::new("unterminated string"))?;
        Ok(rest[..end].iter().map(|&c| c as char).collect())
    }

    fn f32(&self, addr: u32) -> Result<f32> {
        let o = self
            .off_of(addr)
            .ok_or_else(|| DecodeError::new(format!("float at {addr:#x} outside the file")))?;
        let b = self
            .data()
            .get(o..o + 4)
            .ok_or_else(|| DecodeError::new("truncated image"))?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f64(&self, addr: u32) -> Result<f64> {
        let o = self
            .off_of(addr)
            .ok_or_else(|| DecodeError::new(format!("double at {addr:#x} outside the file")))?;
        let b = self
            .data()
            .get(o..o + 8)
            .ok_or_else(|| DecodeError::new("truncated image"))?;
        Ok(f64::from_le_bytes(b.try_into().expect("eight bytes")))
    }

    /// The 32-bit word the file holds at an address (`None` outside it or past its end).
    fn dword(&self, addr: u32) -> Option<u32> {
        let o = self.off_of(addr)?;
        let b = self.data().get(o..o + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// A fake image for tests: the bytes are indexed by `address - base` directly (like the Python
/// tests' `FakePe`), and sections only name address ranges for [`Image::section_of`].
#[derive(Debug, Clone, Default)]
pub struct FlatImage {
    pub base: u32,
    pub data: Vec<u8>,
    pub sections: Vec<Section>,
}

impl FlatImage {
    pub fn new(base: u32, size: usize) -> Self {
        Self {
            base,
            data: vec![0; size],
            sections: Vec::new(),
        }
    }

    /// Names the range `[va, va + size)` (offsets from the base).
    pub fn with_section(mut self, name: &str, va: u32, size: u32) -> Self {
        self.sections.push(Section {
            name: name.into(),
            va,
            vsize: size,
            raw_off: va,
            raw_size: size,
            characteristics: 0,
        });
        self
    }

    pub fn put(&mut self, addr: u32, bytes: &[u8]) {
        let o = (addr - self.base) as usize;
        self.data[o..o + bytes.len()].copy_from_slice(bytes);
    }
}

impl Image for FlatImage {
    fn base(&self) -> u32 {
        self.base
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
    fn sections(&self) -> &[Section] {
        &self.sections
    }
    fn off(&self, rva: u32) -> Option<usize> {
        ((rva as usize) < self.data.len()).then_some(rva as usize)
    }
}
