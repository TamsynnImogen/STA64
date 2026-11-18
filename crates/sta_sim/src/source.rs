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

/// Steps a world, one tick per call.
pub trait WorldSource: Send {
    /// Advance one tick, applying `commands` (sorted by `(player, seq)`).
    fn step(&mut self, commands: &[Command]) -> TickOutput;
    /// The world now: after the last step, or at tick 0 before any.
    fn view(&self) -> Arc<WorldView>;
    /// Optional immutable queue/site metadata for presentation. Sources without
    /// production state (including legacy view logs) return no rows. Command
    /// replays and snapshot restores derive it from their live `Sim`.
    fn production_view(&self) -> Vec<crate::production::view::ProductionView> {
        Vec::new()
    }
    /// Optional immutable cargo metadata for presentation. Stateless sources
    /// return no rows; live simulations derive it from their economy records.
    fn cargo_view(&self) -> Vec<crate::economy::view::CargoView> {
        Vec::new()
    }
    /// Optional resource-node amounts for presentation (UI-48). Stateless sources return no rows.
    fn node_view(&self) -> Vec<crate::economy::view::NodeView> {
        Vec::new()
    }
    /// Optional system states for presentation (UI-48). Stateless sources return no rows.
    fn system_view(&self) -> Vec<crate::combat::damage::SystemView> {
        Vec::new()
    }
    /// The whole world as snapshot bytes ([`Sim::snapshot`]), for sources that
    /// can: the playtest capture (TOOL-14) reads it, nothing in the sim does.
    /// The fakes have no state to save and answer `None`.
    ///
    /// [`Sim::snapshot`]: crate::Sim::snapshot
    fn snapshot(&self) -> Option<Vec<u8>> {
        None
    }
}
