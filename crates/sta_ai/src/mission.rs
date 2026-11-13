//! Mission-side AI plans (AI-06): a mission script gives an AI seat its plan
//! at run time (`AI_Team_Load_AIP`, `Player:loadAIP`).
//!
//! Rules (read from the Armada II executable, static; the file names from the
//! installed AIPs):
//!
//! - All `AI/AIPs/*.aip` are loaded when the game starts; a request names one
//!   by file name, case-insensitively. An unknown name changes nothing.
//! - A mission seat starts with no plan and its AI does nothing (no building,
//!   no waves) until its first load.
//! - A loaded file is an overlay: keys it sets replace the seat's values,
//!   keys it leaves out keep them, and the build list is replaced only by a
//!   file that has `the_build_list` ([`Params::overlay`]).
//! - It takes effect at once, keeps the seat's units, queues and waves, and
//!   has no effect on a seat that is not AI-controlled (no controller here).
//!
//! Not measured: the engine's default for a value no file set (the skirmish
//! defaults stand in).
//!
//! # Replay
//!
//! The plan reaches the seat through a shared [`MissionAiHub`], not through
//! the command stream; the commands the AI issues are recorded as usual, but
//! the plan is not in a snapshot (WORKPLAN SIM-61 lists it). The script must
//! run before the seats in a tick for a load to apply in the same tick; after
//! them it applies on the next. Both are deterministic.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use sta_content::Aip;
use sta_core::PlayerId;
use sta_sim::SimEvent;
use sta_sim::command::Command;
use sta_sim::setup::{Controller, GameSetup};
use sta_sim::view::WorldView;

use crate::skirmish::{Params, SkirmishConfig, SkirmishController};

/// Every AIP of the install, by upper-cased file name.
#[derive(Debug, Clone, Default)]
pub struct AipLibrary {
    by_name: BTreeMap<String, Arc<Aip>>,
}

fn key(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_uppercase()
}

impl AipLibrary {
    pub fn new() -> AipLibrary {
        AipLibrary::default()
    }

    pub fn insert(&mut self, name: &str, aip: Aip) {
        self.by_name.insert(key(name), Arc::new(aip));
    }

    /// Parse each of `names` (paths as `read` knows them); includes resolve
    /// through `read` by the name as written. Files that cannot be read are
    /// skipped.
    pub fn load(names: &[String], read: &mut dyn FnMut(&str) -> Option<String>) -> AipLibrary {
        let mut lib = AipLibrary::new();
        for n in names.iter().filter(|n| n.to_ascii_lowercase().ends_with(".aip")) {
            if let Some(text) = read(n) {
                let mut first = Some(text);
                let (aip, _) = Aip::load(n, &mut |inc| if inc == n.as_str() { first.take() } else { read(inc) });
                lib.insert(n, aip);
            }
        }
        lib
    }

    pub fn get(&self, name: &str) -> Option<&Arc<Aip>> {
        self.by_name.get(&key(name))
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

#[derive(Debug, Default)]
struct Shared {
    queue: Vec<(PlayerId, String)>,
    /// Requests no seat took, or whose file is not in the library.
    unresolved: Vec<(PlayerId, String)>,
}

/// The library and the queue between a mission script and the seats' AI.
#[derive(Debug, Clone)]
pub struct MissionAiHub {
    library: Arc<AipLibrary>,
    shared: Arc<Mutex<Shared>>,
}

impl MissionAiHub {
    pub fn new(library: AipLibrary) -> MissionAiHub {
        MissionAiHub { library: Arc::new(library), shared: Arc::default() }
    }

    /// Queue plan loads in call order (what the script asked for).
    pub fn submit(&self, requests: impl IntoIterator<Item = (PlayerId, String)>) {
        if let Ok(mut s) = self.shared.lock() {
            s.queue.extend(requests);
        }
    }

    /// Requests that changed nothing (unknown file), oldest first.
    pub fn unresolved(&self) -> Vec<(PlayerId, String)> {
        self.shared.lock().map(|s| s.unresolved.clone()).unwrap_or_default()
    }

    /// The AI of `seat`: passive until its first plan arrives. A seat not in
    /// the setup gets nothing to run (an idle controller).
    pub fn controller(&self, setup: &GameSetup, seat: PlayerId) -> Box<dyn Controller> {
        match SkirmishConfig::from_setup(setup, seat) {
            Some(cfg) => Box::new(ScriptedAi { seat, hub: self.clone(), params: cfg.params.clone(), ai: SkirmishController::new(cfg), loaded: false }),
            None => Box::new(crate::IdleController),
        }
    }
}

/// An AI seat of a mission.
#[derive(Debug)]
pub struct ScriptedAi {
    seat: PlayerId,
    hub: MissionAiHub,
    params: Params,
    ai: SkirmishController,
    loaded: bool,
}

impl ScriptedAi {
    /// Whether a plan has been loaded (the seat acts only then).
    pub fn has_plan(&self) -> bool {
        self.loaded
    }

    /// The tuning in force.
    pub fn params(&self) -> &Params {
        &self.params
    }

    fn take_requests(&mut self) {
        let Ok(mut s) = self.hub.shared.lock() else { return };
        let mut keep = Vec::new();
        for (p, name) in std::mem::take(&mut s.queue) {
            if p != self.seat {
                keep.push((p, name));
                continue;
            }
            match self.hub.library.get(&name) {
                Some(aip) => {
                    self.params.overlay(aip);
                    self.ai.set_params(self.params.clone());
                    self.loaded = true;
                }
                None => s.unresolved.push((p, name)),
            }
        }
        s.queue = keep;
    }
}

impl Controller for ScriptedAi {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>) {
        self.take_requests();
        if self.loaded {
            self.ai.think(view, events, out);
        }
    }
}

#[cfg(test)]
mod tests;
