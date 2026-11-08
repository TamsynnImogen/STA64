//! Anything that turns commands into world views (S3): the real [`Sim`],
//! and the fakes [`ScriptedSource`] and [`ViewLog`]. The client's sim link,
//! the bench and net step a `dyn WorldSource` and never care which.
//!
//! [`Sim`]: crate::Sim
//! [`ScriptedSource`]: crate::fake::ScriptedSource
//! [`ViewLog`]: crate::fake::ViewLog

use std::sync::Arc;

use crate::command::Command;
use crate::view::{TickOutput, WorldView};

pub trait WorldSource: Send {
    /// Advance one tick, applying `commands` (sorted by `(player, seq)`).
    fn step(&mut self, commands: &[Command]) -> TickOutput;
    /// The world now: after the last step, or at tick 0 before any.
    fn view(&self) -> Arc<WorldView>;
}
