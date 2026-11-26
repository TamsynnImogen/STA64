//! Test support: builds small 32-bit PE files from bytes the tests author themselves (sections,
//! an import directory by name and by ordinal, an export directory, an entry point). Nothing
//! here is taken from a game file; the images are not loadable programs.

use crate::image::{SCN_CNT_CODE, SCN_MEM_EXECUTE};

const FILE_ALIGN: usize = 0x200;
const SECTION_ALIGN: u32 = 0x1000;
const HEADERS: usize = 0x400;
const PE_OFFSET: usize = 0x40;
/// Optional header size: 96 bytes of standard and NT fields plus 16 data directories.
const OPT_SIZE: usize = 96 + 16 * 8;

/// Section flags for a code section.
pub const CODE: u32 = SCN_CNT_CODE | SCN_MEM_EXECUTE | 0x4000_0000;
/// Section flags for initialised, readable data.
pub const DATA: u32 = 0x40 | 0x4000_0000;

/// One imported symbol of a DLL.
#[derive(Debug, Clone)]
pub enum Import {
    Name(String),
    Ordinal(u16),
}

impl Import {
    pub fn name(s: &str) -> Self {
        Self::Name(s.to_owned())
    }
}

#[derive(Debug, Clone)]
struct Spec {
    name: String,
    va: u32,
    vsize: u32,
    data: Vec<u8>,
    flags: u32,
}

