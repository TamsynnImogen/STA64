//! The sim crate's save container (SIM-81, ADR 0010 V1 to V3), as the client uses it: one place that
//! names what comes from `sta_sim::save`, so the rest of the module does not depend on its layout.

pub use sta_sim::save::store;
pub use sta_sim::save::{
    ClientState, GameKind, InFlight, Loaded, MapId, MapSource, SaveError, SaveInfo, load, read_header, save,
};
