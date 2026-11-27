//! Legacy mission conversion in Rust (LUA-15): the player's own Armada II `.dsl`/`.drl` and
//! Armada I mission DLLs become Lua on their machine. A port of `tools/dsl_to_lua.py`,
//! `drl_to_lua.py`, `a1_to_lua.py` and `convert_install_missions.py`, which stay as the
//! reference until the install-gated parity test is green for every mission.
//!
//! The DLLs are only ever read as bytes (D2): [`pe`] parses the file, [`image`] is what the
//! lifters read from it (addresses, strings, numbers), [`disasm`] will decode the x86 text,
//! nothing is loaded or executed. Nothing converted is committed or shipped (the output goes
//! to the user data directory).
//!
//! So far: the image layer (LUA-15a: the PE reader, the engine `.map` reader, the engine vtable
//! names) and the shared core (LUA-15c: [`pyval`], [`format`], [`msvc`], [`algebra`],
//! [`prelude`]). Decoding and lifting follow in later steps.

/// The converter's version. It is part of the tool token every emitter writes into the first
/// line of its output, and the conversion cache will key on it: raise it when the output of any
/// converter changes.
pub const CONVERTER_VERSION: u32 = 1;

/// The tool token of the header line of every emitted file: `sta_mission_convert (converter 1)`
/// (a token without whitespace, then the bracket text; the Python tools write their own script
/// name and a package id there, which the parity comparison ignores).
pub fn tool_token() -> String {
    format!("sta_mission_convert (converter {CONVERTER_VERSION})")
}

pub mod algebra;
pub mod disasm;
pub mod drl;
#[doc(hidden)]
pub mod fixture;
pub mod format;
pub mod image;
pub mod map;
pub mod msvc;
pub mod pe;
pub mod prelude;
pub mod pyval;

pub use image::{FlatImage, Image, Section};

pub use pe::Pe;

/// A DLL (or a function in it) the converter could not decode; the message says why.
///
/// The Python tools raise `DecodeError` for faults about the file's content, which their callers
/// catch (a function is written as not decoded, the run goes on), and a raw `ValueError`,
/// `struct.error`, `TypeError` or `StopIteration` for a truncated or malformed structure, which
/// aborts the run (A2) or fails the mission (A1); and an `OSError` for a file that cannot be read.
/// All are a `DecodeError` here, told apart by [`ErrorKind`]:
/// - a caller ports `except DecodeError` as "catch [`ErrorKind::Decode`]";
/// - the engine lookup's `except (OSError, DecodeError)` is "catch `Decode` or `Io`" and goes on
///   to the next root (it must not catch `Raw`, which aborts);
/// - for a mission DLL the `OSError` of the open is not caught in Python: treat `Io` as fatal there.
///
/// STA64's own limits on crafted tables (see [`pe`]) are `Raw` faults too.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DecodeError {
    pub message: String,
    pub kind: ErrorKind,
    /// The Python exception class name (`KeyError`, `TypeError`, ...) of a `Raw` fault that
    /// stands for one of Python's own exceptions, `None` for a `Decode` or `Io` fault and for
    /// STA64's own limits. The Armada I converter writes `{class}: {message}` into the Lua it
    /// emits for a function whose rendering raised, so ports of the render path use
    /// [`DecodeError::python`] with Python's exact text.
    pub class: Option<&'static str>,
}

/// Which Python exception a fault stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Python's `DecodeError`.
    Decode,
    /// Python's `OSError`: a file or directory that cannot be opened or listed.
    Io,
    /// Python's `ValueError`, `struct.error`, `TypeError` or `StopIteration`.
    Raw,
}

impl DecodeError {
    /// A fault the Python tools raise as `DecodeError`.
    pub fn new(s: impl Into<String>) -> Self {
        Self {
            message: s.into(),
            kind: ErrorKind::Decode,
            class: None,
        }
    }

    /// A fault the Python tools raise as a raw exception.
    pub fn raw(s: impl Into<String>) -> Self {
        Self {
            message: s.into(),
            kind: ErrorKind::Raw,
            class: None,
        }
    }

    /// A fault of one of Python's own exceptions (`class` is its name, `message` the text
    /// `str(e)` gives); kind `Raw`.
    pub fn python(class: &'static str, s: impl Into<String>) -> Self {
        Self {
            message: s.into(),
            kind: ErrorKind::Raw,
            class: Some(class),
        }
    }

    /// A file or directory that cannot be read (Python `OSError`).
    pub fn io(s: impl Into<String>) -> Self {
        Self {
            message: s.into(),
            kind: ErrorKind::Io,
            class: None,
        }
    }

    pub fn is_raw(&self) -> bool {
        self.kind == ErrorKind::Raw
    }
}

pub type Result<T> = std::result::Result<T, DecodeError>;
