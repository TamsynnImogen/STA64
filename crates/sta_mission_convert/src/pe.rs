//! A 32-bit PE file read as bytes (port of `drl_to_lua.Pe`).

use crate::{DecodeError, Result};
use std::collections::BTreeMap;

fn u16at(b: &[u8], o: usize) -> Result<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| DecodeError::new("truncated PE file"))
}
fn u32at(b: &[u8], o: usize) -> Result<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| DecodeError::new("truncated PE file"))
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub va: u32,
    pub vsize: u32,
    pub raw_off: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

#[derive(Debug, Clone)]
pub struct Pe {
    pub data: Vec<u8>,
    pub base: u32,
    pub sections: Vec<Section>,
    pub export_size: u32,
    /// IAT slot virtual address -> (dll, symbol); ordinals are `#n`.
    pub imports: BTreeMap<u32, (String, String)>,
}

impl Pe {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|e| DecodeError::new(format!("{}: {e}", path.display())))?;
        Self::from_bytes(data)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let not_pe = || DecodeError::new("not a PE file");
        if data.get(..2) != Some(b"MZ") {
            return Err(not_pe());
        }
        let pe = u32at(&data, 0x3C)? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(not_pe());
        }
        let machine = u16at(&data, pe + 4)?;
        let n = u16at(&data, pe + 6)? as usize;
        if machine != 0x14C {
            return Err(DecodeError::new(format!("machine {machine:#x}, expected i386")));
        }
        let opt = u16at(&data, pe + 20)? as usize;
        let base = u32at(&data, pe + 24 + 28)?;
        let mut sections = Vec::new();
        for i in 0..n {
            let o = pe + 24 + opt + i * 40;
            let nm = data.get(o..o + 8).ok_or_else(|| DecodeError::new("truncated PE file"))?;
            let end = nm.iter().position(|&c| c == 0).unwrap_or(8);
            sections.push(Section {
                name: nm[..end].iter().map(|&c| c as char).collect(),
                vsize: u32at(&data, o + 8)?,
                va: u32at(&data, o + 12)?,
                raw_size: u32at(&data, o + 16)?,
                raw_off: u32at(&data, o + 20)?,
                characteristics: u32at(&data, o + 36)?,
            });
        }
        let dd = pe + 24 + 96;
        let export_size = u32at(&data, dd + 4)?;
        let import_rva = u32at(&data, dd + 8)?;
        let mut me = Self { data, base, sections, export_size, imports: BTreeMap::new() };
        let mut p = me.off(import_rva);
        while let Some(po) = p {
            let ilt = u32at(&me.data, po)?;
            let name = u32at(&me.data, po + 12)?;
            let iat = u32at(&me.data, po + 16)?;
            if name == 0 {
                break;
            }
            let dll = me.cstr(base.wrapping_add(name))?;
            let q = me.off(if ilt != 0 { ilt } else { iat }).ok_or_else(|| DecodeError::new("bad import table"))?;
            let mut i = 0usize;
            loop {
                let e = u32at(&me.data, q + 4 * i)?;
                if e == 0 {
                    break;
                }
                let sym = if e & 0x8000_0000 != 0 {
                    format!("#{}", e & 0xFFFF)
                } else {
                    me.cstr(base.wrapping_add(e).wrapping_add(2))?
                };
                me.imports.insert(base.wrapping_add(iat).wrapping_add(4 * i as u32), (dll.clone(), sym));
                i += 1;
            }
            p = Some(po + 20);
        }
        Ok(me)
    }

    /// File offset of an RVA, or `None` outside every section's raw data.
    pub fn off(&self, rva: u32) -> Option<usize> {
        for s in &self.sections {
            if s.va <= rva && (rva as u64) < s.va as u64 + s.vsize.max(s.raw_size) as u64 && rva - s.va < s.raw_size {
                return Some((s.raw_off + rva - s.va) as usize);
            }
        }
        None
    }

    pub fn section_of(&self, addr: u32) -> Option<&str> {
        self.sections
            .iter()
            .find(|s| (self.base as u64 + s.va as u64) <= addr as u64 && (addr as u64) < self.base as u64 + s.va as u64 + s.vsize as u64)
            .map(|s| s.name.as_str())
    }

    /// The NUL-terminated string at a virtual address (bytes as Latin-1, like the Python tool).
    pub fn cstr(&self, addr: u32) -> Result<String> {
        let o = self.off(addr.wrapping_sub(self.base)).ok_or_else(|| DecodeError::new(format!("string at {addr:#x} outside the file")))?;
        let rest = &self.data[o..];
        let end = rest.iter().position(|&c| c == 0).ok_or_else(|| DecodeError::new("unterminated string"))?;
        Ok(rest[..end].iter().map(|&c| c as char).collect())
    }

    pub fn f32(&self, addr: u32) -> Result<f32> {
        let o = self.off(addr.wrapping_sub(self.base)).ok_or_else(|| DecodeError::new(format!("float at {addr:#x} outside the file")))?;
        Ok(f32::from_bits(u32at(&self.data, o)?))
    }

    pub fn f64(&self, addr: u32) -> Result<f64> {
        let o = self.off(addr.wrapping_sub(self.base)).ok_or_else(|| DecodeError::new(format!("double at {addr:#x} outside the file")))?;
        let b = self.data.get(o..o + 8).ok_or_else(|| DecodeError::new("truncated PE file"))?;
        Ok(f64::from_le_bytes(b.try_into().unwrap()))
    }
}
