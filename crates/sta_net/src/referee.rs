//! The referee (NET-05, NET_DECISIONS N5): a headless connection without a
//! seat that runs the whole game from `Start` and every seat's turn records
//! on its own content, compares every seat's reported digest with its own,
//! and says who is right. docs/NET.md, "NET-05 as built", has the flow.
//!
//! # What it is
//!
//! - **A peer without a seat.** It passes the handshake of a joiner (`admit`:
//!   build and rules digest; `Message::RefereeHello`), is named to the room
//!   by the host (`Message::RefereeSeated`), receives every turn record and
//!   sends none. It runs `sta_sim` and no AI and no mission (those stay on the
//!   host, D3). The relay still reads frame headers only.
//! - **Its digest is the truth.** It sends its own digest for every turn
//!   (`Message::Reference`); a peer compares it as it compares a seat's, and
//!   a peer that differs from it asks the referee, not the host, for the
//!   state (`Peer::diverged_from_authority`). The referee serves the same
//!   bounded snapshot transfer the host serves under N3 (`resync.rs`) and
//!   tells everyone with `Resynced`.
//! - **It names the culprit.** For every seat whose digest differs from its
//!   own it records the first tick, and, from the section digests the seat
//!   (and the referee) send once after a mismatch, the sections that differ
//!   ([`Finding`]). The artefacts it writes ([`Referee::write_report`]) are
//!   its replay and a sidecar in the format `Peer::desync_sidecar` writes, so
//!   `sta-bench desync` takes them like any peer's.
//!
//! # Trust
//!
//! Nobody is believed on their word alone.
//!
//! - A seat's digest is checked against the referee's own re-simulation, not
//!   against another seat's. A seat that reports a digest its section digests
//!   do not explain is named as such ([`FindingKind::MisreportedDigest`]).
//! - A seat's `Desync` claim is checked against the digests the referee
//!   holds: when every seat agrees with the referee at the claimed tick the
//!   stop is for nothing and the referee releases the peers it stopped; when
//!   the digest the claim gives as the claimer's own is not the one the
//!   claimer sent in its records, the claim is counted against that seat
//!   ([`Referee::false_claims`]).
//! - The referee is not believed either: a peer that finds every seat agreeing
//!   with it and the referee alone different ignores the referee from then on
//!   ([`judge`]), so one faulty or hostile referee cannot stop a match or
//!   hand out a state that no player holds.
//! - The host is not believed about who the referee is beyond what a host may
//!   do anyway: `RefereeSeated` is a host message, and the slot must be none of
//!   the game's seats.
//!
//! # Bounds
//!
//! Per seat the router queues at most `MAX_QUEUED_TURNS` records (a seat
//! beyond that is faulted, as at every peer), per room it keeps at most 64
//! findings, one pending claim per seat, 64 turns of digests and section
//! digests per seat, and an event list capped at `MAX_EVENTS`. What grows with
//! the match is its replay: [`RefereeLimits`] ends the referee (writing its
//! reports) at a command count, a tick count, or after a stretch of no
//! progress.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, Tick};
use sta_sim::digest::SectionDigests;

use crate::lockstep::{
    admit, Destination, DesyncFiles, HandshakeError, Incoming, Peer, RouterEvent, StepOutcome,
    Transport,
};
use crate::proto::{DesyncNotice, EngineId, Hello, Message, ProtoError, Refusal, Resynced, RulesIdentity, Start, Welcome};

// ---------------------------------------------------------------------------
// The two decisions the design leaves to the code: one function each
// ---------------------------------------------------------------------------

/// Who is odd when a referee's digest differs from a peer's, given every
/// seat's digest at that tick (NET_DECISIONS N5: the referee's digest is the
/// truth; the exception below is this build's, listed in NET.md).
///
/// - [`Verdict::ReferenceStands`]: the referee is the truth; a seat that
///   differs from it is resynced from it.
/// - [`Verdict::RefereeOutvoted`]: every seat reports one and the same digest
///   and the referee another. All players already play the same game, so
///   resyncing all of them to one outlier makes no sense: the referee is the
///   one that is wrong (a faulty or hostile referee, or a bug in its build),
///   and peers stop believing it.
///
/// Open question answered here (one place to change): with no seat agreeing
/// with the referee, is it believed? Yes, unless the seats agree among
/// themselves.
pub fn judge(seats: &[u64], reference: u64) -> Verdict {
    match seats.split_first() {
        Some((first, rest)) if *first != reference && rest.iter().all(|d| d == first) => {
            Verdict::RefereeOutvoted
        }
        _ => Verdict::ReferenceStands,
    }
}

