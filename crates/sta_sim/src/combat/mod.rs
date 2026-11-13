//! Combat: weapons (SIM-04), damage (SIM-05), point defence (SIM-06).

pub mod damage;
pub mod pd;
pub mod weapons;

#[cfg(test)]
mod crew_loss_tests;
#[cfg(test)]
mod damage_tests;
#[cfg(test)]
pub(crate) mod subsystem_tests;
#[cfg(test)]
pub(crate) mod pd_tests;
#[cfg(test)]
mod weapons_tests;
#[cfg(test)]
pub(crate) mod engine_timing_tests;
#[cfg(test)]
mod first_shot_tests;
