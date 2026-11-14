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
//! - It takes effect on the seat's next think (the AI plans every half
//!   second, so its behaviour changes at its next plan), keeps the seat's units, queues and waves, and
//!   has no effect on a seat that is not AI-controlled (no controller here).
//!
//! Not measured: the engine's default for a value no file set (the skirmish
//! defaults stand in).
//!
//! # The AI switches (LUA-20)
//!
//! `DisableAllAITeams`, `EnableAllAITeams`, `EnableTeam`, `DisableTeam`,
//! `SetIsAITeam` and `ResetStrategyCycle` reach the hub in one ordered request
//! list with the plan loads ([`AiRequest`]), so a load and a switch of the
//! same tick keep their order. Per seat the hub keeps two flags, as the
//! original's AI team does (read from the executables, static; nothing
//! measured on the bench): `ai` (true for every seat with a controller here,
//! false for the human's, which has none) and `enabled` (false at start; a
//! plan load that resolves sets it true, in both games). Applied in order:
//!
//! - `Enable` / `Disable`: only on an `ai` seat; elsewhere nothing.
//! - `EnableAll` / `DisableAll`: every `ai` seat; the human seat is untouched.
//! - `SetAi`: the original forces the flag back to true on the next tick for
//!   every non-human seat, so on a seat here it changes nothing at all (the hub does not even
//!   record it). The original is not quite as inert: a `SetIsAITeam(computer, false)` followed
//!   in the same tick by `EnableTeam` / `DisableTeam` would see the flag false and have them
//!   ignored until the next tick forces it back; here they apply. On the human
//!   seat it would hand the player's units to the AI; that is not supported
//!   (the script binding counts `SetIsAITeam(human seat, true)` as
//!   unsupported; no stock mission does it).
//! - `Replan` (`ResetStrategyCycle`): timing only. Each seat plans on its next
//!   think instead of waiting for the next period tick. The flag is used up by that think
//!   whatever the seat is: a seat with no plan or a disabled one drops it, and a later load or
//!   `Enable` does not bring it back. That matches the original, where the call only restarts
//!   the cycle counter and a team that does not plan has nothing to replan (read from the
//!   executable, static). Deviation of the AI, not of the call: the original staggers the
//!   replans (team `t` replans on the `t`-th tick after the call, then every
//!   `recompute_strategy_period` ticks of the AIP); here every seat replans in the same think.
//!
//! What a loaded seat does: enabled, as before; disabled, only the unit-level
//! part (point defence, [`crate::skirmish::Think::UnitsOnly`]) and it still
//! reads events (the original keeps its strategic maps current while
//! disabled). It stops building, planning goals, forming and sending waves and
//! assigning freighters. Orders already given stay with the sim: freighters
//! keep mining, yards finish what is queued (the latter not established for
//! the original).
//!
//! **Known deviation (read from the executable, static; not measured in
//! game):** the original's disabled team still answers attacks through
//! distress calls (they check the AI flag, not the enabled flag) and keeps
//! already formed fleets running. Here `military` mixes waves and defence, so
//! a disabled seat drops both. Splitting a defend-only and wave-upkeep subset
//! out of it is a follow-up once the distress rule (range, selection) is read.
//!
//! The mission end-blocks that LUA-20 cleared (`api/taint.rs`: a2_borg01, a2_borg10, a2_fed05)
//! rest on this deviation not mattering for the end rule: the end queries (`IsCompromised`,
//! `CountStations`) read nothing from the AI, so the original's distress responses and
//! running waves matter only if they change what those queries count. a2_borg01 and a2_borg10
//! disable all AI teams near their ends, which is where it would show. Not measured.
//!
//! # Replay and where the requests are applied
//!
//! The plan and the flags reach the seat through a shared [`MissionAiHub`],
//! not through the command stream; the commands the AI issues are recorded as
//! usual. [`MissionAiHub::submit`] applies the requests at once, in order, and
//! both hosts call it once per tick right after the script has thought and
//! before any seat thinks (`ActiveMission::tick`, `mission_host::Shared::think`),
//! so every seat sees the same flags and the plans of the script's own tick in
//! that tick's think, whatever the seat order. A request submitted after the
//! seats have thought reaches them on the next tick. Both are deterministic for what the
//! script does inside its own `think`: the same script and world give the same requests at
//! the same ticks and the same commands.
//!
//! **Requests raised outside `think` (live client only):** script calls the host makes
//! itself (`deliver_reply_host_side` for a movie, message or cineractive answer, a host
//! `eval`) queue their requests in the runtime like any other, but nothing takes them until
//! the next tick's `take_ai_requests`. They are therefore applied on that tick *before* the
//! requests of that tick's own script `think`, and which tick that is depends on the frame
//! the client handled the player's input in, not on the sim tick, so it is not reproducible
//! from the game alone (the same gap as the replies themselves, WORKPLAN SIM-61). A seat with no plan yet sees no events, so it starts with no
//! memory of the past. Files are found by name only: two files of one name in
//! different folders collapse to the later one in listing order.
//!
//! **Save/restore gap (read from the executable, static):** the original saves
//! the two flags and the strategy cycle with its saved game. Here they are host
//! state beside the plan: not in a sim snapshot, not in the command stream, and
//! there is no mid-mission save yet (WORKPLAN SIM-61 names the plan). No format
//! number: replays record the AI's commands and never run an AI, and network
//! peers never run the AI or the script (D3). A future mission save must carry
//! per seat `ai`, `enabled` and the plan; to be decided with SIM-61.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use sta_content::Aip;
use sta_core::PlayerId;
use sta_sim::SimEvent;
use sta_sim::command::Command;
use sta_sim::setup::{Controller, GameSetup};
use sta_sim::view::WorldView;

