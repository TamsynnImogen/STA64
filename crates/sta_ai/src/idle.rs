//! The controller that does nothing.

use sta_sim::SimEvent;
use sta_sim::command::Command;
use sta_sim::setup::Controller;
use sta_sim::view::WorldView;

/// Owns a seat and never sends a command: the opponent for the first
/// skirmish milestone.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IdleController;

impl Controller for IdleController {
    fn think(&mut self, _view: &WorldView, _events: &[SimEvent], _out: &mut Vec<Command>) {}
}
