//! A 32-bit PE file read as bytes (port of `drl_to_lua.Pe`, `dsl_to_lua.read_exports` and the
//! entry-point read of `a1_to_lua.A1Dll`). The file is never mapped or executed (D2).

use crate::image::{Image, Section};
use crate::{DecodeError, Result};
use indexmap::IndexMap;

fn truncated() -> DecodeError {
    DecodeError::raw("truncated PE file")
}

fn u16at(b: &[u8], o: usize) -> Result<u16> {
    b.get(o..o.wrapping_add(2))
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(truncated)
}

fn u32at(b: &[u8], o: usize) -> Result<u32> {
    b.get(o..o.wrapping_add(4))
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(truncated)
}

/// Offset of the data directory array (the PE signature offset plus the headers before it).
const DIRS: usize = 24 + 96;

#[derive(Debug, Clone)]
pub struct Pe {
    pub data: Vec<u8>,
    pub base: u32,
    pub sections: Vec<Section>,
    /// Size field of the export data directory.
    pub export_size: u32,
    /// `AddressOfEntryPoint`, an offset from the base.
    pub entry_rva: u32,
    /// IAT slot virtual address -> (dll, symbol); ordinals are `#n`. In table order.
    pub imports: IndexMap<u32, (String, String)>,
}

impl Image for Pe {
    fn base(&self) -> u32 {
        self.base
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
    fn sections(&self) -> &[Section] {
        &self.sections
    }
    fn imports(&self) -> &IndexMap<u32, (String, String)> {
        &self.imports
    }
    fn export_size(&self) -> u32 {
        self.export_size
    }
}

/// STA64's own limit on the import and export tables, so a crafted file cannot make the reader
/// run out of memory or time (the Python tools have none): tables and names of a linked file do
/// not overlap, so the entries cannot outnumber the file's words and the strings read (an
/// import's dll name counts again for every entry that holds a copy of it) cannot outgrow a
/// small multiple of the file. Past it the read fails with a raw [`DecodeError`]; no real file
/// comes near.
fn name_budget(len: usize) -> usize {
    len.saturating_mul(4)
}

impl Pe {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let data = std::fs::read(path)
            .map_err(|e| DecodeError::raw(format!("{}: {e}", path.display())))?;
        Self::from_bytes(data)
    }

    /// Parses the headers, the section table and the import directory.
    ///
    /// Error cases as in the Python tool: not `MZ`, not `PE\0\0`, a machine other than i386
    /// (`DecodeError`, not raw). A truncated header, a section table or import table that runs
    /// past the file, or an import table whose thunk array lies outside every section is a raw
    /// error (the Python raises `struct.error` or `TypeError`). Two differences on crafted
    /// files, both declared: an address that would wrap past 2^32 is an error where Python's
    /// unbounded integers go on, and the import tables are bounded (see `name_budget`).
    /// A file that ends inside the last 16 bytes of the last section header parses, as in
    /// Python (its `characteristics` read 0; the Python tool does not read that field).
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
            return Err(DecodeError::new(format!(
                "machine {machine:#x}, expected i386"
            )));
        }
        let opt = u16at(&data, pe + 20)? as usize;
        let entry_rva = u32at(&data, pe + 24 + 16)?;
        let base = u32at(&data, pe + 24 + 28)?;
        let mut sections = Vec::with_capacity(n);
        for i in 0..n {
            let o = pe + 24 + opt + i * 40;
            let nm = data.get(o..o + 8).ok_or_else(truncated)?;
            let end = nm.iter().rposition(|&c| c != 0).map_or(0, |p| p + 1);
            sections.push(Section {
                name: nm[..end].iter().map(|&c| c as char).collect(),
                vsize: u32at(&data, o + 8)?,
                va: u32at(&data, o + 12)?,
                raw_size: u32at(&data, o + 16)?,
                raw_off: u32at(&data, o + 20)?,
                characteristics: u32at(&data, o + 36).unwrap_or(0),
            });
        }
        let dd = pe + DIRS;
        let export_size = u32at(&data, dd + 4)?;
        let import_rva = u32at(&data, dd + 8)?;
        let mut me = Self {
            data,
            base,
            sections,
            export_size,
            entry_rva,
            imports: IndexMap::new(),
        };
        let too_large = || DecodeError::raw("import table too large");
        let (max_entries, max_bytes) = (me.data.len() / 4, name_budget(me.data.len()));
        let (mut entries, mut bytes) = (0usize, 0usize);
        let mut p = me.off(import_rva);
        while let Some(po) = p {
            let ilt = u32at(&me.data, po)?;
            let name = u32at(&me.data, po + 12)?;
            let iat = u32at(&me.data, po + 16)?;
            if name == 0 {
                break;
            }
            let dll = me.cstr(base.wrapping_add(name))?;
            bytes += dll.len();
            if bytes > max_bytes {
                return Err(too_large());
            }
            let q = me
                .off(if ilt != 0 { ilt } else { iat })
                .ok_or_else(|| DecodeError::raw("bad import table"))?;
            let mut i = 0usize;
            loop {
                let e = u32at(&me.data, q + 4 * i)?;
                if e == 0 {
                    break;
                }
                entries += 1;
                if entries > max_entries {
                    return Err(too_large());
                }
                let sym = if e & 0x8000_0000 != 0 {
                    format!("#{}", e & 0xFFFF)
                } else {
                    me.cstr(base.wrapping_add(e).wrapping_add(2))?
                };
                bytes += dll.len() + sym.len();
                if bytes > max_bytes {
                    return Err(too_large());
                }
                me.imports.insert(
                    base.wrapping_add(iat).wrapping_add(4 * i as u32),
                    (dll.clone(), sym),
                );
                i += 1;
            }
            p = Some(po + 20);
        }
        Ok(me)
    }

    /// The entry point's virtual address.
    pub fn entry_point(&self) -> u32 {
        self.base.wrapping_add(self.entry_rva)
    }

    /// Exported names to virtual addresses, in name-table order. A name exported twice keeps its
    /// first position and takes the later address (a Python dict assignment). Empty when the
    /// file has no export directory.
    pub fn exports(&self) -> Result<IndexMap<String, u32>> {
        let b = &self.data;
        let peo = u32at(b, 0x3C)? as usize;
        let erva = u32at(b, peo + DIRS)?;
        let mut out = IndexMap::new();
        if erva == 0 {
            return Ok(out);
        }
        let bad = || DecodeError::raw("bad export table");
        let too_large = || DecodeError::raw("export table too large");
        let o = self.off(erva).ok_or_else(bad)?;
        let nnames = u32at(b, o + 24)? as usize;
        let eat = u32at(b, o + 28)?;
        let enpt = u32at(b, o + 32)?;
        let eord = u32at(b, o + 36)?;
        // STA64's own limit, see `name_budget`: at most one name per word of the file.
        if nnames > b.len() / 4 {
            return Err(too_large());
        }
        let (max_bytes, mut bytes) = (name_budget(b.len()), 0usize);
        for i in 0..nnames {
            let name_rva = u32at(b, self.off(enpt).ok_or_else(bad)? + 4 * i)?;
            let nm = self.cstr(self.base.wrapping_add(name_rva))?;
            bytes += nm.len();
            if bytes > max_bytes {
                return Err(too_large());
            }
            let ordi = u16at(b, self.off(eord).ok_or_else(bad)? + 2 * i)? as usize;
            let target = u32at(b, self.off(eat).ok_or_else(bad)? + 4 * ordi)?;
            out.insert(nm, self.base.wrapping_add(target));
        }
        Ok(out)
    }
}
