//! Parsers for the bench's three text formats: scene `.ini` files, the command
//! vocabulary and plain-text scenarios (`docs/TESTBENCH.md`).
//!
//! Everything here is pure data. Names stay strings and numbers stay `f64` or
//! tick counts; mapping them onto `sta_sim` (content keys, entity ids, fixed
//! point, `Command`/`Edit`) is the runner's job (TOOL-02, TOOL-03), so these
//! types need no change when the sim seam moves.

mod error;
mod value;

pub mod command;
pub mod scenario;
pub mod scene;

pub use command::{BenchCommand, TimedCommand, parse_command, parse_command_list, parse_script};
pub use error::ParseError;
pub use scenario::{Scenario, Step, StepKind};
pub use scene::SceneFile;
pub use value::{System, Target, TimeSpec};