/// See [`judge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    ReferenceStands,
    RefereeOutvoted,
}

/// What the referee decides beyond the design's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefereePolicy {
    /// Whether the referee resyncs the host when the host is the one that
    /// differs (N5: "allowed only if the host's controllers (AI, mission Lua)
    /// can carry on across a reload of the sim"). `false`: the match ends
    /// with the reports (the host's request is refused with
    /// `SnapshotRefusal::HostEndsMatch`).
    ///
    /// Default `false`: the skirmish AI carries on across a reload (proved in
    /// `crates/sta_ai/tests/referee_host_resync.rs`), but mission Lua is not
    /// proven. Flip the default (a manual `Default` impl returning `true`) when it is.
    pub resync_host: bool,
}

/// [`RefereePolicy::resync_host`], the one place the host-divergence
/// question is decided.
pub fn resync_host(policy: &RefereePolicy) -> bool {
    policy.resync_host
}

// ---------------------------------------------------------------------------
// Joining
// ---------------------------------------------------------------------------

/// The host's side of a referee joining: the same checks as a joiner's
/// (engine, rules digest with the differing key named). A referee with
/// another rules digest is refused here, before a game starts.
pub fn admit_referee(
    host_engine: &EngineId,
    host_rules: &dyn RulesIdentity,
    hello: &Hello,
    referee_rules: &dyn RulesIdentity,
) -> Result<(), HandshakeError> {
    admit(host_engine, host_rules, hello, referee_rules)
}

/// The host's answer to an admitted referee: `Welcome` carrying the slot of
/// its connection (no seat of the game). Send `Message::Start` to it with the
/// others', then call `Peer::seat_referee` on the host's peer.
pub fn welcome(slot: PlayerId, host: PlayerId) -> Message {
    Message::Welcome(Welcome {
        slot,
        host,
        rejoin_token: 0,
    })
}

