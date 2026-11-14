//! Repair policies, kept separate from gameplay wiring.

pub mod dock;
pub mod yard;

pub use dock::{DockBounds, a2_dock_offset};
pub use yard::{YardCandidate, select_a2_yard};
