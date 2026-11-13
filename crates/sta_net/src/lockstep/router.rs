//! The lockstep [`CommandRouter`] (docs/NET.md, sections 1, 3, 6, 7, 8).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use sta_core::{PlayerId, Tick};
use sta_sim::command::{Command, CommandRouter};

use super::transport::{Destination, Incoming, Transport, TransportError};
use crate::proto::{
    MAX_SCHEDULE_PER_TURN, Action, DesyncNotice, DropChoice, Message, Occupant, ProtoError, Scheduled, SectionReport, Start, TickDigest,
    TurnCommands, TurnConfig,
};

/// Turns of digests a router keeps for comparison (docs/NET.md, 3.1).
pub const DIGESTS_KEPT: usize = 64;

/// Records one seat may have queued ahead of the turn this peer will run
/// next. An honest peer leads by at most its delay (30) plus ours (30) plus
/// the extended lead of a drop; a seat past this is flooding (NET-03: a
/// hostile peer must not grow the queue without bound).
pub const MAX_QUEUED_TURNS: usize = 128;

/// Unread events kept; later ones are dropped.
pub const MAX_EVENTS: usize = 1024;

/// Why a received message was not applied. The message's sender is faulted:
/// a real transport disconnects it, and the sim stalls on its seat until the
/// host drops it (docs/NET.md, sections 6 and 8).
#[derive(Debug, Clone, PartialEq)]
pub enum RouterError {
    /// Role, bounds or authority (`Message::check_sender`,
    /// `TurnCommands::authorize`, `Message::validate`).
    Proto(ProtoError),
    /// The sender holds no seat that sends turn records.
    UnknownSeat(PlayerId),
    /// A seat's turns must arrive in order without gaps and without repeats.
    OutOfOrder {
        slot: PlayerId,
        expected: u32,
        got: u32,
    },
    /// A player's `seq` must keep increasing across turns.
    SeqNotIncreasing {
        player: PlayerId,
        seq: u32,
        last: u32,
    },
    /// A record after the seat's `last_turn` (it was dropped), or from a
    /// seat already faulted.
    SeatClosed(PlayerId),
    /// The seat is more than [`MAX_QUEUED_TURNS`] records ahead of this peer.
    TooFarAhead(PlayerId),
}

impl std::fmt::Display for RouterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouterError::Proto(e) => write!(f, "{e}"),
            RouterError::UnknownSeat(s) => write!(f, "seat {} sends no turn records", s.0),
            RouterError::OutOfOrder {
                slot,
                expected,
                got,
            } => write!(f, "seat {}: turn {got}, expected {expected}", slot.0),
            RouterError::SeqNotIncreasing { player, seq, last } => {
                write!(f, "player {}: seq {seq} after {last}", player.0)
            }
            RouterError::SeatClosed(s) => write!(f, "seat {} is closed", s.0),
            RouterError::TooFarAhead(s) => {
                write!(f, "seat {} is too many turns ahead", s.0)
            }
        }
    }
}

impl std::error::Error for RouterError {}

/// Things a router noticed that the layers above show or act on.
#[derive(Debug, Clone, PartialEq)]
pub enum RouterEvent {
    /// A command submitted here was refused and not sent.
    LocalRejected {
        player: PlayerId,
        seq: u32,
        why: &'static str,
    },
    /// A message from `from` broke a rule; its seat is faulted.
    Protocol { from: PlayerId, error: RouterError },
    /// This peer found a digest that is not its own. The match halts.
    Desync(DesyncNotice),
    /// Another peer reported a desync. The match halts.
    PeerDesync { from: PlayerId, notice: DesyncNotice },
    /// A scheduled change took effect at `turn`.
    Scheduled { turn: u32, change: Scheduled },
    /// A seat was dropped: its records after `last_turn` do not exist.
    /// `then` says what must follow in the sim; elimination has no command
    /// yet (docs/NET.md, open question 9), so the layer above decides.
    SeatDropped {
        slot: PlayerId,
        last_turn: u32,
        then: DropChoice,
    },
    /// The transport failed.
    Transport(TransportError),
    /// `take_ready` was asked for a turn other than the next one.
    OutOfSequence { asked: u32, expected: u32 },
}