use crate::skirmish::{Params, SkirmishConfig, SkirmishController, Think};

/// First sequence number of a seat's AI commands.
const AI_SEQ_BASE: u32 = 0x4000_0000;

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
        let dir = |n: &str| n.rfind(['/', '\\']).map_or(0, |i| i + 1);
        for n in names.iter().filter(|n| n.to_ascii_lowercase().ends_with(".aip")) {
            if let Some(text) = read(n) {
                let mut first = Some(text);
                let mut load = |inc: &str| {
                    if inc == n.as_str() {
                        return first.take();
                    }
                    // Includes are named by file name: the including file's
                    // folder first, then any file of that name.
                    let want = key(inc);
                    let same = names.iter().find(|c| key(c) == want && c[..dir(c)] == n[..dir(n)]);
                    let path = same.or_else(|| names.iter().find(|c| key(c) == want))?;
                    read(path)
                };
                let (aip, _) = Aip::load(n, &mut load);
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

/// What a mission script asked of the AI, in call order (`take_ai_requests` of the runtime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiRequest {
    /// `AI_Team_Load_AIP`, `Player:loadAIP`: overlay the file on the seat's plan and enable it.
    Load(PlayerId, String),
    /// `EnableTeam`.
    Enable(PlayerId),
    /// `DisableTeam`.
    Disable(PlayerId),
    /// `EnableAllAITeams`.
    EnableAll,
    /// `DisableAllAITeams`.
    DisableAll,
    /// `SetIsAITeam`.
    SetAi(PlayerId, bool),
    /// `ResetStrategyCycle`.
    Replan,
}

/// The two flags of a seat's AI team (module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Flags {
    ai: bool,
    enabled: bool,
}

/// What the hub holds for one seat between a request and the seat's next think.
#[derive(Debug, Default)]
struct Seat {
    flags: Option<Flags>,
    /// Plans loaded since the seat last thought, in order.
    plans: Vec<Arc<Aip>>,
    replan: bool,
}

#[derive(Debug, Default)]
struct Shared {
    /// Requests that changed nothing: no AI seat of that id, or a file that is
    /// not in the library. In the order they were found.
    unresolved: Vec<(PlayerId, String)>,
    /// Seats that have an AI controller.
    seats: BTreeMap<PlayerId, Seat>,
}

impl Shared {
    fn flags(&mut self, p: PlayerId) -> Option<&mut Flags> {
        self.seats.get_mut(&p).and_then(|s| s.flags.as_mut())
    }
}

