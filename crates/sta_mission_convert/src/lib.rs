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
//! This is the image layer (step LUA-15a): the PE reader, the engine `.map` reader and the
//! engine vtable names. Decoding and lifting follow in later steps.

pub mod disasm;
#[doc(hidden)]
pub mod fixture;
pub mod image;
pub mod map;
pub mod pe;

pub use image::{FlatImage, Image, Section};
pub use pe::Pe;

/// A DLL (or a function in it) the converter could not decode; the message says why.
///
/// The Python tools raise `DecodeError` for faults about the file's content, which their callers
/// catch (a function is written as not decoded, the run goes on), and a raw `ValueError`,
/// `struct.error`, `TypeError` or `StopIteration` for a truncated or malformed structure, which
/// aborts the run (A2) or fails the mission (A1). Both are a `DecodeError` here; `raw` tells them
/// apart, so a caller ports the Python `except DecodeError` as "catch it unless `raw`".
///
/// STA64's own limits on crafted tables (see [`pe`]) are raw faults too.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DecodeError {
    pub message: String,
    /// The Python tool raises something other than `DecodeError` here.
    pub raw: bool,
}

impl DecodeError {
    /// A fault the Python tools raise as `DecodeError`.
    pub fn new(s: impl Into<String>) -> Self {
        Self {
            message: s.into(),
            raw: false,
        }
    }

    /// A fault the Python tools raise as a raw exception.
    pub fn raw(s: impl Into<String>) -> Self {
        Self {
            message: s.into(),
            raw: true,
        }
    }
}

pub type Result<T> = std::result::Result<T, DecodeError>;