#[derive(Debug, Default)]
struct SeatState {
    /// The turn this seat's next record must be for.
    next_turn: u32,
    /// Received, not yet consumed; contiguous.
    queue: VecDeque<TurnCommands>,
    /// After a drop: the last turn this seat has a record for.
    last_turn: Option<u32>,
    /// Broke a rule; nothing more is accepted.
    faulted: bool,
}

impl SeatState {
    fn needed_for(&self, turn: u32) -> bool {
        self.last_turn.is_none_or(|l| turn <= l)
    }
}

/// The lockstep router of one peer.
///
/// Per turn it sends one record (docs/NET.md, 3.2: commands submitted during
/// local turn *T* go out as turn *T + delay*), and `take_ready` releases turn
/// *T* only when every seat that still sends has its record for *T*. The
/// driver ([`Peer`](super::Peer), or the game's tick loop) calls
/// [`LockstepRouter::set_digest`] at each turn's first tick, before
/// `take_ready`, so the digest travels with the next record.
pub struct LockstepRouter {
    me: PlayerId,
    host: PlayerId,
    cfg: TurnConfig,
    start_delay: u32,
    transport: Box<dyn Transport>,
    seats: BTreeMap<PlayerId, SeatState>,
    /// Seats the host speaks for: computer seats, widened by AI takeovers.
    host_speaks: Vec<PlayerId>,
    /// Local: commands waiting for the next record, last `seq` submitted.
    pending: Vec<Command>,
    last_submitted: BTreeMap<PlayerId, u32>,
    /// Host: changes queued for the next record.
    schedule: Vec<Scheduled>,
    /// The next turn this peer will send a record for.
    next_send: u32,
    flushed: Option<u32>,
    /// The next turn `take_ready` will release.
    next_turn: u32,
    delivered: Option<(u32, Vec<Command>)>,
    /// Remote: last `seq` accepted per (sender, player). Per sender, because
    /// different senders' frames may arrive in any order (after an AI
    /// takeover two senders speak for one player).
    seq_seen: BTreeMap<(PlayerId, PlayerId), u32>,
    // Pause: the turn whose record paused us, and Resumes not yet matched.
    paused_at: Option<u32>,
    resumed_for: Option<u32>,
    resumes: u32,
    // Digests.
    ours: BTreeMap<Tick, u64>,
    seen: BTreeMap<Tick, BTreeMap<PlayerId, u64>>,
    unchecked: Vec<(PlayerId, TickDigest)>,
    digest_to_send: Option<TickDigest>,
    desync: Option<DesyncNotice>,
    peer_desync: Option<(PlayerId, DesyncNotice)>,
    // Section digests (NET_DECISIONS N6): ours for the same turns as `ours`,
    // those peers sent after a mismatch (at most DIGESTS_KEPT turns per seat),
    // and the turns we already announced ours for.
    our_sections: BTreeMap<Tick, Vec<u64>>,
    their_sections: BTreeMap<PlayerId, BTreeMap<Tick, Vec<u64>>>,
    sections_sent: BTreeSet<Tick>,
    events: Vec<RouterEvent>,
}

