//! `A1Dll` of `tools/a1_to_lua.py`: a release-built Armada I mission or rule DLL.

use std::collections::HashSet;

use indexmap::IndexMap;

use crate::disasm::Ins;
use crate::dsl::pat::M;
use crate::dsl::Dsl;
use crate::format::py_hex;
use crate::image::{short_buffer, Image};
use crate::map::Vtables;
use crate::{DecodeError, Result};

/// `A1Dll(dsl.Dsl)`: the core's [`Dsl`] plus the import stubs, the entries and `DllMain`.
///
/// The Python builds it from a path (it reads the file and runs the disassembler); here it is
/// built from what those give: the image, the listing and the export table. The lifter reads
/// `stubs` and `entries` through [`crate::a1::LifterExtra`] (see [`A1Dll::lifter`]).
pub struct A1Dll<'a> {
    pub dsl: Dsl<'a>,
    /// The file's path, as the Python object keeps it (`self.path`): `convert` writes its base name.
    pub path: String,
    /// `jmp [IAT]` stub address to its import symbol.
    pub stubs: IndexMap<i64, String>,
    /// `set(self.thunks.values()) | set(self.code_names)`.
    pub entries: HashSet<i64>,
    pub entry_point: i64,
    /// The function the C runtime's entry calls with `(hinst, reason, reserved)`.
    pub dllmain: Option<i64>,
}

/// The address of the PE entry point as `A1Dll.__init__` reads it from the file bytes
/// (`pe.base + AddressOfEntryPoint`); a short file is a `struct.error`.
fn read_entry_point(pe: &dyn Image) -> Result<i64> {
    let b = pe.data();
    let u32_at = |o: usize| -> Result<i64> {
        match b.get(o..o + 4) {
            Some(s) => Ok(i64::from(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))),
            None => Err(short_buffer(o, 4, b.len())),
        }
    };
    let peo = u32_at(0x3C)?;
    Ok(i64::from(pe.base()) + u32_at(peo as usize + 24 + 16)?)
}

/// `re.fullmatch(r"jmp\s+DWORD PTR ds:0x([0-9a-f]+)", t)`: the digits.
pub(crate) fn jmp_ds_import(t: &str) -> Option<&str> {
    let mut m = M::new(t);
    if !(m.lit("jmp") && m.ws1() && m.lit("DWORD PTR ds:0x")) {
        return None;
    }
    let h = m.hex1()?;
    m.end_exact().then_some(h)
}

impl<'a> A1Dll<'a> {
    /// `A1Dll.__init__(path, ins=..., pe=..., exports=..., entry_point=...)`. `entry_point` is read
    /// from the file's header when `None`.
    pub fn new(
        path: &str,
        pe: &'a dyn Image,
        ins: Vec<Ins>,
        exports: IndexMap<String, u32>,
        entry_point: Option<i64>,
    ) -> Result<Self> {
        let dsl = Dsl::new(pe, Vtables::new(), ins, exports)?;
        let mut stubs: IndexMap<i64, String> = IndexMap::new();
        for (a, t) in &dsl.ins {
            if let Some(h) = jmp_ds_import(t) {
                let slot = crate::pyval::py_int(h, 16)?;
                if let Some((_, sym)) = u32::try_from(slot).ok().and_then(|s| pe.imports().get(&s)) {
                    stubs.insert(i64::from(*a), sym.clone());
                }
            }
        }
        let mut entries: HashSet<i64> = dsl.thunks.values().copied().collect();
        entries.extend(dsl.code_names.keys().copied());
        let entry_point = match entry_point {
            Some(e) => e,
            None => read_entry_point(pe)?,
        };
        let mut d = A1Dll { dsl, path: path.to_owned(), stubs, entries, entry_point, dllmain: None };
        d.dllmain = d.find_dllmain()?;
        Ok(d)
    }

    /// `A1Dll.entry`: the function address behind a thunk; a `DecodeError` when no instruction
    /// is there.
    pub fn entry(&self, addr: i64) -> Result<i64> {
        let addr = self.dsl.thunks.get(&addr).copied().unwrap_or(addr);
        if !self.dsl.at.contains_key(&addr) {
            return Err(DecodeError::new(format!("no instruction at {}", py_hex(addr))));
        }
        Ok(addr)
    }

    /// `A1Dll.import_sym`: the symbol of an IAT slot.
    pub fn import_sym(&self, iat: i64) -> Option<String> {
        import_sym(self.dsl.pe, iat)
    }

    /// `A1Dll.find_dllmain`: the C runtime's DLL entry calls `DllMain(hinst, reason, reserved)`
    /// through the incremental-link table with its own three arguments.
    fn find_dllmain(&self) -> Result<Option<i64>> {
        let d = &self.dsl;
        let Some(&i) = d.at.get(&self.entry_point) else {
            return Ok(None);
        };
        let mut pushes = 0;
        let mut found: Vec<i64> = Vec::new();
        for (_, t) in d.ins.iter().skip(i).take(160) {
            if t.starts_with("push ") {
                pushes += 1;
                continue;
            }
            if let Some(h) = crate::dsl::pat::call_abs(t) {
                let tgt = crate::pyval::py_int(h, 16)?;
                if pushes >= 3
                    && let Some(&x) = d.thunks.get(&tgt)
                {
                    found.push(x);
                }
            }
            if t.starts_with("ret") {
                break;
            }
            if t.starts_with("call") || t.starts_with('j') {
                pushes = 0;
            }
        }
        Ok(match found.first() {
            Some(&f0) if found.iter().all(|f| *f == f0) => Some(f0),
            _ => None,
        })
    }
}

/// `d.import_sym(iat)`: `pe.imports.get(iat)[1]`.
pub(crate) fn import_sym(pe: &dyn Image, iat: i64) -> Option<String> {
    u32::try_from(iat).ok().and_then(|s| pe.imports().get(&s)).map(|x| x.1.clone())
}
