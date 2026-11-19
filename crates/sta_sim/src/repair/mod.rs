//! Repair policies, kept separate from gameplay wiring.

pub mod dock;
pub mod service;
pub mod ship;
pub mod time;
pub mod yard;

pub use dock::{DockBounds, a2_dock_offset};
pub use service::{Repairs, RepairsRec};
pub use time::{RepairTimeFactors, a2_modified_system_time};
pub use yard::{YardCandidate, select_a2_yard};

#[cfg(test)]
mod service_tests;
#[cfg(test)]
mod ship_tests;