impl LockstepRouter {
    /// The router of seat `me` for the game `start` describes. `me` must be a
    /// person's seat.
    pub fn new(
        start: &Start,
        me: PlayerId,
        transport: Box<dyn Transport>,
    ) -> Result<LockstepRouter, ProtoError> {
        let delay = u32::from(start.turn.delay_turns);
        let mut seats = BTreeMap::new();
        let mut host_speaks = Vec::new();
        for s in &start.seats {
            match &s.occupant {
                Occupant::Peer => {
                    seats.insert(
                        s.slot,
                        SeatState {
                            next_turn: delay,
                            ..SeatState::default()
                        },
                    );
                }
                Occupant::Ai(_) => host_speaks.push(s.slot),
                Occupant::Idle => {}
            }
        }
        if !seats.contains_key(&me) {
            return Err(ProtoError::Invalid("local seat is not a person's seat"));
        }
        if !seats.contains_key(&start.host) {
            return Err(ProtoError::Invalid("host is not a seated peer"));
        }
        if start.turn.ticks_per_turn == 0 || delay == 0 {
            return Err(ProtoError::Invalid("turn configuration"));
        }
        Ok(LockstepRouter {
            me,
            host: start.host,
            cfg: start.turn,
            start_delay: delay,
            transport,
            seats,
            host_speaks,
            pending: Vec::new(),
            last_submitted: BTreeMap::new(),
            schedule: Vec::new(),
            next_send: delay,
            flushed: None,
            next_turn: 0,
            delivered: None,
            seq_seen: BTreeMap::new(),
            paused_at: None,
            resumed_for: None,
            resumes: 0,
            ours: BTreeMap::new(),
            seen: BTreeMap::new(),
            unchecked: Vec::new(),
            digest_to_send: None,
            desync: None,
            peer_desync: None,
            our_sections: BTreeMap::new(),
            their_sections: BTreeMap::new(),
            sections_sent: BTreeSet::new(),
            events: Vec::new(),
        })
    }

    // ---- inspection -------------------------------------------------------

    /// This peer's seat.
    pub fn me(&self) -> PlayerId {
        self.me
    }

    /// Whether this peer is the host.
    pub fn is_host(&self) -> bool {
        self.me == self.host
    }

    /// The current turn timing; `delay_turns` and `speed_percent` follow the
    /// host's scheduled changes.
    pub fn config(&self) -> TurnConfig {
        self.cfg
    }

    /// The next turn that `take_ready` will release.
    pub fn next_turn(&self) -> u32 {
        self.next_turn
    }

    /// The seats the next turn waits on (for the stall dialog): those whose
    /// record for it is missing. Empty when nothing is missing.
    pub fn waiting_on(&self) -> Vec<PlayerId> {
        let turn = self.next_turn;
        if turn < self.start_delay {
            return Vec::new();
        }
        self.seats
            .iter()
            .filter(|(_, s)| s.needed_for(turn) && s.queue.front().map(|r| r.turn) != Some(turn))
            .map(|(&p, _)| p)
            .collect()
    }

    /// The last turn `slot` has a record for here (received, or sent if it
    /// is this seat), consumed or not. What the host reads for
    /// `Action::Drop::last_turn` once the relay has closed the seat.
    pub fn last_record(&self, slot: PlayerId) -> Option<u32> {
        let s = self.seats.get(&slot)?;
        (s.next_turn > self.start_delay).then(|| s.next_turn - 1)
    }

    /// The desync this peer found, the earliest turn if several.
    pub fn desync(&self) -> Option<&DesyncNotice> {
        self.desync.as_ref()
    }

    /// A desync another peer reported to us.
    pub fn peer_desync(&self) -> Option<&(PlayerId, DesyncNotice)> {
        self.peer_desync.as_ref()
    }

    /// Whether the match is stopped by a desync (docs/NET.md, section 7).
    pub fn halted(&self) -> bool {
        self.desync.is_some() || self.peer_desync.is_some()
    }

    /// Whether a scheduled pause holds the next turn.
    pub fn paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// Every digest known for `tick`, ours included, by seat. Kept for
    /// [`DIGESTS_KEPT`] turns.
    pub fn digests_at(&self, tick: Tick) -> Vec<(PlayerId, u64)> {
        self.seen
            .get(&tick)
            .map(|m| m.iter().map(|(&p, &d)| (p, d)).collect())
            .unwrap_or_default()
    }