/// Why a referee did not get into a game.
#[derive(Debug, PartialEq)]
pub enum JoinError {
    /// The host said no.
    Refused(Refusal),
    /// No `Start` in time.
    Timeout,
    /// The transport failed.
    Transport(String),
    /// The host sent a `Start` that is not for a referee.
    BadStart(ProtoError),
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::Refused(r) => write!(f, "the host refused the referee: {r:?}"),
            JoinError::Timeout => write!(f, "no start from the host in time"),
            JoinError::Transport(e) => write!(f, "{e}"),
            JoinError::BadStart(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for JoinError {}

/// Sends `RefereeHello` to `host` and waits for `Start`, polling
/// `transport` (the caller owns the clock: `wait`). Messages from anyone but
/// the host are ignored.
pub fn await_start(
    transport: &mut dyn Transport,
    host: PlayerId,
    hello: Hello,
    wait: Duration,
) -> Result<Start, JoinError> {
    transport
        .send(Destination::Seat(host), &Message::RefereeHello(hello))
        .map_err(|e| JoinError::Transport(e.to_string()))?;
    let end = Instant::now() + wait;
    loop {
        match transport.poll() {
            Ok(Some(Incoming { from, msg })) if from == host => match msg {
                Message::Start(s) => return Ok(s),
                Message::Refuse(r) => return Err(JoinError::Refused(r)),
                _ => {}
            },
            Ok(Some(_)) | Err(crate::lockstep::TransportError::Frame(_)) => {}
            Ok(None) => {
                if Instant::now() > end {
                    return Err(JoinError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => return Err(JoinError::Transport(e.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

/// What the referee concluded about one seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingKind {
    /// The seat's state differs from the referee's in these sections.
    Diverged { sections: Vec<&'static str> },
    /// The seat's digest differs from the referee's but its section digests
    /// are the referee's: the digest it reported is not its sim's.
    MisreportedDigest,
    /// The seat's section digests did not arrive (yet).
    SectionsUnknown,
}

/// A seat whose digest differs from the referee's, at the first tick it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub seat: PlayerId,
    pub is_host: bool,
    pub tick: Tick,
    /// The digest the seat reported.
    pub reported: u64,
    /// The referee's own at the same tick.
    pub reference: u64,
    pub kind: FindingKind,
}

impl Finding {
    /// One line for the sidecar and the log.
    pub fn line(&self) -> String {
        let who = if self.is_host {
            format!("seat {} (the host)", self.seat.0)
        } else {
            format!("seat {}", self.seat.0)
        };
        match &self.kind {
            FindingKind::Diverged { sections } => format!(
                "verdict: {who} diverged at tick {}, sections: {}",
                self.tick,
                sections.join(", ")
            ),
            FindingKind::MisreportedDigest => format!(
                "verdict: {who} reported {:#018x} at tick {}, the referee's is {:#018x}, and its section digests equal the referee's: the digest it reported is not its sim's",
                self.reported, self.tick, self.reference
            ),
            FindingKind::SectionsUnknown => format!(
                "verdict: {who} diverged at tick {} (reported {:#018x}, the referee's is {:#018x}); its section digests did not arrive",
                self.tick, self.reported, self.reference
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The referee
// ---------------------------------------------------------------------------

/// When a referee gives up (and writes its reports). Defaults are the
/// production setting; tests shorten them.
#[derive(Debug, Clone)]
pub struct RefereeLimits {
    /// No tick run and nothing new for this long: the match is over.
    pub idle: Duration,
    /// Ticks run, as a backstop (30 Hz for 6 hours by default).
    pub max_ticks: Tick,
    /// Commands in the referee's replay: the one thing that grows with the
    /// match, and with what a hostile seat sends.
    pub max_logged_commands: usize,
    /// How long a desync claim waits for the digests that settle it.
    pub claim_wait: Duration,
    /// Least time between two releases of peers stopped by a false claim: a
    /// seat that sends claims in a flood costs the room one frame per
    /// interval, not one per claim.
    pub release_interval: Duration,
    /// After the first divergence: how long before the artefacts are
    /// written, so the section digests have arrived.
    pub report_delay: Duration,
}

impl Default for RefereeLimits {
    fn default() -> RefereeLimits {
        RefereeLimits {
            idle: Duration::from_secs(300),
            max_ticks: 30 * 3600 * 6,
            max_logged_commands: 1_000_000,
            claim_wait: Duration::from_secs(10),
            release_interval: Duration::from_millis(250),
            report_delay: Duration::from_secs(2),
        }
    }
}

/// Why [`Referee::step`] ended the referee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// The connection to the room ended.
    Closed,
    /// No progress for [`RefereeLimits::idle`].
    Idle,
    /// [`RefereeLimits::max_ticks`].
    TooLong,
    /// [`RefereeLimits::max_logged_commands`].
    LogFull,
}

/// What one [`Referee::step`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefereeStep {
    /// The sim advanced to this tick.
    Ran(Tick),
    /// Waiting: for the host to seat the referee, or for a record.
    Stalled,
    /// The referee is done; write the reports.
    Ended(EndReason),
}

/// A referee: a [`Peer`] on a spectator router, plus the verdicts and the
/// reports.
pub struct Referee {
    peer: Peer,
    host: PlayerId,
    limits: RefereeLimits,
    found: BTreeMap<PlayerId, Finding>,
    claims: Vec<(PlayerId, DesyncNotice, Instant)>,
    false_claims: BTreeMap<PlayerId, u32>,
    last_progress: Instant,
    last_release: Option<Instant>,
    first_found: Option<Instant>,
    report_written: bool,
    protocol_errors: u64,
    closed: bool,
}

impl Referee {
    /// A referee on the connection in slot `me` for the game `start`
    /// describes, from its own content and map. `me` is no seat of the game.
    /// It does nothing until the host has seated it (`Peer::seat_referee`).
    pub fn new(
        start: &Start,
        me: PlayerId,
        content: Arc<ContentDb>,
        map: Arc<MapDef>,
        transport: Box<dyn Transport>,
        policy: RefereePolicy,
        limits: RefereeLimits,
    ) -> Result<Referee, ProtoError> {
        let mut peer = Peer::spectator(start, me, content, map, transport)?;
        peer.set_referee_policy(policy);
        Ok(Referee {
            peer,
            host: start.host,
            limits,
            found: BTreeMap::new(),
            claims: Vec::new(),
            false_claims: BTreeMap::new(),
            last_progress: Instant::now(),
            last_release: None,
            first_found: None,
            report_written: false,
            protocol_errors: 0,
            closed: false,
        })
    }

    /// Replaces the limits.
    pub fn set_limits(&mut self, limits: RefereeLimits) {
        self.limits = limits;
    }

    /// The underlying peer (its sim, router, replay).
    pub fn peer(&self) -> &Peer {
        &self.peer
    }

    pub fn peer_mut(&mut self) -> &mut Peer {
        &mut self.peer
    }

    /// The seats that differ from the referee, earliest tick first.
    pub fn findings(&self) -> Vec<Finding> {
        let mut v: Vec<Finding> = self.found.values().cloned().collect();
        v.sort_by_key(|f| (f.tick, f.seat));
        v
    }

    /// The first diverging tick, seat and sections.
    pub fn first_finding(&self) -> Option<Finding> {
        self.findings().into_iter().next()
    }

    /// Desync claims by `seat` whose own digest contradicts the records it
    /// sent (a seat that lies about itself; one that repeats a liar's claim
    /// is not counted).
    pub fn false_claims(&self, seat: PlayerId) -> u32 {
        self.false_claims.get(&seat).copied().unwrap_or(0)
    }

    /// Messages that broke a rule (malformed, out of role, out of order).
    pub fn protocol_errors(&self) -> u64 {
        self.protocol_errors
    }

    /// Runs one tick if the room lets it, and reviews everything that
    /// arrived. Never panics on what a seat sends.
    pub fn step(&mut self) -> RefereeStep {
        self.step_at(Instant::now())
    }

    /// [`Referee::step`] at a given time (tests).
    pub fn step_at(&mut self, now: Instant) -> RefereeStep {
        let out = if self.peer.router().seated() {
            self.peer.step_at(now)
        } else {
            // Not seated yet: look at what arrives (the seating) and run
            // nothing.
            self.peer.router_mut().pump();
            StepOutcome::Stalled { waiting: Vec::new() }
        };
        self.review(now);
        if self.closed {
            return RefereeStep::Ended(EndReason::Closed);
        }
        if self.peer.tick() >= self.limits.max_ticks {
            return RefereeStep::Ended(EndReason::TooLong);
        }
        if self.peer.replay().commands.len() > self.limits.max_logged_commands {
            return RefereeStep::Ended(EndReason::LogFull);
        }
        match out {
            StepOutcome::Ran { tick, .. } => {
                self.last_progress = now;
                RefereeStep::Ran(tick)
            }
            _ => {
                if now.saturating_duration_since(self.last_progress) > self.limits.idle {
                    RefereeStep::Ended(EndReason::Idle)
                } else {
                    RefereeStep::Stalled
                }
            }
        }
    }

    /// Whether the artefacts are due: a divergence was found and
    /// [`RefereeLimits::report_delay`] has passed, and they have not been
    /// written yet.
    pub fn report_due(&self, now: Instant) -> bool {
        !self.report_written
            && self
                .first_found
                .is_some_and(|t| now.saturating_duration_since(t) >= self.limits.report_delay)
    }

    fn review(&mut self, now: Instant) {
        for e in self.peer.router_mut().take_events() {
            match e {
                RouterEvent::Protocol { .. } => self.protocol_errors += 1,
                RouterEvent::Transport(crate::lockstep::TransportError::Closed) => self.closed = true,
                _ => {}
            }
        }
        self.peer.take_resync_events();
        self.update_findings(now);
        self.review_claims(now);
    }

    fn update_findings(&mut self, now: Instant) {
        let (me, host) = (self.peer.router().me(), self.host);
        for d in self.peer.router().divergences() {
            let sections = self.peer.router().sections_at(d.theirs.tick);
            let mine = sections.iter().find(|(p, _)| *p == me).map(|(_, v)| v);
            let theirs = sections.iter().find(|(p, _)| *p == d.seat).map(|(_, v)| v);
            let kind = match (mine, theirs) {
                (Some(m), Some(t)) => {
                    let n = m.len().max(t.len());
                    let names: Vec<&'static str> = (0..n)
                        .filter(|&i| m.get(i) != t.get(i))
                        .map(|i| SectionDigests::NAMES.get(i).copied().unwrap_or("section"))
                        .collect();
                    if names.is_empty() {
                        FindingKind::MisreportedDigest
                    } else {
                        FindingKind::Diverged { sections: names }
                    }
                }
                _ => FindingKind::SectionsUnknown,
            };
            let f = Finding {
                seat: d.seat,
                is_host: d.seat == host,
                tick: d.theirs.tick,
                reported: d.theirs.digest,
                reference: d.reference,
                kind,
            };
            // Keep a resolved verdict (the router forgets old section
            // digests); replace an unresolved one or an earlier tick.
            let keep = self.found.get(&d.seat).is_some_and(|old| {
                old.tick < f.tick
                    || (old.tick == f.tick
                        && !matches!(old.kind, FindingKind::SectionsUnknown)
                        && matches!(f.kind, FindingKind::SectionsUnknown))
            });
            if !keep {
                self.found.insert(d.seat, f);
            }
            if self.first_found.is_none() {
                self.first_found = Some(now);
            }
        }
    }

    /// Checks each seat's pending `Desync` claim against the digests held
    /// (see the module docs, "Trust").
    fn review_claims(&mut self, now: Instant) {
        for (from, n) in self.peer.router_mut().take_claims() {
            self.claims.retain(|(s, _, _)| *s != from);
            if self.claims.len() < 16 {
                self.claims.push((from, n, now));
            }
        }
        let pending = std::mem::take(&mut self.claims);
        for (from, n, at) in pending {
            let expired = now.saturating_duration_since(at) > self.limits.claim_wait;
            let malformed = n.ours.tick != n.theirs.tick;
            // The referee may not have reached the claimed tick when the claim
            // arrived: note the claimer's report as soon as it can be compared.
            if !malformed {
                self.peer.router_mut().note_claimed(from, n.ours);
            }
            let tick = n.theirs.tick;
            let router = self.peer.router();
            let (have, missing) = router.digest_coverage(tick);
            let reference = router.own_digest(tick);
            let settled = reference.is_some() && missing.is_empty();
            // Where the claimer's own records contradict the digest it gives
            // as its own, it lied; where they do not (it sent no record for
            // that tick: it stopped), the claim is its report.
            let record = router.digests_at(n.ours.tick).into_iter().find(|(p, _)| *p == from).map(|(_, d)| d);
            let lied = malformed || record.is_some_and(|d| d != n.ours.digest);
            let claimer_differs = !lied && reference.is_some_and(|r| n.ours.digest != r);
            let any_differs = claimer_differs || reference.is_some_and(|r| have.iter().any(|(_, d)| *d != r));
            if !malformed && !settled && !claimer_differs {
                if !expired {
                    self.claims.push((from, n, at));
                    continue;
                }
                // Nothing came to settle it and nobody is known to differ:
                // the stop is for nothing.
                if !self.peer.router().divergences().is_empty() {
                    continue;
                }
            } else if any_differs && !malformed {
                // A real divergence: the normal path (the seat asks).
                continue;
            }
            if self
                .last_release
                .is_some_and(|t| now.saturating_duration_since(t) < self.limits.release_interval)
            {
                // Settled, but released at most once per interval: it waits.
                self.claims.push((from, n, at));
                continue;
            }
            self.last_release = Some(now);
            if lied {
                let count = self.false_claims.entry(from).or_default();
                *count = count.saturating_add(1);
            }
            let turn = self.peer.router().next_turn();
            let msg = Message::Resynced(Resynced {
                slot: from,
                turn,
                count: 0,
                last: true,
            });
            let _ = self.peer.router_mut().send_message(Destination::All, &msg);
        }
    }

    /// The sidecar: the peer's (`first differing tick`, every seat's digests
    /// and sections, the engines and seats) followed by the verdicts.
    pub fn sidecar(&self) -> String {
        let mut s = self.peer.desync_sidecar();
        s.push_str("referee: yes\n");
        for f in self.findings() {
            s.push_str(&f.line());
            s.push('\n');
        }
        for (seat, n) in &self.false_claims {
            s.push_str(&format!(
                "verdict: seat {} sent {n} desync claim(s) that the referee's digests contradict\n",
                seat.0
            ));
        }
        s
    }

    /// Writes `<stem>.star` (the referee's replay: the replay of record) and
    /// `<stem>.star.desync.txt` into `dir`, as `Peer::write_desync_report`
    /// does for a peer.
    pub fn write_report(&mut self, dir: &Path, stem: &str) -> std::io::Result<DesyncFiles> {
        self.report_written = true;
        let replay = dir.join(format!("{stem}.star"));
        let sidecar = dir.join(format!("{stem}.star.desync.txt"));
        std::fs::write(&replay, self.peer.replay().encode())?;
        std::fs::write(&sidecar, self.sidecar())?;
        Ok(DesyncFiles { replay, sidecar })
    }
}
