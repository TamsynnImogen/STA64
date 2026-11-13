//! Legacy mission conversion in Rust (LUA-15): the player's own Armada II `.dsl`/`.drl` and
//! Armada I mission DLLs become Lua on their machine. A port of `tools/dsl_to_lua.py`,
//! `drl_to_lua.py`, `a1_to_lua.py` and `convert_install_missions.py`, which stay as the
//! reference until the install-gated parity test is green for every mission.
//!
//! The DLLs are only ever read as bytes (D2): [`pe`] parses the file, [`disasm`] decodes the
//! x86 text the lifters read, nothing is loaded or executed. Nothing converted is committed
//! or shipped (the output goes to the user data directory).

pub mod disasm;
pub mod pe;

/// A DLL (or a function in it) the converter could not decode; the message says why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DecodeError(pub String);

impl DecodeError {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

pub type Result<T> = std::result::Result<T, DecodeError>;