    /// Every section digest list known for `tick`, ours included, by seat: ours
    /// from [`LockstepRouter::set_sections`], the others' from the
    /// [`Message::Sections`] they sent after a mismatch at `tick`. A list holds
    /// one digest per section, in the sim's section order.
    pub fn sections_at(&self, tick: Tick) -> Vec<(PlayerId, Vec<u64>)> {
        let mut all: BTreeMap<PlayerId, Vec<u64>> = BTreeMap::new();
        for (&seat, m) in &self.their_sections {
            if let Some(d) = m.get(&tick) {
                all.insert(seat, d.clone());
            }
        }
        if let Some(d) = self.our_sections.get(&tick) {
            all.insert(self.me, d.clone());
        }
        all.into_iter().collect()
    }

    /// Takes the events noticed since the last call.
    pub fn take_events(&mut self) -> Vec<RouterEvent> {
        std::mem::take(&mut self.events)
    }

    // ---- driver input -----------------------------------------------------

    /// This peer's section digests at `tick` (`SectionDigests::values`), kept
    /// for the same turns as the digest. Call it with the digest, before
    /// [`LockstepRouter::set_digest`], so a mismatch found there can send them.
    pub fn set_sections(&mut self, tick: Tick, digests: Vec<u64>) {
        self.our_sections.insert(tick, digests);
        while self.our_sections.len() > DIGESTS_KEPT {
            self.our_sections.pop_first();
        }
        let floor = self.our_sections.keys().next().copied().unwrap_or(0);
        self.sections_sent = self.sections_sent.split_off(&floor);
    }

    /// Sends our section digests for `tick` to everyone, once.
    fn announce_sections(&mut self, tick: Tick) {
        if !self.sections_sent.insert(tick) {
            return;
        }
        if let Some(digests) = self.our_sections.get(&tick).cloned() {
            self.send(Destination::All, &Message::Sections(SectionReport { tick, digests }));
        }
    }

    /// This peer's sim digest at `tick`, the first tick of a turn, before its
    /// commands apply. It is compared with every report and rides in the
    /// next record sent.
    pub fn set_digest(&mut self, tick: Tick, digest: u64) {
        if self.ours.insert(tick, digest).is_some() {
            return;
        }
        self.seen.entry(tick).or_default().insert(self.me, digest);
        self.digest_to_send = Some(TickDigest { tick, digest });
        while self.ours.len() > DIGESTS_KEPT {
            let Some(&oldest) = self.ours.keys().next() else {
                break;
            };
            self.ours.remove(&oldest);
            self.seen.remove(&oldest);
        }
        let floor = self.ours.keys().next().copied().unwrap_or(0);
        let waiting = std::mem::take(&mut self.unchecked);
        for (slot, theirs) in waiting {
            if theirs.tick == tick {
                self.compare(slot, theirs);
            } else if theirs.tick >= floor {
                self.unchecked.push((slot, theirs));
            }
        }
    }

    /// Host: puts `action` in the next record, effective at that turn.
    pub fn schedule(&mut self, action: Action) {
        if self.is_host() {
            if let Action::Drop {
                slot,
                then: DropChoice::AiTakeover(_),
                ..
            } = &action
                && !self.host_speaks.contains(slot)
            {
                self.host_speaks.push(*slot);
            }
            self.schedule.push(Scheduled {
                by: self.me,
                action,
            });
        }
    }

    /// Non-host: asks the host to schedule `action` (pause, speed, delay).
    /// On the host it is the same as [`LockstepRouter::schedule`].
    pub fn request(&mut self, action: Action) {
        if self.is_host() {
            self.schedule(action);
        } else {
            self.send(Destination::Seat(self.host), &Message::Request(action));
        }
    }