/// What [`PeBuilder::build`] made.
#[derive(Debug, Clone)]
pub struct Built {
    pub bytes: Vec<u8>,
    /// Virtual address of the IAT slot of every import, DLL by DLL in the order given.
    pub import_slots: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct PeBuilder {
    base: u32,
    entry: u32,
    machine: u16,
    sections: Vec<Spec>,
    imports: Vec<(String, Vec<Import>)>,
    exports: Vec<(String, u32)>,
    with_ilt: bool,
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

impl PeBuilder {
    pub fn new(base: u32) -> Self {
        Self {
            base,
            entry: 0,
            machine: 0x14C,
            sections: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            with_ilt: true,
        }
    }

    /// A section at `va` (offset from the base) holding `data`; its virtual size is the data's.
    pub fn section(self, name: &str, va: u32, data: &[u8], flags: u32) -> Self {
        let vsize = data.len() as u32;
        self.section_sized(name, va, vsize, data, flags)
    }

    /// Like [`Self::section`] with an explicit virtual size (larger than the data: a zeroed tail).
    pub fn section_sized(
        mut self,
        name: &str,
        va: u32,
        vsize: u32,
        data: &[u8],
        flags: u32,
    ) -> Self {
        self.sections.push(Spec {
            name: name.into(),
            va,
            vsize,
            data: data.to_vec(),
            flags,
        });
        self
    }

    pub fn entry(mut self, rva: u32) -> Self {
        self.entry = rva;
        self
    }

    pub fn machine(mut self, m: u16) -> Self {
        self.machine = m;
        self
    }

    pub fn import(mut self, dll: &str, syms: Vec<Import>) -> Self {
        self.imports.push((dll.to_owned(), syms));
        self
    }

    /// Leaves the lookup table of every import descriptor empty, so readers use the IAT.
    pub fn no_lookup_table(mut self) -> Self {
        self.with_ilt = false;
        self
    }

    /// An export by name. The address table is written in reverse of the name order, so a reader
    /// must go through the ordinal table.
    pub fn export(mut self, name: &str, rva: u32) -> Self {
        self.exports.push((name.to_owned(), rva));
        self
    }

    pub fn build(&self) -> Built {
        let mut specs = self.sections.clone();
        let mut next_va = specs
            .iter()
            .map(|s| s.va + s.vsize.max(s.data.len() as u32))
            .max()
            .unwrap_or(SECTION_ALIGN);
        next_va = next_va.div_ceil(SECTION_ALIGN) * SECTION_ALIGN;
        let mut import_slots = Vec::new();
        let (mut import_dir, mut export_dir) = ((0u32, 0u32), (0u32, 0u32));

        if !self.imports.is_empty() {
            let va = next_va;
            let mut d = vec![0u8; (self.imports.len() + 1) * 20];
            for (k, (dll, syms)) in self.imports.iter().enumerate() {
                let ilt = d.len();
                d.resize(ilt + (syms.len() + 1) * 4, 0);
                let iat = d.len();
                d.resize(iat + (syms.len() + 1) * 4, 0);
                let name_at = d.len();
                d.extend_from_slice(dll.as_bytes());
                d.push(0);
                for (i, s) in syms.iter().enumerate() {
                    let entry = match s {
                        Import::Ordinal(n) => 0x8000_0000 | *n as u32,
                        Import::Name(n) => {
                            if d.len() % 2 == 1 {
                                d.push(0);
                            }
                            let hint = va + d.len() as u32;
                            d.extend_from_slice(&[0, 0]);
                            d.extend_from_slice(n.as_bytes());
                            d.push(0);
                            hint
                        }
                    };
                    put32(&mut d, ilt + 4 * i, entry);
                    put32(&mut d, iat + 4 * i, entry);
                    import_slots.push(self.base + va + (iat + 4 * i) as u32);
                }
                let o = k * 20;
                put32(&mut d, o, if self.with_ilt { va + ilt as u32 } else { 0 });
                put32(&mut d, o + 12, va + name_at as u32);
                put32(&mut d, o + 16, va + iat as u32);
            }
            import_dir = (va, (self.imports.len() as u32 + 1) * 20);
            let size = d.len() as u32;
            specs.push(Spec {
                name: ".idata".into(),
                va,
                vsize: size,
                data: d,
                flags: DATA,
            });
            next_va = (va + size).div_ceil(SECTION_ALIGN) * SECTION_ALIGN;
        }

        if !self.exports.is_empty() {
            let va = next_va;
            let n = self.exports.len();
            let mut d = vec![0u8; 40];
            let eat = d.len();
            d.resize(eat + 4 * n, 0);
            let enpt = d.len();
            d.resize(enpt + 4 * n, 0);
            let eord = d.len();
            d.resize(eord + 2 * n, 0);
            for (i, (name, rva)) in self.exports.iter().enumerate() {
                let at = d.len();
                d.extend_from_slice(name.as_bytes());
                d.push(0);
                let ord = n - 1 - i;
                put32(&mut d, eat + 4 * ord, *rva);
                put32(&mut d, enpt + 4 * i, va + at as u32);
                put16(&mut d, eord + 2 * i, ord as u16);
            }
            put32(&mut d, 20, n as u32);
            put32(&mut d, 24, n as u32);
            put32(&mut d, 28, va + eat as u32);
            put32(&mut d, 32, va + enpt as u32);
            put32(&mut d, 36, va + eord as u32);
            export_dir = (va, d.len() as u32);
            specs.push(Spec {
                name: ".edata".into(),
                va,
                vsize: d.len() as u32,
                data: d,
                flags: DATA,
            });
        }

        assert!(
            PE_OFFSET + 24 + OPT_SIZE + specs.len() * 40 <= HEADERS,
            "too many sections for the fixture"
        );
        let mut raw_offs = Vec::new();
        let mut file_len = HEADERS;
        for s in &specs {
            raw_offs.push(file_len);
            file_len += align(s.data.len(), FILE_ALIGN);
        }
        let mut b = vec![0u8; file_len];
        b[..2].copy_from_slice(b"MZ");
        put32(&mut b, 0x3C, PE_OFFSET as u32);
        let pe = PE_OFFSET;
        b[pe..pe + 4].copy_from_slice(b"PE\0\0");
        put16(&mut b, pe + 4, self.machine);
        put16(&mut b, pe + 6, specs.len() as u16);
        put16(&mut b, pe + 20, OPT_SIZE as u16);
        put16(&mut b, pe + 22, 0x2102);
        put16(&mut b, pe + 24, 0x10B);
        put32(&mut b, pe + 24 + 16, self.entry);
        put32(&mut b, pe + 24 + 28, self.base);
        let dd = pe + 24 + 96;
        put32(&mut b, dd, export_dir.0);
        put32(&mut b, dd + 4, export_dir.1);
        put32(&mut b, dd + 8, import_dir.0);
        put32(&mut b, dd + 12, import_dir.1);
        for (i, s) in specs.iter().enumerate() {
            let o = pe + 24 + OPT_SIZE + i * 40;
            let nm = s.name.as_bytes();
            b[o..o + nm.len().min(8)].copy_from_slice(&nm[..nm.len().min(8)]);
            put32(&mut b, o + 8, s.vsize);
            put32(&mut b, o + 12, s.va);
            put32(&mut b, o + 16, align(s.data.len(), FILE_ALIGN) as u32);
            put32(&mut b, o + 20, raw_offs[i] as u32);
            put32(&mut b, o + 36, s.flags);
            b[raw_offs[i]..raw_offs[i] + s.data.len()].copy_from_slice(&s.data);
        }
        Built {
            bytes: b,
            import_slots,
        }
    }
}