fn lock(s: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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

    /// Apply plan loads in call order (see [`MissionAiHub::submit_requests`]).
    pub fn submit(&self, requests: impl IntoIterator<Item = (PlayerId, String)>) {
        self.submit_requests(requests.into_iter().map(|(p, n)| AiRequest::Load(p, n)));
    }

    /// Apply what the script asked for, in call order. This is the one point where requests
    /// take effect (module docs): the hosts call it after the script thought and before the
    /// seats do.
    pub fn submit_requests(&self, requests: impl IntoIterator<Item = AiRequest>) {
        let mut s = lock(&self.shared);
        for r in requests {
            match r {
                AiRequest::Load(p, name) => {
                    let aip = if s.seats.contains_key(&p) { self.library.get(&name).cloned() } else { None };
                    match aip {
                        Some(aip) => {
                            let seat = s.seats.entry(p).or_default();
                            seat.plans.push(aip);
                            if let Some(f) = seat.flags.as_mut() {
                                f.enabled = true;
                            }
                        }
                        None => s.unresolved.push((p, name)),
                    }
                }
                AiRequest::Enable(p) | AiRequest::Disable(p) => {
                    if let Some(f) = s.flags(p).filter(|f| f.ai) {
                        f.enabled = matches!(r, AiRequest::Enable(_));
                    }
                }
                AiRequest::EnableAll | AiRequest::DisableAll => {
                    for f in s.seats.values_mut().filter_map(|seat| seat.flags.as_mut()).filter(|f| f.ai) {
                        f.enabled = r == AiRequest::EnableAll;
                    }
                }
                // The original forces the flag back for every non-human seat on its next tick.
                AiRequest::SetAi(..) => {}
                AiRequest::Replan => {
                    for seat in s.seats.values_mut() {
                        seat.replan = true;
                    }
                }
            }
        }
    }

    /// Whether `seat` has an AI team here (`AI::IsAITeam`).
    pub fn is_ai(&self, seat: PlayerId) -> bool {
        lock(&self.shared).seats.get(&seat).and_then(|s| s.flags).is_some_and(|f| f.ai)
    }

    /// Whether the seat's strategic AI is switched on (`AI::IsEnabled`: `ai` and `enabled`).
    pub fn is_enabled(&self, seat: PlayerId) -> bool {
        lock(&self.shared).seats.get(&seat).and_then(|s| s.flags).is_some_and(|f| f.ai && f.enabled)
    }

    /// Requests that changed nothing: a seat without AI (the human's,
    /// neutral, unknown) or a file not in the library.
    pub fn unresolved(&self) -> Vec<(PlayerId, String)> {
        lock(&self.shared).unresolved.clone()
    }

    /// The AI of `seat`: passive until its first plan arrives. A seat not in
    /// the setup gets nothing to run (an idle controller).
    pub fn controller(&self, setup: &GameSetup, seat: PlayerId) -> Box<dyn Controller> {
        match SkirmishConfig::from_setup(setup, seat) {
            Some(cfg) => {
                let mut s = lock(&self.shared);
                s.seats.entry(seat).or_default().flags = Some(Flags { ai: true, enabled: false });
                let mut ai = SkirmishController::new(cfg.clone());
                // Its sequence numbers stay clear of the script's, which counts
                // up from 1 for the same player, so a rejection is never
                // credited to the wrong sender.
                ai.set_seq_base(AI_SEQ_BASE);
                Box::new(ScriptedAi { seat, hub: self.clone(), params: cfg.params, ai, loaded: false })
            }
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

    /// Take what the hub holds for the seat: plans (overlaid in order), the replan flag, and the
    /// enabled flag as it stands.
    fn take_requests(&mut self) -> (bool, bool) {
        let mut s = lock(&self.hub.shared);
        let Some(seat) = s.seats.get_mut(&self.seat) else { return (false, false) };
        let plans = std::mem::take(&mut seat.plans);
        let replan = std::mem::take(&mut seat.replan);
        let enabled = seat.flags.is_some_and(|f| f.enabled);
        drop(s);
        if !plans.is_empty() {
            for aip in &plans {
                self.params.overlay(aip);
            }
            self.ai.set_params(self.params.clone());
            self.loaded = true;
        }
        (enabled, replan)
    }
}

impl Controller for ScriptedAi {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>) {
        let (enabled, replan) = self.take_requests();
        if !self.loaded {
            return;
        }
        let mode = match (enabled, replan) {
            (false, _) => Think::UnitsOnly,
            (true, true) => Think::FullNow,
            (true, false) => Think::Full,
        };
        self.ai.think_as(view, events, out, mode);
    }
}

#[cfg(test)]
mod tests;