    /// Call it once per drop: every call leads one more turn, and a seat
    /// more than `MAX_QUEUED_TURNS` ahead is faulted by its receivers.
    ///
    /// Sends the next record now, a turn beyond the usual lead (a sender's
    /// lead may be any size, docs/NET.md, 3.2). A host stalled on a missing
    /// seat uses it to put an `Action::Drop` where its peers can read it
    /// without the host advancing.
    pub fn extend_lead(&mut self) {
        if self.halted() {
            return;
        }
        let target = self.next_send;
        self.pending.sort_by_key(Command::sort_key);
        let flushed = self.flushed.take();
        // `flush` sends up to `turn + delay`; ask for one more turn.
        let turn = target.saturating_sub(u32::from(self.cfg.delay_turns));
        self.flush(turn);
        self.flushed = flushed;
    }

    /// Host: resumes a paused game, effective on receipt.
    pub fn resume(&mut self) {
        if self.is_host() && self.paused_at.is_some() {
            self.resumes += 1;
            self.send(Destination::All, &Message::Resume { by: self.me });
        }
    }

    /// Records an event, up to [`MAX_EVENTS`] unread ones: a hostile peer
    /// that provokes errors cannot grow the list without bound.
    fn note(&mut self, e: RouterEvent) {
        // Only error noise is capped; state changes (drops, schedules,
        // desyncs) are never lost to a flood of junk.
        let noise = matches!(e, RouterEvent::Protocol { .. }
                | RouterEvent::Transport(_)
                | RouterEvent::PeerDesync { .. }
                | RouterEvent::LocalRejected { .. }
        );
        if !noise || self.events.len() < MAX_EVENTS {
            self.events.push(e);
        }
    }

    // ---- sending ----------------------------------------------------------

    fn send(&mut self, to: Destination, msg: &Message) {
        if let Err(e) = self.transport.send(to, msg) {
            self.note(RouterEvent::Transport(e));
        }
    }

    fn host_speaks_for(&self, p: PlayerId) -> bool {
        p == PlayerId::SCRIPT || self.host_speaks.contains(&p)
    }

    fn reject(&mut self, c: &Command, why: &'static str) {
        self.note(RouterEvent::LocalRejected {
            player: c.player,
            seq: c.seq,
            why,
        });
    }

    /// Sends this turn's record, and empty ones for any turns between the
    /// last sent and the new lead (docs/NET.md, 3.2). Once per turn.
    fn flush(&mut self, turn: u32) {
        if self.flushed.is_some_and(|f| f >= turn) {
            return;
        }
        self.flushed = Some(turn);
        let target = turn.saturating_add(u32::from(self.cfg.delay_turns));
        self.pending.sort_by_key(Command::sort_key);
        while self.next_send <= target {
            let t = self.next_send;
            self.next_send += 1;
            let last = t == target;
            // At most MAX_SCHEDULE_PER_TURN go in one record (the wire bound);
            // the rest wait for the next one.
            let schedule = if last {
                let n = self.schedule.len().min(MAX_SCHEDULE_PER_TURN);
                self.schedule.drain(..n).collect()
            } else {
                Vec::new()
            };
            let digest = self.digest_to_send.take();
            let mut none = Vec::new();
            let pending = if last { &mut self.pending } else { &mut none };
            let (rec, too_big) = TurnCommands::fill(t, self.me, schedule, digest, pending);
            for c in &too_big {
                self.reject(c, "command too large for a frame");
            }
            self.send(Destination::All, &Message::Turn(rec.clone()));
            if self.is_host() {
                self.note_drops(&rec);
            }
            if let Some(s) = self.seats.get_mut(&self.me) {
                s.queue.push_back(rec);
                s.next_turn = t + 1;
            }
        }
    }

    // ---- receiving --------------------------------------------------------

