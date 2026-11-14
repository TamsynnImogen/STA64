//! Computer players: `sta_sim::setup::Controller`s that read world views and
//! send commands through the router like a player.
//!
//! Owner: the AI lane (docs/WORKPLAN.md): AI-01 (`IdleController`, the
//! registry by name), AI-02 (skirmish AI), AI-03 (soak tests).

mod idle;
pub mod mission;
pub mod skirmish;

use std::collections::BTreeMap;

pub use idle::IdleController;
pub use mission::{AiRequest, AipLibrary, MissionAiHub, ScriptedAi};
use sta_core::PlayerId;
pub use skirmish::{SKIRMISH, SkirmishConfig, SkirmishController};
use sta_sim::setup::{Controller, GameSetup};

/// Name of [`IdleController`] in the registry.
pub const IDLE: &str = "idle";

/// Builds a fresh controller for one seat.
pub type Constructor = fn() -> Box<dyn Controller>;

/// Builds a controller that needs to know its seat, the map and the content:
/// the game setup and the seat it is for.
pub type SeatConstructor = fn(&GameSetup, PlayerId) -> Box<dyn Controller>;

/// No controller is registered under the name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown AI controller {0:?}")]
pub struct UnknownController(pub String);

/// Controllers by name, as in `Control::Ai(name)`. Sorted, so listings are
/// deterministic.
#[derive(Debug, Clone)]
pub struct Registry {
    map: BTreeMap<&'static str, Constructor>,
    seated: BTreeMap<&'static str, SeatConstructor>,
}

impl Registry {
    /// A registry with no controllers.
    pub fn empty() -> Registry {
        Registry { map: BTreeMap::new(), seated: BTreeMap::new() }
    }

    /// The built-in controllers (`"idle"`, `"skirmish"`); later packages add
    /// theirs here.
    pub fn builtin() -> Registry {
        let mut r = Registry::empty();
        r.register(IDLE, || Box::new(IdleController));
        r.register_seat(SKIRMISH, SkirmishController::for_seat);
        r
    }

    /// Add or replace a controller.
    pub fn register(&mut self, name: &'static str, ctor: Constructor) {
        self.seated.remove(name);
        self.map.insert(name, ctor);
    }

    /// Add or replace a controller that is built for one seat of a game.
    pub fn register_seat(&mut self, name: &'static str, ctor: SeatConstructor) {
        self.map.remove(name);
        self.seated.insert(name, ctor);
    }

    /// A new controller for `name`, or an error for an unknown name or one
    /// that needs a seat ([`Registry::create_for_seat`]).
    pub fn create(&self, name: &str) -> Result<Box<dyn Controller>, UnknownController> {
        self.map.get(name).map(|c| c()).ok_or_else(|| UnknownController(name.to_owned()))
    }

    /// The controller of `Control::Ai(name)` for `player` in `setup`: what a
    /// game mode calls for each AI seat. Controllers that need no seat are
    /// built as by [`Registry::create`].
    pub fn create_for_seat(&self, name: &str, setup: &GameSetup, player: PlayerId) -> Result<Box<dyn Controller>, UnknownController> {
        match self.seated.get(name) {
            Some(ctor) => Ok(ctor(setup, player)),
            None => self.create(name),
        }
    }

    /// Registered names, ascending.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<_> = self.map.keys().chain(self.seated.keys()).copied().collect();
        names.sort_unstable();
        names
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::builtin()
    }
}

/// Shorthand for `Registry::builtin().create(name)`.
pub fn create(name: &str) -> Result<Box<dyn Controller>, UnknownController> {
    Registry::builtin().create(name)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_core::{PlayerId, TICK_HZ, TeamId};
    use sta_sim::command::Command;
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{Sim, WorldSource};

    use super::*;

    #[test]
    fn builtin_lists_idle() {
        assert_eq!(Registry::builtin().names(), ["idle", "skirmish"]);
        assert!(create("idle").is_ok());
    }

    #[test]
    fn unknown_name_errors() {
        let err = create("nope").err().unwrap();
        assert_eq!(err, UnknownController("nope".into()));
        assert!(err.to_string().contains("nope"));
        assert!(Registry::empty().create("idle").is_err());
    }

    #[test]
    fn names_are_sorted_and_register_replaces() {
        let mut r = Registry::builtin();
        r.register("alpha", || Box::new(IdleController));
        r.register("idle", || Box::new(IdleController));
        assert_eq!(r.names(), ["alpha", "idle", "skirmish"]);
    }

    #[test]
    fn idle_seat_issues_no_commands_for_ten_minutes() {
        let setup = GameSetup {
            seed: 1,
            map: Arc::new(sta_content::fake::two_bases()),
            content: Arc::new(sta_content::fake::db()),
            players: vec![
                PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: Control::Human },
                PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Ai("idle".into()) },
            ],
            options: GameOptions::default(),
        };
        let mut ctl = create("idle").unwrap();
        let mut sim = Sim::from_setup(&setup);
        let mut issued = 0usize;
        let mut out: Vec<Command> = Vec::new();
        for _ in 0..(10 * 60 * TICK_HZ) {
            out.clear();
            let view = sim.view();
            ctl.think(&view, &[], &mut out);
            issued += out.len();
            sim.step(&out);
        }
        assert_eq!(sim.view().tick, 18000);
        assert_eq!(issued, 0);
    }
}
