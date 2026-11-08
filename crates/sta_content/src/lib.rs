//! The content seam (docs/WORKPLAN.md, S5 and S6): everything the simulation
//! knows about units, weapons and maps, as typed, fixed-point definitions.
//!
//! - Every number reaches the sim as [`Fixed`], converted once here
//!   (`Fixed::from_f64` of the parsed value). No other crate parses ODF
//!   numbers for the sim.
//! - A [`ContentDb`] is immutable once built and shared as `Arc`. Its
//!   [`ContentKey`]s index names sorted bytewise, so they agree on every peer
//!   that loaded the same content; [`ContentDb::hash`] says whether it did.
//! - Profile defaults come first, then per-ODF keys switch features per unit.
//!
//! The real loaders (ODF: CON-03, index: CON-02, BZN maps: CON-07) are not
//! written yet and answer [`ContentError::Unsupported`]; [`ContentDb::builder`]
//! and [`fake`] work today.

pub mod db;
pub mod def;
pub mod fake;
pub mod features;
pub mod index;
pub mod map;

pub use db::{ContentBuilder, ContentDb};
pub use def::{BuildDef, ClassLabel, DefRef, FireArc, FxRefs, Hardpoint, MoveStats, ShieldDef, SoundRefs, UnitDef, WeaponDef, WeaponKind};
pub use features::{Features, Profile};
pub use index::{OdfEntry, OdfIndex};
pub use map::{MapDef, MapObject, Region, RegionKind, RegionShape, Scenery, SceneryLight, SceneryPlanet, StartPos};
pub use sta_core::{ContentKey, Fixed, Vec3};

/// Why content could not be loaded or built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContentError {
    /// A loader that is planned but not written yet.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// Two definitions share a name (names compare ASCII case-insensitively,
    /// as ODF names do).
    #[error("duplicate definition: {0}")]
    Duplicate(String),
    /// A definition names another that does not exist.
    #[error("{from} refers to {to}, which is not defined")]
    Dangling { from: String, to: String },
}

/// Content loading's result.
pub type Result<T, E = ContentError> = std::result::Result<T, E>;

/// Something a loader skipped or guessed, reported instead of failing the
/// whole load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// The file or definition concerned; empty when none applies.
    pub file: String,
    pub message: String,
}

impl Warning {
    pub fn new(file: impl Into<String>, message: impl Into<String>) -> Warning {
        Warning { file: file.into(), message: message.into() }
    }

    /// A planned loader that is not written yet.
    pub fn unsupported(what: &str) -> Warning {
        Warning::new("", format!("unsupported: {what}"))
    }
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.file.is_empty() { f.write_str(&self.message) } else { write!(f, "{}: {}", self.file, self.message) }
    }
}
