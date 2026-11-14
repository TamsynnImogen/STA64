//! Repair-yard selection policy, kept separate from gameplay wiring.

pub mod yard;

pub use yard::{YardCandidate, select_a2_yard};