    fn pump(&mut self) {
        loop {
            match self.transport.poll() {
                Ok(Some(Incoming { from, msg })) => {
                    if let Err(error) = self.receive(from, msg) {
                        self.note(RouterEvent::Protocol { from, error });
                    }
                }
                Ok(None) => break,
                Err(TransportError::Closed) => {
                    self.note(RouterEvent::Transport(TransportError::Closed));
                    break;
                }
                Err(e) => self.note(RouterEvent::Transport(e)),
            }
        }
    }

    /// Handles one message as stamped by the transport. Messages that are
    /// not the lockstep stream's (lobby, chat, pings) are left to the layers
    /// above and ignored here.
    pub fn receive(&mut self, from: PlayerId, msg: Message) -> Result<(), RouterError> {
        msg.validate().map_err(RouterError::Proto)?;
        msg.check_sender(from == self.host)
            .map_err(RouterError::Proto)?;
        if !self.seats.contains_key(&from) {
            return Err(RouterError::UnknownSeat(from));
        }
        match msg {
            Message::Turn(rec) => self.receive_turn(from, rec),
            Message::Desync(notice) => {
                // The reporter halted before it could send its own digest
                // for that turn; the notice carries both. Compare it as if
                // it had arrived in a record, and keep both for the sidecar.
                self.compare(from, notice.ours);
                if notice.slot != self.me && self.ours.contains_key(&notice.theirs.tick) {
                    self.seen
                        .entry(notice.theirs.tick)
                        .or_default()
                        .entry(notice.slot)
                        .or_insert(notice.theirs.digest);
                }
                self.note(RouterEvent::PeerDesync { from, notice });
                self.peer_desync.get_or_insert((from, notice));
                Ok(())
            }
            Message::Sections(report) => {
                let tpt = Tick::from(self.cfg.ticks_per_turn.max(1));
                if !report.tick.is_multiple_of(tpt) {
                    return Err(RouterError::Proto(ProtoError::Invalid(
                        "section report off a turn boundary",
                    )));
                }
                // A hostile peer cannot grow this: a bounded list per turn,
                // DIGESTS_KEPT turns per seat, oldest first out.
                let kept = self.their_sections.entry(from).or_default();
                kept.insert(report.tick, report.digests);
                while kept.len() > DIGESTS_KEPT {
                    kept.pop_first();
                }
                Ok(())
            }
            Message::Resume { .. } => {
                self.resumes += 1;
                Ok(())
            }
            Message::Request(action) => {
                if self.is_host() {
                    // A peer cannot grow the queue without bound.
                    if self.schedule.len() < 4 * MAX_SCHEDULE_PER_TURN {
                        self.schedule.push(Scheduled { by: from, action });
                    }
                    Ok(())
                } else {
                    Err(RouterError::Proto(ProtoError::Authority(
                        "request sent to a non-host",
                    )))
                }
            }
            _ => Ok(()),
        }
    }

    fn receive_turn(&mut self, from: PlayerId, rec: TurnCommands) -> Result<(), RouterError> {
        let result = self.check_turn(from, &rec);
        match result {
            Ok(()) => {
                // A takeover widens the host's authority from its record on,
                // before the commands that rely on it are checked.
                if let Some(s) = self.seats.get_mut(&from) {
                    s.next_turn = rec.turn + 1;
                    s.queue.push_back(rec.clone());
                }
                if from == self.host {
                    self.note_drops(&rec);
                }
                for c in &rec.commands {
                    self.seq_seen.insert((from, c.player), c.seq);
                }
                if let Some(d) = rec.digest {
                    self.compare(from, d);
                }
                Ok(())
            }
            Err(e) => {
                if let Some(s) = self.seats.get_mut(&from) {
                    s.faulted = true;
                }
                Err(e)
            }
        }
    }

