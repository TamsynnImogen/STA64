//! Test bench (owner lane: TOOL; see `docs/TESTBENCH.md`).
//!
//! Bevy-free: scenes, commands and scenarios are plain data here, so the same
//! types serve the in-process runner, `sta64 --scene` and the `sta-bench` CLI.

pub mod marks;
pub mod parse;
pub mod run;
