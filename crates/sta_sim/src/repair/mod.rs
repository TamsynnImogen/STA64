//! Repair policies, kept separate from gameplay wiring.

pub mod dock;
pub mod time;
pub mod yard;

pub use dock::{DockBounds, a2_dock_offset};
pub use time::{RepairTimeFactors, a2_modified_system_time};
pub use yard::{YardCandidate, select_a2_yard};