    fn check_turn(&mut self, from: PlayerId, rec: &TurnCommands) -> Result<(), RouterError> {
        let seat = self.seats.get(&from).ok_or(RouterError::UnknownSeat(from))?;
        if seat.faulted || seat.last_turn.is_some_and(|l| rec.turn > l) {
            return Err(RouterError::SeatClosed(from));
        }
        if seat.queue.len() >= MAX_QUEUED_TURNS {
            return Err(RouterError::TooFarAhead(from));
        }
        if rec.turn != seat.next_turn {
            return Err(RouterError::OutOfOrder {
                slot: from,
                expected: seat.next_turn,
                got: rec.turn,
            });
        }
        if from == self.host {
            for s in &rec.schedule {
                if let Action::Drop {
                    slot,
                    then: DropChoice::AiTakeover(_),
                    ..
                } = &s.action
                    && !self.host_speaks.contains(slot)
                {
                    self.host_speaks.push(*slot);
                }
            }
        }
        let speaks = |p| self.host_speaks_for(p);
        rec.authorize(from, from == self.host, speaks)
            .map_err(RouterError::Proto)?;
        for c in &rec.commands {
            if let Some(&last) = self.seq_seen.get(&(from, c.player))
                && c.seq <= last
            {
                return Err(RouterError::SeqNotIncreasing {
                    player: c.player,
                    seq: c.seq,
                    last,
                });
            }
        }
        Ok(())
    }

    // ---- digests ----------------------------------------------------------

    fn compare(&mut self, slot: PlayerId, theirs: TickDigest) {
        // Only ticks this peer has digests for are kept, so a hostile peer
        // cannot grow the map by naming ticks.
        if self.ours.contains_key(&theirs.tick) {
            self.seen
                .entry(theirs.tick)
                .or_default()
                .insert(slot, theirs.digest);
        }
        match self.ours.get(&theirs.tick) {
            Some(&d) if d != theirs.digest => {
                let notice = DesyncNotice {
                    slot,
                    ours: TickDigest {
                        tick: theirs.tick,
                        digest: d,
                    },
                    theirs,
                };
                let first = self.desync.is_none();
                self.announce_sections(theirs.tick);
                if self.desync.is_none_or(|n| theirs.tick < n.theirs.tick) {
                    self.desync = Some(notice);
                }
                if first {
                    self.note(RouterEvent::Desync(notice));
                    self.send(Destination::All, &Message::Desync(notice));
                }
            }
            Some(_) => {}
            None => {
                let newest = self.ours.keys().next_back().copied();
                if newest.is_none_or(|n| theirs.tick > n) {
                    // Bounded: at most a window of ticks for every seat.
                    // and per seat, so one flooder evicts only its own.
                    let mine = self.unchecked.iter().filter(|(s, _)| *s == slot).count();
                    if mine >= DIGESTS_KEPT
                        && let Some(i) = self.unchecked.iter().position(|(s, _)| *s == slot)
                    {
                        self.unchecked.remove(i);
                    }
                    self.unchecked.push((slot, theirs));
                }
            }
        }
    }

    // ---- releasing turns --------------------------------------------------

    /// Notes the drops in a host record as soon as it is here, not when its
    /// turn comes: a peer stalled on the dropped seat can only read the
    /// record that releases it from the queue ahead (docs/NET.md, section 8).
    /// Safe because `last_turn` is final: every record anyone ran precedes
    /// the departure marker the host waited for.
    fn note_drops(&mut self, rec: &TurnCommands) {
        for s in &rec.schedule {
            if let Action::Drop {
                slot,
                last_turn,
                then,
            } = &s.action
            {
                if *slot == self.host {
                    continue;
                }
                if let DropChoice::AiTakeover(_) = then
                    && !self.host_speaks.contains(slot)
                {
                    self.host_speaks.push(*slot);
                }
                if let Some(seat) = self.seats.get_mut(slot)
                    && seat.last_turn.is_none()
                {
                    seat.last_turn = Some(*last_turn);
                    seat.queue.retain(|r| r.turn <= *last_turn);
                }
            }
        }
    }
}

