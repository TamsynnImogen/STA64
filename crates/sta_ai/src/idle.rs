//! The controller that does nothing.

use sta_sim::SimEvent;
use sta_sim::command::Command;
use sta_sim::setup::{Controller, ControllerState};
use sta_sim::view::WorldView;

/// Owns a seat and never sends a command: the opponent for the first
/// skirmish milestone.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IdleController;

impl Controller for IdleController {
    fn think(&mut self, _view: &WorldView, _events: &[SimEvent], _out: &mut Vec<Command>) {}

    /// Stateless: an empty record of version 1 (ADR 0010 V5).
    fn save_state(&self) -> Option<ControllerState> {
        Some(ControllerState { version: IDLE_STATE_VERSION, data: Vec::new() })
    }

    fn restore_state(&mut self, state: &ControllerState) -> Result<(), String> {
        if state.version == IDLE_STATE_VERSION && state.data.is_empty() {
            Ok(())
        } else {
            Err(format!("idle controller record version {} is not readable", state.version))
        }
    }
}

/// Record version of [`IdleController`]'s (empty) state.
const IDLE_STATE_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_saves_and_restores_an_empty_record() {
        let mut c = IdleController;
        let st = c.save_state().expect("idle opts in");
        assert_eq!(st, ControllerState { version: 1, data: vec![] });
        assert!(c.restore_state(&st).is_ok());
        assert!(c.restore_state(&ControllerState { version: 2, data: vec![] }).is_err());
        assert!(c.restore_state(&ControllerState { version: 1, data: vec![0] }).is_err());
    }
}