impl CommandRouter for LockstepRouter {
    fn submit(&mut self, cmd: Command) {
        let is_host = self.is_host();
        let probe = TurnCommands {
            turn: 0,
            slot: self.me,
            commands: vec![cmd.clone()],
            schedule: Vec::new(),
            digest: None,
        };
        if let Err(e) = probe.authorize(self.me, is_host, |p| self.host_speaks_for(p)) {
            let why = match e {
                ProtoError::Authority(w) => w,
                _ => "not allowed",
            };
            return self.reject(&cmd, why);
        }
        if Message::Turn(probe).validate().is_err() {
            return self.reject(&cmd, "invalid command");
        }
        // Also past anything seen from the seat's own peer (an AI takeover
        // continues the seat's seq, docs/NET.md section 6).
        let seen = self
            .seq_seen
            .iter()
            .filter(|((_, p), _)| *p == cmd.player)
            .map(|(_, &q)| q)
            .max();
        if self
            .last_submitted
            .get(&cmd.player)
            .copied()
            .into_iter()
            .chain(seen)
            .any(|last| cmd.seq <= last)
        {
            return self.reject(&cmd, "seq does not increase");
        }
        self.last_submitted.insert(cmd.player, cmd.seq);
        self.pending.push(cmd);
    }

    fn take_ready(&mut self, tick: Tick) -> Option<Vec<Command>> {
        self.pump();
        if self.halted() {
            return None;
        }
        let tpt = Tick::from(self.cfg.ticks_per_turn);
        if !tick.is_multiple_of(tpt) {
            return Some(Vec::new());
        }
        let turn = u32::try_from(tick / tpt).unwrap_or(u32::MAX);
        if let Some((t, cmds)) = &self.delivered
            && *t == turn
        {
            return Some(cmds.clone());
        }
        if turn != self.next_turn {
            self.note(RouterEvent::OutOfSequence {
                asked: turn,
                expected: self.next_turn,
            });
            return None;
        }
        self.flush(turn);

        let mut released: Vec<TurnCommands> = Vec::new();
        if turn >= self.start_delay {
            // The host's record first: it says who still sends (drops) and
            // whether the turn is held (pause).
            let host_rec = self
                .seats
                .get(&self.host)
                .and_then(|s| s.queue.front())
                .filter(|r| r.turn == turn)
                .cloned();
            let host_rec = host_rec?;
            if !self.waiting_on().is_empty() {
                return None;
            }
            if host_rec
                .schedule
                .iter()
                .any(|s| matches!(s.action, Action::Pause))
                && self.resumed_for != Some(turn)
            {
                if self.resumes > 0 {
                    self.resumes -= 1;
                    self.resumed_for = Some(turn);
                    self.paused_at = None;
                } else {
                    self.paused_at = Some(turn);
                    return None;
                }
            }
            for seat in self.seats.values_mut() {
                if seat.needed_for(turn)
                    && let Some(r) = seat.queue.pop_front()
                {
                    released.push(r);
                }
            }
            for s in &host_rec.schedule {
                self.apply(turn, s);
            }
        }
        let mut cmds: Vec<Command> = released.into_iter().flat_map(|r| r.commands).collect();
        cmds.sort_by_key(Command::sort_key);
        self.next_turn = turn + 1;
        self.delivered = Some((turn, cmds.clone()));
        Some(cmds)
    }
}

impl LockstepRouter {
    fn apply(&mut self, turn: u32, s: &Scheduled) {
        match &s.action {
            Action::Pause => {}
            Action::SetDelay(d) => self.cfg.delay_turns = *d,
            Action::SetSpeed(p) => self.cfg.speed_percent = *p,
            Action::Drop {
                slot,
                last_turn,
                then,
            } => self.note(RouterEvent::SeatDropped {
                slot: *slot,
                last_turn: *last_turn,
                then: then.clone(),
            }),
        }
        self.note(RouterEvent::Scheduled {
            turn,
            change: s.clone(),
        });
    }
}
